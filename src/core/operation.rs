use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use super::media::{Kind, Media};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Compress,
    ConvertVideo,
    Extract,
    Trim,
    Mp3,
    Wav,
    ConvertImage,
    Resize,
    Gif,
    Frames,
    TrimAudio,
    NormalizeAudio,
}

impl Action {
    pub fn for_media(media: &Media) -> Vec<Self> {
        use Action::*;
        match media.kind {
            Kind::Video if !media.audio.is_empty() => {
                vec![Compress, ConvertVideo, Extract, Trim, Gif, Frames]
            }
            Kind::Video => vec![Compress, ConvertVideo, Trim, Gif, Frames],
            Kind::Audio => vec![Mp3, Wav, TrimAudio, NormalizeAudio],
            Kind::Image => vec![ConvertImage, Resize],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Compress => "Compress video",
            Self::ConvertVideo => "Convert video",
            Self::Extract => "Extract audio",
            Self::Trim => "Trim video",
            Self::Mp3 => "Convert to MP3",
            Self::Wav => "Convert audio",
            Self::ConvertImage => "Convert image",
            Self::Resize => "Resize image",
            Self::Gif => "Create GIF",
            Self::Frames => "Extract frames",
            Self::TrimAudio => "Trim audio",
            Self::NormalizeAudio => "Normalize audio",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Mp4,
    Mkv,
    Mp3,
    Wav,
    Png,
    Jpeg,
    Webp,
    Gif,
    Flac,
    Opus,
}

impl Format {
    pub fn choices(action: Action) -> &'static [Self] {
        match action {
            Action::Compress => &[Self::Mp4],
            Action::ConvertVideo => &[Self::Mp4, Self::Mkv],
            Action::Trim => &[Self::Mkv],
            Action::Extract | Action::Mp3 => &[Self::Mp3],
            Action::Wav => &[Self::Mp3, Self::Wav, Self::Flac, Self::Opus],
            Action::TrimAudio | Action::NormalizeAudio => {
                &[Self::Wav, Self::Mp3, Self::Flac, Self::Opus]
            }
            Action::Gif => &[Self::Gif],
            Action::Frames => &[Self::Png, Self::Jpeg],
            Action::ConvertImage | Action::Resize => &[Self::Png, Self::Jpeg, Self::Webp],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::Mkv => "MKV",
            Self::Mp3 => "MP3",
            Self::Wav => "WAV",
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Webp => "WebP",
            Self::Gif => "GIF",
            Self::Flac => "FLAC",
            Self::Opus => "Opus",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
            Self::Gif => "gif",
            Self::Flac => "flac",
            Self::Opus => "opus",
        }
    }

    pub fn muxer(self) -> &'static str {
        match self {
            Self::Mkv => "matroska",
            Self::Png | Self::Jpeg => "image2",
            _ => self.extension(),
        }
    }

    #[cfg(test)]
    pub fn output_path(self, mut path: PathBuf) -> PathBuf {
        let extension = path.extension().and_then(OsStr::to_str);
        if !extension.is_some_and(|ext| {
            ext.eq_ignore_ascii_case(self.extension())
                || (self == Self::Jpeg && ext.eq_ignore_ascii_case("jpeg"))
        }) {
            path.set_extension(self.extension());
        }
        path
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Quality {
    High,
    Balanced,
    Small,
}

impl Quality {
    pub fn from_index(index: i32) -> Self {
        match index {
            0 => Self::High,
            2 => Self::Small,
            _ => Self::Balanced,
        }
    }

    fn crf(self) -> &'static str {
        match self {
            Self::High => "18",
            Self::Balanced => "23",
            Self::Small => "28",
        }
    }

    fn jpeg(self) -> &'static str {
        match self {
            Self::High => "2",
            Self::Balanced => "4",
            Self::Small => "7",
        }
    }

    fn webp(self) -> &'static str {
        match self {
            Self::High => "92",
            Self::Balanced => "80",
            Self::Small => "65",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Range {
    start: f64,
    end: f64,
}

impl Range {
    pub fn parse(start: &str, end: &str, duration: Option<f64>) -> Result<Self, &'static str> {
        let start = parse_time(start).ok_or("Enter a valid start time (seconds or HH:MM:SS).")?;
        let end = parse_time(end).ok_or("Enter a valid end time (seconds or HH:MM:SS).")?;
        if end <= start {
            return Err("End time must be after the start time.");
        }
        if duration.is_some_and(|duration| end > duration) {
            return Err("End time is past the source duration.");
        }
        Ok(Self { start, end })
    }

    pub fn snapshot(start: &str, duration: Option<f64>) -> Result<Self, &'static str> {
        let start = parse_time(start).ok_or("Enter a valid frame time.")?;
        if duration.is_some_and(|duration| start >= duration) {
            return Err("Frame time must be before the end of the video.");
        }
        Ok(Self {
            start,
            end: start + 1.0,
        })
    }

