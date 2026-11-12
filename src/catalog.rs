use crate::core::{
    ffmpeg::Tools,
    media::{Kind, Media},
    operation::{self, Action, Format, Quality, Range},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Video,
    Image,
    Audio,
    Animation,
    Subtitles,
    Streaming,
    MediaAnalysis,
    Documents,
}

impl Category {
    pub const ALL: [Self; 8] = [
        Self::Video,
        Self::Image,
        Self::Audio,
        Self::Animation,
        Self::Subtitles,
        Self::Streaming,
        Self::MediaAnalysis,
        Self::Documents,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "Video tools",
            Self::Image => "Image tools",
            Self::Audio => "Audio tools",
            Self::Animation => "GIF & animation",
            Self::Subtitles => "Subtitles",
            Self::Streaming => "Streaming",
            Self::MediaAnalysis => "File insights",
            Self::Documents => "Document tools",
        }
    }
}

#[derive(Clone, Copy)]
pub enum Handler {
    Process(&'static [Action]),
    Optimize,
    Inspect,
    Planned,
    Pdf,
}

pub struct Tool {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub category: Category,
    pub featured: bool,
    pub icon_path: &'static str,
    pub keywords: &'static str,
    pub input: &'static [Kind],
    pub handler: Handler,
}

const VIDEO: &[Kind] = &[Kind::Video];
const IMAGE: &[Kind] = &[Kind::Image];
const AUDIO: &[Kind] = &[Kind::Audio];
const MEDIA: &[Kind] = &[Kind::Video, Kind::Image, Kind::Audio];

use Category::*;
use Handler::*;

pub const TOOLS: &[Tool] = &[
    Tool {
        id: "compress-video",
        name: "Compress video",
        description: "Make videos smaller with quality presets.",
        category: Video,
        featured: true,
        icon_path: "M 4 4 L 10 10 M 10 5 V 10 H 5 M 20 20 L 14 14 M 14 19 V 14 H 19",
        keywords: "shrink smaller h264 mp4",
        input: VIDEO,
        handler: Process(&[Action::Compress]),
    },
    Tool {
        id: "convert-video",
        name: "Convert video",
        description: "Change format without the guesswork.",
        category: Video,
        featured: true,
        icon_path: "M 4 8 H 20 L 16 4 M 20 16 H 4 L 8 20",
        keywords: "remux format transcode",
        input: VIDEO,
        handler: Process(&[Action::ConvertVideo]),
    },
    Tool {
        id: "trim-video",
        name: "Trim video",
        description: "Keep only the part you need.",
        category: Video,
        featured: true,
        icon_path: "M 8 8 L 20 20 M 8 16 L 20 4 M 8 6 A 3 3 0 1 0 2 6 A 3 3 0 1 0 8 6 Z M 8 18 A 3 3 0 1 0 2 18 A 3 3 0 1 0 8 18 Z",
        keywords: "cut clip time",
        input: VIDEO,
        handler: Process(&[Action::Trim]),
    },
    Tool {
        id: "convert-image",
        name: "Convert image",
        description: "Switch between PNG, JPEG, and WebP.",
        category: Image,
        featured: true,
        icon_path: "M 4 5 H 20 V 19 H 4 Z M 4 15 L 9 10 L 13 14 L 16 11 L 20 15",
        keywords: "format photo png jpeg jpg webp",
        input: IMAGE,
        handler: Process(&[Action::ConvertImage]),
    },
    Tool {
        id: "optimize-image",
        name: "Optimize image",
        description: "Reduce file size without unnecessary quality loss.",
        category: Image,
        featured: true,
        icon_path: "M 4 4 L 10 10 M 10 5 V 10 H 5 M 20 20 L 14 14 M 14 19 V 14 H 19",
        keywords: "compress smaller photo quality target resize",
        input: IMAGE,
        handler: Optimize,
    },
    Tool {
        id: "resize-image",
        name: "Resize image",
        description: "Change dimensions while keeping proportions.",
        category: Image,
        featured: true,
        icon_path: "M 9 4 H 4 V 9 M 15 4 H 20 V 9 M 20 15 V 20 H 15 M 9 20 H 4 V 15",
        keywords: "scale crop width height resolution",
        input: IMAGE,
        handler: Process(&[Action::Resize]),
    },
    Tool {
        id: "extract-audio",
        name: "Extract audio",
        description: "Save the sound from a video.",
        category: Audio,
        featured: true,
        icon_path: "M 3 10 V 14 M 7 6 V 18 M 12 3 V 21 M 17 7 V 17 M 21 10 V 14",
        keywords: "convert sound soundtrack mp3 video",
        input: VIDEO,
        handler: Process(&[Action::Extract]),
    },
    Tool {
        id: "convert-audio",
        name: "Convert audio",
        description: "Switch between common audio formats.",
        category: Audio,
        featured: true,
        icon_path: "M 4 8 H 20 L 16 4 M 20 16 H 4 L 8 20",
        keywords: "convert music sound mp3 wav compress",
        input: AUDIO,
        handler: Process(&[Action::Wav]),
    },
    Tool {
        id: "inspect",
        name: "Inspect file",
        description: "Explore formats, dimensions, and technical details.",
        category: MediaAnalysis,
        featured: true,
        icon_path: "M 16 10 A 6 6 0 1 0 4 10 A 6 6 0 1 0 16 10 Z M 14 15 L 20 21",
        keywords: "inspect metadata info properties codecs duration",
        input: MEDIA,
        handler: Inspect,
    },
    Tool {
        id: "video-gif",
        name: "Create GIF",
        description: "Turn a short clip into an animated GIF.",
        category: Video,
        featured: true,
        icon_path: "M 4 5 H 20 V 19 H 4 Z M 10 9 L 15 12 L 10 15 Z",
        keywords: "convert gif animated loop",
        input: VIDEO,
        handler: Process(&[Action::Gif]),
    },
    Tool {
        id: "extract-frames",
        name: "Extract frames",
        description: "Save video frames as images.",
        category: Video,
        featured: true,
        icon_path: "M 4 5 H 20 V 19 H 4 Z M 4 15 L 9 10 L 13 14 L 16 11 L 20 15",
        keywords: "gif stills screenshot video",
        input: VIDEO,
        handler: Process(&[Action::Frames]),
    },
    Tool {
        id: "normalize-audio",
        name: "Normalize audio",
        description: "Balance volume levels automatically.",
        category: Audio,
        featured: true,
        icon_path: "M 3 10 V 14 M 7 6 V 18 M 12 3 V 21 M 17 7 V 17 M 21 10 V 14",
        keywords: "sound loudness volume level",
        input: AUDIO,
        handler: Process(&[Action::NormalizeAudio]),
    },
    Tool {
        id: "trim-audio",
        name: "Trim audio",
        description: "Cut audio to the part you need.",
        category: Audio,
        featured: true,
        icon_path: "M 8 8 L 20 20 M 8 16 L 20 4 M 8 6 A 3 3 0 1 0 2 6 A 3 3 0 1 0 8 6 Z M 8 18 A 3 3 0 1 0 2 18 A 3 3 0 1 0 8 18 Z",
        keywords: "music sound clip cut time",
        input: AUDIO,
        handler: Process(&[Action::TrimAudio]),
    },
    Tool {
        id: "subtitles",
        name: "Subtitle tools",
        description: "Work with caption and subtitle tracks.",
        category: Subtitles,
        featured: false,
        icon_path: "M 6 3 H 15 L 19 7 V 21 H 6 Z M 9 11 H 16 M 9 15 H 16",
        keywords: "subtitles captions srt vtt",
        input: VIDEO,
        handler: Planned,
    },
    Tool {
        id: "streaming",
        name: "Streaming tools",
        description: "Prepare media for streaming.",
        category: Streaming,
        featured: false,
        icon_path: "M 4 5 H 20 V 19 H 4 Z M 10 9 L 15 12 L 10 15 Z",
        keywords: "hls dash streaming",
        input: VIDEO,
        handler: Planned,
    },
    Tool {
        id: "documents",
        name: "PDF to images",
        description: "Turn PDF pages into image files.",
        category: Documents,
        featured: true,
        icon_path: "M 6 3 H 15 L 19 7 V 21 H 6 Z M 9 11 H 16 M 9 15 H 16",
        keywords: "pdf documents pages images",
        input: &[],
        handler: Pdf,
    },
];

impl Tool {
    pub fn matches(&self, category: Option<Category>, query: &str) -> bool {
        if category.is_some_and(|category| category != self.category) {
            return false;
        }
        if query.trim().is_empty() {
            return true;
        }
        let text = format!(
            "{} {} {} {}",
            self.name,
            self.category.label(),
            self.description,
            self.keywords
        )
        .to_lowercase();
        query
            .split_whitespace()
            .all(|word| text.contains(&word.to_lowercase()))
    }

