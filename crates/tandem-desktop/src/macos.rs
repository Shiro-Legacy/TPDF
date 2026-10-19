//! The app's only native calls (docs/rust-and-ux-plan.md §4, Print):
//! - [`print_pdf`] hands a PDF to macOS's own print panel through PDFKit, so
//!   printing uses the system dialog, preview, printers and "Save as PDF".
//! - [`snapshot_png`] saves what the web view shows (screenshot runs; needs
//!   no Screen Recording permission).
//!
//! Both must run on the main thread; callers use `run_on_main_thread`.
#![allow(unsafe_code)]

use objc2::MainThreadMarker;
use objc2::AnyThread;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSPrintInfo};
use objc2_foundation::{NSData, NSDictionary, NSError};
use objc2_pdf_kit::{PDFDocument, PDFPrintScalingMode};
use objc2_web_kit::WKWebView;
use std::ffi::c_void;

/// How a print went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Printed {
    Printed,
    Cancelled,
    /// PDFKit could not read the bytes (never expected: the engine wrote them).
    Unreadable,
}

/// Shows the system print panel for `bytes` (app-modal) and waits for it.
pub fn print_pdf(bytes: &[u8], mtm: MainThreadMarker) -> Printed {
    let data = NSData::with_bytes(bytes);
    // SAFETY: plain AppKit/PDFKit calls on the main thread (`mtm` proves it)
    // with objects we own; no pointers escape.
    unsafe {
        let Some(document) = PDFDocument::initWithData(PDFDocument::alloc(), &data) else {
            return Printed::Unreadable;
        };
        let info = NSPrintInfo::sharedPrintInfo();
        let Some(operation) =
            document.printOperationForPrintInfo_scalingMode_autoRotate(Some(&info), PDFPrintScalingMode::PageScaleDownToFit, true, mtm)
        else {
            return Printed::Unreadable;
        };
        operation.setShowsPrintPanel(true);
        operation.setShowsProgressPanel(true);
        if operation.runOperation() { Printed::Printed } else { Printed::Cancelled }
    }
}

/// Snapshots the web view to a PNG file; `done` gets the outcome.
///
/// `webview` is Tauri's `PlatformWebview::inner()` (a `WKWebView*`).
pub fn snapshot_png(webview: *mut c_void, path: std::path::PathBuf, done: impl Fn(Result<(), String>) + 'static) {
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
                let why = if error.is_null() { "no image".to_string() } else { (*error).localizedDescription().to_string() };
                done(Err(why));
                return;
            }
            let image: &NSImage = &*image;
            let png = image
                .TIFFRepresentation()
                .and_then(|tiff| NSBitmapImageRep::imageRepWithData(&tiff))
                .and_then(|rep| rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()));
            match png {
                Some(data) => done(std::fs::write(&path, data.to_vec()).map_err(|e| e.to_string())),
                None => done(Err("could not encode PNG".into())),
            }
        });
        view.takeSnapshotWithConfiguration_completionHandler(None, &block);
    }
}

