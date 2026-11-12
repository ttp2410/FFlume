use std::{
    fs, io,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Instant,
};

use slint::{
    ComponentHandle, ModelRc, SharedString, VecModel,
    winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent},
};

use crate::catalog::{Category, Handler, TOOLS, Tool};

use crate::core::{
    destination::{self, Preferences},
    ffmpeg::{self, Availability, Tools},
    image::{self, Artifact, Dimensions, Request, Resize},
    job::{self, Outcome, Progress},
    media::{Kind, Media},
    operation::{self, Action, Format, Quality, Range},
    preview::{self, Preview},
};

slint::include_modules!();

mod pdf_ui;

type SharedState = Arc<Mutex<State>>;

#[derive(Default)]
struct State {
    tools: Option<Tools>,
    pdf: pdf_ui::Session,
    media: Option<Media>,
    selected_tool: Option<&'static Tool>,
    formats: Vec<Format>,
    actions: Vec<Action>,
    action: Option<Action>,
    format: Option<Format>,
    output_folder: Option<PathBuf>,
    preferences: Preferences,
    preferences_dirty: bool,
    analysis: Option<Arc<Artifact>>,
    media_preview: Option<Arc<Preview>>,
    original_audio_preview: Option<PathBuf>,
    had_analysis: bool,
    exported: Option<PathBuf>,
    running: Option<Running>,
}

struct Running {
    cancel: mpsc::Sender<()>,
    worker: thread::JoinHandle<()>,
    preview: bool,
}

pub fn run() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let state = SharedState::default();
    pdf_ui::bind(&ui, &state);
    ui.set_categories(model(
        std::iter::once("All categories").chain(Category::ALL.into_iter().map(Category::label)),
    ));
    if let Ok(state) = state.lock() {
        state.show_catalog(&ui);
    }
    ui.on_filter_tools(callback(&ui, &state, |ui, state| {
        if let Ok(state) = state.lock() {
            state.show_catalog(ui);
        }
    }));
    let window = ui.as_weak();
    let tool_state = state.clone();
    ui.on_choose_tool(move |id| {
        if let Some(ui) = window.upgrade()
            && !ui.get_busy()
            && !ui.get_running()
            && let Some(tool) = TOOLS.iter().find(|tool| tool.id == id.as_str())
        {
            if let Ok(mut state) = tool_state.lock() {
                state.open_tool(&ui, tool);
            }
            if matches!(tool.handler, Handler::Pdf) && !ui.get_pdf_ready() {
                pdf_ui::setup(&ui, &tool_state, false);
            }
        }
    });
    ui.on_back_to_tools(callback(&ui, &state, |ui, state| {
        if !ui.get_busy()
            && !ui.get_running()
            && let Ok(mut state) = state.lock()
        {
            state.selected_tool = None;
            state.refresh_workflow(ui);
            state.show_catalog(ui);
            message(ui, "Ready to go", false);
        }
    }));
    ui.on_browse(callback(&ui, &state, browse));
    ui.on_remove(callback(&ui, &state, |ui, state| {
        if !ui.get_busy()
            && !ui.get_running()
            && let Ok(mut state) = state.lock()
        {
            state.clear_media(ui);
            message(ui, "Ready to go", false);
        }
    }));
    ui.on_choose_destination(callback(&ui, &state, choose_destination));
    ui.on_export_file(callback(&ui, &state, |ui, state| {
        process(ui, state, ProcessMode::Export)
    }));
    ui.on_keep_both(callback(&ui, &state, |ui, state| {
        process(ui, state, ProcessMode::KeepBoth)
    }));
    ui.on_analyze(callback(&ui, &state, |ui, state| {
        process(ui, state, ProcessMode::Preview)
    }));
    ui.on_open_media_preview(callback(&ui, &state, open_media_preview));
    ui.on_listen_original(callback(&ui, &state, |ui, state| {
        let playback = state.lock().ok().and_then(|state| {
            Some((
                state.media_preview.clone()?,
                state.original_audio_preview.clone()?,
            ))
        });
        if let Some((preview, path)) = playback
            && let Err(error) = background(
                ui,
                "listen-original",
                move || {
                    let result = play_preview(&path);
                    drop(preview);
                    result
                },
                |ui, result| {
                    if let Err(error) = result {
                        message_with_details(ui, "Couldn’t play original", error.to_string());
                    }
                },
            )
        {
            message_with_details(ui, "Couldn’t play original", error.to_string());
        }
    }));
    ui.on_preview_zoom(callback(&ui, &state, |ui, state| {
        process(ui, state, ProcessMode::Detail)
    }));
    ui.on_install_ffmpeg(callback(&ui, &state, install_ffmpeg));
    ui.on_pick_ffmpeg(callback(&ui, &state, pick_ffmpeg));
    ui.on_reset_ffmpeg(callback(&ui, &state, |ui, state| {
        reload_tools(ui, state, None)
    }));
    ui.on_settings_changed(callback(&ui, &state, |ui, state| {
        if let Ok(mut state) = state.lock() {
            state.invalidate_analysis(ui);
            state.refresh_preview(ui);
        }
    }));
    ui.on_cancel_export(callback(&ui, &state, |ui, state| {
        if let Ok(state) = state.lock()
            && let Some(running) = &state.running
        {
            ui.set_cancelling(true);
            message(
                ui,
                if running.preview {
                    "Stopping preview..."
                } else {
                    "Stopping export..."
                },
                false,
            );
            let _ = running.cancel.send(());
        }
    }));
    ui.on_open_folder(callback(&ui, &state, |ui, state| {
        open_output(ui, state, true)
    }));
    ui.on_open_export(callback(&ui, &state, |ui, state| {
        open_output(ui, state, false)
    }));
    ui.on_ffmpeg_guide(callback(&ui, &state, |ui, _| {
        if let Err(error) = background(
            ui,
            "setup-guide",
            || open_setup_page("https://ffmpeg.org/download.html"),
            |ui, result| {
                if let Err(error) = result {
                    message_with_details(ui, "Couldn’t open the setup guide", error);
                }
            },
        ) {
            message_with_details(ui, "Couldn’t open the setup guide", error.to_string());
        }
    }));

    let select_action = callback(&ui, &state, |ui, state| {
        if let Ok(mut state) = state.lock() {
            let index = ui.get_action_index();
            if let Some(&action) = state.actions.get(index as usize) {
                state.set_action(ui, action);
            }
        }
    });
    ui.on_select_action(select_action);
    ui.on_select_format(callback(&ui, &state, |ui, state| {
        if let Ok(mut state) = state.lock()
            && let Some(&format) = state.formats.get(ui.get_format_index() as usize)
        {
            state.set_format(ui, format);
        }
    }));

    ui.set_busy(true);
    let detection_state = state.clone();
    if let Err(error) = background(
        &ui,
        "media-tools",
        || {
            let preferences = Preferences::load();
            let tools = detect_tools(preferences.ffmpeg.as_deref());
            (preferences, tools, crate::core::pdf::Backend::detect())
        },
        move |ui, (preferences, tools, pdf_backend)| {
            ui.set_busy(false);
            if let Ok(mut state) = detection_state.lock() {
                state.preferences = preferences;
                state.tools = Some(tools);
                state.pdf.set_backend(ui, pdf_backend);
                state.show_tools(ui);
                state.show_catalog(ui);
            }
        },
    ) {
        ui.set_busy(false);
        ui.set_tool_status("Tool check unavailable".into());
        ui.set_tools_checked(true);
        message(
            &ui,
            format!("Could not check the media tools: {error}"),
            true,
        );
    }

    ui.show()?;
    install_file_drop(&ui, &state);
    let result = slint::run_event_loop();
    let hidden = ui.hide();
    let running = state.lock().ok().and_then(|mut state| state.running.take());
    if let Some(running) = running {
        let _ = running.cancel.send(());
        let _ = running.worker.join();
    }
    if let Ok(state) = state.lock()
        && state.preferences_dirty
        && let Err(error) = state.preferences.save()
    {
        eprintln!("Could not save FFlume preferences: {error}");
    }
    result.and(hidden)
}

impl State {
    fn show_catalog(&self, ui: &AppWindow) {
        let query = ui.get_search_query();
        let category = ui
            .get_category_index()
            .checked_sub(1)
            .and_then(|index| Category::ALL.get(index as usize))
            .copied();
        let suggested = ui
            .get_suggested_only()
            .then_some(self.media.as_ref())
            .flatten();
        let pdf_matches = TOOLS.iter().any(|tool| {
            matches!(tool.handler, Handler::Pdf) && tool.matches(category, query.as_str())
        });
        ui.set_pdf_setup_available(
            ui.get_pdf_checked()
                && !ui.get_pdf_ready()
                && suggested.is_none()
                && (category == Some(Category::Documents) || !query.trim().is_empty())
                && pdf_matches,
        );
        let sections: Vec<_> = Category::ALL
            .into_iter()
            .filter_map(|section| {
                let cards: Vec<_> = TOOLS
                    .iter()
                    .filter(|tool| tool.category == section)
                    .filter(|tool| tool.matches(category, query.as_str()))
                    .filter_map(|tool| {
                        if query.trim().is_empty() && category.is_none() && !tool.featured {
                            return None;
                        }
                        if suggested.is_some_and(|media| !tool.accepts(media)) {
                            return None;
                        }
                        if matches!(tool.handler, Handler::Planned)
                            || (matches!(tool.handler, Handler::Pdf) && !ui.get_pdf_ready())
                            || !tool.status(self.tools.as_ref(), suggested).is_empty()
                        {
                            return None;
                        }
                        Some(ToolCardData {
                            id: tool.id.into(),
                            title: tool.name.into(),
                            description: tool.description.into(),
                            icon_path: tool.icon_path.into(),
                            status: SharedString::default(),
                        })
                    })
                    .collect();
                if cards.is_empty() {
                    return None;
                }
                let rows = cards
                    .chunks(2)
                    .map(|pair| ToolRow {
                        first: pair[0].clone(),
                        second: pair.get(1).cloned().unwrap_or_default(),
                    })
                    .collect::<Vec<_>>();
                Some(ToolSection {
                    title: section.label().into(),
                    rows: ModelRc::new(VecModel::from(rows)),
                })
            })
            .collect();
        ui.set_tool_sections(ModelRc::new(VecModel::from(sections)));
    }

