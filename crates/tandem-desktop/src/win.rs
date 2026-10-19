//! The Windows native calls (the counterpart of macos.rs):
//! - [`alert`]: a question with the app's own buttons, default and Esc button (a Task Dialog;
//!   the dialog plugin cannot choose the default button);
//! - [`print_dialog`] + [`print_pages`]: the system Print dialog (printer, pages, copies),
//!   then the pages drawn by the engine sent to the printer through GDI;
//! - [`copy_text`]: Unicode text on the clipboard;
//! - [`snapshot_png`]: what the web view shows, as a PNG (WebView2 `CapturePreview`), for
//!   screenshot runs (`--shot`, `--walk`).
//!
//! Questions and the Print dialog run on the main thread; printing the pages runs on the
//! calling (core) thread, since a printer's device context is not tied to a thread.
#![allow(unsafe_code)]

use std::path::PathBuf;
use tandem_app::platform::BoxKind;
use tandem_app::printing::{self, PrintDoc};
use windows_sys::Win32::Foundation::{GlobalFree, HGLOBAL, HWND};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps, HALFTONE,
    HDC, HORZRES, LOGPIXELSX, SRCCOPY, SetStretchBltMode, StretchDIBits, VERTRES,
};
use windows_sys::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::UI::Controls::Dialogs::{
    PD_NOCURRENTPAGE, PD_NOSELECTION, PD_PAGENUMS, PD_RESULT_PRINT, PD_RETURNDC,
    PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGEXW, PRINTPAGERANGE, PrintDlgExW, START_PAGE_GENERAL,
};
use windows_sys::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TD_ERROR_ICON, TD_INFORMATION_ICON, TD_WARNING_ICON,
    TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW, TaskDialogIndirect,
};
use windows_sys::Win32::UI::WindowsAndMessaging::IDCANCEL;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The first button's id; button `i` is `FIRST_BUTTON + i`.
const FIRST_BUTTON: i32 = 100;

