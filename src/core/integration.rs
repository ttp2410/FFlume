use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

use super::{
    ffmpeg::{self, Tools},
    image::{self, Dimensions, Request},
    job::{self, Outcome},
    media::{Kind, Media},
    operation::{self, Action, Format, Plan, Quality, Range},
};

struct Fixture {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    work: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let tools = Tools::detect();
        Self {
            ffmpeg: std::env::var_os("FFLUME_TEST_FFMPEG")
                .map(PathBuf::from)
                .unwrap_or_else(|| tools.ffmpeg.tool().expect("FFmpeg required").path.clone()),
            ffprobe: std::env::var_os("FFLUME_TEST_FFPROBE")
                .map(PathBuf::from)
                .unwrap_or_else(|| tools.ffprobe.tool().expect("ffprobe required").path.clone()),
            work: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.work.path().join(name)
    }

    fn make(&self, args: &[&str], name: &str) -> Media {
        let path = self.path(name);
        let output = ffmpeg::command(&self.ffmpeg)
            .args(["-hide_banner", "-v", "error", "-nostdin", "-y"])
            .args(args)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Media::probe(&self.ffprobe, path).unwrap()
    }

    fn video(&self) -> Media {
        self.make(
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=24",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-map",
                "1:a",
                "-t",
                "4",
                "-c:v",
                "libx264",
                "-g",
                "24",
                "-c:a",
                "aac",
            ],
            "source 東京 clip.mp4",
        )
    }

    fn audio(&self) -> Media {
        self.make(
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100",
                "-t",
                "4",
                "-c:a",
                "pcm_s24le",
            ],
            "source tone.wav",
        )
    }

    fn image(&self) -> Media {
        self.make(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=96x64",
                "-frames:v",
                "1",
                "-c:v",
                "png",
                "-f",
                "image2",
                "-update",
                "1",
            ],
            "image without extension",
        )
    }

    fn export(&self, media: &Media, plan: Plan, name: &str) -> Media {
        let target = self.path(name);
        let (_keep_alive, cancel) = mpsc::channel();
        let outcome = job::export(
            &self.ffmpeg,
            &media.path,
            target.clone(),
            plan,
            cancel,
            |_| {},
        )
        .unwrap();
        assert_eq!(outcome, Outcome::Exported(target.clone()));
        Media::probe(&self.ffprobe, target).unwrap()
    }
}

fn make_plan(media: &Media, action: Action, format: Format) -> Plan {
    operation::plan(media, action, format, Quality::Balanced, None, None).unwrap()
}

#[test]
#[ignore = "requires FFmpeg and ffprobe on the test machine"]
fn ffmpeg_video_operations() {
    let fixture = Fixture::new();
    let source = fixture.video();
    assert_eq!(source.kind, Kind::Video);
    assert_eq!(source.audio.len(), 2);
    let compressed = fixture.export(
        &source,
        make_plan(&source, Action::Compress, Format::Mp4),
        "compressed.mp4",
    );
    assert_eq!(compressed.video.unwrap().codec_name, "h264");
    assert_eq!(compressed.audio.len(), 2);
    assert!(compressed.duration.unwrap() > 3.8);

    let mkv = fixture.export(
        &source,
        make_plan(&source, Action::ConvertVideo, Format::Mkv),
        "converted.mkv",
    );
    assert_eq!(mkv.video.as_ref().unwrap().codec_name, "h264");
    assert_eq!(mkv.audio.len(), 2);

    let extracted = fixture.export(
        &source,
        make_plan(&source, Action::Extract, Format::Mp3),
        "audio.mp3",
    );
    assert_eq!(extracted.kind, Kind::Audio);
    assert_eq!(extracted.audio[0].codec_name, "mp3");

    let range = Range::parse("0:01", "2.5", source.duration).unwrap();
    let trim = operation::plan(
        &source,
        Action::Trim,
        Format::Mkv,
        Quality::Balanced,
        Some(range),
        None,
    )
    .unwrap();
    let trimmed = fixture.export(&source, trim, "trimmed.mkv");
    assert_eq!(trimmed.video.unwrap().codec_name, "h264");
    assert_eq!(trimmed.audio.len(), 2);
    assert!(
        (1.3..=1.8).contains(&trimmed.duration.unwrap()),
        "{:?}",
        trimmed.duration
    );

    let webm = fixture.make(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "1",
            "-c:v",
            "libvpx",
            "-c:a",
            "libopus",
        ],
        "source.webm",
    );
    let mp4 = fixture.export(
        &webm,
        make_plan(&webm, Action::ConvertVideo, Format::Mp4),
        "fallback.mp4",
    );
    assert_eq!(mp4.video.unwrap().codec_name, "h264");
    assert_eq!(mp4.audio[0].codec_name, "aac");
}

