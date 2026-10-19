//! The native side of the core's `Platform`: real open and save panels, questions with
//! the app's own buttons, the system print dialog, Open Recent, and the window's title.
//! - macOS: NSAlert, PDFKit's print panel, the system's Open Recent, the proxy icon and
//!   edited dot (src/macos.rs);
//! - Windows: Task Dialogs, the Print dialog and GDI (src/win.rs);
//! - Linux: GTK message dialogs and the GTK Print dialog (src/linux.rs);
//! - Windows and Linux: the title says the file name and "(edited)"; Open Recent is built
//!   from the app's list (`menu::RecentMenu`).
//!
//! Called from the core thread; anything the window system needs runs on the main thread
//! and the core thread waits for it.

#[cfg(target_os = "macos")]
use crate::macos;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
#[cfg(target_os = "macos")]
use tandem_app::platform::BoxKind;
use tandem_app::platform::{Filter, MessageBox, Pick, Platform, PrintOutcome, SavePurpose};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

pub struct TauriPlatform {
    app: AppHandle,
    edited: bool,
    represented: Option<PathBuf>,
    /// The name the window's title shows (Windows, Linux).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    title: String,
    /// In a `--walk` run, native panels and alerts are answered from the script instead.
    pub walk: Option<std::sync::Arc<std::sync::Mutex<WalkAnswers>>>,
}

pub type WalkHandle = std::sync::Arc<std::sync::Mutex<WalkAnswers>>;

/// Where saves went, for a save toast's "Show in Finder": the last path chosen in a save
/// panel and the document's own file. Kept in the app's state (`app.manage`).
#[derive(Default)]
pub struct SaveTargets(pub std::sync::Mutex<(Option<PathBuf>, Option<PathBuf>)>);

impl SaveTargets {
    /// The file a toast names: a file name in `text` that matches the last save panel's
    /// answer, else the document's file.
    pub fn named_in(&self, text: &str) -> Option<PathBuf> {
        let (picked, document) = self.0.lock().ok()?.clone();
        let named = |p: &PathBuf| {
            p.file_name()
                .is_some_and(|n| text.contains(n.to_string_lossy().as_ref()))
        };
        picked
            .filter(named)
            .or_else(|| document.filter(named))
            .filter(|p| p.exists())
    }
}

/// A walkthrough's answers to native panels and alerts (`{"answer": …}` lines).
#[derive(Debug, Default, Clone)]
pub struct WalkAnswers {
    pub open: Option<PathBuf>,
    pub save_to: Option<PathBuf>,
    pub images: Vec<PathBuf>,
    pub folder: Option<PathBuf>,
    /// Alert answers by button label, used in order; otherwise the default button.
    pub buttons: std::collections::VecDeque<String>,
}

