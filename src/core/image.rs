use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
    time::SystemTime,
};

use super::{
    job::{self, Progress},
    media::Media,
    operation::{Format, Plan},
};

const MAX_DIMENSION: u32 = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dimensions(pub u32, pub u32);

impl Dimensions {
    fn scaled(self, factor: f64) -> Self {
        Self(
            ((self.0 as f64 * factor).round() as u32).max(1),
            ((self.1 as f64 * factor).round() as u32).max(1),
        )
    }
}

impl std::fmt::Display for Dimensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} × {}", self.0, self.1)
    }
}

#[derive(Clone, Copy)]
pub enum Resize {
    Original,
    Width(u32),
    Height(u32),
    Exact(u32, u32),
    Percent(u32),
    Max(u32),
}

impl Resize {
    pub fn dimensions(self, original: Dimensions) -> Result<Dimensions, &'static str> {
        if original.0 == 0 || original.1 == 0 {
            return Err("The source dimensions are unavailable.");
        }
        let result = match self {
            Self::Original => return Ok(original),
            Self::Width(n) => original.scaled(n as f64 / original.0 as f64),
            Self::Height(n) => original.scaled(n as f64 / original.1 as f64),
            Self::Exact(w, h) => Dimensions(w.min(original.0), h.min(original.1)),
            Self::Percent(n) => original.scaled(n as f64 / 100.0),
            Self::Max(n) => original.scaled(n as f64 / original.0.max(original.1) as f64),
        };
        let result = if result.0 > original.0 || result.1 > original.1 {
            original
        } else {
            result
        };
        if result.0 == 0 || result.1 == 0 || result.0 > MAX_DIMENSION || result.1 > MAX_DIMENSION {
            return Err("The output dimensions must be between 1 and 16,384 pixels.");
        }
        Ok(result)
    }
}

pub fn parse_target(text: &str) -> Result<Option<u64>, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let number_end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let value = text[..number_end]
        .parse::<f64>()
        .map_err(|_| "Enter a valid file size, like 100 KB or 1 MB.")?;
    let multiplier = match text[number_end..].trim().to_ascii_lowercase().as_str() {
        "" | "kb" => 1000.0,
        "b" => 1.0,
        "mb" => 1_000_000.0,
        "kib" => 1024.0,
        "mib" => 1_048_576.0,
        _ => return Err("Use B, KB, MB, KiB, or MiB for the target size."),
    };
    let bytes = (value * multiplier).floor();
    if !(1.0..=1_000_000_000.0).contains(&bytes) {
        return Err("Enter a target size greater than zero and no larger than 1 GB.");
    }
    Ok(Some(bytes as u64))
}

pub fn saving_percent(original: u64, output: u64) -> Option<f64> {
    (original != 0).then(|| (1.0 - output as f64 / original as f64) * 100.0)
}

pub fn alpha_warning(media: &Media, format: Format) -> bool {
    format == Format::Jpeg
        && media
            .video
            .as_ref()
            .is_some_and(|stream| stream.has_alpha())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub source: PathBuf,
    pub stream: u32,
    pub format: Format,
    pub original: Dimensions,
    pub dimensions: Dimensions,
    pub quality: u8,
    pub lossless: bool,
    pub target: Option<u64>,
    pub shrink: bool,
    pub unchanged_png: bool,
}

impl Request {
    pub fn plan(&self, dimensions: Dimensions, quality: u8) -> Plan {
        let mut args: Vec<OsString> = ["-protocol_whitelist", "file", "-i"]
            .into_iter()
            .map(Into::into)
            .collect();
        args.push(self.source.as_os_str().into());
        args.extend([
            "-map".into(),
            format!("0:{}", self.stream).into(),
            "-an".into(),
            "-frames:v".into(),
            "1".into(),
        ]);
        if dimensions != self.original {
            args.extend([
                "-vf".into(),
                format!(
                    "scale={}:{}:flags=lanczos,setsar=1",
                    dimensions.0, dimensions.1
                )
                .into(),
            ]);
        }
        match self.format {
            Format::Png => {
                args.extend(["-c:v", "png", "-update", "1", "-f", "image2"].map(OsString::from))
            }
            Format::Jpeg => {
                let quantizer = jpeg_quantizer(quality);
                args.extend(
                    [
                        "-c:v",
                        "mjpeg",
                        "-q:v",
                        &quantizer.to_string(),
                        "-update",
                        "1",
                        "-f",
                        "image2",
                    ]
                    .map(OsString::from),
                );
            }
            Format::Webp => args.extend(
                [
                    "-c:v",
                    "libwebp",
                    "-lossless",
                    if self.lossless { "1" } else { "0" },
                    "-quality",
                    &quality.to_string(),
                    "-f",
                    "webp",
                ]
                .map(OsString::from),
            ),
            _ => unreachable!("image requests require an image format"),
        }
        Plan {
            args,
            duration: None,
            format: self.format,
        }
    }

    pub fn copies_source(&self) -> bool {
        self.unchanged_png && self.original == self.dimensions
    }
}

fn jpeg_quantizer(quality: u8) -> u8 {
    2 + (((100 - quality.min(100)) as u16 * 29 + 50) / 100) as u8
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceStamp {
    bytes: u64,
    modified: SystemTime,
}

fn stamp(path: &Path) -> Option<SourceStamp> {
    let metadata = fs::metadata(path).ok()?;
    Some(SourceStamp {
        bytes: metadata.len(),
        modified: metadata.modified().ok()?,
    })
}

pub struct Artifact {
    pub request: Request,
    pub file: tempfile::TempPath,
    pub bytes: u64,
    pub dimensions: Dimensions,
    pub quality: u8,
    source_stamp: Option<SourceStamp>,
    tool: PathBuf,
}

impl Artifact {
    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.file
    }

    pub fn matches(&self, request: &Request, tool: &Path) -> bool {
        self.request == *request
            && self.tool == tool
            && self.source_stamp.is_some()
            && self.source_stamp == stamp(&request.source)
    }
}