#[test]
#[ignore = "requires FFmpeg and ffprobe on the test machine"]
fn ffmpeg_audio_and_image_operations() {
    let fixture = Fixture::new();
    let audio = fixture.audio();
    let mp3 = fixture.export(
        &audio,
        make_plan(&audio, Action::Mp3, Format::Mp3),
        "audio.mp3",
    );
    assert_eq!(mp3.audio[0].codec_name, "mp3");
    let wav = fixture.export(&mp3, make_plan(&mp3, Action::Wav, Format::Wav), "audio.wav");
    assert_eq!(wav.audio[0].codec_name, "pcm_s16le");
    assert!((3.9..=4.2).contains(&wav.duration.unwrap()));

    let image = fixture.image();
    assert_eq!(image.kind, Kind::Image);
    let jpeg = fixture.export(
        &image,
        make_plan(&image, Action::ConvertImage, Format::Jpeg),
        "image.jpg",
    );
    assert_eq!(jpeg.kind, Kind::Image);
    let jpeg_stream = jpeg.video.as_ref().unwrap();
    assert_eq!(
        (jpeg_stream.width, jpeg_stream.height),
        (Some(96), Some(64))
    );
    assert_eq!(jpeg_stream.codec_name, "mjpeg");

    let resize = operation::plan(
        &jpeg,
        Action::Resize,
        Format::Png,
        Quality::High,
        None,
        Some(48),
    )
    .unwrap();
    let png = fixture.export(&jpeg, resize, "resized.png");
    let video = png.video.unwrap();
    assert_eq!(video.codec_name, "png");
    assert_eq!((video.width, video.height), (Some(48), Some(32)));

    let animation = fixture.make(
        &["-f", "lavfi", "-i", "testsrc2=size=96x64:rate=4", "-t", "1"],
        "animated.gif",
    );
    assert_eq!(animation.kind, Kind::Video);
}

#[test]
#[ignore = "requires FFmpeg with libwebp and ffprobe"]
fn ffmpeg_webp_export() {
    use crate::catalog::TOOLS;

    let mut fixture = Fixture::new();
    let detected = Tools::detect();
    let explicit = Tools::with_executable(Some(&detected.ffmpeg.tool().unwrap().path));
    let convert = TOOLS
        .iter()
        .find(|tool| tool.id == "convert-image")
        .unwrap();
    let image = fixture.image();
    let supported = convert.formats(&explicit, Action::ConvertImage, Some(&image));
    assert_eq!(supported.contains(&Format::Webp), explicit.supports_webp());
    if !explicit.has_encoder("libwebp") {
        assert!(!supported.contains(&Format::Webp));
    }

    if std::env::var_os("FFLUME_TEST_FFMPEG").is_none() {
        assert!(
            detected.supports_webp(),
            "Set FFLUME_TEST_FFMPEG to an FFmpeg with libwebp"
        );
        fixture.ffmpeg = detected.ffmpeg.tool().unwrap().path.clone();
    }
    let jpeg = fixture.export(
        &image,
        make_plan(&image, Action::ConvertImage, Format::Jpeg),
        "source.jpg",
    );
    for (index, source) in [&image, &jpeg].into_iter().enumerate() {
        let webp = fixture.export(
            source,
            make_plan(source, Action::ConvertImage, Format::Webp),
            &format!("image-{index}.webp"),
        );
        assert_eq!(webp.kind, Kind::Image);
        assert_eq!(webp.video.as_ref().unwrap().codec_name, "webp");
        let png = fixture.export(
            &webp,
            make_plan(&webp, Action::ConvertImage, Format::Png),
            &format!("roundtrip-{index}.png"),
        );
        let stream = png.video.unwrap();
        assert_eq!(stream.codec_name, "png");
        assert_eq!(stream.dimensions(), Some((96, 64)));
    }
}