    pub fn actions(&self) -> &[Action] {
        match self.handler {
            Process(actions) => actions,
            Optimize => &[Action::ConvertImage],
            _ => &[],
        }
    }

    pub fn accepts(&self, media: &Media) -> bool {
        self.input.contains(&media.kind)
            && (!self.actions().contains(&Action::Extract) || !media.audio.is_empty())
    }

    pub fn formats(&self, tools: &Tools, action: Action, media: Option<&Media>) -> Vec<Format> {
        Format::choices(action)
            .iter()
            .copied()
            .filter(|&format| !matches!(self.handler, Optimize) || format != Format::Png)
            .filter(|&format| supports_format(tools, action, format, media))
            .collect()
    }

    pub fn status(&self, tools: Option<&Tools>, media: Option<&Media>) -> &'static str {
        match self.handler {
            Planned => return "Coming soon",
            Pdf => return "",
            _ => {}
        }
        let Some(tools) = tools else {
            return "Checking media tools…";
        };
        if tools.ffprobe.tool().is_none() {
            return "Requires ffprobe";
        }
        if matches!(self.handler, Inspect) {
            return "";
        }
        if tools.ffmpeg.tool().is_none() {
            return "Requires FFmpeg";
        }
        if self
            .actions()
            .iter()
            .any(|&action| !self.formats(tools, action, media).is_empty())
        {
            ""
        } else {
            "Unavailable with this FFmpeg"
        }
    }
}