impl TauriPlatform {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            edited: false,
            represented: None,
            title: "Tandem PDF".into(),
            walk: None,
        }
    }

    fn walked<R>(&self, f: impl FnOnce(&mut WalkAnswers) -> R) -> Option<R> {
        self.walk
            .as_ref()
            .and_then(|w| w.lock().ok().map(|mut a| f(&mut a)))
    }

    /// Runs `work` on the main thread and waits for its result.
    #[cfg(target_os = "macos")]
    fn on_main<R: Send + 'static>(
        &self,
        work: impl FnOnce(objc2::MainThreadMarker) -> R + Send + 'static,
    ) -> Option<R> {
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

    /// Runs `work` on the main thread and waits for its result.
    #[cfg(not(target_os = "macos"))]
    fn on_main<R: Send + 'static>(&self, work: impl FnOnce() -> R + Send + 'static) -> Option<R> {
        let (tx, rx) = channel();
        self.app
            .run_on_main_thread(move || {
                let _ = tx.send(work());
            })
            .ok()?;
        rx.recv().ok()
    }

    /// The window's handle (Windows; 0 when there is no window).
    #[cfg(windows)]
    fn hwnd(&self) -> isize {
        self.app
            .get_webview_window("main")
            .and_then(|w| w.hwnd().ok())
            .map(|h| h.0 as isize)
            .unwrap_or(0)
    }

    /// Windows and Linux: the title from the document's name and edited state.
    #[cfg(not(target_os = "macos"))]
    fn show_title(&self) {
        if let Some(window) = self.app.get_webview_window("main") {
            let _ = window.set_title(&pc_title(&self.title, self.edited));
        }
    }

    fn file_dialog(
        &self,
        filters: &[Filter],
    ) -> tauri_plugin_dialog::FileDialogBuilder<tauri::Wry> {
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
        if let Some(answer) = self.walked(|a| a.open.take()) {
            eprintln!("walk: open panel ({pick:?}) answered {answer:?}");
            return answer;
        }
        let title = match pick {
            Pick::Open => "Open a PDF",
            Pick::InsertSource => "Choose a PDF",
            Pick::CompareSource => "Choose a PDF to compare with",
            Pick::Certificate => "Choose your certificate file",
            Pick::TrustCertificates => "Choose certificates to trust",
        };
        self.file_dialog(filters)
            .set_title(title)
            .blocking_pick_file()?
            .into_path()
            .ok()
    }

    fn pick_images(&mut self) -> Vec<PathBuf> {
        if let Some(images) = self.walked(|a| std::mem::take(&mut a.images)) {
            eprintln!("walk: image panel answered {images:?}");
            return images;
        }
        self.file_dialog(&[tandem_app::platform::IMAGES_FILTER])
            .set_title("Choose images")
            .blocking_pick_files()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|f| f.into_path().ok())
            .collect()
    }

    fn pick_folder(&mut self) -> Option<PathBuf> {
        if let Some(answer) = self.walked(|a| a.folder.take()) {
            eprintln!("walk: folder panel answered {answer:?}");
            return answer;
        }
        self.file_dialog(&[])
            .set_title("Choose a folder")
            .blocking_pick_folder()?
            .into_path()
            .ok()
    }

    fn pick_save(
        &mut self,
        purpose: SavePurpose,
        default_path: &Path,
        filters: &[Filter],
    ) -> Option<PathBuf> {
        if let Some(answer) = self.walked(|a| a.save_to.take()) {
            eprintln!(
                "walk: save panel ({purpose:?}, suggested {default_path:?}) answered {answer:?}"
            );
            return answer;
        }
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
        let chosen = builder.blocking_save_file()?.into_path().ok();
        if let (Some(path), Some(targets)) = (&chosen, self.app.try_state::<SaveTargets>())
            && let Ok(mut t) = targets.0.lock()
        {
            t.0 = Some(path.clone());
        }
        chosen
    }

    fn ask(&mut self, question: &MessageBox) -> usize {
        if let Some(choice) = self.walked(|a| a.buttons.pop_front()) {
            let index = choice
                .and_then(|label| question.buttons.iter().position(|b| *b == label))
                .unwrap_or(question.default);
            eprintln!(
                "walk: alert {:?} — {} | {} | buttons {:?} → {:?}",
                question.kind,
                question.message,
                question.detail.clone().unwrap_or_default(),
                question.buttons,
                question.buttons.get(index)
            );
            return index;
        }
        let q = question.clone();
        #[cfg(target_os = "macos")]
        let answer = {
            let style = match q.kind {
                BoxKind::Info | BoxKind::Question => macos::AlertStyle::Info,
                BoxKind::Warning => macos::AlertStyle::Warning,
                BoxKind::Error => macos::AlertStyle::Critical,
            };
            self.on_main(move |mtm| {
                macos::alert(
                    style,
                    &q.message,
                    q.detail.as_deref(),
                    &q.buttons,
                    q.default,
                    q.cancel,
                    mtm,
                )
            })
        };
        #[cfg(windows)]
        let answer = {
            let hwnd = self.hwnd();
            self.on_main(move || {
                crate::win::alert(
                    hwnd,
                    q.kind,
                    &q.message,
                    q.detail.as_deref(),
                    &q.buttons,
                    q.default,
                    q.cancel,
                )
            })
        };
        #[cfg(target_os = "linux")]
        let answer = {
            let app = self.app.clone();
            self.on_main(move || {
                let parent = app
                    .get_webview_window("main")
                    .and_then(|w| w.gtk_window().ok());
                crate::linux::alert(
                    parent.as_ref(),
                    q.kind,
                    &q.message,
                    q.detail.as_deref(),
                    &q.buttons,
                    q.default,
                    q.cancel,
                )
            })
        };
        answer.unwrap_or(question.cancel)
    }

    fn open_external(&mut self, uri: &str) {
        if self.walk.is_some() {
            eprintln!("walk: would open {uri}");
            return;
        }
        // The host has already checked the scheme (http, https, mailto) and asked first.
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("/usr/bin/open").arg(uri).spawn();
        #[cfg(windows)]
        crate::win::open_uri(uri);
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(uri).spawn();
    }

    fn print(&mut self, pdf: &[u8], job_name: &str) -> PrintOutcome {
        if self.walk.is_some() {
            eprintln!(
                "walk: print panel would show {} bytes of PDF as job {job_name:?}",
                pdf.len()
            );
            return PrintOutcome::Cancelled;
        }
        let bytes = pdf.to_vec();
        let title = job_name.to_string();
        #[cfg(target_os = "macos")]
        let outcome = match self.on_main(move |mtm| macos::print_pdf(&bytes, &title, mtm)) {
            Some(macos::Printed::Done) => PrintOutcome::Printed,
            Some(macos::Printed::Cancelled) => PrintOutcome::Cancelled,
            _ => PrintOutcome::Failed,
        };
        #[cfg(windows)]
        let outcome = {
            // The pages are drawn here, on the core thread; only the dialog needs the
            // main thread.
            let mut doc = match tandem_app::printing::PrintDoc::open(&bytes) {
                Ok(doc) => doc,
                Err(e) => {
                    eprintln!("print: {e}");
                    return PrintOutcome::Failed;
                }
            };
            let (hwnd, count) = (self.hwnd(), doc.page_count());
            match self.on_main(move || crate::win::print_dialog(hwnd, count)) {
                Some(crate::win::Dialog::Print(setup)) => {
                    match crate::win::print_pages(setup, &mut doc, &title) {
                        Ok(()) => PrintOutcome::Printed,
                        Err(e) => {
                            eprintln!("print: {e}");
                            PrintOutcome::Failed
                        }
                    }
                }
                Some(crate::win::Dialog::Cancelled) => PrintOutcome::Cancelled,
                Some(crate::win::Dialog::Failed(e)) => {
                    eprintln!("print: {e}");
                    PrintOutcome::Failed
                }
                None => PrintOutcome::Failed,
            }
        };
        #[cfg(target_os = "linux")]
        let outcome = {
            let app = self.app.clone();
            match self.on_main(move || {
                let parent = app
                    .get_webview_window("main")
                    .and_then(|w| w.gtk_window().ok());
                crate::linux::print_pdf(parent.as_ref(), &bytes, &title)
            }) {
                Some(crate::linux::Printed::Done) => PrintOutcome::Printed,
                Some(crate::linux::Printed::Cancelled) => PrintOutcome::Cancelled,
                Some(crate::linux::Printed::Failed(e)) => {
                    eprintln!("print: {e}");
                    PrintOutcome::Failed
                }
                None => PrintOutcome::Failed,
            }
        };
        outcome
    }

    /// macOS: the system's Open Recent. Windows and Linux show the app's own list
    /// (`menu::RecentMenu`, kept in step with each state).
    fn add_recent_document(&mut self, path: &Path) {
        #[cfg(target_os = "macos")]
        {
            let path = path.to_path_buf();
            self.on_main(move |mtm| macos::note_recent(&path, mtm));
        }
        #[cfg(not(target_os = "macos"))]
        let _ = path;
    }

    fn clear_recent_documents(&mut self) {
        #[cfg(target_os = "macos")]
        self.on_main(macos::clear_recent);
    }

    fn set_represented_file(&mut self, path: Option<&Path>, title: &str) {
        let Some(window) = self.app.get_webview_window("main") else {
            return;
        };
        let path = path.map(Path::to_path_buf);
        self.represented = path.clone();
        if let Some(targets) = self.app.try_state::<SaveTargets>()
            && let Ok(mut t) = targets.0.lock()
        {
            t.1 = path.clone();
        }
        #[cfg(target_os = "macos")]
        {
            let _ = window.set_title(title);
            let edited = self.edited;
            let ns = window.ns_window().map(|p| p as usize).unwrap_or(0);
            self.on_main(move |_| {
                macos::set_document(ns as *mut std::ffi::c_void, path.as_deref(), edited)
            });
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = window;
            self.title = title.to_string();
            self.show_title();
        }
    }

    fn set_document_state(&mut self, edited: bool, save: bool, save_as: bool) {
        self.edited = edited;
        if let Some(items) = self.app.try_state::<crate::menu::MenuItems<tauri::Wry>>() {
            items.set_enabled([
                ("save:in-place", save),
                ("save:as", save_as),
                ("save:copy", save_as),
            ]);
        }
        #[cfg(target_os = "macos")]
        {
            let Some(window) = self.app.get_webview_window("main") else {
                return;
            };
            let ns = window.ns_window().map(|p| p as usize).unwrap_or(0);
            let path = self.represented.clone();
            self.on_main(move |_| {
                macos::set_document(ns as *mut std::ffi::c_void, path.as_deref(), edited)
            });
        }
        #[cfg(not(target_os = "macos"))]
        self.show_title();
    }
}