    fn open_tool(&mut self, ui: &AppWindow, tool: &'static Tool) {
        let selected = self.media.as_ref().filter(|_| ui.get_suggested_only());
        let pdf_tool = matches!(tool.handler, Handler::Pdf);
        if !pdf_tool
            && (selected.is_some_and(|media| !tool.accepts(media))
                || !tool.status(self.tools.as_ref(), selected).is_empty())
        {
            return;
        }
        self.selected_tool = Some(tool);
        ui.set_advanced_open(matches!(tool.handler, Handler::Optimize));
        self.refresh_workflow(ui);
        if !pdf_tool && !ui.get_can_process() && !(ui.get_inspector() && ui.get_compatible_input())
        {
            message(ui, ui.get_input_hint(), false);
        }
    }

    fn refresh_workflow(&mut self, ui: &AppWindow) {
        self.action = None;
        self.format = None;
        self.formats.clear();
        self.actions.clear();
        self.had_analysis = false;
        self.invalidate_analysis(ui);
        self.clear_result(ui);
        ui.set_can_process(false);
        ui.set_compatible_input(false);
        ui.set_inspector(false);
        let Some(tool) = self.selected_tool else {
            ui.set_active_tool("".into());
            ui.set_source_profile("Media".into());
            return;
        };
        let profile = match (tool.handler, tool.input) {
            (Handler::Pdf, _) => "PDF",
            (_, [kind]) => kind.label(),
            _ => self
                .media
                .as_ref()
                .map_or("Media", |media| media.kind.label()),
        };
        ui.set_source_profile(profile.into());
        let hint = match tool.input {
            [Kind::Video] if tool.actions().contains(&Action::Extract) => {
                "Choose a video with an audio track."
            }
            [Kind::Video] => "Choose a video file.",
            [Kind::Image] => "Choose a still image.",
            [Kind::Audio] => "Choose an audio file.",
            _ => "Choose a video, audio file, or image.",
        };
        ui.set_active_tool(tool.name.into());
        ui.set_input_hint(hint.into());
        ui.set_inspector(matches!(tool.handler, Handler::Inspect));
        let Some(media) = self.media.as_ref().filter(|media| tool.accepts(media)) else {
            return;
        };
        ui.set_compatible_input(true);
        let facts = source_facts(media);
        ui.set_source_facts(model(facts.iter().map(String::as_str)));
        if matches!(tool.handler, Handler::Inspect) {
            ui.set_inspection(inspect_details(media).into());
            message(ui, "Media inspected.", false);
            return;
        }
        let Some(tools) = &self.tools else { return };
        self.actions = tool
            .actions()
            .iter()
            .copied()
            .filter(|&action| !tool.formats(tools, action, Some(media)).is_empty())
            .collect();
        ui.set_actions(model(self.actions.iter().map(|action| action.label())));
        if let Some(&action) = self.actions.first() {
            ui.set_can_process(true);
            ui.set_action_index(0);
            self.set_action(ui, action);
        } else {
            ui.set_input_hint(
                "This file needs a format your FFmpeg installation doesn’t support.".into(),
            );
        }
    }

    fn show_tools(&self, ui: &AppWindow) {
        let Some(tools) = &self.tools else { return };
        let ffmpeg = tools.ffmpeg.tool();
        let ffprobe = tools.ffprobe.tool();
        let ready = ffmpeg.is_some() && ffprobe.is_some();
        ui.set_tools_checked(true);
        ui.set_can_browse(ffprobe.is_some());
        ui.set_custom_tool(self.preferences.ffmpeg.is_some());
        ui.set_tools_ready(ready);
        ui.set_tool_status(
            match (ffmpeg, ffprobe) {
                (Some(tool), Some(_)) => format!("FFmpeg {}", tool.version),
                (None, _) => "FFmpeg unavailable".into(),
                (_, None) => "ffprobe unavailable".into(),
            }
            .into(),
        );
        ui.set_ffprobe_version(ffprobe.map_or("", |tool| tool.version.as_str()).into());
    }

    fn set_media(&mut self, ui: &AppWindow, media: Media) {
        if let Some(tool) = self.selected_tool {
            if !tool.accepts(&media) {
                message(
                    ui,
                    format!(
                        "{} is incompatible with this file. {}",
                        tool.name,
                        ui.get_input_hint()
                    ),
                    true,
                );
                return;
            }
            let status = tool.status(self.tools.as_ref(), Some(&media));
            if !status.is_empty() {
                message(ui, format!("{}: {status} for this file.", tool.name), true);
                return;
            }
        }
        let from_home = self.selected_tool.is_none();
        ui.set_file_name(path_name(&media.path).as_ref().into());
        ui.set_export_label(format!("Export {}", media.kind.label()).into());
        ui.set_is_image(media.kind == Kind::Image);
        ui.set_seek_limit(media.duration.unwrap_or(0.0).min(f32::MAX as f64) as f32);
        ui.set_seek_time(0.0);
        ui.set_divider(0.5);
        ui.set_detail_requested(false);
        ui.set_file_kind(media.kind.label().into());
        ui.set_file_details(home_file_details(&media).into());
        self.output_folder = Some(destination::folder_for(
            &media.path,
            self.preferences.folder.as_deref(),
        ));
        self.media = Some(media);
        self.show_folder(ui);
        self.refresh_workflow(ui);
        if from_home {
            ui.set_search_query("".into());
            ui.set_category_index(0);
            ui.set_suggested_only(true);
            message(ui, "Ready to go", false);
        }
        self.show_catalog(ui);
    }

    fn clear_media(&mut self, ui: &AppWindow) {
        self.media = None;
        self.output_folder = None;
        ui.set_file_name("".into());
        ui.set_suggested_only(false);
        self.refresh_workflow(ui);
        self.show_catalog(ui);
    }

    fn clear_result(&mut self, ui: &AppWindow) {
        self.exported = None;
        ui.set_output_ready(false);
        ui.set_result_summary("".into());
        ui.set_conflict(false);
    }

    fn set_action(&mut self, ui: &AppWindow, action: Action) {
        if self.action == Some(action) {
            return;
        }
        let (Some(tool), Some(tools)) = (self.selected_tool, &self.tools) else {
            return;
        };
        let formats = tool.formats(tools, action, self.media.as_ref());
        let Some(&format) = formats.first() else {
            return;
        };
        self.formats = formats;
        self.action = Some(action);
        if let Some(media) = &self.media {
            let dimensions = media.video.as_ref().and_then(|stream| stream.dimensions());
            ui.set_resize_width(
                dimensions
                    .map(|(w, _)| w.to_string())
                    .unwrap_or_default()
                    .into(),
            );
            ui.set_resize_height(
                dimensions
                    .map(|(_, h)| h.to_string())
                    .unwrap_or_default()
                    .into(),
            );
            let end = if action == Action::Gif {
                Some(media.duration.unwrap_or(5.0).min(5.0))
            } else {
                media.duration
            };
            ui.set_trim_start("0".into());
            ui.set_trim_end(
                end.map(|seconds| format!("{:.6}", (seconds * 1_000_000.0).floor() / 1_000_000.0))
                    .unwrap_or_default()
                    .into(),
            );
        }
        ui.set_frame_series(false);
        ui.set_frame_interval("1".into());
        ui.set_scale_value("100".into());
        ui.set_lock_aspect(true);
        ui.set_custom_quality(80.0);
        ui.set_divider(0.5);
        ui.set_seek_time(0.0);
        ui.set_destination_name("".into());
        ui.set_resize_mode(if action == Action::Resize { 1 } else { 0 });
        ui.set_target_size("".into());
        ui.set_lossless(false);
        ui.set_allow_shrink(false);
        ui.set_quality_index(1);
        ui.set_formats(model(self.formats.iter().map(|format| format.label())));
        ui.set_format_index(0);
        self.set_format(ui, format);
        message(ui, "Ready to go", false);
    }