fn supports_format(tools: &Tools, action: Action, format: Format, media: Option<&Media>) -> bool {
    use Action::*;
    use Format::*;
    if format == Png && action == ConvertImage && media.is_some_and(|m| m.plain_png) {
        return true;
    }
    if !tools.has_muxer(format) {
        return false;
    }
    if matches!(action, ConvertImage | Resize) {
        return (action != Resize || tools.has_filter("scale") && tools.has_filter("setsar"))
            && match format {
                Png => tools.has_encoder("png"),
                Jpeg => tools.has_encoder("mjpeg"),
                Webp => tools.supports_webp(),
                _ => false,
            };
    }
    if action == Action::Gif
        && ![
            "trim",
            "setpts",
            "fps",
            "scale",
            "split",
            "palettegen",
            "paletteuse",
        ]
        .iter()
        .all(|filter| tools.has_filter(filter))
    {
        return false;
    }
    if action == Frames && !tools.has_filter("select") {
        return false;
    }
    if action == NormalizeAudio && !tools.has_filter("loudnorm") {
        return false;
    }
    let Some(media) = media else {
        return match action {
            Compress => tools.has_encoder("libx264") && tools.has_filter("pad"),
            Action::Gif => tools.has_encoder("gif"),
            Frames => tools.has_encoder(if format == Png { "png" } else { "mjpeg" }),
            Action::Wav | TrimAudio | NormalizeAudio => tools.has_encoder(match format {
                Format::Wav => "pcm_s16le",
                Format::Flac => "flac",
                Format::Opus => "libopus",
                Format::Mp3 => "libmp3lame",
                _ => return false,
            }),
            _ => true,
        };
    };
    let trim = Range::parse("0", "1", None).ok();
    operation::plan(media, action, format, Quality::Balanced, trim, None).is_ok_and(|plan| {
        tools.missing_encoder(&plan).is_none()
            && (!plan.args.iter().any(|arg| arg == "-vf") || tools.has_filter("pad"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_terms_across_fields_and_respects_category() {
        let find = |id| TOOLS.iter().find(|tool| tool.id == id).unwrap();
        assert!(find("optimize-image").matches(Some(Image), "COMPRESS target"));
        assert!(!find("optimize-image").matches(Some(Video), "compress"));
        assert!(find("subtitles").matches(None, "srt"));
        assert!(matches!(find("video-gif").handler, Process(_)));
        assert_eq!(find("documents").status(None, None), "");
        let ids = TOOLS
            .iter()
            .map(|tool| tool.id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), TOOLS.len());
    }
}