#[test]
#[ignore = "requires FFmpeg and ffprobe on the test machine"]
fn ffmpeg_cancel_failure_and_no_overwrite() {
    use std::{cell::Cell, fs};
    let fixture = Fixture::new();
    let source = fixture.audio();
    let original = fs::read(&source.path).unwrap();
    let target = fixture.path("protected.mp3");
    let (cancel, request) = mpsc::channel();
    let mut slow = make_plan(&source, Action::Mp3, Format::Mp3);
    slow.args.insert(0, "-re".into());
    let saw_progress = Cell::new(false);
    let outcome = job::export(
        &fixture.ffmpeg,
        &source.path,
        target.clone(),
        slow,
        request,
        |progress| {
            if progress.fraction.is_some_and(|fraction| fraction > 0.0) {
                saw_progress.set(true);
                cancel.send(()).unwrap();
            } else if progress.elapsed > Duration::from_secs(3) {
                cancel.send(()).unwrap();
            }
        },
    )
    .unwrap();
    assert!(
        saw_progress.get(),
        "no FFmpeg progress was received before cancellation"
    );
    assert_eq!(outcome, Outcome::Cancelled);
    assert!(!target.exists());
    assert_no_temps(fixture.work.path());

    let (_keep_alive, request) = mpsc::channel();
    let mut broken = make_plan(&source, Action::Mp3, Format::Mp3);
    let codec = broken.args.iter().position(|arg| arg == "-c:a").unwrap() + 1;
    broken.args[codec] = "fflume_missing_encoder".into();
    assert!(
        job::export(
            &fixture.ffmpeg,
            &source.path,
            target.clone(),
            broken,
            request,
            |_| {}
        )
        .is_err()
    );
    assert!(!target.exists());
    assert_no_temps(fixture.work.path());

    let (_keep_alive, request) = mpsc::channel();
    let mut slow = make_plan(&source, Action::Mp3, Format::Mp3);
    slow.args.insert(0, "-re".into());
    let result = job::export(
        &fixture.ffmpeg,
        &source.path,
        target.clone(),
        slow,
        request,
        |_| {
            if !target.exists() {
                fs::write(&target, b"user-owned data").unwrap();
            }
        },
    );
    assert!(result.unwrap_err().contains("already exists"));
    assert_eq!(fs::read(&target).unwrap(), b"user-owned data");
    assert_eq!(fs::read(&source.path).unwrap(), original);
    assert_no_temps(fixture.work.path());
}

fn assert_no_temps(folder: &Path) {
    assert!(folder.read_dir().unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fflume-")
    }));
}

#[test]
#[ignore = "requires FFmpeg and ffprobe on the test machine"]
fn ffprobe_rejects_damaged_still_and_detects_replaced_source() {
    use std::fs;
    let fixture = Fixture::new();
    let source = fixture.image();
    let original = fs::read(&source.path).unwrap();
    assert!(super::media::verify_source(&source.path, source.bytes, source.modified).is_ok());
    let damaged = fixture.path("truncated.png");
    fs::write(&damaged, &original[..80]).unwrap();
    let error = Media::probe(&fixture.ffprobe, damaged).unwrap_err();
    assert!(
        error.contains("No image frames could be decoded"),
        "{error}"
    );

    fs::write(&source.path, b"replacement").unwrap();
    assert!(
        super::media::verify_source(&source.path, source.bytes, source.modified)
            .unwrap_err()
            .contains("source file changed")
    );
    fs::remove_file(&source.path).unwrap();
    assert!(
        super::media::verify_source(&source.path, source.bytes, source.modified)
            .unwrap_err()
            .contains("no longer available")
    );
}

