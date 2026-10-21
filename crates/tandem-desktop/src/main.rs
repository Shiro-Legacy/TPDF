//! Tandem PDF for macOS, Windows and Linux: the Tauri 2 shell around the headless core.
//!
//! - A headless `--drive` / `--smoke` run never opens a window: it is the
//!   core's own harness (`tandem_app::drive`).
//! - Otherwise one window shows the Leptos UI; a core thread owns the
//!   document; the UI talks to it through `state` / `act` and fetches page
//!   pixels over `tandem://`.
//!
//! On Windows a release build is a GUI-subsystem program, so starting it from the Start menu or
//! Explorer opens no console window; a command-line run attaches to its terminal instead
//! ([`win::attach_parent_console`]).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod core;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod menu;
mod platform;
mod session_core;
mod walk;
#[cfg(windows)]
mod win;
mod window_state;

use crate::core::{AppCore, CoreHandle};
use std::sync::{Arc, Mutex};
use tandem_protocol::ui::{UiAction, UiState};
use tauri::http::Response;
use tauri::menu::AboutMetadata;
use tauri::{Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// Shell options for screenshot runs and tests (not for people).
#[derive(Default, Clone)]
struct ShellArgs {
    /// Files named on the command line (`--open=` or bare `.pdf` paths).
    open: Vec<String>,
    window: Option<(f64, f64)>,
    zoom: Option<f64>,
    shot: Option<(u64, String)>,
    theme: Option<String>,
    walk: Option<String>,
    user_data: Option<String>,
}

fn shell_args() -> ShellArgs {
    let mut out = ShellArgs::default();
    for arg in std::env::args().skip(1) {
        if let Some(v) = arg.strip_prefix("--open=") {
            out.open.push(v.to_string());
        } else if let Some(v) = arg.strip_prefix("--window-size=") {
            if let Some((w, h)) = v.split_once('x') {
                out.window = w.parse().ok().zip(h.parse().ok());
            }
        } else if let Some(v) = arg.strip_prefix("--webview-zoom=") {
            out.zoom = v.parse().ok();
        } else if let Some(v) = arg.strip_prefix("--shot=") {
            // --shot=<ms>:<path.png>
            if let Some((ms, path)) = v.split_once(':') {
                out.shot = ms.parse().ok().map(|ms| (ms, path.to_string()));
            }
        } else if let Some(v) = arg.strip_prefix("--user-data-dir=") {
            out.user_data = Some(v.to_string());
        } else if let Some(v) = arg.strip_prefix("--walk=") {
            out.walk = Some(v.to_string());
        } else if let Some(v) = arg.strip_prefix("--theme=") {
            out.theme = Some(v.to_string());
        } else if !arg.starts_with('-') && arg.to_lowercase().ends_with(".pdf") {
            out.open.push(arg);
        }
    }
    out
}

struct Core(CoreHandle);

/// The user-data folder (window state lives there too, beside recent files).
struct UserData(std::path::PathBuf);

/// Temporary: the web view's errors on stderr.
#[tauri::command]
fn log(msg: String) {
    eprintln!("ui: {msg}");
}

/// The state the window draws.
#[tauri::command]
async fn state(app: tauri::AppHandle, core: tauri::State<'_, Core>) -> Result<UiState, String> {
    let core = core.0.clone();
    let state = tauri::async_runtime::spawn_blocking(move || core.call(|c| c.ui_state()))
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "The app's core has stopped. Restart Tandem PDF.".to_string())?;
    sync_menu(&app, &state);
    Ok(state)
}