    fn set_format(&mut self, ui: &AppWindow, format: Format) {
        let (Some(media), Some(action)) = (&self.media, self.action) else {
            return;
        };
        self.format = Some(format);
        let name = ui.get_destination_name();
        if name.is_empty() {
            let suggested = operation::suggested_path(&media.path, action, format);
            let name: String = path_name(&suggested)
                .chars()
                .map(|c| {
                    if c.is_control() || "/\\:*?\"<>|".contains(c) {
                        '_'
                    } else {
                        c
                    }
                })
                .collect();
            ui.set_destination_name(
                destination::filename(&name, format)
                    .unwrap_or_else(|_| format!("output.{}", format.extension()))
                    .into(),
            );
        } else if let Ok(name) = destination::filename(name.as_str(), format) {
            ui.set_destination_name(name.into());
        }
        ui.set_show_trim(matches!(
            action,
            Action::Trim | Action::Gif | Action::TrimAudio
        ));
        ui.set_show_frames(action == Action::Frames);
        ui.set_show_timeline(matches!(
            action,
            Action::Frames | Action::Trim | Action::Compress
        ));
        ui.set_preview_enabled(
            action != Action::ConvertVideo
                || operation::plan(media, action, format, Quality::Balanced, None, None).is_ok_and(
                    |plan| {
                        plan.args.windows(2).any(|pair| {
                            pair[0].to_string_lossy().starts_with("-c:") && pair[1] != "copy"
                        })
                    },
                ),
        );
        ui.set_export_label(
            match action {
                Action::Gif => "Create GIF",
                Action::Frames => "Export frames",
                _ if media.kind == Kind::Audio || action == Action::Extract => "Export audio",
                _ if media.kind == Kind::Image => "Export image",
                _ => "Export video",
            }
            .into(),
        );
        ui.set_is_webp(format == Format::Webp);
        ui.set_lossy_image(matches!(format, Format::Jpeg | Format::Webp));
        ui.set_show_quality(
            action == Action::Compress
                || (media.kind == Kind::Image && matches!(format, Format::Jpeg | Format::Webp)),
        );
        let hint = match action {
            Action::Compress => "Compression keeps your audio tracks.",
            Action::ConvertVideo => "FFlume converts only the tracks that need it.",
            Action::Extract => "Exports the first audio track as an MP3.",
            Action::Trim => "Fast trim may start or end at a nearby frame.",
            Action::Resize => "Set the new size in pixels or use a percentage.",
            Action::Wav => "Converts the first audio track to the selected format.",
            Action::Gif => "Creates a looping GIF at 12 fps, up to 640 pixels wide.",
            Action::Frames => "Save one frame at a timestamp or extract a sequence at intervals.",
            Action::TrimAudio => "Save the selected part of the first audio track.",
            Action::NormalizeAudio => "Adjusts the first audio track toward −16 LUFS.",
            Action::Mp3 => "Exports the first audio track as an MP3.",
            Action::ConvertImage if format == Format::Png => "PNG uses lossless compression.",
            Action::ConvertImage => "Choose the output quality and any resolution changes.",
        };
        ui.set_action_hint(hint.into());
        self.invalidate_analysis(ui);
        self.refresh_preview(ui);
    }

    fn show_folder(&self, ui: &AppWindow) {
        if let Some(folder) = &self.output_folder {
            ui.set_destination(folder.to_string_lossy().as_ref().into());
        }
    }

    fn invalidate_analysis(&mut self, ui: &AppWindow) {
        self.analysis = None;
        self.media_preview = None;
        self.original_audio_preview = None;
        ui.set_audio_ab(false);
        ui.set_gif_frames(ModelRc::default());
        ui.set_detail_original(slint::Image::default());
        ui.set_detail_converted(slint::Image::default());
        ui.set_detail_requested(false);
        ui.set_has_single_preview(false);
        ui.set_preview_info("".into());
        ui.set_preview_state(
            if self.had_analysis {
                "Outdated"
            } else {
                "Not generated"
            }
            .into(),
        );
        ui.set_media_preview_ready(false);
        ui.set_has_image_comparison(false);
        ui.set_preview_original(slint::Image::default());
        ui.set_preview_converted(slint::Image::default());
        ui.set_conflict(false);
        ui.set_preview_size_label("Preview size".into());
        ui.set_preview_size("—".into());
    }

    fn refresh_preview(&self, ui: &AppWindow) {
        let (Some(media), Some(action), Some(format), Some(tools)) =
            (&self.media, self.action, self.format, &self.tools)
        else {
            return;
        };
        let mut details = vec![format.label().to_owned()];
        ui.set_original_size(format_size(media.bytes).into());
        ui.set_alpha_warning(media.kind == Kind::Image && image::alpha_warning(media, format));
        if media.kind == Kind::Image {
            if let Ok(request) = image_request(ui, media, format) {
                if request.target.is_some() {
                    details.push("Target-size search".into());
                } else if request.lossless {
                    details.push("Lossless".into());
                } else {
                    details.push(format!("Quality {}", request.quality));
                }
                if request.original != request.dimensions {
                    details.push(format!("{} → {}", request.original, request.dimensions));
                } else {
                    details.push(request.original.to_string());
                }
                if request.copies_source() {
                    ui.set_preview_size_label("Output size".into());
                    ui.set_preview_size(
                        format!("{} · 0% change (original copied)", format_size(media.bytes))
                            .into(),
                    );
                }
            }
        } else if action == Action::Wav
            && format == Format::Wav
            && let Some(audio) = media.audio.first()
            && tools.has_encoder("pcm_s16le")
            && let (Some(seconds), Some(channels), Some(rate)) = (
                audio.duration().or(media.duration),
                audio.channels,
                audio.sample_rate(),
            )
        {
            let bytes = (seconds * f64::from(channels) * f64::from(rate) * 2.0) as u64;
            ui.set_preview_size_label("Estimated size".into());
            ui.set_preview_size(
                format!(
                    "≈ {} · {}",
                    format_size(bytes),
                    savings_label(media.bytes, bytes)
                )
                .into(),
            );
        } else if action == Action::Compress {
            details.push(quality_label(ui.get_quality_index()).into());
        }
        if action == Action::Compress {
            let text = media.duration.map_or_else(
                || "Comparison frame".into(),
                |duration| format!("Comparison frame · {duration:.2} s video"),
            );
            ui.set_timeline_duration(text.into());
        } else if matches!(action, Action::Trim | Action::Frames) {
            let start = ui.get_trim_start();
            let end = ui.get_trim_end();
            let range = Range::parse(start.as_str(), end.as_str(), media.duration).ok();
            let text = if action == Action::Frames && !ui.get_frame_series() {
                format!("Frame time: {start}")
            } else if let Some(range) = range {
                format!("{start} → {end} · {:.2} s", range.duration())
            } else {
                format!("{start} → {end}")
            };
            ui.set_timeline_duration(text.into());
        }
        ui.set_preview_details(details.join("  ·  ").into());
        ui.set_recommendation(recommendation(media, tools, action).into());
    }
}

fn install_file_drop(ui: &AppWindow, state: &SharedState) {
    let enabled = cfg!(any(target_os = "macos", windows, target_os = "linux"));
    ui.set_file_drops(enabled);
    if !enabled {
        return;
    }
    let window = ui.as_weak();
    let state = state.clone();
    ui.window().on_winit_window_event(move |_, event| {
        if let Some(ui) = window.upgrade() {
            match event {
                WindowEvent::HoveredFile(_) => {
                    ui.set_drag_hover(
                        (ui.get_can_browse() || ui.get_pdf_ready())
                            && !ui.get_busy()
                            && !ui.get_running(),
                    );
                }
                WindowEvent::HoveredFileCancelled => ui.set_drag_hover(false),
                WindowEvent::DroppedFile(path) => {
                    ui.set_drag_hover(false);
                    import_media(&ui, &state, path.clone());
                }
                _ => {}
            }
        }
        EventResult::Propagate
    });
}

fn browse(ui: &AppWindow, state: &SharedState) {
    if ui.get_busy() || ui.get_running() || !ui.get_can_browse() {
        return;
    }
    let title = if ui.get_active_tool().is_empty() {
        "Choose a media file"
    } else {
        match ui.get_source_profile().as_str() {
            "Image" => "Choose an image",
            "Audio" => "Choose an audio file",
            "Video" => "Choose a video",
            _ => "Choose a media file",
        }
    };
    let dialog = rfd::AsyncFileDialog::new()
        .set_title(title)
        .set_parent(&ui.window().window_handle());
    pick_path(ui, state, dialog, false, import_media);
}

fn import_media(ui: &AppWindow, state: &SharedState, path: PathBuf) {
    if ui.get_busy() || ui.get_running() {
        return;
    }
    let pdf_path = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
    if ui.get_active_tool().is_empty()
        && pdf_path
        && let Ok(mut state) = state.lock()
        && let Some(tool) = TOOLS
            .iter()
            .find(|tool| matches!(tool.handler, Handler::Pdf))
    {
        state.open_tool(ui, tool);
    }
    if ui.get_source_profile() == "PDF" && !ui.get_active_tool().is_empty() {
        pdf_ui::load(ui, state, path);
        return;
    }
    let Some(tool) = state
        .lock()
        .ok()
        .and_then(|state| state.tools.as_ref()?.ffprobe.tool().cloned())
    else {
        return;
    };
    ui.set_busy(true);
    message(ui, "Reading file details...", false);
    let state = state.clone();
    if let Err(error) = background(
        ui,
        "ffprobe",
        move || Media::probe(&tool.path, path),
        move |ui, result| {
            ui.set_busy(false);
            if let Ok(mut state) = state.lock() {
                match result {
                    Ok(media) => state.set_media(ui, media),
                    Err(error) => message_with_details(ui, "Couldn’t read this file", error),
                }
            }
        },
    ) {
        ui.set_busy(false);
        message(
            ui,
            format!("Could not start media inspection: {error}"),
            true,
        );
    }
}

