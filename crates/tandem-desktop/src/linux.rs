//! The Linux native calls (the counterpart of macos.rs), through GTK 3, which Tauri already
//! uses for the window. Everything here runs on the main (GTK) thread.
//! - [`alert`]: a GTK message dialog with the app's own buttons, default and Esc button;
//! - [`print_pdf`]: the GTK Print dialog (printers, pages, copies, "Print to File"), each
//!   page drawn by the engine and painted through Cairo;
//! - [`copy_text`]: text on the clipboard;
//! - [`snapshot_png`]: what the web view shows, as a PNG (WebKitGTK), for `--shot`/`--walk`.
//!
//! No `unsafe`: the gtk, cairo and webkit2gtk crates wrap the C calls.

use gtk::prelude::*;
use std::path::PathBuf;
use tandem_app::platform::BoxKind;
use tandem_app::printing::{self, PrintDoc};

/// A question over `parent`. `buttons[default]` gets Return, `buttons[cancel]` Esc and the
/// close box. Returns the index of the button chosen.
pub fn alert(
    parent: Option<&gtk::ApplicationWindow>,
    kind: BoxKind,
    message: &str,
    detail: Option<&str>,
    buttons: &[String],
    default: usize,
    cancel: usize,
) -> usize {
    let kind = match kind {
        BoxKind::Warning => gtk::MessageType::Warning,
        BoxKind::Error => gtk::MessageType::Error,
        BoxKind::Question => gtk::MessageType::Question,
        BoxKind::Info => gtk::MessageType::Info,
    };
    let dialog = gtk::MessageDialog::new(
        parent,
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        kind,
        gtk::ButtonsType::None,
        message,
    );
    dialog.set_title("Tandem PDF");
    dialog.set_secondary_text(detail);
    for (i, label) in buttons.iter().enumerate() {
        // GTK reads "_" as a mnemonic mark.
        dialog.add_button(
            &label.replace('_', "__"),
            gtk::ResponseType::Other(i as u16),
        );
    }
    dialog.set_default_response(gtk::ResponseType::Other(default as u16));
    let answer = dialog.run();
    dialog.close();
    match answer {
        gtk::ResponseType::Other(i) if usize::from(i) < buttons.len() => usize::from(i),
        _ => cancel,
    }
}

/// How a print went.
pub enum Printed {
    Done,
    Cancelled,
    Failed(String),
}

/// The GTK Print dialog for the print PDF `pdf`, then the job.
pub fn print_pdf(parent: Option<&gtk::ApplicationWindow>, pdf: &[u8], title: &str) -> Printed {
    let doc = match PrintDoc::open(pdf) {
        Ok(doc) => doc,
        Err(e) => return Printed::Failed(e),
    };
    let count = doc.page_count();
    let doc = std::rc::Rc::new(std::cell::RefCell::new(doc));
    let failed: std::rc::Rc<std::cell::RefCell<Option<String>>> = Default::default();
    let op = gtk::PrintOperation::new();
    op.set_n_pages(count as i32);
    op.set_job_name(title);
    op.set_use_full_page(false);
    op.set_embed_page_setup(true);
    let (draw_doc, draw_failed) = (doc.clone(), failed.clone());
    op.connect_draw_page(move |op, ctx, page| {
        let picture = match draw_doc
            .borrow_mut()
            .render(page.max(0) as usize, ctx.dpi_x())
        {
            Ok(p) => p,
            Err(e) => {
                *draw_failed.borrow_mut() = Some(e);
                op.cancel();
                return;
            }
        };
        let bgra = printing::to_bgra_on_white(&picture.rgba);
        let Ok(surface) = gtk::cairo::ImageSurface::create_for_data(
            bgra,
            gtk::cairo::Format::ARgb32,
            picture.width as i32,
            picture.height as i32,
            picture.width as i32 * 4,
        ) else {
            *draw_failed.borrow_mut() = Some("a page could not be drawn".into());
            op.cancel();
            return;
        };
        let Some(cr) = ctx.cairo_context() else {
            *draw_failed.borrow_mut() = Some("the printer gave nothing to draw on".into());
            op.cancel();
            return;
        };
        let (x, y, w, h) = printing::fit(picture.width, picture.height, ctx.width(), ctx.height());
        cr.save().ok();
        cr.translate(x, y);
        cr.scale(w / f64::from(picture.width), h / f64::from(picture.height));
        if cr.set_source_surface(&surface, 0.0, 0.0).is_ok() {
            let _ = cr.paint();
        }
        cr.restore().ok();
    });
    let result = op.run(gtk::PrintOperationAction::PrintDialog, parent);
    if let Some(e) = failed.borrow_mut().take() {
        return Printed::Failed(e);
    }
    match result {
        Ok(gtk::PrintOperationResult::Apply | gtk::PrintOperationResult::InProgress) => {
            Printed::Done
        }
        Ok(gtk::PrintOperationResult::Cancel) => Printed::Cancelled,
        Ok(_) => Printed::Failed("the printer reported a problem".into()),
        Err(e) => Printed::Failed(e.to_string()),
    }
}

/// Puts plain text on the clipboard (and keeps it there after the app quits).
pub fn copy_text(text: &str) {
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    clipboard.set_text(text);
    clipboard.store();
}

/// Saves what the web view shows to a PNG file; `done` gets the outcome.
pub fn snapshot_png(
    webview: &webkit2gtk::WebView,
    path: PathBuf,
    done: impl FnOnce(Result<(), String>) + 'static,
) {
    use webkit2gtk::WebViewExt;
    webview.snapshot(
        webkit2gtk::SnapshotRegion::Visible,
        webkit2gtk::SnapshotOptions::NONE,
        None::<&gtk::gio::Cancellable>,
        move |result| {
            let outcome = result.map_err(|e| e.to_string()).and_then(|surface| {
                let image = gtk::cairo::ImageSurface::try_from(surface)
                    .map_err(|_| "the web view gave no picture".to_string())?;
                let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
                image.write_to_png(&mut file).map_err(|e| e.to_string())
            });
            done(outcome);
        },
    );
}

/// `file://` URL of a path, with everything but unreserved characters and `/` escaped.
pub fn file_url(path: &std::path::Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Shows files in the file manager, selected where it can (the FileManager1 D-Bus call that
/// Files, Dolphin, Nemo and others answer), else their folder.
pub fn reveal(paths: &[PathBuf]) {
    let Some(first) = paths.first() else {
        return;
    };
    let urls: Vec<String> = paths.iter().map(|p| format!("'{}'", file_url(p))).collect();
    let shown = std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.FileManager1",
            "--object-path",
            "/org/freedesktop/FileManager1",
            "--method",
            "org.freedesktop.FileManager1.ShowItems",
            &format!("[{}]", urls.join(",")),
            "",
        ])
        .output()
        .is_ok_and(|o| o.status.success());
    if !shown && let Some(dir) = first.parent() {
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_urls_are_escaped() {
        assert_eq!(
            super::file_url(std::path::Path::new("/home/a b/ü.pdf")),
            "file:///home/a%20b/%C3%BC.pdf"
        );
    }
}
