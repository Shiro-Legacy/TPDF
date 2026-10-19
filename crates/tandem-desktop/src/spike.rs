//! A stand-in core (engine only) until the ported session is wired in:
//! open, page through, zoom. Lets the shell run end to end.

use crate::core::{AppCore, Pixels};
use serde_json::{Value, json};
use std::path::Path;
use tandem_app::engine::EngineHost;
use tandem_protocol::ui::{DocView, PageView, UiAction, UiState, View, ZoomView};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

pub struct SpikeCore {
    app: AppHandle,
    engine: EngineHost,
    view: View,
    viewport: (f64, f64),
}

impl SpikeCore {
    pub fn new(app: AppHandle) -> Self {
        let mut view = View { screen: "start".into(), ..View::default() };
        view.tool.id = "select".into();
        view.zoom = ZoomView { mode: "fit-width".into(), scale: 1.0 };
        view.sidebar.open = true;
        view.sidebar.tab = "pages".into();
        Self { app, engine: EngineHost::new(), view, viewport: (900.0, 700.0) }
    }

    fn refit(&mut self) {
        let Some(doc) = &self.view.doc else { return };
        let Some(p) = doc.pages.get(self.view.page) else { return };
        let (w, h) = self.viewport;
        match self.view.zoom.mode.as_str() {
            "fit-width" => self.view.zoom.scale = ((w - 48.0) / p.width).max(0.1),
            "fit-page" => self.view.zoom.scale = ((w - 48.0) / p.width).min((h - 48.0) / p.height).max(0.1),
            _ => {}
        }
    }
}

impl AppCore for SpikeCore {
    fn ui_state(&mut self) -> UiState {
        UiState { state: json!({}), view: self.view.clone() }
    }

    fn act(&mut self, action: UiAction) {
        match action {
            UiAction::Open { path: None } => {
                if let Some(file) = self.app.dialog().file().add_filter("PDF document", &["pdf"]).blocking_pick_file() {
                    if let Ok(path) = file.into_path() {
                        self.open_path(&path);
                    }
                }
            }
            UiAction::Open { path: Some(p) } => self.open_path(Path::new(&p)),
            UiAction::Goto { page } => self.view.page = page,
            UiAction::Viewport { width, height } => {
                self.viewport = (width, height);
                self.refit();
            }
            UiAction::Zoom { to, scale } => {
                match to.as_str() {
                    "fixed" => {
                        self.view.zoom.mode = "fixed".into();
                        self.view.zoom.scale = scale.unwrap_or(1.0).clamp(0.1, 8.0);
                    }
                    "actual" => self.view.zoom = ZoomView { mode: "fixed".into(), scale: 1.0 },
                    other => self.view.zoom.mode = other.into(),
                }
                self.refit();
            }
            UiAction::Pane { which, open, tab } => {
                let pane = if which == "sidebar" { &mut self.view.sidebar } else { &mut self.view.panel };
                pane.open = open.unwrap_or(!pane.open);
                if let Some(t) = tab {
                    pane.tab = t;
                }
            }
            UiAction::Tool { id, .. } => {
                self.view.tool.chip = (id != "select").then(|| "Esc to stop".into());
                self.view.tool.id = id;
            }
            UiAction::Key { key, .. } if key == "Escape" => {
                self.view.tool.id = "select".into();
                self.view.tool.chip = None;
            }
            _ => {}
        }
    }

    fn menu(&mut self, id: &str) {
        match id {
            "open" => self.act(UiAction::Open { path: None }),
            "view:zoom-in" => self.act(UiAction::Zoom { to: "fixed".into(), scale: Some(self.view.zoom.scale * 1.25) }),
            "view:zoom-out" => self.act(UiAction::Zoom { to: "fixed".into(), scale: Some(self.view.zoom.scale / 1.25) }),
            "view:actual-size" => self.act(UiAction::Zoom { to: "actual".into(), scale: None }),
            "view:fit-width" => self.act(UiAction::Zoom { to: "fit-width".into(), scale: None }),
            "view:fit-page" => self.act(UiAction::Zoom { to: "fit-page".into(), scale: None }),
            id if id.starts_with("tool:") => self.act(UiAction::Tool { id: id[5..].into(), variant: None }),
            _ => {}
        }
    }

    fn open_path(&mut self, path: &Path) {
        let started = std::time::Instant::now();
        let Ok(bytes) = std::fs::read(path) else { return };
        let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        match self.engine.request(json!({"op": "open", "name": name, "pdf": {"$buffer": 0}}), vec![bytes]) {
            Ok(reply) => {
                let pages: Vec<PageView> = reply
                    .get("pages")
                    .and_then(Value::as_array)
                    .map(|ps| {
                        ps.iter()
                            .enumerate()
                            .map(|(i, p)| PageView {
                                width: p.get("width").and_then(Value::as_f64).unwrap_or(612.0),
                                height: p.get("height").and_then(Value::as_f64).unwrap_or(792.0),
                                rotation: 0,
                                label: (i + 1).to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.view.doc = Some(DocView {
                    name,
                    save_state: format!("Opened in {} ms", started.elapsed().as_millis()),
                    page_count: pages.len(),
                    pages,
                    ..DocView::default()
                });
                self.view.screen = "editor".into();
                self.view.page = 0;
                self.view.generation += 1;
                self.refit();
            }
            Err(e) => {
                self.view.notice = Some(tandem_protocol::ui::Notice { kind: "error".into(), text: e.message, actions: vec![] })
            }
        }
    }

    fn pixels(&mut self, kind: &str, page: usize, scale: f64) -> Result<Pixels, String> {
        let op = if kind == "thumb" { "render" } else { "render" };
        let mut reply = self.engine.request(json!({"op": op, "page": page, "scale": scale}), vec![]).map_err(|e| e.message)?;
        let width = reply.get("pixelWidth").and_then(Value::as_f64).unwrap_or(0.0) as u32;
        let height = reply.get("pixelHeight").and_then(Value::as_f64).unwrap_or(0.0) as u32;
        Ok(Pixels { width, height, rgba: reply.take_bytes("pixels").unwrap_or_default() })
    }

    fn close_requested(&mut self) -> bool {
        true
    }
}