fn choose_destination(ui: &AppWindow, state: &SharedState) {
    if ui.get_busy() || ui.get_running() {
        return;
    }
    let Some(folder) = state
        .lock()
        .ok()
        .and_then(|state| state.output_folder.clone())
    else {
        return;
    };
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Choose an output folder")
        .set_parent(&ui.window().window_handle())
        .set_directory(folder);
    pick_path(ui, state, dialog, true, |ui, state, folder| {
        if let Ok(mut state) = state.lock() {
            state.preferences.folder = Some(folder.clone());
            state.preferences_dirty = true;
            state.output_folder = Some(folder);
            state.show_folder(ui);
            ui.set_conflict(false);
            message(ui, "Output folder updated.", false);
        }
    });
}

fn install_ffmpeg(ui: &AppWindow, state: &SharedState) {
    if ui.get_busy() || ui.get_running() || ui.get_tools_ready() {
        return;
    }
    ui.set_busy(true);
    ui.set_installing(true);
    let state = state.clone();
    if let Err(error) = background(ui, "install-ffmpeg", setup_ffmpeg, move |ui, result| {
        ui.set_busy(false);
        ui.set_installing(false);
        match result {
            Ok(SetupResult::Installed(tools)) => {
                if let Ok(mut state) = state.lock() {
                    state.preferences_dirty |= state.preferences.ffmpeg.take().is_some();
                    state.tools = Some(*tools);
                    state.show_tools(ui);
                    state.refresh_workflow(ui);
                    state.show_catalog(ui);
                    message(ui, "FFmpeg is ready.", false);
                }
            }
            Ok(SetupResult::Instructions(hint)) => message(ui, hint, false),
            Err(error) => message(ui, error, true),
        }
    }) {
        ui.set_busy(false);
        ui.set_installing(false);
        message(ui, format!("Could not start FFmpeg setup: {error}"), true);
    }
}

enum SetupResult {
    Installed(Box<Tools>),
    Instructions(&'static str),
}

fn setup_ffmpeg() -> Result<SetupResult, String> {
    if cfg!(target_os = "macos") {
        if let Some(brew) = find_homebrew() {
            install_with_homebrew(&brew, "ffmpeg")?;
            let tools = Tools::detect();
            if tools.ffmpeg.tool().is_none() || tools.ffprobe.tool().is_none() {
                return Err("Homebrew finished, but FFmpeg or ffprobe could not be detected. Choose the installed FFmpeg executable.".into());
            }
            return Ok(SetupResult::Installed(Box::new(tools)));
        }
        open_setup_page("https://brew.sh/")?;
        return Ok(SetupResult::Instructions(
            "Install Homebrew, then return and click Install FFmpeg again.",
        ));
    }
    open_setup_page("https://ffmpeg.org/download.html")?;
    Ok(SetupResult::Instructions(
        "Install FFmpeg and ffprobe, then click Check Again or choose the executable.",
    ))
}

fn install_with_homebrew(brew: &Path, formula: &str) -> Result<(), String> {
    let output = ffmpeg::command(brew)
        .args(["install", formula])
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .output()
        .map_err(|error| format!("Could not start Homebrew: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .find(|line| line.starts_with("Error:"))
            .or_else(|| stderr.lines().rev().find(|line| !line.trim().is_empty()))
            .unwrap_or("Open Terminal to run the installation for more details.");
        return Err(format!(
            "Homebrew install failed. {}",
            detail.chars().take(220).collect::<String>()
        ));
    }
    Ok(())
}

fn find_homebrew() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ])
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("brew"))
        .find(|path| path.is_file())
}

fn open_setup_page(url: &str) -> Result<(), String> {
    let mut launcher = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut command = Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler");
        command
    } else {
        Command::new("xdg-open")
    };
    let status = launcher
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("Could not open {url}: {error}"))?;
    if !status.success() {
        return Err(format!("Could not open {url} ({status})."));
    }
    Ok(())
}

fn pick_ffmpeg(ui: &AppWindow, state: &SharedState) {
    if ui.get_busy() || ui.get_running() {
        return;
    }
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Choose the FFmpeg executable")
        .set_parent(&ui.window().window_handle());
    pick_path(ui, state, dialog, false, |ui, state, path| {
        reload_tools(ui, state, Some(path))
    });
}

fn pick_path(
    ui: &AppWindow,
    state: &SharedState,
    dialog: rfd::AsyncFileDialog,
    directory: bool,
    picked: impl FnOnce(&AppWindow, &SharedState, PathBuf) + 'static,
) {
    ui.set_busy(true);
    let window = ui.as_weak();
    let state = state.clone();
    if let Err(error) = slint::spawn_local(async move {
        let chosen = if directory {
            dialog.pick_folder().await
        } else {
            dialog.pick_file().await
        };
        let Some(ui) = window.upgrade() else { return };
        ui.set_busy(false);
        if let Some(file) = chosen {
            picked(&ui, &state, file.path().to_path_buf());
        }
    }) {
        ui.set_busy(false);
        message(ui, format!("Could not open the picker: {error}"), true);
    }
}

fn detect_tools(preferred: Option<&Path>) -> Tools {
    preferred.map_or_else(Tools::detect, |path| Tools::with_executable(Some(path)))
}

fn reload_tools(ui: &AppWindow, state: &SharedState, preferred: Option<PathBuf>) {
    if ui.get_busy() || ui.get_running() {
        return;
    }
    ui.set_busy(true);
    let state = state.clone();
    let work_path = preferred.clone();
    if let Err(error) = background(
        ui,
        "media-tools",
        move || detect_tools(work_path.as_deref()),
        move |ui, tools| {
            ui.set_busy(false);
            if preferred.is_some() && tools.ffmpeg.tool().is_none() {
                message(ui, tool_description("Selected FFmpeg", &tools.ffmpeg), true);
                return;
            }
            if let Ok(mut state) = state.lock() {
                state.preferences.ffmpeg = preferred;
                state.preferences_dirty = true;
                state.tools = Some(tools);
                state.show_tools(ui);
                state.refresh_workflow(ui);
                state.show_catalog(ui);
                if ui.get_tools_ready() {
                    message(ui, "FFmpeg is ready.", false);
                } else {
                    message(ui, "FFmpeg or ffprobe is still unavailable.", true);
                }
            }
        },
    ) {
        ui.set_busy(false);
        message(ui, format!("Could not check FFmpeg: {error}"), true);
    }
}

fn positive_setting(value: &str) -> Result<u32, &'static str> {
    value
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|n| (1..=16384).contains(n))
        .ok_or("Enter a value from 1 to 16,384 for the resolution.")
}

fn image_request(ui: &AppWindow, media: &Media, format: Format) -> Result<Request, &'static str> {
    let stream = media
        .video
        .as_ref()
        .ok_or("The source has no image stream.")?;
    let (width, height) = stream
        .dimensions()
        .ok_or("The source dimensions are unavailable.")?;
    let original = Dimensions(width, height);
    let resize = match ui.get_resize_mode() {
        1 if ui.get_lock_aspect() => {
            Resize::Width(positive_setting(ui.get_resize_width().as_str())?)
        }
        1 => Resize::Exact(
            positive_setting(ui.get_resize_width().as_str())?,
            positive_setting(ui.get_resize_height().as_str())?,
        ),
        2 => Resize::Height(positive_setting(ui.get_resize_height().as_str())?),
        3 => Resize::Percent(positive_setting(ui.get_scale_value().as_str())?),
        4 => Resize::Max(positive_setting(ui.get_scale_value().as_str())?),
        _ => Resize::Original,
    };
    let dimensions = resize.dimensions(original)?;
    if format == Format::Webp && dimensions.0.max(dimensions.1) > 16_383 {
        return Err("WebP supports dimensions up to 16,383 pixels. Reduce the output dimensions.");
    }
    let lossless = format == Format::Png || (format == Format::Webp && ui.get_lossless());
    let target = if lossless {
        None
    } else {
        image::parse_target(ui.get_target_size().as_str())?
    };
    let quality = match ui.get_quality_index() {
        0 => 92,
        2 => 55,
        3 => ui.get_custom_quality().round().clamp(0.0, 100.0) as u8,
        _ => 80,
    };
    Ok(Request {
        source: media.path.clone(),
        stream: stream.index,
        format,
        original,
        dimensions,
        quality,
        lossless,
        target,
        shrink: target.is_some() && ui.get_allow_shrink(),
        unchanged_png: format == Format::Png && media.plain_png,
    })
}

fn quality_label(index: i32) -> &'static str {
    match index {
        0 => "High quality",
        2 => "Smaller file",
        _ => "Balanced",
    }
}

fn savings_label(original: u64, output: u64) -> String {
    match image::saving_percent(original, output) {
        Some(value) if value >= 0.05 => format!("{value:.1}% saved"),
        Some(value) if value <= -0.05 => format!("{:.1}% larger", -value),
        Some(_) => "0% change".into(),
        None => "saving unavailable".into(),
    }
}

