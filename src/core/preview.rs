use std::{
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
};

use super::{
    job::{self, Progress},
    operation::{Format, Plan},
};

pub const SAMPLE_SECONDS: f64 = 10.0;

/// Start a re-encoded sample at the frame chosen for comparison.
pub fn seek_compression_sample(plan: &mut Plan, seconds: f64) {
    if seconds > 0.0 {
        plan.args
            .splice(0..0, ["-ss".into(), seconds.to_string().into()]);
        plan.duration = plan
            .duration
            .map(|duration| (duration - seconds).max(0.001));
    }
}

pub struct Preview {
    _directory: tempfile::TempDir,
    pub playback: PathBuf,
    pub sample: PathBuf,
}

pub fn render(
    ffmpeg: &Path,
    mut plan: Plan,
    cancel: &Receiver<()>,
    update: impl Fn(Progress),
) -> Result<Option<Preview>, String> {
    let directory = tempfile::Builder::new()
        .prefix("fflume-preview-")
        .tempdir()
        .map_err(|error| format!("Could not create preview folder: {error}"))?;
    let sample = directory
        .path()
        .join(format!("FFlume Preview.{}", plan.format.extension()));
    if !matches!(plan.format, Format::Png | Format::Jpeg)
        && !plan.args.iter().any(|arg| arg == "-t")
    {
        let seconds = plan.duration.unwrap_or(SAMPLE_SECONDS).min(SAMPLE_SECONDS);
        plan.args.extend(["-t".into(), seconds.to_string().into()]);
        plan.duration = Some(seconds);
    }
    if !job::encode(ffmpeg, &plan, &sample, cancel, &update)? {
        return Ok(None);
    }

    // Use common playback formats for outputs that may have no system player.
    let playback = match plan.format {
        Format::Mkv => playback_copy(ffmpeg, &sample, Format::Mp4, cancel, update)?,
        Format::Flac | Format::Opus => playback_copy(ffmpeg, &sample, Format::Wav, cancel, update)?,
        _ => Some(sample.clone()),
    };
    Ok(playback.map(|playback| Preview {
        _directory: directory,
        playback,
        sample,
    }))
}

impl Preview {
    pub fn original_audio(
        &self,
        ffmpeg: &Path,
        source: &Path,
        stream: u32,
        segment: (&str, f64),
        cancel: &Receiver<()>,
        update: impl Fn(Progress),
    ) -> Result<Option<PathBuf>, String> {
        let path = self._directory.path().join("Original audio.wav");
        let mut args = vec![
            "-ss".into(),
            segment.0.into(),
            "-protocol_whitelist".into(),
            "file".into(),
            "-i".into(),
            source.as_os_str().into(),
            "-map".into(),
            format!("0:{stream}").into(),
        ];
        args.extend(
            [
                "-t",
                &segment.1.to_string(),
                "-vn",
                "-c:a",
                "pcm_s16le",
                "-f",
                "wav",
            ]
            .map(Into::into),
        );
        let plan = Plan {
            args,
            duration: Some(segment.1),
            format: Format::Wav,
        };
        Ok(job::encode(ffmpeg, &plan, &path, cancel, update)?.then_some(path))
    }
}

fn playback_copy(
    ffmpeg: &Path,
    sample: &Path,
    format: Format,
    cancel: &Receiver<()>,
    update: impl Fn(Progress),
) -> Result<Option<PathBuf>, String> {
    let path = sample.with_extension(format.extension());
    let mut args = vec![
        "-protocol_whitelist".into(),
        "file".into(),
        "-i".into(),
        sample.as_os_str().into(),
    ];
    let options: &[&str] = if format == Format::Mp4 {
        &[
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-vf",
            "pad=ceil(iw/2)*2:ceil(ih/2)*2",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-f",
            "mp4",
        ]
    } else {
        &["-map", "0:a:0", "-c:a", "pcm_s16le", "-f", "wav"]
    };
    args.extend(options.iter().map(Into::into));
    let plan = Plan {
        args,
        duration: None,
        format,
    };
    if job::encode(ffmpeg, &plan, &path, cancel, update)? {
        Ok(Some(path))
    } else {
        Ok(None)
    }
}