#[test]
#[ignore = "requires FFmpeg and ffprobe on the test machine"]
fn ffmpeg_image_preview_target_alpha_and_cleanup() {
    let fixture = Fixture::new();
    let media = fixture.make(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=red@0.4:s=128x96,format=rgba",
            "-frames:v",
            "1",
            "-update",
            "1",
        ],
        "transparent.png",
    );
    let stream = media.video.as_ref().unwrap();
    assert!(image::alpha_warning(&media, Format::Jpeg));
    assert!(!image::alpha_warning(&media, Format::Png));
    let mut settings = Request {
        source: media.path.clone(),
        stream: stream.index,
        format: Format::Jpeg,
        original: Dimensions(128, 96),
        dimensions: Dimensions(64, 48),
        quality: 80,
        lossless: false,
        target: Some(4_000),
        shrink: false,
        unchanged_png: false,
    };
    let (cancel, receiver) = mpsc::channel();
    let preview = image::render(&fixture.ffmpeg, settings.clone(), &receiver, |_| {})
        .unwrap()
        .unwrap();
    assert!(preview.bytes <= 4_000);
    assert_eq!(preview.quality, 100);
    assert!(preview.matches(&settings, &fixture.ffmpeg));
    let target = fixture.path("target.jpg");
    assert!(image::save(&preview, &target, &receiver).unwrap());
    assert_eq!(std::fs::metadata(&target).unwrap().len(), preview.bytes);
    assert_eq!(
        std::fs::read(&target).unwrap(),
        std::fs::read(preview.path()).unwrap()
    );
    let exported = Media::probe(&fixture.ffprobe, target.clone()).unwrap();
    assert_eq!(exported.video.unwrap().dimensions(), Some((64, 48)));
    assert_eq!(
        image::save(&preview, &target, &receiver),
        Err(job::CONFLICT.into())
    );
    let temp = preview.path().to_owned();
    assert!(temp.exists());
    drop(preview);
    assert!(!temp.exists());
    assert_no_temps(fixture.work.path());

    settings.target = Some(1);
    assert!(
        image::render(&fixture.ffmpeg, settings.clone(), &receiver, |_| {})
            .err()
            .unwrap()
            .contains("Target not reached")
    );
    cancel.send(()).unwrap();
    assert!(
        image::render(&fixture.ffmpeg, settings, &receiver, |_| {})
            .unwrap()
            .is_none()
    );
}

#[test]
#[ignore = "requires FFmpeg and ffprobe on the test machine"]
fn ffmpeg_oriented_jpeg_resizes_in_display_coordinates() {
    let fixture = Fixture::new();
    let jpeg = fixture.make(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=96x64",
            "-frames:v",
            "1",
            "-update",
            "1",
        ],
        "source.jpg",
    );
    let original = std::fs::read(jpeg.path).unwrap();
    // Little-endian EXIF orientation 6 (90 degrees clockwise).
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let path = fixture.path("rotated.jpg");
    let length = ((exif.len() + 2) as u16).to_be_bytes();
    let jpeg = [&original[..2], b"\xff\xe1", &length, exif, &original[2..]].concat();
    std::fs::write(&path, jpeg).unwrap();
    let media = Media::probe(&fixture.ffprobe, path.clone()).unwrap();
    let stream = media.video.as_ref().unwrap();
    assert_eq!(stream.dimensions(), Some((64, 96)));
    let settings = Request {
        source: path,
        stream: stream.index,
        format: Format::Png,
        original: Dimensions(64, 96),
        dimensions: Dimensions(32, 48),
        quality: 80,
        lossless: true,
        target: None,
        shrink: false,
        unchanged_png: false,
    };
    let (_cancel, receiver) = mpsc::channel();
    let preview = image::render(&fixture.ffmpeg, settings, &receiver, |_| {})
        .unwrap()
        .unwrap();
    let target = fixture.path("upright.png");
    assert!(image::save(&preview, &target, &receiver).unwrap());
    let exported = Media::probe(&fixture.ffprobe, target).unwrap();
    assert_eq!(exported.video.unwrap().dimensions(), Some((32, 48)));
}

#[test]
#[ignore = "requires FFmpeg and ffprobe on the test machine"]
fn ffmpeg_catalog_suggests_supported_tools_for_the_source() {
    use crate::catalog::TOOLS;

    let fixture = Fixture::new();
    let tools = Tools::with_executable(Some(&fixture.ffmpeg));
    let suggested = |media: &Media| {
        TOOLS
            .iter()
            .filter(|tool| tool.accepts(media) && tool.status(Some(&tools), Some(media)).is_empty())
            .map(|tool| tool.id)
            .collect::<Vec<_>>()
    };

    let mut video = fixture.video();
    assert_eq!(
        suggested(&video),
        [
            "compress-video",
            "convert-video",
            "trim-video",
            "extract-audio",
            "inspect",
            "video-gif",
            "extract-frames"
        ]
    );
    video.audio.clear();
    assert!(!suggested(&video).contains(&"extract-audio"));
    assert_eq!(
        suggested(&fixture.image()),
        ["convert-image", "optimize-image", "resize-image", "inspect"]
    );
    assert_eq!(
        suggested(&fixture.audio()),
        ["convert-audio", "inspect", "normalize-audio", "trim-audio"]
    );
}