fn recommendation(media: &Media, tools: &Tools, action: Action) -> String {
    match media.kind {
        Kind::Image => {
            let Some(stream) = &media.video else {
                return String::new();
            };
            let mut tips = Vec::new();
            if stream.codec_name == "png" && tools.supports_webp() {
                tips.push("Try WebP to compare lossless and smaller lossy output.");
            } else if stream.codec_name == "mjpeg" {
                tips.push(if tools.supports_webp() {
                    "Compare JPEG quality or try WebP at the original resolution."
                } else {
                    "Compare JPEG quality at the original resolution."
                });
            }
            if stream.dimensions().is_some_and(|(w, h)| w.max(h) > 2048) {
                tips.push("If you do not need full resolution, try limiting the longest side.");
            }
            tips.join(" ")
        }
        Kind::Video if action == Action::ConvertVideo => {
            let can_copy = [Format::Mp4, Format::Mkv].into_iter().any(|format| {
                media
                    .video
                    .as_ref()
                    .is_some_and(|s| operation::copy_video(format, &s.codec_name))
                    && media
                        .audio
                        .iter()
                        .all(|s| operation::copy_audio(format, &s.codec_name))
            });
            if can_copy {
                "Compatible tracks can keep their original quality.".into()
            } else {
                String::new()
            }
        }
        Kind::Audio if action == Action::Wav => {
            if tools.has_encoder("pcm_s16le") && tools.has_encoder("libmp3lame") {
                "Choose MP3 for smaller files or WAV for uncompressed audio.".into()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

enum Work {
    Image(Request, Option<Arc<Artifact>>),
    Media(operation::Plan, bool),
}

type Thumbnail = slint::SharedPixelBuffer<slint::Rgba8Pixel>;

fn thumbnail(
    tool: &Path,
    source: &Path,
    stream: u32,
    seek: f32,
    detail: bool,
    cancel: &mpsc::Receiver<()>,
    update: impl Fn(Progress),
) -> Result<Option<Thumbnail>, String> {
    let output = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let args = [
        "-protocol_whitelist",
        "file",
        "-i",
        "",
        "-map",
        "",
        "-frames:v",
        "1",
        "-vf",
        "scale=w='max(1,round(iw*min(1,600/max(iw,ih))))':h='max(1,round(ih*min(1,600/max(iw,ih))))'",
        "-c:v",
        "png",
        "-pix_fmt",
        "rgba",
        "-update",
        "1",
        "-f",
        "image2",
    ];
    let mut args: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
    args[3] = source.as_os_str().into();
    args[5] = format!("0:{stream}").into();
    if detail {
        args[9] = "format=rgba,crop=w=min(600\\,iw):h=min(600\\,ih):exact=1".into();
    }
    if seek > 0.0 {
        args.splice(0..0, ["-ss".into(), seek.to_string().into()]);
    }
    let plan = operation::Plan {
        args,
        duration: None,
        format: Format::Png,
    };
    if !job::encode(tool, &plan, output.path(), cancel, update)? {
        return Ok(None);
    }
    let data = fs::read(output.path()).map_err(|e| e.to_string())?;
    let pixels = ::image::load_from_memory_with_format(&data, ::image::ImageFormat::Png)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    Ok(Some(Thumbnail::clone_from_slice(
        pixels.as_raw(),
        pixels.width(),
        pixels.height(),
    )))
}

// Decode only the bounded sample. The full-size GIF stays on disk.
fn gif_frames(
    tool: &Path,
    source: &Path,
    (width, height): (u32, u32),
    cancel: &mpsc::Receiver<()>,
    update: impl Fn(Progress),
) -> Result<Option<Vec<Thumbnail>>, String> {
    const EDGE: u32 = 256;
    const MAX_FRAMES: usize = 120;
    let longest = width.max(height).max(EDGE);
    let width = (width * EDGE / longest).max(1);
    let height = (height * EDGE / longest).max(1);
    let output = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let mut args = vec![
        "-protocol_whitelist".into(),
        "file".into(),
        "-i".into(),
        source.as_os_str().into(),
        "-vf".into(),
        format!("fps=12,scale={width}:{height}:flags=area").into(),
        "-frames:v".into(),
        MAX_FRAMES.to_string().into(),
    ];
    args.extend(
        [
            "-an",
            "-c:v",
            "rawvideo",
            "-threads:v",
            "1",
            "-pix_fmt",
            "rgba",
            "-f",
            "rawvideo",
        ]
        .map(Into::into),
    );
    let plan = operation::Plan {
        args,
        duration: None,
        format: Format::Gif,
    };
    if !job::encode(tool, &plan, output.path(), cancel, update)? {
        return Ok(None);
    }
    let file = fs::File::open(output.path()).map_err(|e| e.to_string())?;
    let frame_size = u64::from(width) * u64::from(height) * 4;
    let bytes = file.metadata().map_err(|e| e.to_string())?.len();
    let count = bytes / frame_size;
    if count == 0 || count > MAX_FRAMES as u64 || bytes % frame_size != 0 {
        return Err("Could not decode the GIF preview frames.".into());
    }
    let mut reader = BufReader::new(file);
    let mut frames = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if job::cancelled(cancel) {
            return Ok(None);
        }
        let mut frame = Thumbnail::new(width, height);
        reader
            .read_exact(frame.make_mut_bytes())
            .map_err(|e| e.to_string())?;
        frames.push(frame);
    }
    Ok(Some(frames))
}

enum Finished {
    Detail(Thumbnail, Thumbnail),
    Preview {
        result: Arc<Preview>,
        original_audio: Option<PathBuf>,
        before: Option<Thumbnail>,
        after: Option<Thumbnail>,
        info: String,
        bytes: Option<u64>,
        gif: Vec<Thumbnail>,
    },
    Analyzed(Arc<Artifact>, Result<(Thumbnail, Thumbnail), String>),
    Exported(PathBuf, u64, u64, std::time::Duration),
    Frames(PathBuf, usize),
    Cancelled,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProcessMode {
    Export,
    KeepBoth,
    Preview,
    Detail,
}

fn process(ui: &AppWindow, state: &SharedState, mode: ProcessMode) {
    let analyze = matches!(mode, ProcessMode::Preview | ProcessMode::Detail);
    let detail = mode == ProcessMode::Detail;
    let keep_both = mode == ProcessMode::KeepBoth;
    let Ok(mut app) = state.lock() else { return };
    if app.running.is_some() || ui.get_busy() {
        return;
    }
    let (Some(media), Some(action), Some(format), Some(tools)) = (
        app.media.as_ref(),
        app.action,
        app.format,
        app.tools.as_ref(),
    ) else {
        return;
    };
    let Some(tool) = tools.ffmpeg.tool() else {
        return;
    };
    if format == Format::Webp && !tools.supports_webp() {
        return message(
            ui,
            "This FFmpeg doesn’t support WebP encoding. Choose another format or select an FFmpeg with libwebp.",
            true,
        );
    }
    if detail && ui.get_preview_state() != "Ready" {
        return;
    }
    if analyze && !ui.get_preview_enabled() {
        return;
    }
    let seek = ui.get_seek_time();
    let timeline = matches!(action, Action::Frames | Action::Trim);
    let snapshot = if analyze
        && action == Action::Frames
        && ui.get_frame_series()
        && ui.get_seek_limit() > 0.0
    {
        seek.to_string()
    } else {
        ui.get_trim_start().to_string()
    };
    let series = !analyze && action == Action::Frames && ui.get_frame_series();
    let destination = if analyze {
        None
    } else {
        let Some(folder) = &app.output_folder else {
            return;
        };
        match destination::filename(ui.get_destination_name().as_str(), format) {
            Ok(name) if series => Some(folder.join(name).with_extension("frames")),
            Ok(name) => Some(folder.join(name)),
            Err(error) => return message(ui, error, true),
        }
    };
    let work = if media.kind == Kind::Image {
        let request = match image_request(ui, media, format) {
            Ok(request) => request,
            Err(error) => return message(ui, error, true),
        };
        if (request.dimensions != request.original || request.shrink)
            && !(tools.has_filter("scale") && tools.has_filter("setsar"))
        {
            return message(
                ui,
                "FFmpeg can’t resize this format. Keep the original dimensions or choose another format.",
                true,
            );
        }
        if format == Format::Png && !request.copies_source() && !tools.has_encoder("png") {
            return message(
                ui,
                "Your FFmpeg installation can’t export PNG. Choose another format.",
                true,
            );
        }
        Work::Image(request, app.analysis.clone())
    } else {
        let interval = if series {
            match ui.get_frame_interval().trim().parse::<f64>() {
                Ok(seconds) if seconds.is_finite() && seconds >= 0.001 => Some(seconds),
                _ => {
                    return message(
                        ui,
                        "Enter a frame interval of at least 0.001 seconds.",
                        true,
                    );
                }
            }
        } else {
            None
        };
        let trim = match action {
            Action::Frames if !series => Range::snapshot(&snapshot, media.duration).map(Some),
            Action::Trim | Action::Gif | Action::Frames | Action::TrimAudio => Range::parse(
                ui.get_trim_start().as_str(),
                ui.get_trim_end().as_str(),
                media.duration,
            )
            .map(Some),
            _ => Ok(None),
        };
        let trim = match trim {
            Ok(trim) if analyze => trim.map(|range| range.limited(preview::SAMPLE_SECONDS)),
            Ok(trim) => trim,
            Err(error) => return message(ui, error, true),
        };
        let mut plan = match operation::plan_with_interval(
            media,
            action,
            format,
            Quality::from_index(ui.get_quality_index()),
            trim,
            None,
            interval,
        ) {
            Ok(plan) => plan,
            Err(error) => return message(ui, error, true),
        };
        if let Some(encoder) = tools.missing_encoder(&plan) {
            return message_with_details(
                ui,
                "This format isn’t supported. Choose another format.",
                format!("Your FFmpeg installation is missing the {encoder} encoder."),
            );
        }
        if analyze && action == Action::Compress {
            preview::seek_compression_sample(&mut plan, f64::from(seek));
        }
        Work::Media(plan, series)
    };
    let known_duration = matches!(&work, Work::Media(plan, _) if plan.duration.is_some());
    let source = media.path.clone();
    let original = media.bytes;
    let modified = media.modified;
    let ffmpeg = tool.path.clone();
    let ffprobe = tools.ffprobe.tool().map(|tool| tool.path.clone());
    let video_index = media.video.as_ref().map(|stream| stream.index);
    let audio = media.audio.first().map(|stream| stream.index);
    let is_audio = media.kind == Kind::Audio || action == Action::Extract;
    let cached_media = if detail {
        app.media_preview.clone()
    } else {
        None
    };
    let (cancel, request) = mpsc::channel();
    let window = ui.as_weak();
    let completion_state = state.clone();
    let worker = thread::Builder::new()
        .name("media-export".into())
        .spawn(move || {
            let started = Instant::now();
            let result: Result<Finished, String> = (|| {
                crate::core::media::verify_source(&source, original, modified)?;
                let destination = destination
                    .map(|path| {
                        if keep_both && destination::exists(&path)? {
                            destination::unique_path(&path)
                        } else {
                            Ok(path)
                        }
                    })
                    .transpose()
                    .map_err(|e| format!("Could not choose an output name: {e}"))?;
                if let Some(path) = &destination {
                    job::validate(&source, path)?;
                }
                let update = |mut progress: Progress| {
                    progress.elapsed = started.elapsed();
                    let _ = window.upgrade_in_event_loop(move |ui| show_progress(&ui, progress));
                };
                match work {
                    Work::Image(settings, cached) => {
                        if let Some(folder) = destination.as_deref().and_then(Path::parent) {
                            tempfile::NamedTempFile::new_in(folder).map_err(|e| {
                                format!("Could not write to the output folder: {e}")
                            })?;
                        }
                        let artifact = match cached {
                            Some(artifact) if artifact.matches(&settings, &ffmpeg) => artifact,
                            _ => {
                                let Some(artifact) =
                                    image::render(&ffmpeg, settings, &request, update)?
                                else {
                                    return Ok(Finished::Cancelled);
                                };
                                Arc::new(artifact)
                            }
                        };
                        if let Some(path) = destination {
                            if !image::save(&artifact, &path, &request)? {
                                return Ok(Finished::Cancelled);
                            }
                            Ok(Finished::Exported(
                                path,
                                original,
                                artifact.bytes,
                                started.elapsed(),
                            ))
                        } else {
                            let comparison = (|| {
                                let before = thumbnail(
                                    &ffmpeg,
                                    &source,
                                    artifact.request.stream,
                                    0.0,
                                    detail,
                                    &request,
                                    update,
                                )?;
                                let Some(before) = before else {
                                    return Ok(None);
                                };
                                let after = thumbnail(
                                    &ffmpeg,
                                    &artifact.file,
                                    0,
                                    0.0,
                                    detail,
                                    &request,
                                    update,
                                )?;
                                Ok(after.map(|after| (before, after)))
                            })();
                            match comparison {
                                Ok(None) => Ok(Finished::Cancelled),
                                Ok(Some((before, after))) if detail => {
                                    Ok(Finished::Detail(before, after))
                                }
                                Ok(Some(pair)) => Ok(Finished::Analyzed(artifact, Ok(pair))),
                                Err(error) if detail => Err(error),
                                Err(error) => Ok(Finished::Analyzed(artifact, Err(error))),
                            }
                        }
                    }
                    Work::Media(plan, _) if analyze => {
                        let start = plan
                            .args
                            .windows(2)
                            .find(|pair| pair[0] == "-ss")
                            .and_then(|pair| pair[1].to_str())
                            .unwrap_or("0")
                            .to_owned();
                        let sample_duration = plan
                            .duration
                            .unwrap_or(preview::SAMPLE_SECONDS)
                            .min(preview::SAMPLE_SECONDS);
                        let result = if let Some(result) = cached_media {
                            result
                        } else {
                            let Some(preview) = preview::render(&ffmpeg, plan, &request, update)?
                            else {
                                return Ok(Finished::Cancelled);
                            };
                            Arc::new(preview)
                        };
                        let mut before = None;
                        let mut after = None;
                        let mut original_audio = None;
                        let mut gif = Vec::new();
                        if is_audio && let Some(index) = audio {
                            original_audio = result.original_audio(
                                &ffmpeg,
                                &source,
                                index,
                                (&start, sample_duration),
                                &request,
                                update,
                            )?;
                            if original_audio.is_none() {
                                return Ok(Finished::Cancelled);
                            }
                        } else if action == Action::Compress
                            && let Some(index) = video_index
                        {
                            before =
                                thumbnail(&ffmpeg, &source, index, seek, detail, &request, update)?;
                            if before.is_none() {
                                return Ok(Finished::Cancelled);
                            }
                            after = thumbnail(
                                &ffmpeg,
                                &result.sample,
                                0,
                                0.0,
                                detail,
                                &request,
                                update,
                            )?;
                            if after.is_none() {
                                return Ok(Finished::Cancelled);
                            }
                            if detail {
                                return Ok(match (before, after) {
                                    (Some(before), Some(after)) => Finished::Detail(before, after),
                                    _ => Finished::Cancelled,
                                });
                            }
                        } else if action == Action::Trim
                            && let Some(index) = video_index
                        {
                            after =
                                thumbnail(&ffmpeg, &source, index, seek, false, &request, update)?;
                            if after.is_none() {
                                return Ok(Finished::Cancelled);
                            }
                        } else if matches!(action, Action::Gif | Action::Frames) {
                            after = thumbnail(
                                &ffmpeg,
                                &result.sample,
                                0,
                                0.0,
                                false,
                                &request,
                                update,
                            )?;
                            let Some(frame) = &after else {
                                return Ok(Finished::Cancelled);
                            };
                            if action == Action::Gif {
                                let Some(frames) = gif_frames(
                                    &ffmpeg,
                                    &result.sample,
                                    (frame.width(), frame.height()),
                                    &request,
                                    update,
                                )?
                                else {
                                    return Ok(Finished::Cancelled);
                                };
                                after = frames.first().cloned();
                                gif = frames;
                            }
                        }
                        let info = ffprobe
                            .as_ref()
                            .and_then(|tool| Media::probe(tool, result.sample.clone()).ok())
                            .map(|media| preview_metadata(&media))
                            .unwrap_or_default();
                        let info = if action == Action::Compress {
                            format!("Compared at {seek:.2}s · Export uses the full video\n{info}")
                        } else if timeline && action == Action::Trim {
                            format!(
                                "Source frame at {seek:.2}s · Play processed to review the trim"
                            )
                        } else if action == Action::Gif {
                            format!("12 fps · GIF sample\n{info}")
                        } else {
                            info
                        };
                        let bytes = fs::metadata(&result.sample).ok().map(|meta| meta.len());
                        Ok(Finished::Preview {
                            result,
                            original_audio,
                            before,
                            after,
                            info,
                            bytes,
                            gif,
                        })
                    }
                    Work::Media(plan, series) => {
                        let path = destination.ok_or("Choose an output folder.")?;
                        if series {
                            return match job::export_frames(
                                &ffmpeg, &path, &plan, &request, update,
                            )? {
                                Some((first, count)) => Ok(Finished::Frames(first, count)),
                                None => Ok(Finished::Cancelled),
                            };
                        }
                        match job::export(&ffmpeg, &source, path, plan, request, update)? {
                            Outcome::Exported(path) => {
                                let bytes = fs::metadata(&path)
                                    .map_err(|e| {
                                        format!("File saved, but its size could not be read: {e}")
                                    })?
                                    .len();
                                Ok(Finished::Exported(path, original, bytes, started.elapsed()))
                            }
                            Outcome::Cancelled => Ok(Finished::Cancelled),
                        }
                    }
                }
            })();
            let _ = window.upgrade_in_event_loop(move |ui| {
                if let Ok(mut state) = completion_state.lock() {
                    state.running = None;
                    ui.set_running(false);
                    ui.set_cancelling(false);
                    ui.set_preview_pixel_scale(ui.window().scale_factor());
                    match result {
                        Ok(Finished::Preview {
                            result,
                            original_audio,
                            before,
                            after,
                            info,
                            bytes,
                            gif,
                        }) => {
                            let compare = before.is_some() && after.is_some();
                            ui.set_has_single_preview(after.is_some() && !compare);
                            ui.set_gif_frames(ModelRc::new(VecModel::from(
                                gif.into_iter()
                                    .map(slint::Image::from_rgba8)
                                    .collect::<Vec<_>>(),
                            )));
                            if let Some(before) = before {
                                ui.set_preview_original(slint::Image::from_rgba8(before));
                            }
                            if let Some(after) = after {
                                ui.set_preview_converted(slint::Image::from_rgba8(after));
                            }
                            ui.set_has_image_comparison(compare);
                            ui.set_media_preview_ready(true);
                            ui.set_audio_ab(original_audio.is_some());
                            ui.set_preview_info(info.into());
                            if let Some(bytes) = bytes {
                                ui.set_preview_size(format_size(bytes).into());
                                ui.set_preview_size_label("Sample size".into());
                            }
                            state.had_analysis = true;
                            state.media_preview = Some(result);
                            state.original_audio_preview = original_audio;
                            ui.set_preview_state("Ready".into());
                            message(&ui, "Preview is ready", false);
                        }
                        Ok(Finished::Detail(before, after)) => {
                            ui.set_detail_original(slint::Image::from_rgba8(before));
                            ui.set_detail_converted(slint::Image::from_rgba8(after));
                            ui.set_detail_requested(true);
                            message(&ui, "Preview is ready", false);
                        }
                        Ok(Finished::Analyzed(artifact, comparison)) => {
                            let preview_available = match comparison {
                                Ok((before, after)) => {
                                    ui.set_preview_original(slint::Image::from_rgba8(before));
                                    ui.set_preview_converted(slint::Image::from_rgba8(after));
                                    ui.set_has_image_comparison(true);
                                    true
                                }
                                Err(error) => {
                                    message_with_details(
                                        &ui,
                                        "Couldn’t display the preview",
                                        error,
                                    );
                                    false
                                }
                            };
                            ui.set_preview_state(
                                if preview_available { "Ready" } else { "Failed" }.into(),
                            );
                            let original = state.media.as_ref().map_or(0, |media| media.bytes);
                            let measured = format!(
                                "{} · {}",
                                format_size(artifact.bytes),
                                savings_label(original, artifact.bytes)
                            );
                            ui.set_preview_size_label("Measured size".into());
                            ui.set_preview_size(measured.into());
                            let quality = if artifact.request.lossless {
                                "Lossless".to_owned()
                            } else {
                                format!("Quality {}", artifact.quality)
                            };
                            ui.set_preview_details(
                                format!(
                                    "{}  ·  {quality}  ·  {}",
                                    artifact.request.format.label(),
                                    format_args!(
                                        "{} → {}",
                                        artifact.request.original, artifact.dimensions
                                    )
                                )
                                .into(),
                            );
                            state.had_analysis = true;
                            let larger = artifact.bytes > original;
                            state.analysis = Some(artifact);
                            if preview_available {
                                message(
                                    &ui,
                                    if larger {
                                        "These settings produce a larger file."
                                    } else {
                                        "Preview is ready"
                                    },
                                    false,
                                );
                            }
                        }
                        Ok(Finished::Exported(path, original, bytes, elapsed)) => {
                            ui.set_destination_name(path_name(&path).as_ref().into());
                            ui.set_result_summary(
                                format!(
                                    "{} → {}  ·  {}  ·  {:.1}s",
                                    format_size(original),
                                    format_size(bytes),
                                    savings_label(original, bytes),
                                    elapsed.as_secs_f64()
                                )
                                .into(),
                            );
                            message(&ui, "Your file is ready", false);
                            state.exported = Some(path);
                            ui.set_output_ready(true);
                        }
                        Ok(Finished::Frames(first, count)) => {
                            let summary = format!(
                                "Saved {count} {}.",
                                if count == 1 { "frame" } else { "frames" }
                            );
                            ui.set_result_summary(summary.into());
                            message(&ui, "Your files are ready", false);
                            state.exported = Some(first);
                            ui.set_output_ready(true);
                        }
                        Ok(Finished::Cancelled) => {
                            if analyze && !detail {
                                ui.set_preview_state("Canceled".into());
                            }
                            message(
                                &ui,
                                if analyze {
                                    "Preview canceled"
                                } else {
                                    "Export canceled"
                                },
                                false,
                            );
                        }

                        Err(error) => {
                            if analyze && !detail {
                                ui.set_preview_state("Failed".into());
                            }
                            let conflict = error == job::CONFLICT;
                            ui.set_conflict(conflict);
                            if conflict {
                                message(&ui, error, true);
                            } else {
                                let summary = if analyze {
                                    "Couldn’t prepare the preview"
                                } else {
                                    "Couldn’t finish the export. Your original file is safe."
                                };
                                message_with_details(&ui, summary, error);
                            }
                        }
                    }
                }
            });
        });
    match worker {
        Ok(worker) => {
            if !analyze {
                app.clear_result(ui);
            }
            app.running = Some(Running {
                cancel,
                worker,
                preview: analyze,
            });
            if analyze && !detail {
                app.media_preview = None;
                app.original_audio_preview = None;
                ui.set_preview_original(slint::Image::default());
                ui.set_preview_converted(slint::Image::default());
                ui.set_gif_frames(ModelRc::default());
                ui.set_detail_requested(false);
                ui.set_detail_original(slint::Image::default());
                ui.set_detail_converted(slint::Image::default());
                ui.set_has_image_comparison(false);
                ui.set_has_single_preview(false);
                ui.set_media_preview_ready(false);
                ui.set_preview_size_label("Preview size".into());
                ui.set_preview_size("—".into());
                ui.set_preview_state("Generating".into());
            }
            ui.set_running(true);
            ui.set_cancelling(false);
            ui.set_indeterminate(!known_duration);
            ui.set_fraction(0.0);
            ui.set_progress_label(
                if known_duration {
                    "0%  ·  0:00"
                } else {
                    "0:00 elapsed"
                }
                .into(),
            );
            message(
                ui,
                if detail {
                    "Loading detail at 100%..."
                } else if analyze {
                    "Preparing your preview..."
                } else {
                    "Exporting your file..."
                },
                false,
            );
        }
        Err(error) => message_with_details(
            ui,
            if analyze {
                "Couldn’t start the preview"
            } else {
                "Couldn’t start the export"
            },
            error.to_string(),
        ),
    }
}

fn show_progress(ui: &AppWindow, progress: Progress) {
    let elapsed = clock(progress.elapsed.as_secs());
    if let Some(fraction) = progress.fraction {
        ui.set_fraction(fraction);
        ui.set_progress_label(
            format!("{}%  ·  {elapsed}", (fraction * 100.0).round() as u32).into(),
        );
    } else {
        ui.set_progress_label(format!("{elapsed} elapsed").into());
    }
}

fn play_preview(path: &Path) -> io::Result<()> {
    if cfg!(target_os = "macos")
        && matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("mp4" | "mp3" | "wav")
        )
    {
        let status = Command::new("open")
            .args(["-b", "com.apple.QuickTimePlayerX"])
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        return if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "QuickTime could not open the preview ({status})"
            )))
        };
    }
    open_local_path(path, false)
}

