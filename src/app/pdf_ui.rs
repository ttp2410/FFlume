use super::{
    AppWindow, SharedState, background, callback, destination, find_homebrew,
    install_with_homebrew, message, message_with_details, open_setup_page, path_name, pick_path,
};
use crate::core::{
    operation::Format,
    pdf::{self, Backend, Document},
};
use slint::{ComponentHandle, Image, Rgb8Pixel, SharedPixelBuffer};
use std::path::PathBuf;

#[derive(Default)]
pub(super) struct Session {
    backend: Option<Backend>,
    document: Option<Document>,
    preview_page: Option<u32>,
    preview_format: Option<Format>,
    requested_page: Option<u32>,
    preview_generation: u64,
    rendering: bool,
    preview_cache: Vec<(u32, Format, SharedPixelBuffer<Rgb8Pixel>)>,
    pending: Option<PathBuf>,
    pub(super) exported: Option<PathBuf>,
}

impl Session {
    pub(super) fn set_backend(&mut self, ui: &AppWindow, backend: Option<Backend>) {
        ui.set_pdf_checked(true);
        ui.set_pdf_ready(backend.is_some());
        self.backend = backend;
    }
}

pub(super) fn bind(ui: &AppWindow, state: &SharedState) {
    ui.on_pdf_install(callback(ui, state, |ui, state| setup(ui, state, true)));
    ui.on_pdf_check(callback(ui, state, |ui, state| setup(ui, state, false)));
    ui.on_pdf_browse(callback(ui, state, |ui, state| {
        if ui.get_busy() || !ui.get_pdf_ready() {
            return;
        }
        let dialog = rfd::AsyncFileDialog::new()
            .set_title("Open PDF")
            .add_filter("PDF document", &["pdf"])
            .set_parent(&ui.window().window_handle());
        pick_path(ui, state, dialog, false, load);
    }));
    ui.on_pdf_preview(callback(ui, state, preview));
    ui.on_pdf_format_changed(callback(ui, state, |ui, state| {
        let page = state.lock().ok().and_then(|state| state.pdf.requested_page);
        if let Some(page) = page {
            request_preview(ui, state, page);
        }
    }));
    ui.on_pdf_previous(callback(ui, state, |ui, state| step_preview(ui, state, -1)));
    ui.on_pdf_next(callback(ui, state, |ui, state| step_preview(ui, state, 1)));
    ui.on_pdf_export(callback(ui, state, choose_export_folder));
}

pub(super) fn setup(ui: &AppWindow, state: &SharedState, install: bool) {
    if ui.get_busy() || ui.get_running() {
        return;
    }
    ui.set_busy(true);
    ui.set_pdf_installing(install);
    if install {
        message(ui, "Setting up PDF export...", false);
    }
    let state = state.clone();
    if let Err(error) = background(
        ui,
        "pdf-setup",
        move || {
            if let Some(backend) = Backend::detect() {
                return Ok(Some(backend));
            }
            if !install {
                return Ok(None);
            }
            if cfg!(target_os = "macos") {
                if let Some(brew) = find_homebrew() {
                    install_with_homebrew(&brew, "poppler")?;
                    return Backend::detect().map(Some).ok_or_else(|| {
                        "Homebrew finished, but pdfinfo or pdftoppm could not be detected.".into()
                    });
                }
                open_setup_page("https://brew.sh/")?;
            } else {
                open_setup_page("https://poppler.freedesktop.org/")?;
            }
            Ok(None)
        },
        move |ui, result: Result<Option<Backend>, String>| {
            ui.set_busy(false);
            ui.set_pdf_installing(false);
            ui.set_pdf_checked(true);
            match result {
                Ok(backend) => {
                    let ready = backend.is_some();
                    ui.set_pdf_ready(ready);
                    let pending = state.lock().ok().and_then(|mut state| {
                        state.pdf.set_backend(ui, backend);
                        state.show_catalog(ui);
                        if ready {
                            state.pdf.pending.take()
                        } else {
                            None
                        }
                    });
                    if let Some(path) = pending {
                        load(ui, &state, path);
                    } else if ready {
                        message(ui, "Ready to go", false);
                    } else if install {
                        let hint = if cfg!(target_os = "macos") {
                            "Install Homebrew, then return and click Install Poppler again."
                        } else {
                            "Install Poppler utilities, then click Check Again."
                        };
                        message(ui, hint, false);
                    }
                }
                Err(error) => message(ui, error, true),
            }
        },
    ) {
        ui.set_busy(false);
        ui.set_pdf_installing(false);
        ui.set_pdf_checked(true);
        message(ui, format!("Could not start PDF setup: {error}"), true);
    }
}