/// A question over the window `hwnd` (0: none). `buttons[default]` gets Return,
/// `buttons[cancel]` Esc and the close box. Returns the index of the button chosen.
pub fn alert(
    hwnd: isize,
    kind: BoxKind,
    message: &str,
    detail: Option<&str>,
    buttons: &[String],
    default: usize,
    cancel: usize,
) -> usize {
    let title = wide("Tandem PDF");
    let instruction = wide(message);
    let content = detail.map(wide);
    let labels: Vec<Vec<u16>> = buttons.iter().map(|b| wide(b)).collect();
    let items: Vec<TASKDIALOG_BUTTON> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| TASKDIALOG_BUTTON {
            nButtonID: FIRST_BUTTON + i as i32,
            pszButtonText: label.as_ptr(),
        })
        .collect();
    let mut config: TASKDIALOGCONFIG = unsafe { std::mem::zeroed() };
    config.cbSize = std::mem::size_of::<TASKDIALOGCONFIG>() as u32;
    config.hwndParent = hwnd as HWND;
    config.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW;
    config.pszWindowTitle = title.as_ptr();
    config.Anonymous1.pszMainIcon = match kind {
        BoxKind::Warning => TD_WARNING_ICON,
        BoxKind::Error => TD_ERROR_ICON,
        BoxKind::Info | BoxKind::Question => TD_INFORMATION_ICON,
    };
    config.pszMainInstruction = instruction.as_ptr();
    config.pszContent = content.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
    config.cButtons = items.len() as u32;
    config.pButtons = items.as_ptr();
    config.nDefaultButton = FIRST_BUTTON + default.min(buttons.len().saturating_sub(1)) as i32;
    let mut chosen = 0i32;
    // SAFETY: every pointer in `config` points into `title`, `instruction`, `content`,
    // `labels` and `items`, which outlive the call; `chosen` is a valid out pointer.
    let hr = unsafe {
        TaskDialogIndirect(
            &config,
            &mut chosen,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if hr < 0 || chosen == IDCANCEL {
        return cancel;
    }
    let index = (chosen - FIRST_BUTTON) as usize;
    if index < buttons.len() { index } else { cancel }
}

/// What the Print dialog answered.
pub enum Dialog {
    /// Print these pages (0-based) on this printer.
    Print(PrintSetup),
    Cancelled,
    Failed(String),
}

/// A printer chosen in the Print dialog and the pages to print. Its device context is
/// released by [`print_pages`].
pub struct PrintSetup {
    dc: isize,
    pub pages: Vec<usize>,
}

/// The system Print dialog over `hwnd` for a job of `count` pages (main thread).
pub fn print_dialog(hwnd: isize, count: usize) -> Dialog {
    let mut ranges = [PRINTPAGERANGE {
        nFromPage: 1,
        nToPage: count as u32,
    }; 16];
    let mut pd: PRINTDLGEXW = unsafe { std::mem::zeroed() };
    pd.lStructSize = std::mem::size_of::<PRINTDLGEXW>() as u32;
    pd.hwndOwner = hwnd as HWND;
    pd.Flags = PD_RETURNDC | PD_NOSELECTION | PD_NOCURRENTPAGE | PD_USEDEVMODECOPIESANDCOLLATE;
    pd.nMinPage = 1;
    pd.nMaxPage = count.max(1) as u32;
    pd.nCopies = 1;
    pd.nMaxPageRanges = ranges.len() as u32;
    pd.nPageRanges = 1;
    pd.lpPageRanges = ranges.as_mut_ptr();
    pd.nStartPage = START_PAGE_GENERAL;
    // SAFETY: `pd` is initialised as the API requires; `ranges` outlives the call.
    let hr = unsafe { PrintDlgExW(&mut pd) };
    let free = |pd: &PRINTDLGEXW| {
        // SAFETY: the handles came from the dialog (or are null, which GlobalFree ignores).
        unsafe {
            if !pd.hDevMode.is_null() {
                GlobalFree(pd.hDevMode as HGLOBAL);
            }
            if !pd.hDevNames.is_null() {
                GlobalFree(pd.hDevNames as HGLOBAL);
            }
        }
    };
    if hr < 0 {
        free(&pd);
        return Dialog::Failed(format!("the Print dialog could not open ({hr:#x})"));
    }
    if pd.dwResultAction != PD_RESULT_PRINT {
        free(&pd);
        if !pd.hDC.is_null() {
            // SAFETY: a DC the dialog returned.
            unsafe { DeleteDC(pd.hDC) };
        }
        return Dialog::Cancelled;
    }
    let pages = if pd.Flags & PD_PAGENUMS != 0 {
        let mut pages = Vec::new();
        for r in &ranges[..(pd.nPageRanges as usize).min(ranges.len())] {
            for p in r.nFromPage.min(r.nToPage)..=r.nFromPage.max(r.nToPage) {
                let p = p as usize;
                if p >= 1 && p <= count && !pages.contains(&(p - 1)) {
                    pages.push(p - 1);
                }
            }
        }
        pages
    } else {
        (0..count).collect()
    };
    free(&pd);
    if pd.hDC.is_null() {
        return Dialog::Failed("the printer could not be opened".into());
    }
    Dialog::Print(PrintSetup {
        dc: pd.hDC as isize,
        pages,
    })
}

/// Prints `setup.pages` of `doc` on the chosen printer: each page drawn by the engine at
/// the printer's resolution (up to 300 dpi), as large as fits the printable area, centred.
pub fn print_pages(setup: PrintSetup, doc: &mut PrintDoc, title: &str) -> Result<(), String> {
    let dc = setup.dc as HDC;
    let name = wide(title);
    let mut info: DOCINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<DOCINFOW>() as i32;
    info.lpszDocName = name.as_ptr();
    // SAFETY: `dc` is the printer DC from the dialog; `info` points into `name`.
    let result = unsafe {
        if StartDocW(dc, &info) <= 0 {
            Err("the printer did not start the job".to_string())
        } else {
            let dpi = f64::from(GetDeviceCaps(dc, LOGPIXELSX as i32).max(72));
            let (area_w, area_h) = (
                f64::from(GetDeviceCaps(dc, HORZRES as i32)),
                f64::from(GetDeviceCaps(dc, VERTRES as i32)),
            );
            SetStretchBltMode(dc, HALFTONE);
            let mut outcome = Ok(());
            for &page in &setup.pages {
                let picture = match doc.render(page, dpi) {
                    Ok(p) => p,
                    Err(e) => {
                        outcome = Err(e);
                        break;
                    }
                };
                let bgra = printing::to_bgra_on_white(&picture.rgba);
                let (x, y, w, h) = printing::fit(picture.width, picture.height, area_w, area_h);
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: picture.width as i32,
                    // Negative: the rows run top to bottom.
                    biHeight: -(picture.height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..std::mem::zeroed()
                };
                if StartPage(dc) <= 0 {
                    outcome = Err("the printer did not take a page".into());
                    break;
                }
                StretchDIBits(
                    dc,
                    x.round() as i32,
                    y.round() as i32,
                    w.round() as i32,
                    h.round() as i32,
                    0,
                    0,
                    picture.width as i32,
                    picture.height as i32,
                    bgra.as_ptr().cast(),
                    &bmi,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
                if EndPage(dc) <= 0 {
                    outcome = Err("the printer did not finish a page".into());
                    break;
                }
            }
            if outcome.is_ok() {
                EndDoc(dc);
            } else {
                AbortDoc(dc);
            }
            outcome
        }
    };
    // SAFETY: the DC is ours to release.
    unsafe { DeleteDC(dc) };
    result
}

/// Puts `text` on the clipboard as Unicode text. False when the clipboard was busy.
pub fn copy_text(hwnd: isize, text: &str) -> bool {
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    const CF_UNICODETEXT: u32 = 13;
    // SAFETY: the clipboard is opened and closed here; the global block is filled while
    // locked and handed to the clipboard, which then owns it (freed by us only on failure).
    unsafe {
        if OpenClipboard(hwnd as HWND) == 0 {
            return false;
        }
        EmptyClipboard();
        let block = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2);
        let mut ok = false;
        if !block.is_null() {
            let dst = GlobalLock(block).cast::<u16>();
            if !dst.is_null() {
                std::ptr::copy_nonoverlapping(units.as_ptr(), dst, units.len());
                GlobalUnlock(block);
                ok = !SetClipboardData(CF_UNICODETEXT, block).is_null();
            }
            if !ok {
                GlobalFree(block);
            }
        }
        CloseClipboard();
        ok
    }
}

/// Saves what the web view shows to a PNG file (main thread); `done` gets the outcome.
pub fn snapshot_png(
    controller: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller,
    path: PathBuf,
    done: impl FnOnce(Result<(), String>) + 'static,
) {
    use webview2_com::CapturePreviewCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG;
    use windows::Win32::System::Com::StructuredStorage::{
        CreateStreamOnHGlobal, GetHGlobalFromStream,
    };
    use windows::Win32::System::Com::{STATFLAG_NONAME, STATSTG};
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    // The outcome goes to `done` once, from the completion handler or from a failed start.
    let done = std::rc::Rc::new(std::cell::RefCell::new(Some(done)));
    let finish = {
        let done = done.clone();
        move |outcome: Result<(), String>| {
            if let Some(done) = done.borrow_mut().take() {
                done(outcome);
            }
        }
    };
    let finish_later = finish.clone();
    // SAFETY: COM calls on the main thread with live interfaces; the stream's memory is read
    // while locked, no further than the stream's own size.
    let started = unsafe {
        (|| -> windows::core::Result<()> {
            let webview = controller.CoreWebView2()?;
            let stream = CreateStreamOnHGlobal(Default::default(), true)?;
            let read = stream.clone();
            let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
                let outcome = result.map_err(|e| e.to_string()).and_then(|()| {
                    let mut stat = STATSTG::default();
                    read.Stat(&mut stat, STATFLAG_NONAME)
                        .map_err(|e| e.to_string())?;
                    let memory = GetHGlobalFromStream(&read).map_err(|e| e.to_string())?;
                    let ptr = GlobalLock(memory).cast::<u8>();
                    let size = stat.cbSize as usize;
                    if ptr.is_null() || size == 0 {
                        return Err("the web view gave no picture".to_string());
                    }
                    let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
                    let _ = GlobalUnlock(memory);
                    std::fs::write(&path, bytes).map_err(|e| e.to_string())
                });
                finish_later(outcome);
                Ok(())
            }));
            webview.CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &handler,
            )
        })()
    };
    if let Err(e) = started {
        finish(Err(e.to_string()));
    }
}