fn open_media_preview(ui: &AppWindow, state: &SharedState) {
    let Ok(state) = state.lock() else { return };
    let media = state.media_preview.clone();
    let image = state.analysis.clone();
    drop(state);
    if let Err(error) = background(
        ui,
        "open-preview",
        move || {
            if let Some(preview) = media {
                play_preview(&preview.playback)
            } else if let Some(artifact) = image {
                open_local_path(&artifact.file, false)
            } else {
                Ok(())
            }
        },
        |ui, result| {
            if let Err(error) = result {
                message_with_details(ui, "Couldn’t open the preview", error.to_string());
            }
        },
    ) {
        message_with_details(ui, "Couldn’t open the preview", error.to_string());
    }
}

fn open_output(ui: &AppWindow, state: &SharedState, folder: bool) {
    let Some(path) = state.lock().ok().and_then(|state| {
        let exported = if state
            .selected_tool
            .is_some_and(|tool| matches!(tool.handler, Handler::Pdf))
        {
            &state.pdf.exported
        } else {
            &state.exported
        };
        let path = exported.as_ref()?;
        Some(path.clone())
    }) else {
        return;
    };
    let failure = if folder {
        "Couldn’t open the output folder"
    } else {
        "Couldn’t open the exported file"
    };
    if let Err(error) = background(
        ui,
        "open-output",
        move || open_local_path(&path, folder),
        move |ui, result| {
            if let Err(error) = result {
                message_with_details(ui, failure, error.to_string());
            }
        },
    ) {
        message_with_details(ui, failure, error.to_string());
    }
}

