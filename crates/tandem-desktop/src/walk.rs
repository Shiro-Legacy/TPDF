//! `--walk=<script.jsonl>`: drives the real window for screenshot walkthroughs.
//!
//! One JSON object per line (`#` lines are comments):
//! - `{"act": <UiAction>}` — any UI action, as the window sends it;
//! - `{"edit": "op", "contents": …}` — a menu or drive op (`UiAction::Edit`);
//! - `{"open": "path"}` — open a file (relative paths from the current folder);
//! - `{"menu": "id"}` — a menu item (`menu.rs` ids);
//! - `{"wait": ms}` — let the window settle;
//! - `{"shot": "file.png"}` — snapshot the web view;
//! - `{"state": "file.json"}` — write the current `UiState`;
//! - `{"answer": {"open": path, "saveTo": path, "images": [paths], "folder": path,
//!   "buttons": ["label", …]}}` — answers for the next native panels and alerts (an alert
//!   with no queued answer gets its default button; every one is logged on stderr);
//! - `{"drag": [paths]}` — files dragged over the window (the drop veil), `{"drag": []}`
//!   ends the drag; `{"drop": [paths]}` — files dropped on the window.
//!
//! The app quits after the last line. Not for people: a test and review tool.

use crate::core::CoreHandle;
use serde_json::Value;
use std::sync::mpsc::channel;
use std::time::Duration;
use tandem_protocol::ui::UiAction;
use tauri::{AppHandle, Emitter, Manager};

pub fn run(app: AppHandle, core: CoreHandle, script: String, answers: crate::platform::WalkHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        let text = std::fs::read_to_string(&script).unwrap_or_default();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Ok(step) = serde_json::from_str::<Value>(line) else {
                eprintln!("walk: line {}: not JSON", n + 1);
                continue;
            };
            let emit = |core: &CoreHandle| {
                let app = app.clone();
                core.call(move |c| {
                    let state = c.ui_state();
                    crate::sync_menu(&app, &state);
                    let _ = app.emit("state", state);
                });
            };
            if let Some(a) = step.get("answer") {
                let path = |k: &str| {
                    a.get(k)
                        .and_then(Value::as_str)
                        .map(|p| std::path::absolute(p).unwrap_or_else(|_| p.into()))
                };
                if let Ok(mut w) = answers.lock() {
                    if let Some(p) = path("open") {
                        w.open = Some(p);
                    }
                    if let Some(p) = path("saveTo") {
                        w.save_to = Some(p);
                    }
                    if let Some(p) = path("folder") {
                        w.folder = Some(p);
                    }
                    if let Some(list) = a.get("images").and_then(Value::as_array) {
                        w.images = list
                            .iter()
                            .filter_map(Value::as_str)
                            .map(std::path::PathBuf::from)
                            .collect();
                    }
                    if let Some(list) = a.get("buttons").and_then(Value::as_array) {
                        w.buttons
                            .extend(list.iter().filter_map(Value::as_str).map(str::to_string));
                    }
                }
            } else if let Some(action) = step.get("act") {
                match serde_json::from_value::<UiAction>(action.clone()) {
                    Ok(action) => {
                        core.call(move |c| c.act(action));
                        emit(&core);
                    }
                    Err(e) => eprintln!("walk: line {}: {e}", n + 1),
                }
            } else if let Some(op) = step.get("edit").and_then(Value::as_str) {
                let action = UiAction::Edit {
                    op: op.to_string(),
                    contents: step.get("contents").cloned(),
                };
                core.call(move |c| c.act(action));
                emit(&core);
            } else if let Some(path) = step.get("open").and_then(Value::as_str) {
                let path = std::path::absolute(path).unwrap_or_else(|_| path.into());
                core.call(move |c| c.open_path(&path));
                emit(&core);
            } else if let Some(id) = step.get("menu").and_then(Value::as_str) {
                let id = id.to_string();
                core.call(move |c| c.menu(&id));
                emit(&core);
            } else if let Some(list) = step.get("drag").and_then(Value::as_array) {
                let paths: Vec<std::path::PathBuf> = list
                    .iter()
                    .filter_map(Value::as_str)
                    .map(std::path::PathBuf::from)
                    .collect();
                crate::drag_veil(&app, &paths);
            } else if let Some(list) = step.get("drop").and_then(Value::as_array) {
                let paths = list
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|p| std::path::absolute(p).unwrap_or_else(|_| p.into()))
                    .collect();
                crate::drag_veil(&app, &[]);
                crate::open_handed(&app, paths);
            } else if let Some(ms) = step.get("wait").and_then(Value::as_u64) {
                std::thread::sleep(Duration::from_millis(ms));
            } else if let Some(file) = step.get("state").and_then(Value::as_str) {
                let file = file.to_string();
                core.call(move |c| {
                    let _ = std::fs::write(
                        &file,
                        serde_json::to_string_pretty(&c.ui_state()).unwrap_or_default(),
                    );
                });
            } else if let Some(file) = step.get("shot").and_then(Value::as_str) {
                std::thread::sleep(Duration::from_millis(400));
                let (tx, rx) = channel();
                let Some(window) = app.get_webview_window("main") else {
                    break;
                };
                let w2 = window.clone();
                let file = std::path::absolute(file).unwrap_or_else(|_| file.into());
                let _ = window.run_on_main_thread(move || {
                    let _ = w2.with_webview(move |wv| {
                        #[cfg(target_os = "macos")]
                        crate::macos::snapshot_png(wv.inner(), file, move |r| {
                            let _ = tx.send(r);
                        });
                    });
                });
                if let Ok(Err(e)) = rx.recv_timeout(Duration::from_secs(10)) {
                    eprintln!("walk: shot failed: {e}");
                }
            }
        }
        std::thread::sleep(Duration::from_millis(200));
        app.exit(0);
    });
}
