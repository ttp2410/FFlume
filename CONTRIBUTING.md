# Contributing to FFlume

## Report an issue

Check [existing issues](https://github.com/ttp2410/FFlume/issues) before opening one. Include your macOS version, Mac architecture, FFlume version, and the versions of FFmpeg or Poppler involved. For a bug, describe the steps to reproduce it, what you expected, and what happened. Attach a small sample you can share if the issue depends on a specific file.

## Submit a pull request

1. Fork the repository and create a branch for your change. Follow the [source build instructions](README.md#build-from-source) to run FFlume locally.
2. Keep Rust changes formatted with `cargo fmt`. For Slint changes, use the shared UI components and theme where possible. Check the affected screen and workflow in the app.
3. Run the checks below. For a media processing change, run the relevant integration tests with the dependencies listed in the [QA report](docs/FINAL_QA_REPORT.md).
4. Open a pull request describing what changed and how you verified it. Link the related issue and include a screenshot for visible UI changes.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```