fn open_local_path(path: &Path, reveal: bool) -> io::Result<()> {
    let launcher = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer.exe"
    } else {
        "xdg-open"
    };
    let mut command = Command::new(launcher);
    let target = if reveal && cfg!(target_os = "macos") {
        command.arg("-R");
        path
    } else if reveal {
        path.parent()
            .ok_or_else(|| io::Error::other("Output folder unavailable"))?
    } else {
        path
    };
    let status = command
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() || (cfg!(windows) && status.code() == Some(1)) {
        Ok(())
    } else {
        Err(io::Error::other(format!("{launcher} returned {status}")))
    }
}

fn callback(
    ui: &AppWindow,
    state: &SharedState,
    run: impl Fn(&AppWindow, &SharedState) + 'static,
) -> impl Fn() + 'static {
    let window = ui.as_weak();
    let state = state.clone();
    move || {
        if let Some(ui) = window.upgrade() {
            run(&ui, &state);
        }
    }
}

fn background<T: Send + 'static>(
    ui: &AppWindow,
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
    finished: impl FnOnce(&AppWindow, T) + Send + 'static,
) -> io::Result<()> {
    let window = ui.as_weak();
    thread::Builder::new().name(name.into()).spawn(move || {
        let result = work();
        let _ = window.upgrade_in_event_loop(move |ui| finished(&ui, result));
    })?;
    Ok(())
}

