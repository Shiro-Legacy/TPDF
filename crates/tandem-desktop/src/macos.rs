//! The app's only native calls (docs/rust-and-ux-plan.md §4, Print):
//! - [`print_pdf`] hands a PDF to macOS's own print panel through PDFKit, so
//!   printing uses the system dialog, preview, printers and "Save as PDF".
//! - [`snapshot_png`] saves what the web view shows (screenshot runs; needs
//!   no Screen Recording permission).
//!
//! Both must run on the main thread; callers use `run_on_main_thread`.
#![allow(unsafe_code)]

use objc2::AnyThread;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSPrintInfo};
use objc2_foundation::{NSData, NSDictionary, NSError};
use objc2_pdf_kit::{PDFDocument, PDFPrintScalingMode};
use objc2_web_kit::WKWebView;
use std::ffi::c_void;

/// How a print went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Printed {
    Done,
    Cancelled,
    /// PDFKit could not read the bytes (never expected: the engine wrote them).
    Unreadable,
}

/// Shows the system print panel for `bytes` (app-modal) and waits for it.
/// `title` names the job in the print queue (the document's name).
pub fn print_pdf(bytes: &[u8], title: &str, mtm: MainThreadMarker) -> Printed {
    let data = NSData::with_bytes(bytes);
    // SAFETY: plain AppKit/PDFKit calls on the main thread (`mtm` proves it)
    // with objects we own; no pointers escape.
    unsafe {
        let Some(document) = PDFDocument::initWithData(PDFDocument::alloc(), &data) else {
            return Printed::Unreadable;
        };
        let info = NSPrintInfo::sharedPrintInfo();
        let Some(operation) = document.printOperationForPrintInfo_scalingMode_autoRotate(
            Some(&info),
            PDFPrintScalingMode::PageScaleDownToFit,
            true,
            mtm,
        ) else {
            return Printed::Unreadable;
        };
        if !title.is_empty() {
            operation.setJobTitle(Some(&objc2_foundation::NSString::from_str(title)));
        }
        operation.setShowsPrintPanel(true);
        operation.setShowsProgressPanel(true);
        if operation.runOperation() {
            Printed::Done
        } else {
            Printed::Cancelled
        }
    }
}

/// Snapshots the web view to a PNG file; `done` gets the outcome.
///
/// `webview` is Tauri's `PlatformWebview::inner()` (a `WKWebView*`).
pub fn snapshot_png(
    webview: *mut c_void,
    path: std::path::PathBuf,
    done: impl FnOnce(Result<(), String>) + 'static,
) {
    // The completion block may be called more than once in principle; `done` runs once.
    let once = std::cell::Cell::new(Some(done));
    let done = move |r: Result<(), String>| {
        if let Some(done) = once.take() {
            done(r);
        }
    };
    if webview.is_null() {
        done(Err("no web view".into()));
        return;
    }
    // SAFETY: Tauri hands us its live WKWebView on the main thread; we only
    // borrow it for this call. The block owns `path` and `done`.
    unsafe {
        let view: &WKWebView = &*(webview as *const WKWebView);
        let block = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            if image.is_null() {
                let why = if error.is_null() {
                    "no image".to_string()
                } else {
                    (*error).localizedDescription().to_string()
                };
                done(Err(why));
                return;
            }
            let image: &NSImage = &*image;
            let png = image
                .TIFFRepresentation()
                .and_then(|tiff| NSBitmapImageRep::imageRepWithData(&tiff))
                .and_then(|rep| {
                    rep.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )
                });
            match png {
                Some(data) => done(std::fs::write(&path, data.to_vec()).map_err(|e| e.to_string())),
                None => done(Err("could not encode PNG".into())),
            }
        });
        view.takeSnapshotWithConfiguration_completionHandler(None, &block);
    }
}

