//! Pages of a print job as pictures, for the print paths that draw pages themselves
//! (Windows GDI and the GTK print dialog). macOS hands the PDF to PDFKit instead. Pages
//! are laid out as the Mac's panel does ([`place`]).
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

/// A print job's pages drawn on a thread of their own with the core's roomy stack (deep
/// content streams need it), for print paths that run on the main thread (GTK).
/// One page to draw for a [`PrintWorker`], and where its picture goes.
type PrintJob = (
    usize,
    f64,
    std::sync::mpsc::Sender<Result<PagePicture, String>>,
);

pub struct PrintWorker {
    jobs: std::sync::mpsc::Sender<PrintJob>,
    pub pages: Vec<PageSize>,
}

impl PrintWorker {
    /// Opens the print PDF on its own thread.
    pub fn open(pdf: Vec<u8>) -> Result<PrintWorker, String> {
        let (jobs, inbox) = std::sync::mpsc::channel::<PrintJob>();
        let (opened_tx, opened) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("tandem-print".into())
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                let mut doc = match PrintDoc::open(&pdf) {
                    Ok(doc) => {
                        let _ = opened_tx.send(Ok(doc.pages.clone()));
                        doc
                    }
                    Err(e) => {
                        let _ = opened_tx.send(Err(e));
                        return;
                    }
                };
                for (page, dpi, reply) in inbox {
                    let _ = reply.send(doc.render(page, dpi));
                }
            })
            .map_err(|e| format!("print: {e}"))?;
        let pages = opened
            .recv()
            .map_err(|_| "print: the page renderer stopped".to_string())??;
        Ok(PrintWorker { jobs, pages })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// [`PrintDoc::render`] on the worker thread; waits for the picture.
    pub fn render(&self, page: usize, dpi: f64) -> Result<PagePicture, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.jobs
            .send((page, dpi, tx))
            .map_err(|_| "print: the page renderer stopped".to_string())?;
        rx.recv()
            .map_err(|_| "print: the page renderer stopped".to_string())?
    }
}

/// Where a page goes on the paper, in device units: drawn turned a quarter clockwise when
/// `rotate`, into the box at `x`, `y` of `width`×`height`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub rotate: bool,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// As the Mac's print panel lays pages out (scale down to fit, rotate to the paper): a page
/// of `page_w`×`page_h` points at its real size on a printable area of `area_w`×`area_h`
/// device units at `dpi_x`×`dpi_y`, turned when its orientation differs from the area's,
/// shrunk only when it does not fit, centred.
pub fn place(
    page_w: f64,
    page_h: f64,
    area_w: f64,
    area_h: f64,
    dpi_x: f64,
    dpi_y: f64,
) -> Placement {
    let rotate = page_w != page_h && area_w != area_h && (page_w > page_h) != (area_w > area_h);
    let (w, h) = if rotate {
        (page_h, page_w)
    } else {
        (page_w, page_h)
    };
    let (w, h) = (w.max(1.0) * dpi_x / 72.0, h.max(1.0) * dpi_y / 72.0);
    let s = (area_w / w).min(area_h / h).min(1.0);
    let (width, height) = (w * s, h * s);
    Placement {
        rotate,
        x: (area_w - width) / 2.0,
        y: (area_h - height) / 2.0,
        width,
        height,
    }
}

/// A `w`×`h` picture of 4-byte pixels turned a quarter clockwise (it becomes `h`×`w`).
pub fn rotate_clockwise(px: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; px.len()];
    for y in 0..h {
        for x in 0..w {
            // (x, y) lands at column h-1-y of row x.
            let from = (y * w + x) * 4;
            let to = (x * h + (h - 1 - y)) * 4;
            out[to..to + 4].copy_from_slice(&px[from..from + 4]);
        }
    }
    out
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
        // A Letter page on Letter paper at 72 dpi: real size, centred in a bigger area.
        let p = place(612.0, 792.0, 700.0, 800.0, 72.0, 72.0);
        assert_eq!(
            (p.rotate, p.x, p.y, p.width, p.height),
            (false, 44.0, 4.0, 612.0, 792.0)
        );
        // Too big: shrunk to fit, never enlarged.
        let p = place(1224.0, 1584.0, 612.0, 792.0, 72.0, 72.0);
        assert_eq!((p.width, p.height), (612.0, 792.0));
        // Landscape page, portrait paper: turned, and in device units at 300×600 dpi.
        let p = place(792.0, 612.0, 2550.0, 6600.0, 300.0, 600.0);
        assert!(p.rotate);
        assert_eq!((p.width, p.height), (2550.0, 6600.0));
        // 2×3 pixels turned clockwise: the bottom-left pixel comes to the top-left.
        let px: Vec<u8> = (0..6u8).flat_map(|i| [i, 0, 0, 0]).collect();
        let turned: Vec<u8> = rotate_clockwise(&px, 2, 3)
            .chunks(4)
            .map(|c| c[0])
            .collect();
        assert_eq!(turned, [4, 2, 0, 5, 3, 1]);
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
