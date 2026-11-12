use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use super::{destination, ffmpeg, job, operation::Format};

#[derive(Clone)]
pub struct Backend {
    info: PathBuf,
    render: PathBuf,
}

pub struct Document {
    pub path: PathBuf,
    pub pages: u32,
}

impl Backend {
    pub fn detect() -> Option<Self> {
        let path = env::var_os("PATH").unwrap_or_default();
        let mut dirs: Vec<_> = env::split_paths(&path)
            .filter(|dir| dir.is_absolute())
            .collect();
        if cfg!(target_os = "macos") {
            dirs.extend(
                [
                    "/opt/homebrew/bin",
                    "/usr/local/bin",
                    "/opt/homebrew/opt/poppler/bin",
                    "/usr/local/opt/poppler/bin",
                    "/opt/local/bin",
                ]
                .map(PathBuf::from),
            );
        }
        let find = |name: &str| {
            let name = if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.to_owned()
            };
            dirs.iter().map(|dir| dir.join(&name)).find(|path| {
                ffmpeg::command(path)
                    .arg("-v")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success())
            })
        };
        Some(Self {
            info: find("pdfinfo")?,
            render: find("pdftoppm")?,
        })
    }

    pub fn open(&self, path: PathBuf) -> Result<Document, String> {
        if !path.is_absolute() || !path.is_file() {
            return Err("Choose a local PDF file.".into());
        }
        let output = run(ffmpeg::command(&self.info).arg(&path), "Open PDF")?;
        let pages = String::from_utf8_lossy(&output.stdout)
            .lines()
            .find_map(|line| line.strip_prefix("Pages:")?.trim().parse::<u32>().ok())
            .filter(|&pages| pages > 0)
            .ok_or("Could not read the PDF page count.")?;
        Ok(Document { path, pages })
    }

    fn render_page(
        &self,
        source: &Path,
        page: u32,
        format: Format,
        size: u32,
        prefix: &Path,
    ) -> Result<PathBuf, String> {
        let page = page.to_string();
        let flag = if format == Format::Jpeg {
            "-jpeg"
        } else {
            "-png"
        };
        run(
            ffmpeg::command(&self.render)
                .args([
                    "-f",
                    &page,
                    "-l",
                    &page,
                    "-singlefile",
                    "-scale-to",
                    &size.to_string(),
                    flag,
                ])
                .arg(source)
                .arg(prefix),
            "Render page",
        )?;
        let output = prefix.with_extension(format.extension());
        if !fs::metadata(&output).is_ok_and(|metadata| metadata.len() > 0) {
            return Err("Poppler did not produce an image for this page.".into());
        }
        Ok(output)
    }

    pub fn preview(
        &self,
        source: &Path,
        page: u32,
        format: Format,
    ) -> Result<(tempfile::TempDir, PathBuf), String> {
        let dir =
            tempfile::tempdir().map_err(|error| format!("Could not prepare preview: {error}"))?;
        let path = self.render_page(source, page, format, 760, &dir.path().join("preview"))?;
        Ok((dir, path))
    }

    pub fn export(
        &self,
        source: &Path,
        pages: &[u32],
        folder: &Path,
        format: Format,
    ) -> Result<PathBuf, String> {
        let stem = source
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("document");
        let stem: String = stem
            .chars()
            .map(|c| {
                if c.is_control() || "/\\:*?\"<>|".contains(c) {
                    '_'
                } else {
                    c
                }
            })
            .scan(0, |len, c| {
                *len += c.len_utf8();
                (*len <= 220).then_some(c)
            })
            .collect();
        let targets = pages.iter().map(|page| {
            let path = folder.join(format!("{stem}-page-{page}.{}", format.extension()));
            match destination::exists(&path) {
                Ok(false) => Ok(path),
                Ok(true) => Err(format!("{} already exists. Choose another folder or remove that page from the selection.", path.file_name().unwrap_or_default().to_string_lossy())),
                Err(error) => Err(format!("Could not check the output folder: {error}")),
            }
        }).collect::<Result<Vec<_>, _>>()?;

        let staged = tempfile::tempdir_in(folder)
            .map_err(|error| format!("Could not write to the output folder: {error}"))?;
        let mut rendered = Vec::with_capacity(pages.len());
        for page in pages {
            rendered.push(self.render_page(
                source,
                *page,
                format,
                2400,
                &staged.path().join(page.to_string()),
            )?);
        }
        for (index, (image, target)) in rendered.into_iter().zip(&targets).enumerate() {
            let temporary =
                tempfile::TempPath::try_from_path(image).map_err(|error| error.to_string())?;
            job::publish(temporary, target)
                .map_err(|error| format!("{index} page(s) exported. {error}"))?;
        }
        targets
            .into_iter()
            .next()
            .ok_or_else(|| "Choose at least one page.".into())
    }
}

