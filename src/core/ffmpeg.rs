use std::{
    collections::HashSet,
    env, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[cfg(target_os = "macos")]
const FALLBACK_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/local/bin",
    "/opt/homebrew/opt/ffmpeg/bin",
    "/usr/local/opt/ffmpeg/bin",
];
#[cfg(not(target_os = "macos"))]
const FALLBACK_DIRS: &[&str] = &[];

#[derive(Debug, PartialEq, Eq)]
pub enum Availability {
    Ready(Tool),
    Missing,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool {
    pub path: PathBuf,
    pub version: String,
}

impl Availability {
    pub fn tool(&self) -> Option<&Tool> {
        match self {
            Self::Ready(tool) => Some(tool),
            _ => None,
        }
    }
}

pub struct Tools {
    pub ffmpeg: Availability,
    pub ffprobe: Availability,
    capabilities: Capabilities,
}

#[derive(Default)]
struct Capabilities {
    encoders: HashSet<String>,
    muxers: HashSet<String>,
    filters: HashSet<String>,
}

impl Capabilities {
    fn read(path: &Path) -> Self {
        Self {
            encoders: capability_list(path, "-encoders", |flags| {
                flags.len() == 6 && flags.starts_with(['V', 'A', 'S'])
            }),
            muxers: capability_list(path, "-muxers", |flags| flags.contains('E')),
            filters: capability_list(path, "-filters", |flags| {
                !flags.is_empty() && flags.chars().all(|c| matches!(c, '.' | 'T' | 'S' | 'C'))
            }),
        }
    }

    fn supports_webp(&self) -> bool {
        self.encoders.contains("libwebp") && self.muxers.contains("webp")
    }
}

impl Tools {
    pub fn detect() -> Self {
        Self::with_executable(None)
    }

    pub fn with_executable(preferred: Option<&Path>) -> Self {
        let ffmpeg = preferred.map_or_else(
            || detect("ffmpeg"),
            |path| match tool_at(path.to_path_buf(), "ffmpeg") {
                Ok(tool) => Availability::Ready(tool),
                Err(error) => Availability::Failed(format!("{}: {error}", path.display())),
            },
        );
        let ffprobe = ffmpeg
            .tool()
            .and_then(|tool| {
                let sibling = tool.path.parent()?.join(if cfg!(windows) {
                    "ffprobe.exe"
                } else {
                    "ffprobe"
                });
                sibling
                    .is_absolute()
                    .then(|| tool_at(sibling, "ffprobe").ok())
                    .flatten()
            })
            .map_or_else(|| detect("ffprobe"), Availability::Ready);
        let capabilities = ffmpeg
            .tool()
            .map(|tool| Capabilities::read(&tool.path))
            .unwrap_or_default();
        Self {
            ffmpeg,
            ffprobe,
            capabilities,
        }
    }

    pub fn supports_webp(&self) -> bool {
        self.capabilities.supports_webp()
    }

    pub fn has_encoder(&self, name: &str) -> bool {
        self.capabilities.encoders.contains(name)
    }

    pub fn has_muxer(&self, format: super::operation::Format) -> bool {
        self.capabilities.muxers.contains(format.muxer())
    }

    pub fn has_filter(&self, name: &str) -> bool {
        self.capabilities.filters.contains(name)
    }

    pub fn missing_encoder<'a>(&self, plan: &'a super::operation::Plan) -> Option<&'a str> {
        plan.args.windows(2).find_map(|pair| {
            let flag = pair[0].to_str()?;
            let encoder = pair[1].to_str()?;
            ((flag == "-c" || flag.starts_with("-c:"))
                && encoder != "copy"
                && !self.has_encoder(encoder))
            .then_some(encoder)
        })
    }
}

fn tool_at(path: PathBuf, program: &str) -> io::Result<Tool> {
    let version = version(&path, program)?;
    Ok(Tool { path, version })
}

fn detect(program: &str) -> Availability {
    let mut failure = None;
    let file = if cfg!(windows) {
        format!("{program}.exe")
    } else {
        program.to_owned()
    };
    let path = env::var_os("PATH").unwrap_or_default();
    let paths = env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(&file))
        .chain(
            option_env!("FFLUME_BREW_FFMPEG_DIR")
                .into_iter()
                .chain(FALLBACK_DIRS.iter().copied())
                .map(Path::new)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join(&file)),
        );
    for path in paths {
        match tool_at(path, program) {
            Ok(tool) => return Availability::Ready(tool),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                failure.get_or_insert_with(|| error.to_string());
            }
        }
    }

    failure.map_or(Availability::Missing, Availability::Failed)
}

fn version(path: &Path, program: &str) -> io::Result<String> {
    let output = command(path)
        .arg("-version")
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "version check returned {}",
            output.status
        )));
    }

    parse_version(&String::from_utf8_lossy(&output.stdout), program)
        .map(str::to_owned)
        .ok_or_else(|| io::Error::other("unrecognized version output"))
}

pub fn command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn capability_list(
    path: &Path,
    option: &str,
    matches_flags: impl Fn(&str) -> bool,
) -> HashSet<String> {
    command(path)
        .args(["-hide_banner", option])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| parse_capabilities(&String::from_utf8_lossy(&output.stdout), matches_flags))
        .unwrap_or_default()
}

fn parse_capabilities(output: &str, matches_flags: impl Fn(&str) -> bool) -> HashSet<String> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let flags = fields.next()?;
            let names = fields.next()?;
            (matches_flags(flags) && names != "=").then_some(names.split(','))
        })
        .flatten()
        .map(str::to_owned)
        .collect()
}

fn parse_version<'a>(output: &'a str, program: &str) -> Option<&'a str> {
    output.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        (fields.next() == Some(program) && fields.next() == Some("version"))
            .then(|| fields.next())
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_from_the_matching_banner() {
        let banner = "launcher output\nffprobe version N-118000-gabc123 Copyright\nconfiguration:";
        assert_eq!(parse_version(banner, "ffprobe"), Some("N-118000-gabc123"));
        assert_eq!(parse_version(banner, "ffmpeg"), None);
        assert_eq!(parse_version("ffprobe version", "ffprobe"), None);
    }

    #[test]
    fn encoder_detection_uses_exact_names_and_ignores_the_legend() {
        let list = parse_capabilities(
            "Encoders:\n V..... = Video\n V....D libwebp_anim WebP animation\n A....D libmp3lame MP3",
            |flags| flags.len() == 6 && flags.starts_with(['V', 'A', 'S']),
        );
        assert!(list.contains("libmp3lame"));
        assert!(!list.contains("libwebp"));
        assert!(!list.contains("="));
    }

    #[test]
    fn reports_required_encoders_but_allows_stream_copy() {
        use crate::core::operation::{Format, Plan};

        let tools = Tools {
            ffmpeg: Availability::Missing,
            ffprobe: Availability::Missing,
            capabilities: Capabilities {
                encoders: HashSet::from(["aac".into()]),
                ..Capabilities::default()
            },
        };
        let mut plan = Plan {
            args: ["-c:v", "copy", "-c:a:0", "aac"].map(Into::into).into(),
            duration: None,
            format: Format::Mp4,
        };
        assert_eq!(tools.missing_encoder(&plan), None);
        plan.args[1] = "libx264".into();
        assert_eq!(tools.missing_encoder(&plan), Some("libx264"));
    }

    #[test]
    fn nonexistent_executable_is_reported_as_missing() {
        let name = "fflume-test-executable-that-does-not-exist";
        assert_eq!(detect(name), Availability::Missing);
    }
}
