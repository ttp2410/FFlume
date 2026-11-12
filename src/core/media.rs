use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::Deserialize;

use super::ffmpeg;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Video,
    Audio,
    Image,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Image => "Image",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Stream {
    pub index: u32,
    codec_type: Option<String>,
    #[serde(default)]
    pub codec_name: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub channels: Option<u32>,
    #[serde(rename = "pix_fmt")]
    pub pixel_format: Option<String>,
    sample_rate: Option<String>,
    bit_rate: Option<String>,
    #[serde(default)]
    side_data_list: Vec<SideData>,
    duration: Option<String>,
    #[serde(default)]
    disposition: Disposition,
}

impl Stream {
    pub fn duration(&self) -> Option<f64> {
        positive(self.duration.as_deref())
    }
}

#[derive(Debug, Deserialize)]
struct SideData {
    rotation: Option<i32>,
}

impl Stream {
    pub fn bit_rate(&self) -> Option<u64> {
        self.bit_rate
            .as_deref()?
            .parse::<u64>()
            .ok()
            .filter(|&rate| rate > 0)
    }

    pub fn sample_rate(&self) -> Option<u32> {
        self.sample_rate.as_deref()?.parse().ok()
    }

    pub fn dimensions(&self) -> Option<(u32, u32)> {
        let (width, height) = (self.width?, self.height?);
        if self
            .side_data_list
            .iter()
            .filter_map(|s| s.rotation)
            .any(|angle| angle.rem_euclid(180) == 90)
        {
            Some((height, width))
        } else {
            Some((width, height))
        }
    }

    pub fn has_alpha(&self) -> bool {
        self.pixel_format.as_deref().is_some_and(|pixel| {
            pixel.starts_with("yuva")
                || pixel.starts_with("gbrap")
                || pixel.starts_with("rgba")
                || pixel.starts_with("bgra")
                || pixel.starts_with("ya")
                || matches!(pixel, "argb" | "abgr" | "pal8")
        })
    }
}

#[derive(Debug, Default, Deserialize)]
struct Disposition {
    #[serde(default)]
    attached_pic: u8,
}

#[derive(Default, Deserialize)]
struct Format {
    #[serde(default)]
    format_name: String,
    duration: Option<String>,
}

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<Stream>,
    #[serde(default)]
    format: Format,
}

#[derive(Debug)]
pub struct Media {
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: Option<SystemTime>,
    pub kind: Kind,
    pub format: String,
    pub plain_png: bool,
    pub duration: Option<f64>,
    pub video: Option<Stream>,
    pub audio: Vec<Stream>,
}

impl Media {
    pub fn probe(tool: &Path, path: PathBuf) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err("Choose a local file with an absolute path.".into());
        }
        let meta = fs::metadata(&path).map_err(|e| format!("Could not read the source: {e}"))?;
        if !meta.is_file() {
            return Err("Choose a regular media file.".into());
        }
        let mut raw: Probe = probe_json(
            tool,
            &path,
            &[
                "-show_entries",
                "format=format_name,duration:stream=index,codec_type,codec_name,width,height,channels,sample_rate,bit_rate,pix_fmt,duration:stream_disposition=attached_pic:stream_side_data=rotation",
            ],
        )?;

        let frame_count = if image_demuxer(&raw.format.format_name)
            && !raw.streams.iter().any(is_audio)
        {
            if let Some(stream) = raw.streams.iter_mut().find(|s| is_video(s)) {
                let mut frames = image_frames(tool, &path, stream.index)?;
                let count = frames.len() as u32;
                if count == 0 {
                    return Err("No image frames could be decoded. The file may be damaged.".into());
                }
                if count == 1
                    && !stream
                        .side_data_list
                        .iter()
                        .any(|data| data.rotation.is_some())
                    && let Some(frame) = frames.pop()
                {
                    stream.side_data_list = frame.side_data_list;
                }
                Some(count)
            } else {
                None
            }
        } else {
            None
        };
        let kind = classify(&raw, frame_count)
            .ok_or("No supported video, audio, or image stream was found.")?;
        let duration = (kind != Kind::Image)
            .then(|| {
                positive(raw.format.duration.as_deref()).or_else(|| {
                    raw.streams
                        .iter()
                        .filter(|s| is_video(s) || is_audio(s))
                        .filter_map(|s| positive(s.duration.as_deref()))
                        .reduce(f64::max)
                })
            })
            .flatten();
        let mut video = None;
        let mut audio = Vec::new();
        for stream in raw.streams {
            if video.is_none() && is_video(&stream) {
                video = Some(stream);
            } else if is_audio(&stream) {
                audio.push(stream);
            }
        }
        let plain_png = kind == Kind::Image
            && matches!(raw.format.format_name.as_str(), "png_pipe" | "image2")
            && video
                .as_ref()
                .is_some_and(|stream| stream.codec_name == "png");
        let format = display_format(&raw.format.format_name, kind, video.as_ref());
        Ok(Self {
            path,
            bytes: meta.len(),
            modified: meta.modified().ok(),
            kind,
            format,
            plain_png,
            duration,
            video,
            audio,
        })
    }
}

