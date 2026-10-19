//! Thumbnails: the Pages sidebar (renderer.ts R:5431-5660), and lane 3's own state.
//!
//! Fixed-row page list. Every page gets a sized placeholder (no bitmap yet), so the scroll
//! height of a 500-page document is right before a single pixel is rendered; only the rows in
//! the viewport plus THUMB_BUFFER are ever asked for, one at a time, and never in front of a
//! page render.
#![allow(dead_code)]

use super::Session;
use super::consts::{THUMB_BOX_PX, THUMB_BUFFER, THUMB_ROW_PX};
use super::dom_ids::{PAGES_EMPTY_EL, THUMBS_EL, THUMBS_TOGGLE};
use super::types::{PageSize, RenderCache, ThumbItem, ThumbReply};
use crate::host::util::js_number;
use serde_json::json;
use std::collections::BTreeMap;
use tandem_protocol::engine::RedactPageCount;
use tandem_protocol::limits::THUMB_MAX_EDGE;

/// State lane 3 needs that the renderer kept in the DOM (`Session::lane3`).
#[derive(Clone, Debug, PartialEq)]
pub struct Lane3State {
    /// `thumbsEl.scrollTop` (CSS px); the UI reports it with [`Session::on_thumbs_scroll`].
    pub thumbs_scroll_top: f64,
    /// `thumbsEl.clientHeight` (CSS px). Until the UI reports it, the Electron window's list
    /// height is assumed (860 px content minus the toolbars), so a smoke's `P1_THUMBS` line
    /// still reports a first visible batch.
    pub thumbs_client_height: f64,
    /// The rows' `aria-label`s ("Page 2 (ii) of 9"), fixed when the rows are built.
    pub thumb_labels: Vec<String>,
    /// What each row's canvas holds once painted (the canvas bitmap in the renderer).
    pub thumb_bitmaps: BTreeMap<usize, ThumbReply>,
    /// The confirm dialog's "Boxes per page" table body (`previewTable.tBodies[0]`, U:388).
    pub redact_confirm_rows: Vec<RedactPageCount>,
}

impl Default for Lane3State {
    fn default() -> Self {
        Lane3State {
            thumbs_scroll_top: 0.0,
            thumbs_client_height: 720.0,
            thumb_labels: Vec::new(),
            thumb_bitmaps: BTreeMap::new(),
            redact_confirm_rows: Vec::new(),
        }
    }
}

/// `aria-pressed` written as the renderer did, mirrored into `checked` (see `dom::Node`).
pub(crate) fn set_pressed(dom: &mut crate::dom::Dom, id: &str, on: bool) {
    dom.set_attr(id, "aria-pressed", if on { "true" } else { "false" });
    dom.set_checked(id, on);
}

// `createRenderCache` (engine-shared.ts:114) on the session's caches; free functions so they
// never clash with the methods the render part gives `RenderCache`.

pub(crate) fn cache_get<'a, T>(cache: &'a RenderCache<T>, key: &str) -> Option<&'a T> {
    cache.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Evicts oldest-first; the newest entry always stays, even alone over the byte budget.
pub(crate) fn cache_set_thumb(cache: &mut RenderCache<ThumbReply>, key: &str, value: ThumbReply) {
    if let Some(at) = cache.entries.iter().position(|(k, _)| k == key)
        && let Some((_, previous)) = cache.entries.remove(at)
    {
        cache.bytes -= previous.pixels.len();
    }
    cache.bytes += value.pixels.len();
    cache.entries.push_back((key.to_string(), value));
    while cache.entries.len() > 1
        && (cache.entries.len() > cache.limit_entries || cache.bytes > cache.limit_bytes)
    {
        if let Some((_, evicted)) = cache.entries.pop_front() {
            cache.bytes -= evicted.pixels.len();
        }
    }
}

pub(crate) fn cache_clear<T>(cache: &mut RenderCache<T>) {
    cache.entries.clear();
    cache.bytes = 0;
}