pub(super) fn load(ui: &AppWindow, state: &SharedState, path: PathBuf) {
    if ui.get_busy() || ui.get_running() {
        return;
    }
    let backend = {
        let Ok(mut guard) = state.lock() else { return };
        match &guard.pdf.backend {
            Some(backend) => backend.clone(),
            None => {
                guard.pdf.pending = Some(path);
                drop(guard);
                setup(ui, state, false);
                return;
            }
        }
    };
    ui.set_busy(true);
    message(ui, "Reading file details...", false);
    let state = state.clone();
    if let Err(error) = background(
        ui,
        "pdf-open",
        move || backend.open(path),
        move |ui, result| {
            ui.set_busy(false);
            match result {
                Ok(document) => {
                    ui.set_pdf_name(path_name(&document.path).as_ref().into());
                    ui.set_pdf_page_count(document.pages.to_string().into());
                    ui.set_pdf_selection("1".into());
                    ui.set_pdf_image(Image::default());
                    ui.set_pdf_preview_label("".into());
                    ui.set_pdf_can_previous(false);
                    ui.set_pdf_can_next(false);
                    ui.set_pdf_output_ready(false);
                    if let Ok(mut state) = state.lock() {
                        state.pdf.document = Some(document);
                        state.pdf.preview_page = None;
                        state.pdf.preview_format = None;
                        state.pdf.requested_page = None;
                        state.pdf.preview_generation = state.pdf.preview_generation.wrapping_add(1);
                        state.pdf.rendering = false;
                        state.pdf.preview_cache.clear();
                        state.pdf.exported = None;
                    }
                    message(ui, "Ready to go", false);
                    preview(ui, &state);
                }
                Err(error) => message_with_details(ui, "Couldn’t open this PDF", error),
            }
        },
    ) {
        ui.set_busy(false);
        message(ui, format!("Could not open PDF: {error}"), true);
    }
}

fn selected(ui: &AppWindow, state: &SharedState) -> Option<(Backend, PathBuf, Vec<u32>)> {
    if ui.get_busy() || ui.get_running() || !ui.get_pdf_ready() {
        return None;
    }
    let state = state.lock().ok()?;
    let document = state.pdf.document.as_ref()?;
    match pdf::parse_pages(ui.get_pdf_selection().as_str(), document.pages) {
        Ok(pages) => Some((state.pdf.backend.clone()?, document.path.clone(), pages)),
        Err(error) => {
            message(ui, error, true);
            None
        }
    }
}

fn preview(ui: &AppWindow, state: &SharedState) {
    if let Some((_, _, pages)) = selected(ui, state) {
        request_preview(ui, state, pages[0]);
    }
}

fn step_preview(ui: &AppWindow, state: &SharedState, offset: i32) {
    if ui.get_busy() || ui.get_running() || !ui.get_pdf_ready() {
        return;
    }
    let page = state
        .lock()
        .ok()
        .and_then(|state| state.pdf.requested_page?.checked_add_signed(offset));
    if let Some(page) = page {
        request_preview(ui, state, page);
    }
}

fn output_format(ui: &AppWindow) -> Format {
    if ui.get_pdf_format_index() == 1 {
        Format::Jpeg
    } else {
        Format::Png
    }
}

fn request_preview(ui: &AppWindow, state: &SharedState, page: u32) {
    {
        let Ok(mut state) = state.lock() else { return };
        let session = &mut state.pdf;
        let Some(document) = &session.document else {
            return;
        };
        if !(1..=document.pages).contains(&page) {
            return;
        }
        session.requested_page = Some(page);
        session.preview_cache.retain(|(cached, format, _)| {
            *format == output_format(ui) && cached.abs_diff(page) <= 2
        });
        show_cached_preview(ui, session);
    }
    queue_preview(ui, state);
}

fn show_cached_preview(ui: &AppWindow, session: &mut Session) {
    let (Some(page), Some(document)) = (session.requested_page, &session.document) else {
        return;
    };
    ui.set_pdf_can_previous(page > 1);
    ui.set_pdf_can_next(page < document.pages);
    let format = output_format(ui);
    if (session.preview_page != Some(page) || session.preview_format != Some(format))
        && let Some((_, _, buffer)) = session
            .preview_cache
            .iter()
            .find(|(cached, cached_format, _)| *cached == page && *cached_format == format)
    {
        ui.set_pdf_image(Image::from_rgb8(buffer.clone()));
        ui.set_pdf_preview_label(
            format!("Page {page} of {} · {}", document.pages, format.label()).into(),
        );
        session.preview_page = Some(page);
        session.preview_format = Some(format);
    }
}