/// A selected file may be replaced by another program while the UI is open.
pub fn verify_source(path: &Path, bytes: u64, modified: Option<SystemTime>) -> Result<(), String> {
    let metadata = fs::metadata(path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .ok_or("The source file is no longer available. Choose it again to continue.")?;
    if metadata.len() != bytes
        || modified.is_some_and(|time| metadata.modified().ok() != Some(time))
    {
        return Err("The source file changed. Reopen it to refresh its media details.".into());
    }
    Ok(())
}

fn probe_json<T: serde::de::DeserializeOwned>(
    tool: &Path,
    path: &Path,
    args: &[&str],
) -> Result<T, String> {
    let output = ffmpeg::command(tool)
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file",
            "-max_streams",
            "128",
            "-of",
            "json",
        ])
        .args(args)
        .arg("-i")
        .arg(path)
        .output()
        .map_err(|e| format!("Could not start ffprobe: {e}"))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        let detail = message
            .lines()
            .next()
            .unwrap_or("unrecognized or damaged file");
        return Err(format!("Could not inspect this file: {detail}"));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Could not read ffprobe metadata: {e}"))
}

#[derive(Deserialize)]
struct ImageFrame {
    #[serde(default)]
    side_data_list: Vec<SideData>,
}

fn image_frames(tool: &Path, path: &Path, index: u32) -> Result<Vec<ImageFrame>, String> {
    #[derive(Deserialize)]
    struct Frames {
        #[serde(default)]
        frames: Vec<ImageFrame>,
    }

    // Still-image decoders can attach EXIF orientation to the frame instead of the stream.
    let data: Frames = probe_json(
        tool,
        path,
        &[
            "-select_streams",
            &index.to_string(),
            "-read_intervals",
            "%+#4",
            "-show_frames",
            "-show_entries",
            "frame=width:frame_side_data=rotation",
        ],
    )?;
    Ok(data.frames)
}

fn is_video(stream: &Stream) -> bool {
    stream.codec_type.as_deref() == Some("video") && stream.disposition.attached_pic == 0
}

fn is_audio(stream: &Stream) -> bool {
    stream.codec_type.as_deref() == Some("audio")
}

fn image_demuxer(name: &str) -> bool {
    name.ends_with("_pipe")
        || matches!(
            name,
            "image2" | "image2pipe" | "gif" | "apng" | "ico" | "avif"
        )
}

fn classify(probe: &Probe, frames: Option<u32>) -> Option<Kind> {
    let audio = probe.streams.iter().any(is_audio);
    if probe.streams.iter().any(is_video) {
        if !audio && image_demuxer(&probe.format.format_name) && frames == Some(1) {
            Some(Kind::Image)
        } else {
            Some(Kind::Video)
        }
    } else {
        audio.then_some(Kind::Audio)
    }
}

fn positive(value: Option<&str>) -> Option<f64> {
    value?
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n > 0.0)
}

fn display_format(format: &str, kind: Kind, video: Option<&Stream>) -> String {
    if kind == Kind::Image {
        return match video.map(|s| s.codec_name.as_str()) {
            Some("mjpeg") => "JPEG".into(),
            Some("webp") => "WebP".into(),
            Some(codec) if !codec.is_empty() => codec.to_ascii_uppercase(),
            _ => "Image".into(),
        };
    }
    match format {
        "mov,mp4,m4a,3gp,3g2,mj2" => "MP4 / QuickTime".into(),
        "matroska,webm" => "Matroska / WebM".into(),
        "" => "Unknown format".into(),
        other => other.to_ascii_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_stills_animations_and_regular_video_by_probe_data() {
        let mut data: Probe = serde_json::from_str(
            r#"{
            "format": {"format_name":"png_pipe"},
            "streams": [{"index":0,"codec_type":"video","codec_name":"png"}]
        }"#,
        )
        .unwrap();
        assert_eq!(classify(&data, Some(1)), Some(Kind::Image));
        assert_eq!(classify(&data, Some(2)), Some(Kind::Video));
        data.format.format_name = "mov,mp4,m4a,3gp,3g2,mj2".into();
        assert_eq!(classify(&data, Some(1)), Some(Kind::Video));
    }

    #[test]
    fn album_art_does_not_turn_audio_into_video() {
        let data = serde_json::from_str(
            r#"{
            "format": {"format_name":"mp3"},
            "streams": [{"index":0,"codec_type":"audio"},
                {"index":1,"codec_type":"video","disposition":{"attached_pic":1}}]
        }"#,
        )
        .unwrap();
        assert_eq!(classify(&data, None), Some(Kind::Audio));
    }
}