fn run(command: &mut Command, action: &str) -> Result<Output, String> {
    let output = command
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| format!("{action}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail: String = stderr
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("Poppler could not read the PDF.")
            .chars()
            .take(220)
            .collect();
        return Err(format!("{action}: {detail}"));
    }
    Ok(output)
}

pub fn parse_pages(selection: &str, total: u32) -> Result<Vec<u32>, &'static str> {
    let parse = |part: &str| {
        part.trim()
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=total).contains(n))
            .ok_or("Use page numbers within the document, for example 1, 3-5.")
    };
    let mut pages = BTreeSet::new();
    for part in selection.split(',') {
        if let Some((start, end)) = part.split_once('-') {
            let (start, end) = (parse(start)?, parse(end)?);
            if start > end {
                return Err("Enter page ranges in ascending order, for example 3-5.");
            }
            pages.extend(start..=end);
        } else {
            pages.insert(parse(part)?);
        }
    }
    Ok(pages.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires Poppler on the test machine"]
    fn poppler_exports_selected_pages_and_preserves_existing_images() {
        use std::fmt::Write as _;

        let backend = Backend::detect().expect("Poppler required");
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("sample.pdf");
        let pages = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 144 72] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 144] >>",
        ];
        let mut fixture = "%PDF-1.4\n".to_owned();
        let mut offsets = Vec::new();
        for (index, object) in pages.iter().enumerate() {
            offsets.push(fixture.len());
            writeln!(fixture, "{} 0 obj\n{object}\nendobj", index + 1).unwrap();
        }
        let xref = fixture.len();
        fixture.push_str("xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets {
            writeln!(fixture, "{offset:010} 00000 n ").unwrap();
        }
        writeln!(
            fixture,
            "trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
        )
        .unwrap();
        fs::write(&source, &fixture).unwrap();
        let document = backend.open(source.clone()).unwrap();
        assert_eq!(document.pages, 2);
        let (_preview_dir, preview) = backend.preview(&source, 2, Format::Png).unwrap();
        let bytes = fs::read(preview).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 760);
        let (_jpeg_dir, jpeg_preview) = backend.preview(&source, 2, Format::Jpeg).unwrap();
        assert_eq!(::image::image_dimensions(jpeg_preview).unwrap().1, 760);
        let png = backend
            .export(&source, &[2], dir.path(), Format::Png)
            .unwrap();
        assert_eq!(png.file_name().unwrap(), "sample-page-2.png");
        let png_bytes = fs::read(&png).unwrap();
        assert_eq!(
            u32::from_be_bytes(png_bytes[20..24].try_into().unwrap()),
            2400
        );
        assert!(!dir.path().join("sample-page-1.png").exists());
        assert!(
            backend
                .export(&source, &[1, 2], dir.path(), Format::Png)
                .is_err()
        );
        assert!(!dir.path().join("sample-page-1.png").exists());
        assert_eq!(fs::read(png).unwrap(), png_bytes);
        let jpeg = backend
            .export(&source, &[1], dir.path(), Format::Jpeg)
            .unwrap();
        assert!(fs::read(jpeg).unwrap().starts_with(b"\xff\xd8\xff"));
        assert_eq!(fs::read_to_string(source).unwrap(), fixture);
    }

    #[test]
    fn validates_and_deduplicates_page_ranges() {
        assert_eq!(parse_pages("5, 1-3, 3", 5), Ok(vec![1, 2, 3, 5]));
        for invalid in ["", "0", "6", "3-1", "1-", "1,,2", "1-2-3"] {
            assert!(parse_pages(invalid, 5).is_err(), "{invalid}");
        }
    }
}
