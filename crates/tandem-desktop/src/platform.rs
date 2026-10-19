//! The native side of the core's `Platform`: real open and save panels,
//! NSAlert questions, the system print panel, Open Recent, and the window's
//! proxy icon and edited dot. Called from the core thread; anything AppKit
//! needs runs on the main thread and the core thread waits for it.

use crate::macos;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use tandem_app::platform::{BoxKind, Filter, MessageBox, Pick, Platform, PrintOutcome, SavePurpose};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

pub struct TauriPlatform {
    app: AppHandle,
    edited: bool,
    represented: Option<PathBuf>,
}

impl TauriPlatform {
    pub fn new(app: AppHandle) -> Self {
        Self { app, edited: false, represented: None }
    }

    /// Runs `work` on the main thread and waits for its result.
    fn on_main<R: Send + 'static>(&self, work: impl FnOnce(objc2::MainThreadMarker) -> R + Send + 'static) -> Option<R> {
        let (tx, rx) = channel();
        self.app
            .run_on_main_thread(move || {
                if let Some(mtm) = objc2::MainThreadMarker::new() {
                    let _ = tx.send(work(mtm));
                }
            })
            .ok()?;
        rx.recv().ok()
    }

    fn file_dialog(&self, filters: &[Filter]) -> tauri_plugin_dialog::FileDialogBuilder<tauri::Wry> {
        let mut builder = self.app.dialog().file();
        if let Some(window) = self.app.get_webview_window("main") {
            builder = builder.set_parent(&window);
        }
        for filter in filters {
            builder = builder.add_filter(filter.name, filter.extensions);
        }
        builder
    }
}

impl Platform for TauriPlatform {
    fn pick_file(&mut self, pick: Pick, filters: &[Filter]) -> Option<PathBuf> {
        let title = match pick {
            Pick::Open => "Open a PDF",
            Pick::InsertSource => "Choose a PDF",
            Pick::Certificate => "Choose your certificate file",
            Pick::TrustCertificates => "Choose certificates to trust",
        };
        self.file_dialog(filters).set_title(title).blocking_pick_file()?.into_path().ok()
    }

    fn pick_images(&mut self) -> Vec<PathBuf> {
        self.file_dialog(&[tandem_app::platform::IMAGES_FILTER])
            .set_title("Choose images")
            .blocking_pick_files()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|f| f.into_path().ok())
            .collect()
    }

    fn pick_folder(&mut self) -> Option<PathBuf> {
        self.file_dialog(&[]).set_title("Choose a folder").blocking_pick_folder()?.into_path().ok()
    }

    fn pick_save(&mut self, purpose: SavePurpose, default_path: &Path, filters: &[Filter]) -> Option<PathBuf> {
        let title = match purpose {
            SavePurpose::Document => "Save",
            SavePurpose::New => "Save as a new file",
            SavePurpose::Signed => "Save the signed copy",
            SavePurpose::Print => "Save the print as a PDF",
        };
        let mut builder = self.file_dialog(filters).set_title(title);
        if let Some(name) = default_path.file_name() {
            builder = builder.set_file_name(name.to_string_lossy());
        }
        if let Some(dir) = default_path.parent().filter(|d| d.is_dir()) {
            builder = builder.set_directory(dir);
        }
        builder.blocking_save_file()?.into_path().ok()
    }

    fn ask(&mut self, question: &MessageBox) -> usize {
        let style = match question.kind {
            BoxKind::Info | BoxKind::Question => macos::AlertStyle::Info,
            BoxKind::Warning => macos::AlertStyle::Warning,
            BoxKind::Error => macos::AlertStyle::Critical,
        };
        let q = question.clone();
        self.on_main(move |mtm| macos::alert(style, &q.message, q.detail.as_deref(), &q.buttons, q.default, q.cancel, mtm))
            .unwrap_or(question.cancel)
    }

    fn open_external(&mut self, uri: &str) {
        // The host has already checked the scheme (http, https, mailto) and asked first.
        let _ = std::process::Command::new("/usr/bin/open").arg(uri).spawn();
    }

    fn print(&mut self, pdf: &[u8], _job_name: &str) -> PrintOutcome {
        let bytes = pdf.to_vec();
        match self.on_main(move |mtm| macos::print_pdf(&bytes, mtm)) {
            Some(macos::Printed::Printed) => PrintOutcome::Printed,
            Some(macos::Printed::Cancelled) => PrintOutcome::Cancelled,
            _ => PrintOutcome::Failed,
        }
    }

    fn add_recent_document(&mut self, path: &Path) {
        let path = path.to_path_buf();
        self.on_main(move |mtm| macos::note_recent(&path, mtm));
    }

    fn clear_recent_documents(&mut self) {
        self.on_main(macos::clear_recent);
    }

    fn set_represented_file(&mut self, path: Option<&Path>, title: &str) {
        let Some(window) = self.app.get_webview_window("main") else { return };
        let _ = window.set_title(title);
        let path = path.map(Path::to_path_buf);
        let edited = self.edited;
        let ns = window.ns_window().map(|p| p as usize).unwrap_or(0);
        self.represented = path.clone();
        self.on_main(move |_| macos::set_document(ns as *mut std::ffi::c_void, path.as_deref(), edited));
    }

    fn set_document_state(&mut self, edited: bool, save: bool, save_as: bool) {
        self.edited = edited;
        if let Some(menu) = self.app.menu() {
            for (id, on) in [("save:in-place", save), ("save:as", save_as), ("save:copy", save_as)] {
                if let Some(item) = crate::menu::find_item(&menu, id) {
                    let _ = item.set_enabled(on);
                }
            }
        }
        let Some(window) = self.app.get_webview_window("main") else { return };
        let ns = window.ns_window().map(|p| p as usize).unwrap_or(0);
        let path = self.represented.clone();
        self.on_main(move |_| macos::set_document(ns as *mut std::ffi::c_void, path.as_deref(), edited));
    }
}

/// Puts plain text on the general pasteboard (page text copied with ⌘C).
pub fn copy_text(app: &AppHandle, text: String) {
    let _ = app.run_on_main_thread(move || {
        if objc2::MainThreadMarker::new().is_some() {
            crate::macos::copy_text(&text);
        }
    });
}
