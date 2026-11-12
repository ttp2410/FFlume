# FFlume final QA report

**2026-11-12 · FFlume 0.2.0 · READY FOR BETA (local macOS arm64 build)**

The main file-first and tool-first workflows passed on the native app. No source overwrite, destination overwrite, or partial output after cancellation was observed in the tested scenarios. The release candidate is suitable for local beta testing on the tested Mac; this audit does not clear a public distribution build or certify other platforms.

Environment: macOS 27.2.1 (26B131), arm64, Winit/FemtoVG, FFmpeg and ffprobe 9.0.2 (Homebrew; ffmpeg-full for WebP), Poppler 26.11.0. Fixtures were generated locally, and output files were inspected independently with ffprobe. App screenshots and the accessibility tree were inspected on macOS. The checked-out workspace had no Git metadata; there was no release workflow or cask in the supplied source tree.

## Implementation inventory and audit scope

Reviewed Cargo configuration and lockfile, Slint home/source/settings/compare/PDF screens, app state and worker handoff, registry and capabilities, FFmpeg/ffprobe plans and argument handling, preview artifacts, export staging, preferences, Poppler, and existing tests. Implemented tools: still image convert, optimize and resize; video convert, compress, trim, GIF and frame extraction; audio extraction, conversion, trimming and loudness normalization; metadata inspection; PDF page preview and image export. Subtitle and streaming cards are placeholders; they were excluded from functional passes.

## Results

| Journey / area | Status | Evidence |
| --- | --- | --- |
| 1. PNG → WebP | PASS — GUI + integration | Opened Unicode panorama, generated preview, dragged divider from 50% to 77%, switched to native detail, exported 6,702-byte WebP at 1920 × 360. |
| 2. Optimize image | PASS — GUI + integration | Smaller file preset, preview and export; ffprobe identified JPEG at 1920 × 360. High quality invalidated the prior comparison. A 1-byte target failed with the smallest trial in diagnostics. Integration checked the target-size bound and byte-for-byte preview reuse. |
| 3. Resize image | PASS — GUI + integration | Locked aspect ratio, 1920 × 360 → 960 × 180, preview and PNG export. Integration also checked rotated JPEG dimensions. |
| 4. Compress video | PASS — GUI + integration | Preview at 2 seconds and exported H.264/AAC MP4 at 640 × 360 and 6 seconds. A two-color fixture checked timestamp alignment independently. Automated conversion covered MKV, WebM → MP4, multiple audio tracks and trim. |
| 5. Extract audio | PASS — GUI + integration | Tool-first import of a video with audio; 6-second MP3 at 48 kHz, mono. Automated tests also cover WAV/FLAC/Opus, trimming, normalization and playable A/B sample files. |
| 6. Cancellation | PASS — GUI + integration | Canceled an in-flight 24-second 1080p input export. UI returned to “Export canceled”; no destination or export staging entry remained. Integration checked progress, process exit and cleanup. |
| 7. Invalid input | PASS — GUI + integration | Truncated PNG rejected with “Couldn’t read this file”; next valid import worked. Regression test also covers a replaced or deleted source. |
| 8. Navigation | PASS — GUI | File-first and tool-first, back retains input, replace/remove, GIF end 5 seconds → Trim end 6 seconds, no stale compare in new workflow. |
| 9. Export conflict | PASS — GUI + integration | Existing name rejected; Keep both produced a numbered copy with identical bytes. Integration exercises a file appearing during encode and asserts original bytes preserved. |
| 10. Missing encoder | PASS — backend; NOT TESTED — GUI | Explicit detection of actual Homebrew FFmpeg without libwebp excludes WebP in the format model; ffmpeg-full encoded it. Missing executable/encoder paths covered by tests. No machine-level tool changes or installs made. |
| Home and search | PASS — GUI + unit | Brand/hero, suggestions, no match, mixed-case partial query, format/description matching, clear search, category changes and return to all categories. |
| Smart preview | PASS — sampled GUI + integration | Generating/ready/outdated/failed observed; divider, fit/detail, measured larger output, real encode, correct release artifact reuse. Preview cancellation path is covered at worker level; separate GUI cancellation of preview was not exercised. |
| PDF | PASS — GUI + integration | File-first open, Next, PNG → JPEG preview, select all, native folder picker, two JPEG pages at 1600 × 2400 and 2400 × 1600. Integration checks selected-page export and collisions. |
| GIF, frame sequences, audio A/B | PASS — GUI playback controls + integration | Inline GIF switched Play → Pause → Play. Original WAV and processed MP3 opened in QuickTime and each accepted Play; audible output was not independently monitored. Integration verifies frame sequences, bounded GIF frames, codecs and temp cleanup. |
| Visual and accessibility | PASS — sampled GUI; NOT TESTED — full matrix | Home, comparison, export/result, PDF and modal controls inspected; accessibility tree exposes named inputs, actions and divider. Final pass did not verify minimum-size window, all keyboard focus paths, VoiceOver, or drag and drop. |
| Preferences and OS integration | PASS — unit/partial GUI | Folder defaults and numbered names tested; native file/folder pickers used. Persistence across relaunch, Finder reveal, and live executable replacement not verified. The executable picker is only exposed when setup is unavailable. |
| Fault injection and cross-platform | NOT TESTED | Disk full, deliberately failing temp cleanup, read-only/permission-denied matrix, install flows, Intel macOS, Windows and Linux. |