    pub fn limited(self, seconds: f64) -> Self {
        Self {
            end: self.end.min(self.start + seconds),
            ..self
        }
    }

    pub fn duration(self) -> f64 {
        self.end - self.start
    }
}

pub fn parse_time(text: &str) -> Option<f64> {
    let mut fields = text.trim().rsplit(':');
    let seconds_text = fields.next()?;
    if !seconds_text
        .bytes()
        .all(|b| b.is_ascii_digit() || b == b'.')
    {
        return None;
    }
    let mut seconds = seconds_text.parse::<f64>().ok()?;
    if let Some(minutes) = fields.next() {
        let minutes = minutes.parse::<u64>().ok()?;
        if seconds >= 60.0 {
            return None;
        }
        seconds += minutes as f64 * 60.0;
        if let Some(hours) = fields.next() {
            if minutes >= 60 || fields.next().is_some() {
                return None;
            }
            seconds += hours.parse::<u64>().ok()? as f64 * 3600.0;
        }
    }
    (seconds.is_finite() && seconds <= i64::MAX as f64 / 1_000_000.0).then_some(seconds)
}

#[derive(Debug)]
pub struct Plan {
    pub args: Vec<OsString>,
    pub duration: Option<f64>,
    pub format: Format,
}

pub fn plan(
    media: &Media,
    action: Action,
    format: Format,
    quality: Quality,
    trim: Option<Range>,
    width: Option<u32>,
) -> Result<Plan, &'static str> {
    plan_with_interval(media, action, format, quality, trim, width, None)
}

