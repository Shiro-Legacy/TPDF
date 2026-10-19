//! Lane 1: renderer.ts 2708–2767, "zoom". Fit zooms read `#viewport`'s client size, which
//! the UI reports with [`Session::set_viewport`] (the drive keeps the Electron window's,
//! `shell::DRIVE_VIEWPORT`).
#![allow(dead_code)]

use super::Session;
use super::consts::{MAX_ZOOM, ZOOM_STEPS};
use super::dom_ids as ids;
use super::types::ZoomMode;
use crate::js::{js_num, js_round};
use tandem_protocol::limits::{MAX_PIXEL_EDGE, MAX_SCALE, MIN_SCALE};

/// R:2736 `target()`: the one wanted render, page + scale; anything newer supersedes it.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderTarget {
    pub page: i64,
    pub zoom: f64,
    pub scale: f64,
    /// `${page}:${scale}:${revision}`: a mutation repaints without a full reset.
    pub key: String,
}

impl Session {
    /// R:2711 `zoomNow`: CSS pixels per PDF point. Fit modes read the viewport.
    pub fn zoom_now(&self, page: i64) -> f64 {
        let Some(doc) = &self.doc else {
            return self.fixed_zoom;
        };
        if self.zoom_mode == ZoomMode::Fixed {
            return self.fixed_zoom;
        }
        let size = usize::try_from(page)
            .ok()
            .and_then(|p| doc.pages.get(p))
            .or_else(|| doc.pages.first());
        let Some(size) = size else {
            return self.fixed_zoom;
        };
        let (client_width, client_height) = self.lane1.viewport;
        let width = (client_width - 2.0).max(1.0);
        let fit = width / size.width;
        if self.zoom_mode == ZoomMode::FitPage {
            let height = (client_height - 2.0).max(1.0);
            return fit.min(height / size.height).min(MAX_ZOOM);
        }
        fit.min(MAX_ZOOM)
    }

    /// R:2725 `renderScale`: zoom × devicePixelRatio, inside the engine bounds and the
    /// 4096 px edge (clamped down, never over: the engine rejects anything past the edge).
    pub fn render_scale(&self, page: i64) -> f64 {
        let raw = self.zoom_now(page) * self.lane1.device_pixel_ratio;
        let clamped = MAX_SCALE.min(MIN_SCALE.max(raw));
        let size = self
            .doc
            .as_ref()
            .and_then(|doc| usize::try_from(page).ok().and_then(|p| doc.pages.get(p)));
        let Some(size) = size else { return clamped };
        let edge = (MAX_PIXEL_EDGE as f64 / size.width).min(MAX_PIXEL_EDGE as f64 / size.height);
        clamped.min((edge * 100.0).floor() / 100.0)
    }

    /// R:2736 `target()`: None while there is no document or it is locked.
    pub fn target(&self) -> Option<RenderTarget> {
        let doc = self.doc.as_ref()?;
        if doc.info.needs_password {
            return None;
        }
        let page = self.wanted_page;
        let zoom = self.zoom_now(page);
        let scale = self.render_scale(page);
        Some(RenderTarget {
            page,
            zoom,
            scale,
            key: format!("{page}:{}:{}", js_num(scale), self.revision),
        })
    }

    /// R:2745 `setZoom(mode, value = fixedZoom)`: pass `self.fixed_zoom` for the TS default.
    pub fn set_zoom(&mut self, mode: ZoomMode, value: f64) {
        self.zoom_mode = mode;
        if mode == ZoomMode::Fixed {
            self.fixed_zoom = value;
        }
        self.sync_zoom();
        self.drain();
    }

    /// R:2752 `zoomStep`: the next step strictly above or below the current zoom (0.001
    /// tolerance), clamped at the ends; always `fixed`.
    pub fn zoom_step(&mut self, direction: i64) {
        if self.doc.is_none() {
            return;
        }
        let current = self.zoom_now(self.wanted_page);
        let next = if direction > 0 {
            ZOOM_STEPS
                .iter()
                .copied()
                .find(|step| *step > current + 0.001)
                .unwrap_or(ZOOM_STEPS[ZOOM_STEPS.len() - 1])
        } else {
            ZOOM_STEPS
                .iter()
                .rev()
                .copied()
                .find(|step| *step < current - 0.001)
                .unwrap_or(ZOOM_STEPS[0])
        };
        self.set_zoom(ZoomMode::Fixed, next);
    }

