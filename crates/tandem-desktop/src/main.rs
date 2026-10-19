//! Tandem PDF for macOS: the Tauri 2 shell around the headless core.
//!
//! - A headless `--drive` / `--smoke` run never opens a window: it is the
//!   core's own harness (`tandem_app::drive`).
//! - Otherwise one window shows the Leptos UI; a core thread owns the
//!   document; the UI talks to it through `state` / `act` and fetches page
//!   pixels over `tandem://`.

mod core;
#[cfg(target_os = "macos")]
mod macos;
mod menu;
mod spike;
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
    open: Option<String>,
    window: Option<(f64, f64)>,
    zoom: Option<f64>,
    shot: Option<(u64, String)>,
    theme: Option<String>,
}

fn shell_args() -> ShellArgs {
    let mut out = ShellArgs::default();
    for arg in std::env::args().skip(1) {
        if let Some(v) = arg.strip_prefix("--open=") {
            out.open = Some(v.to_string());
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
        } else if let Some(v) = arg.strip_prefix("--theme=") {
            out.theme = Some(v.to_string());
        } else if !arg.starts_with('-') && arg.to_lowercase().ends_with(".pdf") {
            out.open = Some(arg);
        }
    }
    out
}

struct Core(CoreHandle);

/// Temporary: the web view's errors on stderr.
#[tauri::command]
fn log(msg: String) {
    eprintln!("ui: {msg}");
}

/// The state the window draws.
#[tauri::command]
async fn state(core: tauri::State<'_, Core>) -> Result<UiState, String> {
    let core = core.0.clone();
    tauri::async_runtime::spawn_blocking(move || core.call(|c| c.ui_state()))
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "The app's core has stopped. Restart Tandem PDF.".to_string())
}

/// One UI action; answers with the new state.
#[tauri::command]
async fn act(action: UiAction, core: tauri::State<'_, Core>) -> Result<UiState, String> {
    let core = core.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        core.call(move |c| {
            c.act(action);
            c.ui_state()
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "The app's core has stopped. Restart Tandem PDF.".to_string())
}

fn post_and_emit(app: &tauri::AppHandle, core: &CoreHandle, work: impl FnOnce(&mut dyn AppCore) + Send + 'static) {
    let app = app.clone();
    core.post(move |c| {
        work(c);
        let _ = app.emit("state", c.ui_state());
    });
}

fn main() {
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

    let protocol_core: Arc<Mutex<Option<CoreHandle>>> = Arc::new(Mutex::new(None));
    let protocol_core_setup = protocol_core.clone();
    let args_setup = args.clone();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![state, act, log])
        .register_asynchronous_uri_scheme_protocol("tandem", move |_ctx, request, responder| {
            let uri = request.uri().clone();
            let mut parts = uri.path().trim_start_matches('/').split('/');
            let kind = parts.next().unwrap_or("page").to_string();
            let page = parts.next().and_then(|p| p.parse::<usize>().ok()).unwrap_or(0);
            let scale = uri
                .query()
                .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("scale=")))
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(1.0)
                .clamp(0.02, 16.0);
            let core = protocol_core.lock().ok().and_then(|c| c.clone());
            std::thread::spawn(move || {
                let started = std::time::Instant::now();
                let pixels = core.and_then(|c| c.call(move |c| c.pixels(&kind, page, scale)));
                let response = match pixels {
                    Some(Ok(px)) => Response::builder()
                        .header("Content-Type", "application/octet-stream")
                        .header("Access-Control-Allow-Origin", "*")
                        .header("Access-Control-Expose-Headers", "x-width, x-height, x-render-ms")
                        .header("x-width", px.width.to_string())
                        .header("x-height", px.height.to_string())
                        .header("x-render-ms", started.elapsed().as_millis().to_string())
                        .body(px.rgba),
                    _ => Response::builder().status(404).header("Access-Control-Allow-Origin", "*").body(Vec::new()),
                };
                if let Ok(response) = response {
                    responder.respond(response);
                }
            });
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            let core_app = handle.clone();
            let core = CoreHandle::spawn(move || Box::new(spike::SpikeCore::new(core_app)) as Box<dyn AppCore>);
            if let Ok(mut slot) = protocol_core_setup.lock() {
                *slot = Some(core.clone());
            }
            app.manage(Core(core.clone()));

            app.set_menu(menu::build(&handle, about)?)?;
            let menu_core = core.clone();
            app.on_menu_event(move |app, event| {
                let id = event.id().as_ref().to_string();
                if id == "view:find" {
                    let _ = app.emit("focus", "search-input");
                    return;
                }
                post_and_emit(app, &menu_core, move |c| c.menu(&id));
            });

            let data_dir = app.path().app_data_dir().ok();
            let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Tandem PDF")
                .inner_size(1280.0, 820.0)
                .min_inner_size(900.0, 600.0)
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true)
                .visible(false);
            if let Some(theme) = &args_setup.theme {
                builder = builder.theme(match theme.as_str() {
                    "dark" => Some(tauri::Theme::Dark),
                    "light" => Some(tauri::Theme::Light),
                    _ => None,
                });
            }
            let window = builder.build()?;
            match (args_setup.window, data_dir.as_deref().and_then(window_state::load)) {
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
            window.show()?;

            if let Some(path) = args_setup.open.clone() {
                post_and_emit(&handle, &core, move |c| c.open_path(std::path::Path::new(&path)));
            }
            #[cfg(target_os = "macos")]
            if let Some((ms, path)) = args_setup.shot.clone() {
                let window = window.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    let w2 = window.clone();
                    let _ = window.run_on_main_thread(move || {
                        let _ = w2.with_webview(move |wv| {
                            macos::snapshot_png(wv.inner(), path.into(), |r| {
                                if let Err(e) = r {
                                    eprintln!("snapshot failed: {e}");
                                }
                                std::process::exit(0);
                            });
                        });
                    });
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Tandem PDF could not start");

    app.run(|handle, event| match event {
        RunEvent::WindowEvent { label, event: WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }), .. }
            if label == "main" =>
        {
            if let Some(core) = handle.try_state::<Core>() {
                if let Some(path) = paths.into_iter().find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))) {
                    post_and_emit(handle, &core.0, move |c| c.open_path(&path));
                }
            }
        }
        RunEvent::WindowEvent { label, event: WindowEvent::CloseRequested { api, .. }, .. } if label == "main" => {
            if let (Some(window), Ok(dir)) = (handle.get_webview_window("main"), handle.path().app_data_dir()) {
                if let Some(saved) = window_state::capture(&window) {
                    window_state::save(&dir, &saved);
                }
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
            if let Some(core) = handle.try_state::<Core>() {
                for url in urls {
                    if let Ok(path) = url.to_file_path() {
                        post_and_emit(handle, &core.0, move |c| c.open_path(&path));
                    }
                }
            }
        }
        _ => {}
    });
}