pub fn plan_with_interval(
    media: &Media,
    action: Action,
    format: Format,
    quality: Quality,
    trim: Option<Range>,
    width: Option<u32>,
    interval: Option<f64>,
) -> Result<Plan, &'static str> {
    use Action::*;
    if !Action::for_media(media).contains(&action) || !Format::choices(action).contains(&format) {
        return Err("This action or format does not match the source.");
    }
    let mut args = Vec::new();
    let mut duration = media.duration;
    if matches!(action, Trim | Gif | Frames | TrimAudio) {
        let range = trim.ok_or("Choose a start and end time.")?;
        push(&mut args, &["-ss", &format!("{:.6}", range.start)]);
        duration = Some(range.duration());
    }
    push(&mut args, &["-protocol_whitelist", "file", "-i"]);
    args.push(media.path.as_os_str().into());
    if matches!(action, Gif | Frames | TrimAudio)
        && let Some(seconds) = duration
    {
        push(&mut args, &["-t", &format!("{seconds:.6}")]);
    }
    match action {
        Gif | Frames => {
            let video = media
                .video
                .as_ref()
                .ok_or("The source has no video track.")?;
            if action == Gif {
                // Trim before generating the palette so its colors reflect only the selected clip.
                let seconds = duration.ok_or("Enter a GIF range.")?;
                let filter = format!(
                    "[0:{}]trim=duration={seconds:.6},setpts=PTS-STARTPTS,fps=12,scale=w='min(640,iw)':h=-1:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse[gif]",
                    video.index
                );
                push(
                    &mut args,
                    &[
                        "-filter_complex",
                        &filter,
                        "-map",
                        "[gif]",
                        "-an",
                        "-c:v",
                        "gif",
                        "-loop",
                        "0",
                    ],
                );
            } else {
                map(&mut args, video.index);
                let encoder = if format == Format::Png {
                    "png"
                } else {
                    "mjpeg"
                };
                push(&mut args, &["-an", "-c:v", encoder]);
                if let Some(interval) = interval {
                    push(
                        &mut args,
                        &[
                            "-vf",
                            &format!(
                                "select=isnan(prev_selected_t)+gte(t-prev_selected_t\\,{interval:.6})"
                            ),
                            "-fps_mode",
                            "vfr",
                        ],
                    );
                } else {
                    push(&mut args, &["-frames:v", "1", "-update", "1"]);
                    duration = None;
                }
            }
        }
        Compress | ConvertVideo | Trim => {
            let video = media
                .video
                .as_ref()
                .ok_or("The source has no video track.")?;
            map(&mut args, video.index);
            for audio in &media.audio {
                map(&mut args, audio.index);
            }
            if action == Trim {
                let seconds = duration.ok_or("Choose a start and end time.")?;
                push(
                    &mut args,
                    &[
                        "-t",
                        &format!("{seconds:.6}"),
                        "-c",
                        "copy",
                        "-map_chapters",
                        "-1",
                        "-avoid_negative_ts",
                        "make_zero",
                    ],
                );
            } else {
                if action == ConvertVideo && copy_video(format, &video.codec_name) {
                    push(&mut args, &["-c:v", "copy"]);
                    if format == Format::Mp4 && video.codec_name == "hevc" {
                        push(&mut args, &["-tag:v", "hvc1"]);
                    }
                } else {
                    let crf = if action == Compress {
                        quality.crf()
                    } else {
                        Quality::Balanced.crf()
                    };
                    push(
                        &mut args,
                        &[
                            "-c:v",
                            "libx264",
                            "-preset",
                            "fast",
                            "-crf",
                            crf,
                            "-vf",
                            "pad=ceil(iw/2)*2:ceil(ih/2)*2",
                            "-pix_fmt",
                            "yuv420p",
                        ],
                    );
                }
                for (i, audio) in media.audio.iter().enumerate() {
                    if copy_audio(format, &audio.codec_name) {
                        push(&mut args, &[&format!("-c:a:{i}"), "copy"]);
                    } else {
                        push(
                            &mut args,
                            &[&format!("-c:a:{i}"), "aac", &format!("-b:a:{i}"), "192k"],
                        );
                    }
                }
            }
            if format == Format::Mp4 {
                push(&mut args, &["-movflags", "+faststart"]);
            }
        }
        Extract | Mp3 | Wav | TrimAudio | NormalizeAudio => {
            let audio = media
                .audio
                .first()
                .ok_or("The source has no audio track.")?;
            if action != TrimAudio {
                duration = audio.duration().or(duration);
            }
            map(&mut args, audio.index);
            push(&mut args, &["-vn", "-map_chapters", "-1"]);
            if action == NormalizeAudio {
                push(
                    &mut args,
                    &["-af", "loudnorm=I=-16:TP=-1.5:LRA=11", "-ar", "48000"],
                );
            }
            if format == Format::Wav {
                push(&mut args, &["-c:a", "pcm_s16le"]);
            } else if format == Format::Flac {
                push(&mut args, &["-c:a", "flac"]);
            } else if format == Format::Opus {
                push(&mut args, &["-c:a", "libopus", "-b:a", "96k"]);
                if audio.channels.is_some_and(|channels| channels > 2) {
                    push(&mut args, &["-ac", "2"]);
                }
            } else if matches!(action, Extract | Mp3 | Wav) && audio.codec_name == "mp3" {
                push(&mut args, &["-c:a", "copy"]);
            } else {
                push(&mut args, &["-c:a", "libmp3lame", "-q:a", "2"]);
                if audio.channels.is_some_and(|channels| channels > 2) {
                    push(&mut args, &["-ac", "2"]);
                }
            }
        }
        ConvertImage | Resize => {
            let image = media
                .video
                .as_ref()
                .ok_or("The source has no image stream.")?;
            map(&mut args, image.index);
            push(&mut args, &["-an", "-frames:v", "1"]);
            if action == Resize {
                let width = width
                    .filter(|width| (1..=16384).contains(width))
                    .ok_or("Enter a width between 1 and 16384 pixels.")?;
                push(
                    &mut args,
                    &["-vf", &format!("scale={width}:max(1\\,round(ow*ih/iw))")],
                );
            }
            match format {
                Format::Png => push(&mut args, &["-c:v", "png", "-update", "1"]),
                Format::Jpeg => push(
                    &mut args,
                    &["-c:v", "mjpeg", "-q:v", quality.jpeg(), "-update", "1"],
                ),
                Format::Webp => push(&mut args, &["-c:v", "libwebp", "-quality", quality.webp()]),
                _ => return Err("Choose an image format."),
            }
        }
    }
    push(&mut args, &["-f", format.muxer()]);
    Ok(Plan {
        args,
        duration,
        format,
    })
}

pub fn copy_video(format: Format, codec: &str) -> bool {
    matches!(codec, "h264" | "hevc" | "av1" | "mpeg4" | "mjpeg")
        || (format == Format::Mkv
            && matches!(
                codec,
                "vp8" | "vp9" | "mpeg1video" | "mpeg2video" | "ffv1" | "theora" | "prores"
            ))
}

pub fn copy_audio(format: Format, codec: &str) -> bool {
    matches!(codec, "aac" | "mp3" | "alac" | "ac3" | "eac3")
        || (format == Format::Mkv
            && (codec.starts_with("pcm_")
                || matches!(codec, "opus" | "vorbis" | "flac" | "dts" | "truehd" | "mp2")))
}