fn queue_preview(ui: &AppWindow, state: &SharedState) {
    let format = output_format(ui);
    let Some((backend, path, generation, page)) = state.lock().ok().and_then(|mut state| {
        let session = &mut state.pdf;
        if session.rendering {
            return None;
        }
        let requested = session.requested_page?;
        let document = session.document.as_ref()?;
        // Prioritize the requested page, then warm its two immediate neighbors.
        let page = [
            Some(requested),
            requested.checked_add(1),
            requested.checked_sub(1),
        ]
        .into_iter()
        .flatten()
        .filter(|page| (1..=document.pages).contains(page))
        .find(|page| {
            !session
                .preview_cache
                .iter()
                .any(|(cached, cached_format, _)| cached == page && *cached_format == format)
        })?;
        let backend = session.backend.clone()?;
        session.rendering = true;
        Some((
            backend,
            document.path.clone(),
            session.preview_generation,
            page,
        ))
    }) else {
        return;
    };

    let completion = state.clone();
    if let Err(error) = background(
        ui,
        "pdf-preview",
        move || {
            let (_temporary, preview) = backend.preview(&path, page, format)?;
            let pixels = image::open(preview)
                .map_err(|error| format!("Couldn’t read the page preview: {error}"))?
                .into_rgb8();
            Ok(SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
                pixels.as_raw(),
                pixels.width(),
                pixels.height(),
            ))
        },
        move |ui, result| {
            {
                let Ok(mut state) = completion.lock() else {
                    return;
                };
                let session = &mut state.pdf;
                if session.preview_generation != generation {
                    return;
                }
                session.rendering = false;
                if format == output_format(ui) {
                    match result {
                        Ok(buffer) => {
                            if session
                                .requested_page
                                .is_some_and(|requested| requested.abs_diff(page) <= 2)
                            {
                                session.preview_cache.push((page, format, buffer));
                            }
                            show_cached_preview(ui, session);
                        }
                        Err(error) if session.requested_page == Some(page) => {
                            session.requested_page = session.preview_page;
                            show_cached_preview(ui, session);
                            if ui.get_source_profile() == "PDF" && !ui.get_busy() {
                                message_with_details(ui, "Couldn’t show this page", error);
                            }
                            return;
                        }
                        Err(_) if session.requested_page == session.preview_page => return,
                        Err(_) => {}
                    }
                }
            }
            queue_preview(ui, &completion);
        },
    ) && let Ok(mut state) = state.lock()
    {
        state.pdf.rendering = false;
        if state.pdf.requested_page == Some(page) {
            state.pdf.requested_page = state.pdf.preview_page;
            show_cached_preview(ui, &mut state.pdf);
            message_with_details(ui, "Couldn’t start the page preview", error.to_string());
        }
    }
}

fn choose_export_folder(ui: &AppWindow, state: &SharedState) {
    let Some((backend, path, pages)) = selected(ui, state) else {
        return;
    };
    let folder = state
        .lock()
        .ok()
        .map(|state| destination::folder_for(&path, state.preferences.folder.as_deref()));
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title("Choose a folder for page images")
        .set_parent(&ui.window().window_handle());
    if let Some(folder) = folder {
        dialog = dialog.set_directory(folder);
    }
    let format = output_format(ui);
    pick_path(ui, state, dialog, true, move |ui, state, folder| {
        let count = pages.len();
        ui.set_busy(true);
        message(ui, "Exporting your file...", false);
        let state = state.clone();
        if let Err(error) = background(
            ui,
            "pdf-export",
            move || backend.export(&path, &pages, &folder, format),
            move |ui, result| {
                ui.set_busy(false);
                match result {
                    Ok(first) => {
                        if let Ok(mut state) = state.lock() {
                            state.pdf.exported = Some(first);
                        }
                        let unit = if count == 1 { "page" } else { "pages" };
                        ui.set_pdf_result_summary(
                            format!("Saved {count} {unit} as images.").into(),
                        );
                        ui.set_pdf_output_ready(true);
                        message(
                            ui,
                            if count == 1 {
                                "Your file is ready"
                            } else {
                                "Your files are ready"
                            },
                            false,
                        );
                    }
                    Err(error) => message_with_details(ui, "Couldn’t finish the export", error),
                }
            },
        ) {
            ui.set_busy(false);
            message(ui, format!("Could not start PDF export: {error}"), true);
        }
    });
}