#[test]
#[ignore = "requires FFmpeg and ffprobe on the test machine"]
fn ffmpeg_gif_frames_and_audio_tools() {
    let fixture = Fixture::new();
    let video = fixture.video();
    let audio = fixture.audio();
    let range = Range::parse("1", "3", None).unwrap();
    let gif = operation::plan(
        &video,
        Action::Gif,
        Format::Gif,
        Quality::Balanced,
        Some(range),
        None,
    )
    .unwrap();
    let gif = fixture.export(&video, gif, "clip.gif");
    assert_eq!(gif.video.unwrap().codec_name, "gif");
    assert!(gif.audio.is_empty());
    assert!((1.8..=2.1).contains(&gif.duration.unwrap()));
    let frame = operation::plan(
        &video,
        Action::Frames,
        Format::Jpeg,
        Quality::Balanced,
        Some(Range::snapshot("2", video.duration).unwrap()),
        None,
    )
    .unwrap();
    assert_eq!(fixture.export(&video, frame, "still.jpg").kind, Kind::Image);
    let sequence = operation::plan_with_interval(
        &video,
        Action::Frames,
        Format::Png,
        Quality::Balanced,
        Some(range),
        None,
        Some(0.5),
    )
    .unwrap();
    let (_keep_alive, cancel) = mpsc::channel();
    let folder = fixture.path("frames 100%.frames");
    let (first, count) = job::export_frames(&fixture.ffmpeg, &folder, &sequence, &cancel, |_| {})
        .unwrap()
        .unwrap();
    assert_eq!(count, 4);
    assert_eq!(
        Media::probe(&fixture.ffprobe, first).unwrap().kind,
        Kind::Image
    );
    assert_eq!(
        job::export_frames(&fixture.ffmpeg, &folder, &sequence, &cancel, |_| {}).unwrap_err(),
        job::CONFLICT
    );
    for (action, format, name, codec) in [
        (Action::TrimAudio, Format::Flac, "trim.flac", "flac"),
        (
            Action::NormalizeAudio,
            Format::Opus,
            "normalized.opus",
            "opus",
        ),
    ] {
        let plan =
            operation::plan(&audio, action, format, Quality::Balanced, Some(range), None).unwrap();
        let result = fixture.export(&audio, plan, name);
        assert_eq!(result.audio[0].codec_name, codec);
        if action == Action::TrimAudio {
            assert!((1.95..=2.05).contains(&result.duration.unwrap()));
        }
    }
}

#[test]
#[ignore = "requires FFmpeg and ffprobe on the test machine"]
fn ffmpeg_audio_and_video_previews_are_playable_and_temporary() {
    use super::preview;
    let fixture = Fixture::new();
    let video = fixture.video();
    let audio = fixture.make(
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-t",
            "12",
            "-c:a",
            "pcm_s16le",
        ],
        "long-source.wav",
    );

    for (source, action, format, playback_kind, codec) in [
        (&video, Action::Compress, Format::Mp4, Kind::Video, "h264"),
        (
            &video,
            Action::ConvertVideo,
            Format::Mkv,
            Kind::Video,
            "mpeg4",
        ),
        (&video, Action::Extract, Format::Mp3, Kind::Audio, "mp3"),
        (&audio, Action::Wav, Format::Mp3, Kind::Audio, "mp3"),
        (&audio, Action::Wav, Format::Flac, Kind::Audio, "pcm_s16le"),
        (&audio, Action::Wav, Format::Opus, Kind::Audio, "pcm_s16le"),
    ] {
        let plan = operation::plan(source, action, format, Quality::Balanced, None, None).unwrap();
        let (_sender, cancel) = mpsc::channel();
        let sample = preview::render(&fixture.ffmpeg, plan, &cancel, |_| {})
            .unwrap()
            .unwrap();
        let path = sample.playback.clone();
        let playback = Media::probe(&fixture.ffprobe, path.clone()).unwrap();
        assert_eq!(playback.kind, playback_kind);
        let stream = playback.video.as_ref().or(playback.audio.first()).unwrap();
        assert_eq!(stream.codec_name, codec);
        assert!(playback.duration.unwrap() <= 10.1);
        drop(sample);
        assert!(!path.exists());
        assert!(source.path.exists());
    }
}