/// A native alert (NSAlert, app-modal). `buttons[default]` gets Return,
/// `buttons[cancel]` gets Escape. Returns the index of the button chosen.
pub fn alert(
    style: AlertStyle,
    message: &str,
    detail: Option<&str>,
    buttons: &[String],
    default: usize,
    cancel: usize,
    mtm: MainThreadMarker,
) -> usize {
    use objc2_app_kit::{NSAlert, NSAlertStyle};
    use objc2_foundation::NSString;
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(message));
    if let Some(detail) = detail {
        alert.setInformativeText(&NSString::from_str(detail));
    }
    alert.setAlertStyle(match style {
        AlertStyle::Info => NSAlertStyle::Informational,
        AlertStyle::Warning => NSAlertStyle::Warning,
        AlertStyle::Critical => NSAlertStyle::Critical,
    });
    // NSAlert puts the first button on the right and gives it Return; add the
    // default first, then the rest in order, and map the answer back.
    let first = default.min(buttons.len().saturating_sub(1));
    let mut order: Vec<usize> = vec![first];
    order.extend((0..buttons.len()).filter(|i| *i != first));
    for (position, index) in order.iter().enumerate() {
        let button = alert.addButtonWithTitle(&NSString::from_str(&buttons[*index]));
        if *index == cancel && position != 0 {
            button.setKeyEquivalent(&NSString::from_str("\u{1b}"));
        }
    }
    let response = alert.runModal();
    // NSAlertFirstButtonReturn = 1000.
    let position = (response - 1000).max(0) as usize;
    order.get(position).copied().unwrap_or(cancel)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertStyle {
    Info,
    Warning,
    Critical,
}

/// Adds a file to File ▸ Open Recent and the Dock menu.
pub fn note_recent(path: &std::path::Path, mtm: MainThreadMarker) {
    use objc2_app_kit::NSDocumentController;
    use objc2_foundation::{NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    NSDocumentController::sharedDocumentController(mtm).noteNewRecentDocumentURL(&url);
}

pub fn clear_recent(mtm: MainThreadMarker) {
    use objc2_app_kit::NSDocumentController;
    // SAFETY: a plain AppKit call on the main thread with no sender.
    unsafe { NSDocumentController::sharedDocumentController(mtm).clearRecentDocuments(None) };
}

/// The window's proxy icon file and its edited dot.
pub fn set_document(ns_window: *mut c_void, path: Option<&std::path::Path>, edited: bool) {
    use objc2_app_kit::NSWindow;
    use objc2_foundation::NSString;
    if ns_window.is_null() {
        return;
    }
    // SAFETY: Tauri's live NSWindow, borrowed on the main thread for these two setters.
    let window: &NSWindow = unsafe { &*(ns_window as *const NSWindow) };
    let name = path
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    window.setRepresentedFilename(&NSString::from_str(&name));
    window.setDocumentEdited(edited);
}

/// Puts plain text on the general pasteboard.
pub fn copy_text(text: &str) {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::NSString;
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    // SAFETY: a framework constant, read once.
    let kind = unsafe { NSPasteboardTypeString };
    board.setString_forType(&NSString::from_str(text), kind);
}

/// Opens a web or mail link in the default app (no `/usr/bin/open` child, which the App
/// Sandbox would not want).
pub fn open_uri(uri: &str) {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(uri)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

/// Shows files in the Finder, selected (a save toast's "Show in Finder").
pub fn reveal(paths: &[std::path::PathBuf]) {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSArray, NSString, NSURL};
    let urls: Vec<objc2::rc::Retained<NSURL>> = paths
        .iter()
        .map(|p| NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy())))
        .collect();
    let list = NSArray::from_retained_slice(&urls);
    NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&list);
}

/// Gives the Help menu's item the Mac's ⌘? (a shifted "/", which the accelerator parser
/// cannot express). Runs on the main thread right after the menu bar is set.
pub fn help_key(title: &str) {
    use objc2_app_kit::{NSApplication, NSEventModifierFlags};
    use objc2_foundation::NSString;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(menu) = NSApplication::sharedApplication(mtm).mainMenu() else {
        return;
    };
    let help = menu
        .itemWithTitle(&NSString::from_str("Help"))
        .and_then(|item| item.submenu());
    let Some(help) = help else {
        return;
    };
    if let Some(item) = help.itemWithTitle(&NSString::from_str(title)) {
        item.setKeyEquivalent(&NSString::from_str("?"));
        item.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
    }
    NSApplication::sharedApplication(mtm).setHelpMenu(Some(&help));
}