/// One UI action; answers with the new state.
#[tauri::command]
async fn act(
    action: UiAction,
    app: tauri::AppHandle,
    core: tauri::State<'_, Core>,
) -> Result<UiState, String> {
    let core = core.0.clone();
    let state = tauri::async_runtime::spawn_blocking(move || {
        core.call(move |c| {
            c.act(action);
            c.ui_state()
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "The app's core has stopped. Restart Tandem PDF.".to_string())?;
    sync_menu(&app, &state);
    Ok(state)
}

/// A note the shell shows in the window (drops and Finder opens it could not take as
/// asked). Kept until the window asks for it, so one sent before the UI listens is not lost.
#[derive(Default)]
struct PendingNote(Mutex<Option<String>>);

/// A save toast's "Show in Finder": the file the toast names, selected in the Finder.
/// Answers whether there was such a file.
#[tauri::command]
fn reveal_saved(
    text: String,
    app: tauri::AppHandle,
    targets: tauri::State<'_, platform::SaveTargets>,
) -> bool {
    let Some(path) = targets.named_in(&text) else {
        return false;
    };
    #[cfg(target_os = "macos")]
    let _ = app.run_on_main_thread(move || macos::reveal(&[path]));
    #[cfg(windows)]
    let _ = (app, win::reveal(&[path]));
    #[cfg(target_os = "linux")]
    let _ = (app, linux::reveal(&[path]));
    true
}

#[tauri::command]
fn take_note(note: tauri::State<'_, PendingNote>) -> Option<String> {
    note.0.lock().ok().and_then(|mut n| n.take())
}

fn show_note(app: &tauri::AppHandle, text: String) {
    if let Some(pending) = app.try_state::<PendingNote>()
        && let Ok(mut slot) = pending.0.lock()
    {
        *slot = Some(text.clone());
    }
    let _ = app.emit("note", text);
}

/// Greys menu items the document does not allow (Next Page on the last page, Undo with
/// nothing to undo, document items on the start screen).
pub(crate) fn sync_menu(app: &tauri::AppHandle, state: &UiState) {
    if let Some(items) = app.try_state::<menu::MenuItems<tauri::Wry>>() {
        items.set_enabled(menu::enabled_for(&state.view));
    }
    // Windows and Linux: File ▸ Open Recent shows the app's list.
    #[cfg(not(target_os = "macos"))]
    if let Some(recent) = app.try_state::<menu::RecentMenu>() {
        let files: Vec<(String, String)> = state
            .view
            .recent
            .iter()
            .map(|r| (r.path.clone(), r.name.clone()))
            .collect();
        recent.update(app, &files);
    }
}

/// A menu item chosen (or its shortcut pressed).
fn run_menu(app: &tauri::AppHandle, core: &CoreHandle, id: String) {
    if id == "view:find" {
        let _ = app.emit("focus", "search-input");
        return;
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Close and Quit: the window's own close, which asks about unsaved changes.
        if id == "app:close" || id == "app:quit" {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.close();
            }
            return;
        }
        if id != "recent:clear" && id.starts_with("recent:") {
            if let Some(path) = app
                .try_state::<menu::RecentMenu>()
                .and_then(|r| r.path(&id))
            {
                post_and_emit(app, core, move |c| c.open_path(std::path::Path::new(&path)));
            }
            return;
        }
    }
    post_and_emit(app, core, move |c| c.menu(&id));
}

/// Saves what the web view shows to a PNG file; `done` gets the outcome (main thread).
pub(crate) fn snapshot(
    window: &tauri::WebviewWindow,
    path: std::path::PathBuf,
    done: impl FnOnce(Result<(), String>) + Send + 'static,
) {
    let _ = window.with_webview(move |wv| {
        #[cfg(target_os = "macos")]
        macos::snapshot_png(wv.inner(), path, done);
        #[cfg(windows)]
        win::snapshot_png(&wv.controller(), path, done);
        #[cfg(target_os = "linux")]
        linux::snapshot_png(&wv.inner(), path, done);
    });
}

fn is_pdf(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Files handed over by a drop or by Finder: the first PDF opens; the rest are named in a
/// note (one document at a time, never a silent choice). The note comes only once the open
/// went through (a refused open says why itself), after the state that shows the document.
pub(crate) fn open_handed(app: &tauri::AppHandle, paths: Vec<std::path::PathBuf>) {
    let (pdfs, others): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| is_pdf(p));
    let note = handed_note(&pdfs, &others);
    let Some(first) = pdfs.into_iter().next() else {
        if let Some(note) = note {
            show_note(app, note);
        }
        return;
    };
    let Some(core) = app.try_state::<Core>() else {
        return;
    };
    let app = app.clone();
    core.0.post(move |c| {
        let before = c.ui_state().view.generation;
        c.open_path(&first);
        let state = c.ui_state();
        let opened = state.view.generation != before && shows_file(&state, &first);
        sync_menu(&app, &state);
        let _ = app.emit("state", state);
        if let (true, Some(note)) = (opened, note) {
            show_note(&app, note);
        }
    });
}

/// Whether the window now shows `path` (open, or asking for its password).
fn shows_file(state: &UiState, path: &std::path::Path) -> bool {
    let name = file_name(path);
    match state.view.screen.as_str() {
        "password" => state.view.password.as_ref().is_some_and(|p| p.name == name),
        "editor" => state.view.doc.as_ref().is_some_and(|d| d.name == name),
        _ => false,
    }
}

/// What to tell the person about files that were handed over but not opened. With a PDF
/// it is said after that PDF opened, so it starts "Opened".
fn handed_note(pdfs: &[std::path::PathBuf], others: &[std::path::PathBuf]) -> Option<String> {
    let Some(first) = pdfs.first() else {
        return match others {
            [] => None,
            [one] => Some(format!(
                "“{}” is not a PDF. Tandem PDF opens PDF files only.",
                file_name(one)
            )),
            _ => Some("None of those files is a PDF. Tandem PDF opens PDF files only.".into()),
        };
    };
    if pdfs.len() == 1 && others.is_empty() {
        return None;
    }
    let mut note = format!("Opened “{}”.", file_name(first));
    if pdfs.len() > 1 {
        note.push_str(
            " Tandem PDF opens one file at a time; open the other PDFs from File ▸ Open.",
        );
    }
    match others {
        [] => {}
        [one] => note.push_str(&format!(
            " “{}” is not a PDF and was left alone.",
            file_name(one)
        )),
        many => note.push_str(&format!(
            " The {} files that are not PDFs were left alone.",
            many.len()
        )),
    }
    Some(note)
}

/// Shows the drop veil for files dragged over the window; no files hides it.
pub(crate) fn drag_veil(app: &tauri::AppHandle, paths: &[std::path::PathBuf]) {
    let _ = app.emit(
        "drag",
        DragView {
            over: !paths.is_empty(),
            pdf: paths.iter().any(|p| is_pdf(p)),
            count: paths.len(),
        },
    );
}

/// The drop veil's state for the window.
#[derive(serde::Serialize, Clone)]
struct DragView {
    over: bool,
    /// At least one of the dragged files is a PDF (the veil says what will happen).
    pdf: bool,
    count: usize,
}

fn post_and_emit(
    app: &tauri::AppHandle,
    core: &CoreHandle,
    work: impl FnOnce(&mut dyn AppCore) + Send + 'static,
) {
    let app = app.clone();
    core.post(move |c| {
        work(c);
        let state = c.ui_state();
        sync_menu(&app, &state);
        let _ = app.emit("state", state);
    });
}

/// A `--smoke` or `--drive` run without `--gui`: the core's headless harness, no window.
fn headless_run() -> bool {
    let argv: Vec<String> = std::env::args().collect();
    let smoke = argv
        .iter()
        .any(|a| a == "--smoke" || a == "--drive" || a.starts_with("--drive="));
    // `--mcp`: the MCP server for agents (tandem_app::mcp), never a window.
    argv.iter().any(|a| a == "--mcp") || smoke && !argv.iter().any(|a| a == "--gui")
}

fn main() {
    #[cfg(windows)]
    if std::env::args()
        .any(|a| a == "--smoke" || a == "--drive" || a.starts_with("--drive=") || a == "--mcp")
    {
        win::attach_parent_console();
    }
    if headless_run() {
        tandem_app::drive::main(tandem_app::session::Session::for_drive);
    }
    let args = shell_args();
    let about = AboutMetadata {
        name: Some("Tandem PDF".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        copyright: Some("© 2026 Tandem PDF contributors".into()),
        credits: Some(
            "A local, offline PDF editor. MIT licence.\n\nThis software is based in part on the work of the Independent JPEG Group."
                .into(),
        ),
        ..AboutMetadata::default()
    };

    // One window per user-data folder (Electron's single-instance lock). A second launch (a
    // command line, Explorer's or the file manager's "Open with") hands its files to the
    // running window and quits; Finder reuses the running app by itself (RunEvent::Opened).
    let user_data = args
        .user_data
        .clone()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(tandem_app::host::default_user_data);
    let files: Vec<std::path::PathBuf> = args.open.iter().map(std::path::PathBuf::from).collect();
    use tandem_app::host::handover::{self, Claim};
    let handover = match handover::claim(&user_data, &files, std::time::Duration::from_secs(3)) {
        Ok(Claim::First(lock, listener)) => {
            // Held for the life of the process.
            std::mem::forget(lock);
            listener
        }
        Ok(Claim::HandedOver) => std::process::exit(0),
        Ok(Claim::Busy) | Err(_) => {
            eprintln!("Tandem PDF is already running.");
            std::process::exit(3);
        }
    };

    let protocol_core: Arc<Mutex<Option<CoreHandle>>> = Arc::new(Mutex::new(None));
    let protocol_core_setup = protocol_core.clone();
    let args_setup = args.clone();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(PendingNote::default())
        .manage(platform::SaveTargets::default())
        .invoke_handler(tauri::generate_handler![
            state,
            act,
            log,
            take_note,
            reveal_saved
        ])
        .register_asynchronous_uri_scheme_protocol("tandem", move |_ctx, request, responder| {
            let uri = request.uri().clone();
            let mut parts = uri.path().trim_start_matches('/').split('/');
            let kind = parts.next().unwrap_or("page").to_string();
            let page = parts
                .next()
                .and_then(|p| p.parse::<usize>().ok())
                .unwrap_or(0);
            let scale = uri
                .query()
                .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("scale=")))
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(1.0)
                .clamp(0.02, 16.0);
            // `hide`: the annotation being dragged, drawn by the window instead.
            let hide = uri
                .query()
                .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("hide=")))
                .and_then(|s| s.parse::<i64>().ok());
            let core = protocol_core.lock().ok().and_then(|c| c.clone());
            std::thread::spawn(move || {
                let started = std::time::Instant::now();
                let pixels = core.and_then(|c| c.call(move |c| c.pixels(&kind, page, scale, hide)));
                let response = match pixels {
                    Some(Ok(px)) => Response::builder()
                        .header("Content-Type", "application/octet-stream")
                        .header("Access-Control-Allow-Origin", "*")
                        .header(
                            "Access-Control-Expose-Headers",
                            "x-width, x-height, x-render-ms",
                        )
                        .header("x-width", px.width.to_string())
                        .header("x-height", px.height.to_string())
                        .header("x-render-ms", started.elapsed().as_millis().to_string())
                        .body(px.rgba),
                    _ => Response::builder()
                        .status(404)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(Vec::new()),
                };
                if let Ok(response) = response {
                    responder.respond(response);
                }
            });
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            let core_app = handle.clone();
            let handed = handle.clone();
            handover.listen(move |paths| {
                if let Some(window) = handed.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                if !paths.is_empty() {
                    open_handed(&handed, paths);
                }
            });
            // The Electron development builds' stores, once, when this is the default folder.
            let imported = if args_setup.user_data.is_none() {
                let from = tandem_app::host::import::electron_folder();
                tandem_app::host::import::import_once(&from, &user_data)
                    .ok()
                    .and_then(|copied| tandem_app::host::import::note(&copied))
            } else {
                None
            };
            app.manage(UserData(user_data.clone()));
            let walk_answers: Option<platform::WalkHandle> = args_setup.walk.as_ref().map(|_| {
                std::sync::Arc::new(std::sync::Mutex::new(platform::WalkAnswers::default()))
            });
            let walk_for_core = walk_answers.clone();
            let core = CoreHandle::spawn(move || {
                Box::new(session_core::SessionCore::new(
                    core_app,
                    user_data,
                    walk_for_core,
                )) as Box<dyn AppCore>
            });
            if let Ok(mut slot) = protocol_core_setup.lock() {
                *slot = Some(core.clone());
            }
            app.manage(Core(core.clone()));

            let menu_bar = menu::build(&handle, about)?;
            app.manage(menu::MenuItems::collect(&menu_bar));
            app.set_menu(menu_bar)?;
            #[cfg(target_os = "macos")]
            macos::help_key(menu::HELP_LABEL);
            #[cfg(not(target_os = "macos"))]
            app.manage(menu::RecentMenu::new());
            let menu_core = core.clone();
            app.on_menu_event(move |app, event| {
                run_menu(app, &menu_core, event.id().as_ref().to_string());
            });

            let data_dir = app.try_state::<UserData>().map(|u| u.0.clone());
            let mut builder =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("Tandem PDF")
                    .inner_size(1280.0, 820.0)
                    .min_inner_size(900.0, 600.0)
                    .visible(false);
            #[cfg(target_os = "macos")]
            {
                builder = builder
                    .title_bar_style(tauri::TitleBarStyle::Overlay)
                    .hidden_title(true);
            }
            if let Some(theme) = &args_setup.theme {
                builder = builder.theme(match theme.as_str() {
                    "dark" => Some(tauri::Theme::Dark),
                    "light" => Some(tauri::Theme::Light),
                    _ => None,
                });
            }
            let window = builder.build()?;
            match (
                args_setup.window,
                data_dir.as_deref().and_then(window_state::load),
            ) {
                (Some((w, h)), _) => {
                    let _ = window.set_size(tauri::LogicalSize::new(w, h));
                    let _ = window.center();
                }
                (None, Some(saved)) => window_state::apply(&window, &saved),
                (None, None) => {
                    let _ = window.center();
                }
            }
            if let Some(z) = args_setup.zoom {
                let _ = window.set_zoom(z);
            }
            // Windows: menu shortcuts pressed while the page has the keyboard (WebView2 takes
            // keys in its own process, out of reach of the menu's accelerator table).
            #[cfg(windows)]
            {
                let (keys_app, keys_core) = (handle.clone(), core.clone());
                let _ = window.with_webview(move |wv| {
                    let shortcut = move |vk, ctrl, shift, alt, held: bool| {
                        let Some(id) = menu::pc_item_for_key(vk, ctrl, shift, alt) else {
                            return false;
                        };
                        let on = keys_app
                            .try_state::<menu::MenuItems<tauri::Wry>>()
                            .is_some_and(|items| items.enabled(id));
                        // A held key repeats only what is meant to repeat (paging, zoom,
                        // undo), never Open… or Print….
                        if on && (!held || menu::repeats(id)) {
                            let (app, core) = (keys_app.clone(), keys_core.clone());
                            // After the key event returns: the browser waits for it.
                            tauri::async_runtime::spawn(async move {
                                run_menu(&app, &core, id.to_string());
                            });
                        }
                        on
                    };
                    if let Err(e) = win::menu_keys(&wv.controller(), shortcut) {
                        eprintln!("menu keys: {e}");
                    }
                });
            }
            window.show()?;
            if let Some(note) = imported {
                show_note(&handle, note);
            }

            if let Some(script) = args_setup.walk.clone() {
                walk::run(
                    handle.clone(),
                    core.clone(),
                    script,
                    walk_answers.clone().unwrap_or_default(),
                );
            }
            if !args_setup.open.is_empty() {
                open_handed(
                    &handle,
                    args_setup
                        .open
                        .iter()
                        .map(std::path::PathBuf::from)
                        .collect(),
                );
            }
            if let Some((ms, path)) = args_setup.shot.clone() {
                let window = window.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    snapshot(&window, path.into(), |r| {
                        if let Err(e) = r {
                            eprintln!("snapshot failed: {e}");
                        }
                        std::process::exit(0);
                    });
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Tandem PDF could not start");

    app.run(|handle, event| match event {
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::DragDrop(drag),
            ..
        } if label == "main" => match drag {
            tauri::DragDropEvent::Enter { paths, .. } => drag_veil(handle, &paths),
            tauri::DragDropEvent::Leave => drag_veil(handle, &[]),
            tauri::DragDropEvent::Drop { paths, .. } => {
                drag_veil(handle, &[]);
                open_handed(handle, paths);
            }
            _ => {}
        },
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" => {
            let walk = std::env::args()
                .any(|a| a.starts_with("--walk=") || a.starts_with("--window-size="));
            if let (Some(window), Some(dir), false) = (
                handle.get_webview_window("main"),
                handle.try_state::<UserData>(),
                walk,
            ) && let Some(saved) = window_state::capture(&window)
            {
                window_state::save(&dir.0, &saved);
            }
            if let Some(core) = handle.try_state::<Core>() {
                api.prevent_close();
                let core = core.0.clone();
                let app = handle.clone();
                std::thread::spawn(move || {
                    if core.call(|c| c.close_requested()).unwrap_or(true) {
                        app.exit(0);
                    } else {
                        post_and_emit(&app, &core, |_| {});
                    }
                });
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Opened { urls } => {
            let paths = urls
                .into_iter()
                .filter_map(|u| u.to_file_path().ok())
                .collect();
            open_handed(handle, paths);
        }
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::handed_note;
    use std::path::PathBuf;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|n| PathBuf::from(format!("/x/{n}")))
            .collect()
    }

    #[test]
    fn handed_files_are_explained() {
        assert_eq!(handed_note(&paths(&["a.pdf"]), &[]), None);
        assert_eq!(
            handed_note(&[], &paths(&["photo.png"])).as_deref(),
            Some("“photo.png” is not a PDF. Tandem PDF opens PDF files only.")
        );
        assert!(
            handed_note(&paths(&["a.pdf", "b.pdf"]), &[]).is_some_and(|n| n
                .starts_with("Opened “a.pdf”.")
                && n.contains("one file at a time"))
        );
        assert_eq!(
            handed_note(&paths(&["a.pdf"]), &paths(&["b.txt"])).as_deref(),
            Some("Opened “a.pdf”. “b.txt” is not a PDF and was left alone.")
        );
        // Several PDFs and something else: both are said (walkthrough open#5).
        assert!(
            handed_note(&paths(&["a.pdf", "b.pdf"]), &paths(&["c.png"])).is_some_and(|n| n
                .contains("one file at a time")
                && n.contains("“c.png” is not a PDF"))
        );
        assert!(handed_note(&[], &paths(&["b.txt", "c.txt"])).is_some());
    }
}