/// `visibleRange` (engine-shared.ts:159): the half-open `[start, end)` rows visible in a
/// scroll container, widened by `buffer` rows each side and clamped to the list.
pub(crate) fn visible_range(
    scroll_top: f64,
    viewport_height: f64,
    tops: &[f64],
    heights: &[f64],
    buffer: usize,
) -> (usize, usize) {
    let count = tops.len().min(heights.len());
    let top = if scroll_top.is_finite() {
        scroll_top
    } else {
        0.0
    };
    let height = if viewport_height.is_finite() && viewport_height > 0.0 {
        viewport_height
    } else {
        0.0
    };
    let bottom = top + height;
    let mut first: Option<usize> = None;
    let mut last = 0;
    for index in 0..count {
        if tops[index] < bottom && tops[index] + heights[index] > top {
            if first.is_none() {
                first = Some(index);
            }
            last = index;
        }
    }
    match first {
        None => (0, 0),
        Some(first) => (first.saturating_sub(buffer), count.min(last + 1 + buffer)),
    }
}

/// The CSS size of a row's canvas: the page fitted into THUMB_BOX_PX (R:5485-5489).
pub fn thumb_canvas_size(size: &PageSize) -> Option<(f64, f64)> {
    if !(size.width > 0.0 && size.height > 0.0) {
        return None;
    }
    let fit = THUMB_BOX_PX / size.width.max(size.height);
    Some((
        crate::js::js_round(size.width * fit).max(1.0),
        crate::js::js_round(size.height * fit).max(1.0),
    ))
}

impl Session {
    /// R:5439.
    pub fn reset_thumbs(&mut self) {
        self.dom.set_hidden(THUMBS_EL, true);
        self.dom.set_disabled(THUMBS_TOGGLE, true);
        set_pressed(&mut self.dom, THUMBS_TOGGLE, false);
        self.thumb_items.clear();
        self.lane3.thumb_labels.clear();
        self.lane3.thumb_bitmaps.clear();
        self.thumb_tried.clear();
        self.thumb_painted.clear();
        cache_clear(&mut self.thumb_cache);
        self.thumb_current = -1;
        self.thumb_tab = -1;
        self.thumb_open = false;
        self.thumb_rendered = 0;
        self.thumb_ms = 0.0;
        self.thumb_smoke_done = false;
        // thumbBusy is left alone: an in-flight request settles on its own.
    }

    /// R:5458.
    pub fn refresh_thumbs(&mut self) {
        self.reset_thumbs();
        let Some(doc) = &self.doc else {
            return;
        };
        let pages = doc.pages.clone();
        let open = self.sidebar_open && doc.page_count >= 2 && !doc.info.needs_password;
        self.build_thumbs(&pages);
        self.set_thumb_current(self.wanted_page);
        // Default open for a document with pages to page through, closed otherwise.
        // The user's own collapse survives a page op: only a new document resets it.
        self.set_thumb_sidebar(open);
    }

    /// R:5468: one row per page (the canvas size comes from [`thumb_canvas_size`]).
    pub fn build_thumbs(&mut self, pages: &[PageSize]) {
        for page in 0..pages.len() {
            let label = format!(
                "Page {}{} of {}",
                page + 1,
                self.label_suffix(page),
                pages.len()
            );
            self.lane3.thumb_labels.push(label);
            self.thumb_items.push(ThumbItem {
                page,
                current: false,
                tab_stop: false,
            });
        }
    }

    /// A click on a row (R:5494).
    pub fn on_thumb_click(&mut self, page: usize) {
        self.go_to(page as i64);
    }

    /// R:5503: arrow keys move focus inside the list; Enter/click is what changes the page.
    /// Returns the row to focus (the key was handled), or None (the key goes on).
    pub fn on_thumb_key(&mut self, page: usize, key: &str) -> Option<usize> {
        let len = self.thumb_items.len() as i64;
        let target = match key {
            "ArrowDown" | "ArrowRight" => page as i64 + 1,
            "ArrowUp" | "ArrowLeft" => page as i64 - 1,
            "Home" => 0,
            "End" => len - 1,
            _ => return None,
        };
        let target = target.max(0).min(len - 1);
        self.set_thumb_tab(target);
        (target >= 0).then_some(target as usize)
    }

    /// R:5519 (the UI scrolls the current row into view while the sidebar is open).
    pub fn set_thumb_current(&mut self, page: i64) {
        if self.thumb_current != page {
            if let Some(item) = self.thumb_item_mut(self.thumb_current) {
                item.current = false;
            }
            self.thumb_current = page;
            if let Some(item) = self.thumb_item_mut(page) {
                item.current = true;
            }
        }
        self.set_thumb_tab(page);
    }

