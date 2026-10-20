//! The Windows native calls (the counterpart of macos.rs):
//! - [`alert`]: a question with the app's own buttons, default and Esc button (a Task Dialog;
//!   the dialog plugin cannot choose the default button);
//! - [`print_dialog`] + [`print_pages`]: the system Print dialog (printer, pages, copies),
//!   then the pages drawn by the engine sent to the printer through GDI;
//! - [`copy_text`]: Unicode text on the clipboard;
//! - [`snapshot_png`]: what the web view shows, as a PNG (WebView2 `CapturePreview`), for
//!   screenshot runs (`--shot`, `--walk`);
//! - [`menu_keys`]: menu shortcuts while the page has the keyboard. WebView2 takes key
//!   presses in its own (browser) process, so the window's accelerator table never sees
//!   them; its AcceleratorKeyPressed event hands us each Ctrl/Alt/function key first. The
//!   browser's own keys (F5 reload, Ctrl+P page print, Ctrl+F find bar …) are switched off.
//!
//! Questions and the Print dialog run on the main thread; printing the pages runs on the
//! calling (core) thread, since a printer's device context is not tied to a thread.
#![allow(unsafe_code)]

use std::path::PathBuf;
use tandem_app::platform::BoxKind;
use tandem_app::printing::{self, PrintDoc};
use windows_sys::Win32::Foundation::{GlobalFree, HGLOBAL, HWND};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps, HALFTONE, HDC,
    HORZRES, LOGPIXELSX, SRCCOPY, SetStretchBltMode, StretchDIBits, VERTRES,
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
    TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW,
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
    let Some(task_dialog) = task_dialog_indirect() else {
        eprintln!("question: the Task Dialog is not available");
        return cancel;
    };
    let mut chosen = 0i32;
    // SAFETY: every pointer in `config` points into `title`, `instruction`, `content`,
    // `labels` and `items`, which outlive the call; `chosen` is a valid out pointer.
    let hr = unsafe {
        task_dialog(
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

type TaskDialogIndirectFn = unsafe extern "system" fn(
    *const TASKDIALOGCONFIG,
    *mut i32,
    *mut i32,
    *mut windows_sys::core::BOOL,
) -> windows_sys::core::HRESULT;

/// comctl32's `TaskDialogIndirect`, looked up when first asked: it exists only in Common
/// Controls 6, which the app's manifest selects. Linking it directly would stop binaries
/// without that manifest (the test binaries) from starting at all.
fn task_dialog_indirect() -> Option<TaskDialogIndirectFn> {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    let name = wide("comctl32.dll");
    // SAFETY: NUL-terminated names; the library stays loaded for the process; the symbol
    // has exactly this signature (CommCtrl.h).
    unsafe {
        let module = LoadLibraryW(name.as_ptr());
        if module.is_null() {
            return None;
        }
        let symbol = GetProcAddress(module, c"TaskDialogIndirect".as_ptr().cast())?;
        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            TaskDialogIndirectFn,
        >(symbol))
    }
}

/// What the Print dialog answered.
pub enum Dialog {
    /// Print these pages (0-based) on this printer.
    Print(PrintSetup),
    Cancelled,
    /// The machine has no printer (the host then offers to save a PDF).
    NoPrinter,
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
        use windows_sys::Win32::UI::Controls::Dialogs::{
            CommDlgExtendedError, PDERR_NODEFAULTPRN, PDERR_NODEVICES,
        };
        // SAFETY: reads the dialog's last error.
        let why = unsafe { CommDlgExtendedError() };
        if why == PDERR_NODEFAULTPRN || why == PDERR_NODEVICES {
            return Dialog::NoPrinter;
        }
        return Dialog::Failed(format!("the Print dialog could not open ({hr:#x}, {why})"));
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

/// How a print job ended.
pub enum Job {
    Printed,
    /// The printer's own Save dialog (Microsoft Print to PDF, XPS, OneNote) was cancelled.
    Cancelled,
}

/// Rows of a page sent to the printer at a time (drivers can refuse one huge bitmap).
const BAND_ROWS: usize = 256;

/// Prints `setup.pages` of `doc` on the chosen printer (the core thread): each page drawn by
/// the engine at the printer's resolution (up to 300 dpi) and laid out as the Mac's panel
/// does (real size, shrunk only to fit, turned to the paper), sent in bands.
pub fn print_pages(setup: PrintSetup, doc: &mut PrintDoc, title: &str) -> Result<Job, String> {
    use windows_sys::Win32::Foundation::{ERROR_CANCELLED, GetLastError};
    use windows_sys::Win32::Graphics::Gdi::{LOGPIXELSY, SetBrushOrgEx};
    use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    let dc = setup.dc as HDC;
    let name = wide(title);
    let mut info: DOCINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<DOCINFOW>() as i32;
    info.lpszDocName = name.as_ptr();
    // SAFETY: `dc` is the printer DC from the dialog; `info` points into `name`; every
    // bitmap handed to GDI is a live buffer of exactly the size its header says. COM is set
    // up on this thread for printer drivers that show their own Save dialog.
    let result = unsafe {
        let _ = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        if StartDocW(dc, &info) <= 0 {
            if GetLastError() == ERROR_CANCELLED {
                Ok(Job::Cancelled)
            } else {
                Err("the printer did not start the job".to_string())
            }
        } else {
            let dpi_x = f64::from(GetDeviceCaps(dc, LOGPIXELSX as i32).max(72));
            let dpi_y = f64::from(GetDeviceCaps(dc, LOGPIXELSY as i32).max(72));
            let (area_w, area_h) = (
                f64::from(GetDeviceCaps(dc, HORZRES as i32)),
                f64::from(GetDeviceCaps(dc, VERTRES as i32)),
            );
            SetStretchBltMode(dc, HALFTONE);
            SetBrushOrgEx(dc, 0, 0, std::ptr::null_mut());
            let mut outcome = Ok(Job::Printed);
            for &page in &setup.pages {
                let Some(size) = doc.pages.get(page).cloned() else {
                    continue;
                };
                let picture = match doc.render(page, dpi_x.max(dpi_y)) {
                    Ok(p) => p,
                    Err(e) => {
                        outcome = Err(e);
                        break;
                    }
                };
                let at = printing::place(size.width, size.height, area_w, area_h, dpi_x, dpi_y);
                let (mut w, mut h) = (picture.width as usize, picture.height as usize);
                let mut bgra = printing::to_bgra_on_white(&picture.rgba);
                drop(picture);
                if at.rotate {
                    bgra = printing::rotate_clockwise(&bgra, w, h);
                    (w, h) = (h, w);
                }
                if StartPage(dc) <= 0 {
                    outcome = Err("the printer did not take a page".into());
                    break;
                }
                let mut row = 0;
                while row < h && outcome.is_ok() {
                    let rows = BAND_ROWS.min(h - row);
                    let top = (at.y + at.height * row as f64 / h as f64).round() as i32;
                    let bottom = (at.y + at.height * (row + rows) as f64 / h as f64).round() as i32;
                    let mut bmi: BITMAPINFO = std::mem::zeroed();
                    bmi.bmiHeader = BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: w as i32,
                        // Negative: the rows run top to bottom.
                        biHeight: -(rows as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB,
                        ..std::mem::zeroed()
                    };
                    let band = &bgra[row * w * 4..(row + rows) * w * 4];
                    let copied = StretchDIBits(
                        dc,
                        at.x.round() as i32,
                        top,
                        at.width.round() as i32,
                        (bottom - top).max(1),
                        0,
                        0,
                        w as i32,
                        rows as i32,
                        band.as_ptr().cast(),
                        &bmi,
                        DIB_RGB_COLORS,
                        SRCCOPY,
                    );
                    if copied == 0 {
                        outcome = Err("the printer did not take a page's picture".into());
                    }
                    row += rows;
                }
                if outcome.is_err() {
                    break;
                }
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
    // Windows text has CRLF line ends.
    let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    const CF_UNICODETEXT: u32 = 13;
    // SAFETY: the clipboard is opened and closed here; the global block is filled while
    // locked and handed to the clipboard, which then owns it (freed by us only on failure).
    unsafe {
        // Another app may hold the clipboard for a moment.
        let mut tries = 0;
        while OpenClipboard(hwnd as HWND) == 0 {
            tries += 1;
            if tries == 10 {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
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

/// Hands key presses in the page with Ctrl, Alt or a function key to `on_key` (main
/// thread) as (virtual key, Ctrl, Shift, Alt, held down: an auto-repeat); it answers whether
/// a menu item took the key, which then goes no further. It must answer at once (the
/// browser waits) and run the item later. Also turns off the browser's
/// own keys, which would reload the page or print it as a web page.
pub fn menu_keys(
    controller: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller,
    on_key: impl Fn(u32, bool, bool, bool, bool) -> bool + 'static,
) -> Result<(), String> {
    use webview2_com::AcceleratorKeyPressedEventHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
        COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN, COREWEBVIEW2_PHYSICAL_KEY_STATUS,
        ICoreWebView2Settings3,
    };
    use windows::core::Interface;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
    };
    let down = |vk: u16| {
        // SAFETY: GetKeyState only reads this thread's keyboard state.
        let state = unsafe { GetKeyState(i32::from(vk)) };
        state < 0
    };
    let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else {
            return Ok(());
        };
        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
        let mut vk = 0u32;
        let mut status = COREWEBVIEW2_PHYSICAL_KEY_STATUS::default();
        // SAFETY: COM calls on the main thread on the event's own arguments.
        unsafe {
            args.KeyEventKind(&mut kind)?;
            args.VirtualKey(&mut vk)?;
            args.PhysicalKeyStatus(&mut status)?;
        }
        if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
            && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
        {
            return Ok(());
        }
        let held = status.WasKeyDown.as_bool();
        if on_key(vk, down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU), held) {
            // SAFETY: as above.
            unsafe { args.SetHandled(true)? };
        }
        Ok(())
    }));
    // SAFETY: COM calls on the main thread with a live controller.
    unsafe {
        let mut token = 0i64;
        controller
            .add_AcceleratorKeyPressed(&handler, &mut token)
            .map_err(|e| e.to_string())?;
        if let Ok(settings) = controller
            .CoreWebView2()
            .and_then(|w| w.Settings())
            .and_then(|s| s.cast::<ICoreWebView2Settings3>())
        {
            let _ = settings.SetAreBrowserAcceleratorKeysEnabled(false);
        }
    }
    Ok(())
}

/// Opens a link in the default browser or mail app (the host has checked the scheme).
pub fn open_uri(uri: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let (verb, target) = (wide("open"), wide(uri));
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Shows a file selected in File Explorer.
pub fn reveal(paths: &[PathBuf]) {
    use std::os::windows::process::CommandExt;
    if let Some(path) = paths.first() {
        // Explorer reads its own command line: `/select,"C:\path\file.pdf"`.
        let _ = std::process::Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn();
    }
}