/// Windows and Linux: the window title, "report.pdf — Tandem PDF", with "(edited)" after
/// the name while there are unsaved changes; the start screen's is "Tandem PDF".
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn pc_title(name: &str, edited: bool) -> String {
    if name == "Tandem PDF" {
        return name.to_string();
    }
    let mark = if edited { " (edited)" } else { "" };
    format!("{name}{mark} — Tandem PDF")
}

/// Puts plain text on the clipboard (page text copied with ⌘C / Ctrl+C).
pub fn copy_text(app: &AppHandle, text: String) {
    #[cfg(windows)]
    let hwnd = app
        .get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize)
        .unwrap_or(0);
    let _ = app.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        if objc2::MainThreadMarker::new().is_some() {
            crate::macos::copy_text(&text);
        }
        #[cfg(windows)]
        if !crate::win::copy_text(hwnd, &text) {
            eprintln!("copy: the clipboard was busy");
        }
        #[cfg(target_os = "linux")]
        crate::linux::copy_text(&text);
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn pc_titles() {
        assert_eq!(super::pc_title("Tandem PDF", false), "Tandem PDF");
        assert_eq!(super::pc_title("a.pdf", false), "a.pdf — Tandem PDF");
        assert_eq!(
            super::pc_title("a.pdf", true),
            "a.pdf (edited) — Tandem PDF"
        );
    }
}