fn tool_description(name: &str, availability: &Availability) -> String {
    match availability {
        Availability::Ready(tool) => format!("{name} {} · {}", tool.version, tool.path.display()),
        Availability::Missing => format!("{name} was not found."),
        Availability::Failed(error) => format!("{name} could not run: {error}"),
    }
}

fn message(ui: &AppWindow, text: impl Into<SharedString>, error: bool) {
    ui.set_status_details("".into());
    ui.set_status(text.into());
    ui.set_status_error(error);
}

fn message_with_details(ui: &AppWindow, summary: &str, details: String) {
    message(ui, summary, true);
    ui.set_status_details(details.into());
}

fn model<'a>(values: impl Iterator<Item = &'a str>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        values.map(SharedString::from).collect::<Vec<_>>(),
    ))
}

fn path_name(path: &Path) -> std::borrow::Cow<'_, str> {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
}

fn clock(seconds: u64) -> String {
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

fn source_facts(media: &Media) -> [String; 2] {
    let mut technical = Vec::new();
    if let Some(stream) = &media.video {
        if let Some((width, height)) = stream.dimensions() {
            technical.push(format!("{width} × {height} px"));
        }
        if media.kind == Kind::Image && stream.has_alpha() {
            technical.push("Alpha channel".into());
        }
    }
    if let Some(duration) = media.duration {
        technical.push(format!("{duration:.2} s"));
    }
    if media.kind == Kind::Audio {
        if let Some(audio) = media.audio.first() {
            if let Some(rate) = audio.sample_rate() {
                technical.push(format!("{rate} Hz"));
            }
            if let Some(channels) = audio.channels {
                technical.push(format!("{channels} ch"));
            }
        }
    } else if media.kind == Kind::Video {
        technical.push(match media.audio.len() {
            0 => "No audio".into(),
            1 => "1 audio track".into(),
            count => format!("{count} audio tracks"),
        });
    }
    [
        format!("{}  ·  {}", media.format, format_size(media.bytes)),
        technical.join("  ·  "),
    ]
}

fn home_file_details(media: &Media) -> String {
    let mut details = vec![media.format.clone()];
    if let Some((width, height)) = media.video.as_ref().and_then(|video| video.dimensions()) {
        details.push(format!("{width} × {height}"));
    }
    if let Some(duration) = media.duration {
        let seconds = duration as u64;
        let time = if seconds >= 3600 {
            clock(seconds)
        } else {
            format!("{:02}:{:02}", seconds / 60, seconds % 60)
        };
        details.push(time);
    }
    if media.kind == Kind::Audio
        && let Some(rate) = media.audio.first().and_then(|stream| stream.bit_rate())
    {
        details.push(format!("{:.0} kbps", rate as f64 / 1000.0));
    }
    details.push(format_size(media.bytes));
    details.join("  ·  ")
}

fn preview_metadata(media: &Media) -> String {
    let mut parts = Vec::new();
    if let Some(video) = &media.video {
        parts.push(video.codec_name.to_ascii_uppercase());
        if let Some((width, height)) = video.dimensions() {
            parts.push(format!("{width} × {height}"));
        }
    }
    if let Some(audio) = media.audio.first() {
        parts.push(audio.codec_name.to_ascii_uppercase());
        if let Some(rate) = audio.sample_rate() {
            parts.push(format!("{rate} Hz"));
        }
        if let Some(channels) = audio.channels {
            parts.push(format!("{channels} ch"));
        }
    }
    if let Some(duration) = media.duration {
        parts.push(format!("{duration:.2} s"));
    }
    parts.join("  ·  ")
}

fn inspect_details(media: &Media) -> String {
    let mut details = vec![
        format!("Format  {}", media.format),
        format!("Size  {} ({} bytes)", format_size(media.bytes), media.bytes),
    ];
    if let Some(duration) = media.duration {
        details.push(format!("Duration  {duration:.3} s"));
    }
    if let Some(video) = &media.video {
        let codec = video.codec_name.to_ascii_uppercase();
        let label = if media.kind == Kind::Image {
            "Image"
        } else {
            "Video"
        };
        let dimensions = video
            .dimensions()
            .map(|(w, h)| format!(" · {w} × {h}"))
            .unwrap_or_default();
        details.push(format!("{label}  {codec}{dimensions}"));
    }
    for (index, audio) in media.audio.iter().enumerate() {
        let mut track = format!(
            "Audio {}  {}",
            index + 1,
            audio.codec_name.to_ascii_uppercase()
        );
        if let Some(rate) = audio.sample_rate() {
            track.push_str(&format!(" · {rate} Hz"));
        }
        if let Some(channels) = audio.channels {
            track.push_str(&format!(" · {channels} ch"));
        }
        details.push(track);
    }
    details.join("\n")
}

fn format_size(bytes: u64) -> String {
    for (unit, scale) in [
        ("TiB", 1 << 40),
        ("GiB", 1 << 30),
        ("MiB", 1 << 20),
        ("KiB", 1 << 10),
    ] {
        if bytes >= scale {
            return format!("{:.1} {unit}", bytes as f64 / scale as f64);
        }
    }
    format!("{bytes} bytes")
}

#[cfg(all(test, unix))]
mod setup_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn homebrew_installer_uses_ffmpeg_formula_and_reports_failure() {
        let dir = tempfile::tempdir().unwrap();
        let brew = dir.path().join("brew");
        fs::write(
            &brew,
            r#"#!/bin/sh
[ "$1" = install ] && [ "$2" = ffmpeg ] && [ "$HOMEBREW_NO_AUTO_UPDATE" = 1 ] || exit 2
echo 'Error: test package unavailable' >&2
exit 1
"#,
        )
        .unwrap();
        fs::set_permissions(&brew, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            install_with_homebrew(&brew, "ffmpeg").unwrap_err(),
            "Homebrew install failed. Error: test package unavailable"
        );
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    #[ignore = "requires FFmpeg and ffprobe on the test machine"]
    fn moving_video_sample_matches_the_seek_and_gif_frames_are_bounded() {
        let tools = Tools::detect();
        let ffmpeg = &tools.ffmpeg.tool().unwrap().path;
        let ffprobe = &tools.ffprobe.tool().unwrap().path;
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("two colors.mp4");
        let status = ffmpeg::command(ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=red:s=320x180:r=24:d=1",
                "-f",
                "lavfi",
                "-i",
                "color=blue:s=320x180:r=24:d=2",
                "-filter_complex",
                "[0:v][1:v]concat=n=2:v=1:a=0",
                "-c:v",
                "libx264",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(status.success());
        let media = Media::probe(ffprobe, source.clone()).unwrap();
        let (_sender, cancel) = mpsc::channel();
        let update = |_| {};
        let mut plan = operation::plan(
            &media,
            Action::Compress,
            Format::Mp4,
            Quality::Balanced,
            None,
            None,
        )
        .unwrap();
        preview::seek_compression_sample(&mut plan, 1.5);
        let sample = preview::render(ffmpeg, plan, &cancel, update)
            .unwrap()
            .unwrap();
        let sample_media = Media::probe(ffprobe, sample.sample.clone()).unwrap();
        assert!((sample_media.duration.unwrap() - 1.5).abs() < 0.1);
        let original = thumbnail(ffmpeg, &source, 0, 1.5, false, &cancel, update)
            .unwrap()
            .unwrap();
        let compressed = thumbnail(ffmpeg, &sample.sample, 0, 0.0, false, &cancel, update)
            .unwrap()
            .unwrap();
        for frame in [original, compressed] {
            let pixel = &frame.as_bytes()[..4];
            assert!(
                pixel[2] > 220 && pixel[0] < 25,
                "expected blue at the chosen time"
            );
        }

        let range = Range::parse("0", "3", media.duration).unwrap();
        let plan = operation::plan(
            &media,
            Action::Gif,
            Format::Gif,
            Quality::Balanced,
            Some(range),
            None,
        )
        .unwrap();
        let gif = preview::render(ffmpeg, plan, &cancel, update)
            .unwrap()
            .unwrap();
        let frames = gif_frames(ffmpeg, &gif.sample, (320, 180), &cancel, update)
            .unwrap()
            .unwrap();
        assert!((35..=37).contains(&frames.len()));
        assert_eq!((frames[0].width(), frames[0].height()), (256, 144));
        assert!(frames.first().unwrap().as_bytes()[0] > 220);
        assert!(frames.last().unwrap().as_bytes()[2] > 220);
    }
}