    /// R:5533: roving tabindex: exactly one row is a tab stop.
    pub fn set_thumb_tab(&mut self, page: i64) {
        if self.thumb_tab == page {
            return;
        }
        if let Some(item) = self.thumb_item_mut(self.thumb_tab) {
            item.tab_stop = false;
        }
        self.thumb_tab = page;
        if let Some(item) = self.thumb_item_mut(page) {
            item.tab_stop = true;
        }
    }

    fn thumb_item_mut(&mut self, page: i64) -> Option<&mut ThumbItem> {
        usize::try_from(page)
            .ok()
            .and_then(|p| self.thumb_items.get_mut(p))
    }

    /// R:5544: the Pages collapse. The thumbnail list goes away and so does the column
    /// (`sidebar_open` false is `.collapsed`); the tab rail stays. Moving focus out of a
    /// collapsing sidebar is the UI's.
    pub fn set_thumb_sidebar(&mut self, open: bool) {
        self.sidebar_open = open;
        self.thumb_open = open && self.doc.as_ref().is_some_and(|d| !d.info.needs_password);
        self.dom.set_hidden(THUMBS_EL, !self.thumb_open);
        self.dom.set_hidden(PAGES_EMPTY_EL, self.doc.is_some());
        // The button is named for the thumbnails it shows, so it reports those, not the column.
        set_pressed(&mut self.dom, THUMBS_TOGGLE, self.thumb_open);
        if !self.thumb_open {
            return;
        }
        self.set_thumb_current(self.wanted_page);
        self.pump_thumbs();
    }

    /// A scroll on `#thumbs` (R:11296): the viewport moved or resized; more rows may be due.
    pub fn on_thumbs_scroll(&mut self, scroll_top: f64, client_height: f64) {
        self.lane3.thumbs_scroll_top = scroll_top;
        self.lane3.thumbs_client_height = client_height;
        self.pump_thumbs();
    }

    /// R:5561: the rows in the sidebar viewport, ± THUMB_BUFFER, as `[start, end)`.
    pub fn thumb_range(&self) -> (usize, usize) {
        let tops: Vec<f64> = (0..self.thumb_items.len())
            .map(|p| p as f64 * THUMB_ROW_PX)
            .collect();
        let heights = vec![THUMB_ROW_PX; self.thumb_items.len()];
        visible_range(
            self.lane3.thumbs_scroll_top,
            self.lane3.thumbs_client_height,
            &tops,
            &heights,
            THUMB_BUFFER,
        )
    }

    /// R:5571.
    pub fn thumb_done(&self, page: i64) -> bool {
        self.thumb_painted.contains(&page) || self.thumb_tried.contains(&page)
    }

    /// R:5576: a mutation only repaints the affected page's thumbnail; the rest stay valid.
    pub fn invalidate_thumb(&mut self, page: i64) {
        if page < 0 {
            return;
        }
        self.thumb_painted.remove(&page);
        self.thumb_tried.remove(&page);
    }

    /// R:5583: true while the wanted page is not the painted one: a page render always wins.
    pub fn page_render_pending(&self) -> bool {
        match &self.doc {
            None => return false,
            Some(doc) if doc.info.needs_password => return false,
            Some(_) => {}
        }
        match self.target() {
            Some(goal) => {
                Some(goal.key.as_str()) != self.shown_key.as_deref() || goal.zoom != self.shown_zoom
            }
            None => false,
        }
    }

    /// R:5589. The renderer requested one row and came back here when it settled; the
    /// engine answers at once now, so this loops over the due rows instead.
    pub fn pump_thumbs(&mut self) {
        loop {
            if !self.thumb_open || self.doc.as_ref().is_none_or(|d| d.info.needs_password) {
                return;
            }
            if self.thumb_busy
                || self.in_flight.is_some()
                || self.saving
                || self.page_render_pending()
            {
                return;
            }
            let (start, end) = self.thumb_range();
            let next = (start..end).find(|page| !self.thumb_done(*page as i64));
            match next {
                None => {
                    self.report_thumbs(start, end);
                    return;
                }
                Some(page) => self.render_thumb_once(page),
            }
        }
    }

    /// R:5609.
    pub fn render_thumb(&mut self, page: usize) {
        self.render_thumb_once(page);
        self.pump_thumbs();
    }