pub fn render(
    tool: &Path,
    request: Request,
    cancel: &Receiver<()>,
    update: impl Fn(Progress),
) -> Result<Option<Artifact>, String> {
    let source_stamp = stamp(&request.source);
    let temp = || {
        tempfile::Builder::new()
            .suffix(&format!(".{}", request.format.extension()))
            .tempfile()
            .map(|file| file.into_temp_path())
            .map_err(|e| format!("Could not create a temporary preview: {e}"))
    };
    let mut attempt = |dimensions, quality| -> Result<Option<(tempfile::TempPath, u64)>, String> {
        if job::cancelled(cancel) {
            return Ok(None);
        }
        let output = temp()?;
        let bytes = if request.copies_source() {
            fs::copy(&request.source, &output)
                .map_err(|e| format!("Could not copy the image: {e}"))?
        } else {
            if !job::encode(
                tool,
                &request.plan(dimensions, quality),
                &output,
                cancel,
                &update,
            )? {
                return Ok(None);
            }
            fs::metadata(&output)
                .map_err(|e| format!("Could not measure the image: {e}"))?
                .len()
        };
        Ok(Some((output, bytes)))
    };

    let candidate = if let Some(target) = request.target {
        search(&request, target, cancel, &mut attempt)?
    } else {
        attempt(request.dimensions, request.quality)?
            .map(|(path, bytes)| (path, bytes, request.dimensions, request.quality))
    };
    if job::cancelled(cancel) {
        return Ok(None);
    }
    if source_stamp.is_some() && stamp(&request.source) != source_stamp {
        return Err("The source changed during processing. Reopen it and try again.".into());
    }
    Ok(
        candidate.map(|(file, bytes, dimensions, quality)| Artifact {
            request,
            file,
            bytes,
            dimensions,
            quality,
            source_stamp,
            tool: tool.to_path_buf(),
        }),
    )
}

type Trial = (tempfile::TempPath, u64);
type Candidate = (tempfile::TempPath, u64, Dimensions, u8);

fn search(
    request: &Request,
    target: u64,
    cancel: &Receiver<()>,
    attempt: &mut impl FnMut(Dimensions, u8) -> Result<Option<Trial>, String>,
) -> Result<Option<Candidate>, String> {
    let scales: &[f64] = if request.shrink {
        &[1.0, 0.75, 0.5, 0.25]
    } else {
        &[1.0]
    };
    let mut qualities: Vec<u8> = (0..=100).rev().collect();
    if request.format == Format::Jpeg {
        // Multiple UI quality values map to the same JPEG quantizer.
        qualities.dedup_by_key(|quality| jpeg_quantizer(*quality));
    } else {
        qualities.retain(|quality| quality.is_multiple_of(10));
    }
    let mut smallest = u64::MAX;
    let mut previous = None;
    for &scale in scales {
        let dimensions = request.dimensions.scaled(scale);
        if previous == Some(dimensions) {
            continue;
        }
        previous = Some(dimensions);
        let mut best = None;
        for &quality in &qualities {
            let Some((file, bytes)) = attempt(dimensions, quality)? else {
                return Ok(None);
            };
            smallest = smallest.min(bytes);
            if bytes <= target {
                best = Some((file, bytes, dimensions, quality));
                break;
            }
        }
        if let Some(mut best) = best {
            if request.format == Format::Jpeg {
                return Ok(Some(best));
            }
            // Refine the best coarse interval, retaining measured candidates even when sizes fluctuate.
            for quality in ((best.3 + 1)..=(best.3.saturating_add(9).min(100))).rev() {
                let Some((file, bytes)) = attempt(dimensions, quality)? else {
                    return Ok(None);
                };
                if bytes <= target {
                    best = (file, bytes, dimensions, quality);
                    break;
                }
            }
            return Ok(Some(best));
        }
    }
    if job::cancelled(cancel) {
        return Ok(None);
    }
    Err(format!(
        "Target not reached. The smallest trial was {} bytes. Increase the target, reduce the dimensions, or try another format.",
        smallest
    ))
}

pub fn save(
    artifact: &Artifact,
    destination: &Path,
    cancel: &Receiver<()>,
) -> Result<bool, String> {
    job::validate(&artifact.request.source, destination)?;
    let parent = destination.parent().ok_or("Choose an output folder.")?;
    let temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("Could not write to the output folder: {e}"))?
        .into_temp_path();
    fs::copy(&artifact.file, &temp).map_err(|e| format!("Could not write the export: {e}"))?;
    if job::cancelled(cancel) {
        return Ok(false);
    }
    job::publish(temp, destination)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_targets_dimensions_and_signed_savings() {
        assert_eq!(parse_target("1.5 MB"), Ok(Some(1_500_000)));
        assert_eq!(parse_target("100"), Ok(Some(100_000)));
        for value in ["0 KB", "-2 MB", "NaN", "1 GB", "2e3 KB"] {
            assert!(parse_target(value).is_err());
        }
        assert_eq!(
            Resize::Max(100).dimensions(Dimensions(300, 200)).unwrap(),
            Dimensions(100, 67)
        );
        assert_eq!(
            Resize::Percent(200)
                .dimensions(Dimensions(300, 200))
                .unwrap(),
            Dimensions(300, 200)
        );
        assert_eq!(saving_percent(100, 125), Some(-25.0));
        assert_eq!(saving_percent(0, 125), None);
    }
}
