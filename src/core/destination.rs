use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::operation::Format;

#[derive(Default, Serialize, Deserialize)]
pub struct Preferences {
    pub folder: Option<PathBuf>,
    pub ffmpeg: Option<PathBuf>,
}

impl Preferences {
    fn path() -> Option<PathBuf> {
        let root = if cfg!(target_os = "macos") {
            PathBuf::from(env::var_os("HOME")?).join("Library/Application Support")
        } else if cfg!(windows) {
            env::var_os("APPDATA")?.into()
        } else {
            env::var_os("XDG_CONFIG_HOME")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .or_else(|| Some(PathBuf::from(env::var_os("HOME")?).join(".config")))?
        };
        Some(root.join("FFlume").join("settings.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        use std::io::Write;
        let Some(path) = Self::path() else {
            return Ok(());
        };
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("No settings directory"))?;
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(&mut file, self)?;
        file.flush()?;
        file.persist(path)?;
        Ok(())
    }
}

pub fn folder_for(source: &Path, remembered: Option<&Path>) -> PathBuf {
    remembered
        .filter(|p| p.is_absolute() && p.is_dir())
        .or_else(|| source.parent())
        .unwrap_or(source)
        .to_path_buf()
}

pub fn filename(name: &str, format: Format) -> Result<String, &'static str> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        return Err("Enter an output filename.");
    }
    if name
        .chars()
        .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
        || name.ends_with('.')
    {
        return Err("The filename contains an unsupported character or ends in a dot.");
    }
    let (stem, suffix) = name.rsplit_once('.').unwrap_or((name, ""));
    let known = [
        "mp4", "mkv", "mp3", "wav", "png", "jpg", "jpeg", "webp", "gif", "flac", "opus",
    ];
    let stem = if known.iter().any(|ext| suffix.eq_ignore_ascii_case(ext)) {
        stem
    } else {
        name
    };
    let base = stem
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    if matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && matches!(base.get(..3), Some("COM" | "LPT"))
            && base.as_bytes()[3].is_ascii_digit())
        || stem.trim_matches(['.', ' ']).is_empty()
    {
        return Err("Choose a different filename.");
    }
    let result = format!("{}.{}", stem.trim_end(), format.extension());
    if result.len() > 255 {
        return Err("The filename is too long.");
    }
    Ok(result)
}

pub fn exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

pub fn unique_path(path: &Path) -> io::Result<PathBuf> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    for number in 2..=10_000 {
        let suffix = format!(" ({number}).{ext}");
        let mut end = stem.len().min(255usize.saturating_sub(suffix.len()));
        while !stem.is_char_boundary(end) {
            end -= 1;
        }
        let candidate = path.with_file_name(format!("{}{suffix}", &stem[..end]));
        if !exists(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(io::Error::other("Could not find a free numbered filename"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_defaults_and_conflict_resolution() {
        let folder = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let source = folder.path().join("image.png");
        assert_eq!(folder_for(&source, Some(other.path())), other.path());
        assert_eq!(
            folder_for(&source, Some(&other.path().join("missing"))),
            folder.path()
        );
        fs::write(&source, b"original").unwrap();
        assert!(exists(&source).unwrap());
        let new = unique_path(&source).unwrap();
        assert_eq!(new.file_name().unwrap(), "image (2).png");
        assert!(!exists(&new).unwrap());
        assert_eq!(fs::read(source).unwrap(), b"original");
    }

    #[test]
    fn filenames_replace_known_extensions_and_keep_dotted_stems() {
        assert_eq!(filename("ảnh.v2.PNG", Format::Webp).unwrap(), "ảnh.v2.webp");
        assert_eq!(filename("ảnh.v2", Format::Webp).unwrap(), "ảnh.v2.webp");
        for invalid in ["", "../x", "a:b", "COM1.png", "x.", ".jpg"] {
            assert!(filename(invalid, Format::Png).is_err(), "{invalid}");
        }
    }
}