    /// R:5609-5634: one row, without the trailing `pumpThumbs()`.
    fn render_thumb_once(&mut self, page: usize) {
        let started_generation = self.generation;
        let key = format!("{page}:{THUMB_MAX_EDGE}:{}", self.revision);
        let started = std::time::Instant::now();
        self.thumb_busy = true;
        let cached = cache_get(&self.thumb_cache, &key).cloned();
        let was_cached = cached.is_some();
        let reply = match cached {
            Some(reply) => Ok(reply),
            None => self
                .request(
                    json!({ "op": "thumbnail", "page": page, "maxEdge": THUMB_MAX_EDGE }),
                    Vec::new(),
                )
                .map(|mut reply| {
                    let pixels = reply.take_bytes("pixels").unwrap_or_default();
                    let mut thumb: ThumbReply =
                        super::parse_reply_or_default(&reply.value);
                    thumb.pixels = pixels;
                    thumb
                }),
        };
        match reply {
            Ok(reply) => {
                if started_generation == self.generation {
                    self.thumb_ms += crate::js::js_round(started.elapsed().as_secs_f64() * 1000.0);
                    if self.draw_thumb(page, &reply) {
                        if !was_cached {
                            cache_set_thumb(&mut self.thumb_cache, &key, reply);
                        }
                        self.thumb_rendered += 1;
                    } else {
                        // No canvas: mark the page tried, or the cached reply would be found
                        // again and re-drawn in a loop behind pumpThumbs.
                        self.thumb_tried.insert(page as i64);
                    }
                }
            }
            Err(_) => {
                // Decoration only: the placeholder stays, and a failure never reaches
                // showError, the status line or workerFailed.
                if started_generation == self.generation {
                    self.thumb_tried.insert(page as i64);
                }
            }
        }
        self.thumb_busy = false;
    }

    /// R:5639: false when the row cannot be painted; the caller must then stop retrying it.
    pub fn draw_thumb(&mut self, page: usize, reply: &ThumbReply) -> bool {
        if page >= self.thumb_items.len() {
            return false;
        }
        self.lane3.thumb_bitmaps.insert(page, reply.clone());
        self.thumb_painted.insert(page as i64);
        true
    }

    /// R:5651: one `P1_THUMBS` line per open, once the first visible batch is settled.
    pub fn report_thumbs(&mut self, start: usize, end: usize) {
        if self.thumb_smoke_done || end <= start {
            return;
        }
        self.thumb_smoke_done = true;
        let payload = json!({
            "visible": end - start,
            "rendered": self.thumb_rendered,
            "thumbMs": js_number(self.thumb_ms),
        });
        self.console_line("P1_THUMBS", payload);
    }

    /// The row's `aria-label` (R:5477).
    pub fn thumb_label(&self, page: usize) -> Option<&str> {
        self.lane3.thumb_labels.get(page).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_rows_with_buffer() {
        let tops: Vec<f64> = (0..20).map(|i| i as f64 * 88.0).collect();
        let heights = vec![88.0; 20];
        assert_eq!(visible_range(0.0, 720.0, &tops, &heights, 4), (0, 13));
        assert_eq!(visible_range(880.0, 176.0, &tops, &heights, 4), (6, 16));
        assert_eq!(visible_range(0.0, 0.0, &tops, &heights, 4), (0, 0));
        assert_eq!(
            visible_range(f64::NAN, 100.0, &tops[..1], &heights[..1], 4),
            (0, 1)
        );
    }

    #[test]
    fn canvas_fits_the_box() {
        let size = PageSize {
            width: 612.0,
            height: 792.0,
        };
        assert_eq!(thumb_canvas_size(&size), Some((49.0, 64.0)));
        assert_eq!(
            thumb_canvas_size(&PageSize {
                width: 0.0,
                height: 10.0
            }),
            None
        );
    }

    #[test]
    fn cache_evicts_oldest_but_keeps_newest() {
        let mut cache: RenderCache<ThumbReply> = RenderCache::new(2, 10);
        let thumb = |n: usize| ThumbReply {
            pixels: vec![0; n],
            ..ThumbReply::default()
        };
        cache_set_thumb(&mut cache, "a", thumb(4));
        cache_set_thumb(&mut cache, "b", thumb(4));
        cache_set_thumb(&mut cache, "c", thumb(4));
        assert!(cache_get(&cache, "a").is_none());
        assert_eq!(cache.bytes, 8);
        cache_set_thumb(&mut cache, "d", thumb(40));
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.bytes, 40);
        cache_clear(&mut cache);
        assert_eq!(cache.bytes, 0);
    }
}
