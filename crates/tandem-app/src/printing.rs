//! Pages of a print job as pictures, for the print paths that draw pages themselves
//! (Windows GDI and the GTK print dialog). macOS hands the PDF to PDFKit instead.
//!
//! The print PDF (the core already built it: the chosen pages, annotations flattened as
//! the document shows them) is opened in an engine of its own, so printing never touches
//! the open document.

use crate::engine::EngineHost;
use serde_json::{Value, json};
use tandem_protocol::engine::{OpenReply, PageSize};
use tandem_protocol::limits::{MAX_PIXEL_EDGE, MAX_SCALE, MIN_SCALE};

/// The most pages are drawn at for a printer (the engine's largest scale, 4×, is 288 dpi):
/// enough for text and line art at normal sizes; an A4 page stays near 35 MB of pixels.
pub const PRINT_DPI: f64 = 300.0;

/// One page drawn: `width`×`height` RGBA pixels, top row first.
#[derive(Debug)]
pub struct PagePicture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// An opened print job.
pub struct PrintDoc {
    engine: EngineHost,
    /// Each page's size in points (as the engine shows it, rotation applied).
    pub pages: Vec<PageSize>,
}

impl PrintDoc {
    /// Opens the print PDF.
    pub fn open(pdf: &[u8]) -> Result<PrintDoc, String> {
        let mut engine = EngineHost::new();
        let reply = engine
            .request(
                json!({"op": "open", "name": "print.pdf", "pdf": {"$buffer": 0}}),
                vec![pdf.to_vec()],
            )
            .map_err(|e| e.message)?;
        let opened: OpenReply =
            serde_json::from_value(reply.value.clone()).map_err(|e| format!("print: {e}"))?;
        Ok(PrintDoc {
            engine,
            pages: opened.pages,
        })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Draws page `page` (0-based) at `dpi` (at most [`PRINT_DPI`] and the engine's edge).
    pub fn render(&mut self, page: usize, dpi: f64) -> Result<PagePicture, String> {
        let size = self
            .pages
            .get(page)
            .ok_or_else(|| format!("print: no page {}", page + 1))?;
        let longest = size.width.max(size.height).max(1.0);
        let dpi = if dpi.is_finite() && dpi > 0.0 {
            dpi.min(PRINT_DPI)
        } else {
            PRINT_DPI
        };
        let edge = (MAX_PIXEL_EDGE as f64 / longest * 1000.0).floor() / 1000.0;
        let scale = (dpi / 72.0).min(edge).clamp(MIN_SCALE, MAX_SCALE);
        let mut reply = self
            .engine
            .request(
                json!({"op": "render", "page": page, "scale": scale}),
                vec![],
            )
            .map_err(|e| e.message)?;
        let dim = |key: &str| reply.get(key).and_then(Value::as_f64).unwrap_or(0.0) as u32;
        let (width, height) = (dim("pixelWidth"), dim("pixelHeight"));
        let rgba = reply.take_bytes("pixels").unwrap_or_default();
        if width == 0 || height == 0 || rgba.len() != width as usize * height as usize * 4 {
            return Err(format!("print: page {} could not be drawn", page + 1));
        }
        Ok(PagePicture {
            width,
            height,
            rgba,
        })
    }
}

/// Where a `w`×`h` picture goes inside an area `area_w`×`area_h` (device units), as large as
/// fits with its shape kept, centred: `(x, y, width, height)`.
pub fn fit(w: u32, h: u32, area_w: f64, area_h: f64) -> (f64, f64, f64, f64) {
    let (w, h) = (f64::from(w.max(1)), f64::from(h.max(1)));
    let s = (area_w / w).min(area_h / h);
    let (dw, dh) = (w * s, h * s);
    ((area_w - dw) / 2.0, (area_h - dh) / 2.0, dw, dh)
}

/// RGBA to the BGRA order Windows bitmaps and Cairo (little-endian ARGB32) use, over white
/// (a printer has no transparency).
pub fn to_bgra_on_white(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.as_chunks::<4>().0 {
        let a = u16::from(px[3]);
        let over = |c: u8| ((u16::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
        out.extend_from_slice(&[over(px[2]), over(px[1]), over(px[0]), 255]);
    }
    out
}

/// Pages from a page-range text such as `1-3, 5` (1-based, as people type them in print
/// dialogs), within `count` pages; an empty text is every page.
pub fn pages_in(range: &str, count: usize) -> Vec<usize> {
    if range.trim().is_empty() {
        return (0..count).collect();
    }
    let mut out = Vec::new();
    for part in range.split(',') {
        let part = part.trim();
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim().parse::<usize>(), b.trim().parse::<usize>()),
            None => (part.parse::<usize>(), part.parse::<usize>()),
        };
        if let (Ok(a), Ok(b)) = (a, b) {
            for p in a.min(b)..=a.max(b) {
                if p >= 1 && p <= count && !out.contains(&(p - 1)) {
                    out.push(p - 1);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_each_page_of_a_job() {
        let pdf = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/nav.pdf"),
        )
        .unwrap();
        let mut doc = PrintDoc::open(&pdf).unwrap();
        assert_eq!(doc.page_count(), 3);
        let page = doc.render(0, 600.0).unwrap();
        // Capped at 300 dpi and the engine's largest scale.
        let want = (doc.pages[0].width * (PRINT_DPI / 72.0).min(MAX_SCALE)).round() as u32;
        assert!(page.width.abs_diff(want) <= 1, "{} vs {want}", page.width);
        assert!(doc.render(3, 300.0).is_err());
    }

    #[test]
    fn fitting_and_colours() {
        assert_eq!(fit(100, 200, 400.0, 400.0), (100.0, 0.0, 200.0, 400.0));
        assert_eq!(to_bgra_on_white(&[10, 20, 30, 255]), [30, 20, 10, 255]);
        assert_eq!(to_bgra_on_white(&[0, 0, 0, 0]), [255, 255, 255, 255]);
    }

    #[test]
    fn ranges() {
        assert_eq!(pages_in("", 3), [0, 1, 2]);
        assert_eq!(pages_in("2-3, 1", 3), [1, 2, 0]);
        assert_eq!(pages_in("5, 2", 3), [1]);
        assert_eq!(pages_in("3-2", 3), [1, 2]);
    }
}
