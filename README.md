<p align="center">
  <img src="assets/mark.svg" alt="FFlume logo" width="64" height="64">
</p>

<h1 align="center">FFlume</h1>

<p align="center"><strong>Make every byte count.</strong></p>
<p align="center">Convert, compress, and preview media in one lightweight desktop app.</p>

<p align="center">
  <a href="#installation"><strong>Get started</strong></a> ·
  <a href="#features">Features</a> ·
  <a href="#screenshots">Screenshots</a>
</p>

<p align="center">
  <img src="docs/images/dashboard.jpg" alt="FFlume dashboard with a file picker, tool search, and categorized media tools" width="690">
</p>

## Why FFlume

- **Compare before you export.** Drag the divider between the original and encoded image, then check the measured output size. For video compression, compare frames at the same timestamp.
- **Choose a target file size.** Set a size for lossy JPEG or WebP and let FFlume try different quality settings.
- **Preview the media you’re working with.** Play GIFs inside FFlume, open audio A/B samples in your system player, and preview PDF pages before exporting.

## Features

| Media | What you can do |
| --- | --- |
| Video | Compress to H.264, convert between MP4 and MKV, trim clips, extract MP3 audio, make looping GIFs, and export frames. |
| Audio | Convert to MP3, WAV, FLAC, or Opus. Trim clips and normalize loudness. |
| Images | Convert to PNG, JPEG, or WebP. Resize by dimensions or scale, choose a quality preset, or set a target size for lossy JPEG and WebP. |
| PDF | Preview pages and export selected pages, ranges, or the whole document as PNG or JPEG. Requires Poppler. |

## Screenshots

<p align="center">
  <img src="docs/images/before-after.jpg" alt="Image comparison showing the original PNG, the encoded JPEG, and its measured size" width="600">
</p>

*Image preview with a draggable comparison divider and measured output size.*

## Installation

Verified on Apple Silicon Macs.

### Homebrew

Homebrew installation is coming soon. The [release checklist](docs/HOMEBREW_RELEASE.md) tracks what remains before the formula is published.

### Build from source

On macOS, install Rust 1.92 or newer and FFmpeg. The commands below use Homebrew to install the FFmpeg dependency.

```sh
git clone https://github.com/ttp2410/FFlume.git
cd FFlume
brew install ffmpeg
cargo build --release --locked
./target/release/fflume
```

| Optional feature | Dependency |
| --- | --- |
| PDF export | Install Poppler with `brew install poppler` or use **Install Poppler** in FFlume. |
| WebP export | Use an FFmpeg build with `libwebp`. To select one, click the FFmpeg version in the footer and choose **Choose FFmpeg…**. |

## Development

FFlume uses Rust and Slint, FFmpeg and ffprobe for media, and Poppler for PDFs.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Run `cargo run --locked` for a development build. Run `sh scripts/bundle-macos.sh` after the build to create a local app bundle.

For integration test coverage and packaging instructions, see the [QA report](docs/FINAL_QA_REPORT.md) and the [Homebrew release checklist](docs/HOMEBREW_RELEASE.md).

## Contributing

Bug reports, feature suggestions, and pull requests are welcome. See the [contribution guide](CONTRIBUTING.md) for what to include and how to run checks.

## License and acknowledgements

FFlume is licensed under [GPL-3.0-only](LICENSE) and uses Slint under its GPL-3.0 option. Media processing uses FFmpeg and ffprobe, and PDF export uses Poppler. The bundled Geist Pixel, Geist Sans, and Geist Mono fonts are distributed under the [SIL Open Font License](assets/fonts/Geist-OFL.txt). See the [font copyright notice](assets/fonts/Geist-LICENSE.txt).