fn push(args: &mut Vec<OsString>, values: &[&str]) {
    args.extend(values.iter().map(OsString::from));
}

fn map(args: &mut Vec<OsString>, index: u32) {
    push(args, &["-map", &format!("0:{index}")]);
}

pub fn suggested_path(input: &Path, action: Action, format: Format) -> PathBuf {
    let mut name = input
        .file_stem()
        .unwrap_or(OsStr::new("media"))
        .to_os_string();
    name.push(match action {
        Action::Compress => "-compressed",
        Action::Extract => "-audio",
        Action::Trim | Action::TrimAudio => "-trimmed",
        Action::NormalizeAudio => "-normalized",
        Action::Frames => "-frames",
        Action::Resize => "-resized",
        _ => "-converted",
    });
    name.push(".");
    name.push(format.extension());
    input.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video() -> Media {
        let v = serde_json::from_str(r#"{"index":2,"codec_name":"h264"}"#).unwrap();
        let a = serde_json::from_str(r#"{"index":4,"codec_name":"opus"}"#).unwrap();
        Media {
            path: "/input clip 東京.mkv".into(),
            bytes: 7,
            modified: None,
            kind: Kind::Video,
            format: "MKV".into(),
            plain_png: false,
            duration: Some(10.0),
            video: Some(v),
            audio: vec![a],
        }
    }

    fn includes(args: &[OsString], pair: [&str; 2]) -> bool {
        args.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1])
    }

    #[test]
    fn compression_presets_select_distinct_crfs_and_keep_audio() {
        for (index, crf) in ["18", "23", "28"].into_iter().enumerate() {
            let p = plan(
                &video(),
                Action::Compress,
                Format::Mp4,
                Quality::from_index(index as i32),
                None,
                None,
            )
            .unwrap();
            assert!(includes(&p.args, ["-crf", crf]));
            assert!(includes(&p.args, ["-map", "0:4"]));
            assert!(includes(&p.args, ["-c:a:0", "aac"]));
        }
    }

    #[test]
    fn conversion_copies_compatible_streams_and_only_encodes_incompatible_ones() {
        let media = video();
        let mp4 = plan(
            &media,
            Action::ConvertVideo,
            Format::Mp4,
            Quality::Balanced,
            None,
            None,
        )
        .unwrap();
        assert!(includes(&mp4.args, ["-c:v", "copy"]));
        assert!(includes(&mp4.args, ["-c:a:0", "aac"]));
        assert!(includes(&mp4.args, ["-i", "/input clip 東京.mkv"]));
        let mkv = plan(
            &media,
            Action::ConvertVideo,
            Format::Mkv,
            Quality::Balanced,
            None,
            None,
        )
        .unwrap();
        assert!(includes(&mkv.args, ["-c:a:0", "copy"]));
        let mut unsupported = video();
        unsupported.video.as_mut().unwrap().codec_name = "vp8".into();
        let fallback = plan(
            &unsupported,
            Action::ConvertVideo,
            Format::Mp4,
            Quality::Balanced,
            None,
            None,
        )
        .unwrap();
        assert!(includes(&fallback.args, ["-c:v", "libx264"]));
    }

    #[test]
    fn output_names_keep_the_stem_and_never_match_the_source() {
        let input = Path::new("/media/a.b 東京.mp4");
        assert_eq!(
            suggested_path(input, Action::Compress, Format::Mp4),
            Path::new("/media/a.b 東京-compressed.mp4")
        );
        assert_eq!(
            Format::Jpeg.output_path("photo.JPEG".into()),
            Path::new("photo.JPEG")
        );
    }

    #[test]
    fn validates_trim_times_and_builds_a_keyframe_seek() {
        let range = Range::parse("0:01.25", "0:03.75", Some(10.0)).unwrap();
        assert_eq!(range.duration(), 2.5);
        for (start, end) in [
            ("2", "1"),
            ("0", "11"),
            ("-1", "2"),
            ("0:60", "2"),
            ("NaN", "2"),
            ("1:60:00", "2"),
        ] {
            assert!(
                Range::parse(start, end, Some(10.0)).is_err(),
                "{start}, {end}"
            );
        }
        let p = plan(
            &video(),
            Action::Trim,
            Format::Mkv,
            Quality::Balanced,
            Some(range),
            None,
        )
        .unwrap();
        assert!(includes(&p.args, ["-ss", "1.250000"]));
        assert!(includes(&p.args, ["-c", "copy"]));
        assert_eq!(p.duration, Some(2.5));
    }
}
