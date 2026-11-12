# FFlume Homebrew release checklist

The distribution is a **source formula** for macOS. The tracked [formula template](../homebrew-tap/Formula/fflume.rb) uses the [Homebrew Rust helpers](https://docs.brew.sh/Language-Specific-Formulae#rust): fetch locked dependencies first, then install offline. FFmpeg is a runtime dependency, Rust is build-only, and Poppler remains optional. No app bundle is installed by the formula.

## Source and checksum

1. Update `version` in `Cargo.toml` and refresh the root package entry in `Cargo.lock` with `cargo check --offline`. Review the lockfile diff; do not update unrelated dependencies. Confirm `cargo check --locked` and that the binary prints the new version.
2. Run formatting, Clippy, tests and a release build. Commit the source, lockfile, Slint files, fonts, notices, and assets. Confirm the source has no dependency on files from `target/` or the developer's machine.
3. Create the annotated `v<version>` tag on that reviewed commit and publish the tag to the actual GitHub source repository. Use the exact Cargo version. These are maintainer actions.
4. Download the archive from the exact tag URL. For example, after replacing the placeholders:

   ```sh
   release_owner=REPLACE_SOURCE_OWNER
   release_repo=REPLACE_SOURCE_REPO
   release_version=REPLACE_CARGO_VERSION
   archive="fflume-v${release_version}.tar.gz"
   curl --fail --location --output "$archive" \
     "https://github.com/${release_owner}/${release_repo}/archive/refs/tags/v${release_version}.tar.gz"
   shasum -a 256 "$archive"
   tar -tzf "$archive"
   ```

   Verify the archive contains `Cargo.lock`, `build.rs`, `src/`, `ui/`, `assets/`, and licenses. Build from that archive, not a working tree or an archive generated locally. Treat a published tag as immutable; make a new release for corrections.

## Tap and install

5. Replace the formula homepage and URL placeholders and insert the computed SHA-256 in the `sha256` stanza. Check that the formula's inferred version matches the binary's version.
6. Commit `Formula/fflume.rb` to the actual `USERNAME/homebrew-tap` repository. Publish it only after the source archive and checksum are final. Update the README with the real tap name and availability only after a successful install.
7. On a clean Apple Silicon Mac and an Intel Mac, run the following with the actual tap owner:

   ```sh
   brew update
   brew install --build-from-source USERNAME/tap/fflume
   brew test USERNAME/tap/fflume
   fflume --version
   fflume --help
   (cd /tmp && fflume)
   ```

   Check the GUI fonts/logo, file picker, preview and compare, and a real image or video export. Confirm FFmpeg and ffprobe are recognized from standard Homebrew paths; verify that a missing encoder disables its format and that an explicitly selected executable still works.
8. Verify upgrade using a previous installed version and a newly published tap revision: `brew update`, `brew upgrade USERNAME/tap/fflume`, and `brew test USERNAME/tap/fflume`. Verify `brew uninstall USERNAME/tap/fflume` removes the command without deleting user preferences or exported files.

## Licensing and scope

- The project declares `GPL-3.0-only` in Cargo and includes its license in the source and installed notices. Slint 1.18.1 offers GPL-3.0-only as one of its license choices, which is the option used here.
- Geist Sans, Geist Mono and Geist Pixel are bundled with their checked-in SIL OFL and copyright notices. The UI logo is the repository's `assets/mark.svg`; no separately licensed third-party pixel art is referenced by the UI. Confirm provenance and dependency notices before publishing the tagged source.
- FFlume invokes FFmpeg/ffprobe as separate executables. The formula does not redistribute an FFmpeg binary or declare a license on Homebrew's behalf. FFmpeg's distribution terms depend on the build and enabled components.
- The GitHub owner/repository, tagged archive and SHA-256 have not been provided. The template is intentionally unpublished. Do not advertise install, upgrade, or uninstall as verified until the actual tap workflow is tested.

## Local verification for the template

- Apple Silicon, Homebrew 7.0.13: loaded the formula with Homebrew's Ruby loader and checked the version, license, macOS requirement, dependencies and the helpers' locked/offline flags.
- Created a local source archive with the lockfile, fonts, and UI. Extracted it into a temporary directory, fetched locked crates, and ran an offline Cargo install into a temporary prefix. The Cargo build cache was reused. This was not a Homebrew Cellar installation or a test of a GitHub tag.
- The staged binary printed `fflume 0.2.0` and help with a minimal PATH and an invalid Slint backend. GUI QA used the same installed binary in a temporary inspection harness: native file picker, embedded typography/logo, comparison and real JPEG output passed. WebP was absent with ordinary FFmpeg and worked after explicitly selecting ffmpeg-full; ffprobe verified both exported files. The final binary also returned from explicit selection to automatic discovery.
- A clean `brew install`, `brew test`, upgrade/uninstall, and Intel macOS remain release checks. The source archive URL and checksum are still placeholders.