    /// R:2761 `syncZoom`: the readout and the two pressed fit buttons.
    pub fn sync_zoom(&mut self) {
        let unlocked = self
            .doc
            .as_ref()
            .is_some_and(|doc| !doc.info.needs_password);
        let percent = if unlocked {
            js_round(self.zoom_now(self.wanted_page) * 100.0)
        } else {
            100.0
        };
        self.dom
            .set_text(ids::ZOOM_LEVEL, format!("{}%", js_num(percent)));
        let mode = self.zoom_mode;
        self.set_pressed(ids::FIT_WIDTH_BUTTON, mode == ZoomMode::FitWidth);
        self.set_pressed(ids::FIT_PAGE_BUTTON, mode == ZoomMode::FitPage);
    }

    /// `canvas.clientWidth` / `clientHeight`: the painted page's CSS size (R:2980
    /// `Math.round(result.width * zoom)`), or the 300 × 150 of a canvas never painted.
    pub fn canvas_client_size(&self) -> (f64, f64) {
        match &self.last_result {
            Some(result) => (
                js_round(result.width * self.shown_zoom),
                js_round(result.height * self.shown_zoom),
            ),
            None => (300.0, 150.0),
        }
    }

    /// `#zoom-out` (R:11303+).
    pub fn on_zoom_out(&mut self) {
        self.zoom_step(-1);
        self.run_pending_tasks();
    }

    /// `#zoom-in`.
    pub fn on_zoom_in(&mut self) {
        self.zoom_step(1);
        self.run_pending_tasks();
    }

    /// `#fit-width` (also ⌘0).
    pub fn on_fit_width(&mut self) {
        let value = self.fixed_zoom;
        self.set_zoom(ZoomMode::FitWidth, value);
        self.run_pending_tasks();
    }

    /// `#fit-page`.
    pub fn on_fit_page(&mut self) {
        let value = self.fixed_zoom;
        self.set_zoom(ZoomMode::FitPage, value);
        self.run_pending_tasks();
    }

    /// `#zoom-100`.
    pub fn on_zoom_100(&mut self) {
        self.set_zoom(ZoomMode::Fixed, 1.0);
        self.run_pending_tasks();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::types::OpenDoc;
    use tandem_protocol::engine::PageSize;

    fn with_letter() -> Session {
        let mut s = Session::new();
        s.doc = Some(OpenDoc {
            name: "a.pdf".into(),
            page_count: 1,
            pages: vec![PageSize {
                width: 612.0,
                height: 792.0,
            }],
            ..OpenDoc::default()
        });
        s
    }

    #[test]
    fn fit_math() {
        let mut s = with_letter();
        s.set_viewport(802.0, 675.0);
        assert!((s.zoom_now(0) - 800.0 / 612.0).abs() < 1e-12);
        s.zoom_mode = ZoomMode::FitPage;
        assert!((s.zoom_now(0) - 673.0 / 792.0).abs() < 1e-12);
        s.zoom_mode = ZoomMode::Fixed;
        s.fixed_zoom = 2.0;
        assert_eq!(s.zoom_now(5), 2.0);
        s.set_viewport(100_000.0, 100_000.0);
        s.zoom_mode = ZoomMode::FitWidth;
        assert_eq!(s.zoom_now(0), MAX_ZOOM);
        assert_eq!(s.render_scale(0), 4.0);
        let target = s.target().unwrap();
        assert_eq!(target.key, "0:4:0");
    }

    #[test]
    fn edge_cap() {
        let mut s = with_letter();
        s.doc.as_mut().unwrap().pages[0] = PageSize {
            width: 3000.0,
            height: 1000.0,
        };
        s.zoom_mode = ZoomMode::Fixed;
        s.fixed_zoom = 4.0;
        assert_eq!(s.render_scale(0), 1.36);
    }
}
