//! Remembers the window's size, position and full-screen state between
//! launches (`window.json` in the app's data folder).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{LogicalPosition, LogicalSize, Runtime, WebviewWindow};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub maximized: bool,
    pub fullscreen: bool,
}

pub fn file(dir: &Path) -> PathBuf {
    dir.join("window.json")
}

pub fn load(dir: &Path) -> Option<Saved> {
    let text = std::fs::read_to_string(file(dir)).ok()?;
    let saved: Saved = serde_json::from_str(&text).ok()?;
    // Never restore a window too small to use or absurdly large.
    (saved.width >= 600.0
        && saved.height >= 400.0
        && saved.width < 20000.0
        && saved.height < 20000.0)
        .then_some(saved)
}

/// Reads the window's current state (logical points).
pub fn capture<R: Runtime>(window: &WebviewWindow<R>) -> Option<Saved> {
    let scale = window.scale_factor().ok()?;
    let maximized = window.is_maximized().unwrap_or(false);
    let fullscreen = window.is_fullscreen().unwrap_or(false);
    let pos = window.outer_position().ok()?.to_logical::<f64>(scale);
    let size = window.inner_size().ok()?.to_logical::<f64>(scale);
    Some(Saved {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
        maximized,
        fullscreen,
    })
}

pub fn save(dir: &Path, saved: &Saved) {
    let _ = std::fs::create_dir_all(dir);
    let tmp = dir.join("window.json.tmp");
    if std::fs::write(&tmp, serde_json::to_string(saved).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(tmp, file(dir));
    }
}

pub fn apply<R: Runtime>(window: &WebviewWindow<R>, saved: &Saved) {
    let _ = window.set_size(LogicalSize::new(saved.width, saved.height));
    let _ = window.set_position(LogicalPosition::new(saved.x, saved.y));
    if saved.maximized {
        let _ = window.maximize();
    }
    if saved.fullscreen {
        let _ = window.set_fullscreen(true);
    }
}