Automated checks passed: `cargo fmt --check`, `cargo check --locked --offline`, `cargo clippy --locked --offline --all-targets -- -D warnings`, default `cargo test` (17 pass, 12 integration tests intentionally ignored), full `cargo test -- --include-ignored` with ffmpeg-full (29 pass, zero ignored), and `cargo build --release --locked --offline`. The bundle passed plist lint, executable comparison before signing, and strict signature verification after signing. Linked libraries inspected with `otool -L` were Apple system libraries/frameworks; runtime media helpers remain external.

## Defects fixed

| Severity | Symptom and root cause | Fix and verification |
| --- | --- | --- |
| High | A truncated PNG could appear as video. ffprobe returned a recognized image stream but decoded zero frames, falling into the video branch. | `src/core/media.rs`, `src/core/integration.rs`: reject a zero-frame image probe. Reproduced with ffprobe and retested in the GUI and integration suite. |
| High | A replaced file could be processed with stale stream, dimensions or copy decisions. The worker only looked up its current size. | `src/core/media.rs`, `src/app.rs`, `src/core/operation.rs`, `src/core/integration.rs`: compare source size and modification time before work; prompt to reopen changed or missing input. Integration verifies replacement and deletion. This is an ordinary change check, not an integrity guarantee against concurrent writes. |
| Medium | Timing and image controls could carry over across tools because defaults were attached to import rather than action changes. | `src/app.rs`: initialize action settings on entry, preserve settings across format switches. GUI verified the GIF-to-Trim boundary with the same loaded clip. |
| Medium | Divider could move into blank space on narrower images because drag bounds used the entire viewport. | `ui/compare.slint`: map interaction to the visible image canvas and clip at that position. GUI drag and zoom kept the divider position; narrow-window and portrait bounds were not separately exercised. |
| Medium | Failed preview told users they could still export even when the same settings could not succeed. | `ui/app.slint`: direct users to diagnostics or settings. Confirmed on a real 1-byte target, which fails with a minimum observed trial of 18,374 bytes. |
| Low | Original A/B playback only captured a path; a settings change could drop its temp directory during handoff. | `src/app.rs`: retain the preview for the open operation and report worker-start failures. Compile/clippy and integration pass; timing race not reproduced manually. |
| High (distribution) | Local bundle was assembled manually; strict signature verification failed after resources were added. | `scripts/bundle-macos.sh`, `README.md`: reproducible plist, license notices and ad-hoc bundle signing. Strict verification passes; this does not supply Developer ID signing or notarization. |

## Performance measurements

Sampled `ps` for 147.5 seconds (1,053 samples, roughly every 140 ms). RSS is resident process memory. Child figures include only FFmpeg; summed RSS may count shared pages twice. These are observed sample maxima, not guaranteed peaks, and very short encodes can fall between samples.

| Measurement | Observed |
| --- | --- |
| Idle window, first 30 samples | FFlume median 105.4 MiB RSS, median 0.0% CPU; no FFmpeg child. |
| Image preview/export sampling window | FFlume max 118.5 MiB RSS; FFmpeg max 12.8 MiB; max same-sample sum 124.5 MiB. |
| Short video/audio sampling window | FFlume max 126.5 MiB RSS; FFmpeg max 16.2 MiB; max same-sample sum 142.7 MiB. Sparse child samples. |
| 24-second 1080p input, export canceled | FFlume max 108.4 MiB RSS; FFmpeg max 539.6 MiB; max same-sample sum 646.7 MiB. |
| Across the sample | FFlume max CPU 17.7%; FFmpeg max CPU 36.7%, as reported by `ps`. Per-operation CPU and sustained playback were not isolated. |
| Release binary / app bundle | About 13.2 MiB each; exact bundle includes license files and the ad-hoc signature. |
| Startup | App launched and reached Home. Cold/warm application-only timing was not isolated from desktop automation overhead. |
| Temporary disk | WebP artifact 6,702 bytes and JPEG result 21,508 bytes. No staging entry after the sampled cancellation; peak temporary disk use was not measured. |
| LoMux | NOT TESTED; no comparable benchmark was run. |

## Remaining release work

- For public macOS distribution: build and test the signed/notarized artifact and clean-machine install; add a Finder/Dock app icon and release workflow. The local beta package is ad-hoc signed and ships FFlume/Geist notices; Slint 1.18.1 is used under its GPL-3.0 option. A complete third-party notice and distribution review remains outstanding.
- Before a broad beta, cover drag-and-drop, reduced-size windows, full keyboard and VoiceOver journeys, and missing/changed tools on a clean machine. Cold/warm startup and worst-case processing memory remain unmeasured.
- No critical data-loss or processing defect remains open from the tested cases. Batch frame/PDF publication stages outputs before publishing, but individual outputs can be committed if a later publication fails; the error reports how many were saved. Transactional rollback was not exercised or claimed.

**Recommendation: READY FOR BETA**, limited to local testing on the verified macOS arm64 configuration.
