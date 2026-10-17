// SPDX-License-Identifier: AGPL-3.0-or-later
//! redact-preview and redact-apply stay within memory and time on crafted
//! files (acceptance round 2, fix round 3):
//!
//! - clip paths inside a touched Form under many mark rects: `form_parts`
//!   recorded an entry per clip and rect before any bound (50,000 clips
//!   under 100 search hits held 973 MB; 4,000 clips under 2,000 marks
//!   1,345 MB);
//! - 262,144 file-borne rects (Redact annotations sharing one /QuadPoints
//!   array): region queries cloned and sorted every wide rect per query
//!   (74-89 s);
//! - the pixel check walked every pixel of every rect of every mark (290 s
//!   at 262,144 page-size quads).
//!
//! Each case measures how far this process's resident memory rises while
//! the op runs (sampled; the cases run one at a time) and its time. The
//! WASM build's memory, which never shrinks and so records the peak
//! exactly, is measured by `tests/rust-redact-bounds.test.mjs`. The sizes
//! are the review's; the bounds hold far below what the old code reached,
//! without reproducing it.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};

/// One measured case at a time (resident memory is process-wide).
static SERIAL: Mutex<()> = Mutex::new(());

/// This process's resident memory (bytes), as `ps` reports it.
fn resident() -> usize {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.trim().parse::<usize>().ok())
        .map_or(0, |kb| kb * 1024)
}

/// `run`'s result, how far resident memory rose above where it started
/// (sampled while it runs) and its time.
fn measure<T>(run: impl FnOnce() -> T) -> (T, usize, Duration) {
    let base = resident();
    let peak = Arc::new(AtomicUsize::new(base));
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let (peak, stop) = (Arc::clone(&peak), Arc::clone(&stop));
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                peak.fetch_max(resident(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    };
    let start = Instant::now();
    let out = run();
    let took = start.elapsed();
    peak.fetch_max(resident(), Ordering::Relaxed);
    stop.store(true, Ordering::Relaxed);
    sampler.join().ok();
    (out, peak.load(Ordering::Relaxed).saturating_sub(base), took)
}

const MB: usize = 1 << 20;

/// A classic-xref PDF from object bodies numbered from 1.
fn classic(objects: &[String]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body.as_bytes());
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn stream(dict: &str, content: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
}

const HELV: &str =
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session)
}

fn op(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    engine.dispatch(session, request, Vec::new()).0
}

fn none() -> Value {
    json!({"info": false, "xmp": false, "attachments": false, "outlines": false,
        "annots": false, "links": false, "fields": false, "layers": false, "struct": false})
}

/// redact-apply's reply and the copy (empty on a refusal).
fn apply(engine: &mut AppEngine, session: &mut AppSession) -> (Value, Vec<u8>) {
    let request = json!({"id": 9, "op": "redact-apply", "sanitize": none(),
        "entropy": {"$buffer": 0}});
    let (reply, mut outputs) = engine.dispatch(session, request, vec![vec![7u8; 32]]);
    let bytes = match reply["ok"] == true {
        true => outputs.remove(0),
        false => Vec::new(),
    };
    (reply, bytes)
}

fn holds(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// Every string, name and decoded stream of a copy.
fn everything(bytes: &[u8]) -> Vec<u8> {
    use tandem_pdf_engine::document::{Document, DocumentLimits};
    use tandem_pdf_engine::object::Object;
    use tandem_pdf_engine::stream::StreamLimits;
    let doc = Document::open_with_password(bytes, DocumentLimits::default(), b"").unwrap();
    let mut out = Vec::new();
    for object in doc.objects() {
        let mut stack = vec![&object.value];
        while let Some(value) = stack.pop() {
            match value {
                Object::String(s) | Object::Name(s) => {
                    out.extend_from_slice(s);
                    out.push(b'\n');
                }
                Object::Array(items) => stack.extend(items),
                Object::Dictionary(dict) => stack.extend(dict.values()),
                _ => {}
            }
        }
        if object.stream.is_some() {
            if let Ok(decoded) = doc.decoded_stream(object.id, StreamLimits::default()) {
                out.extend_from_slice(&decoded);
                out.push(b'\n');
            }
        }
    }
    out
}

/// One 612 × 792 page drawing `page` (object 4) with Helvetica (5), the
/// Form /F (6, page-size /BBox) drawing `form` and `xobjects`; `annots`,
/// and more objects from 7.
fn form_page(page: &str, form: &str, xobjects: &str, annots: &str, more: &[String]) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /XObject << /F 6 0 R {xobjects} >> >> /Contents 4 0 R {annots} >>"
        ),
        stream("", page),
        HELV.to_string(),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>",
            form,
        ),
    ];
    objects.extend_from_slice(more);
    classic(&objects)
}

#[test]
fn clips_in_a_form_under_many_search_hits_stay_bounded() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Review probe p46: a page-size Form of 50,000 clips (`q re W n Q`)
    // and one search with 100 hits. Before: 973 MB of RSS for the preview
    // (an entry per clip and hit), 983 MB for apply.
    let clips = "q 0 0 612 792 re W n Q\n".repeat(50_000);
    let words: String = (0..100)
        .map(|i| {
            format!(
                "1 0 0 1 {} {} Tm (ZQ) Tj ",
                20 + (i % 20) * 28,
                770 - (i / 20) * 15
            )
        })
        .collect();
    let bytes = form_page(
        &format!("/F Do BT /F1 6 Tf {words}ET"),
        &format!("{clips}0.9 g 0 0 10 10 re f"),
        "",
        "",
        &[],
    );
    let (mut engine, mut session) = session_with(bytes);
    let found = op(
        &mut engine,
        &mut session,
        json!({"op": "redact-search", "text": "ZQ"}),
    );
    assert_eq!(found["added"], 100, "{found}");
    let (preview, peak, took) =
        measure(|| op(&mut engine, &mut session, json!({"op": "redact-preview"})));
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(preview["shared"], json!([]), "{preview}");
    eprintln!("p46 preview: {} MB peak, {took:?}", peak / MB);
    assert!(peak < 160 * MB, "preview peak {} MB", peak / MB);
    let ((reply, out), peak, took) = measure(|| apply(&mut engine, &mut session));
    eprintln!("p46 apply: {reply}, {} MB peak, {took:?}", peak / MB);
    assert!(peak < 400 * MB, "apply peak {} MB", peak / MB);
    if reply["ok"] == true {
        assert!(!holds(&everything(&out), "ZQ"));
    } else {
        assert_eq!(reply["reason"], "too-large", "{reply}");
    }
}

#[test]
fn clips_in_a_form_under_the_mark_cap_stay_bounded() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Review probe p40 at 2,000 clips under 2,000 marks (the document's
    // cap): 4 million (clip, rect) entries before (about 700 MB of RSS;
    // 4,000 clips reached 1,345 MB).
    let clips = "0 0 612 792 re W n\n".repeat(2_000);
    let bytes = form_page(
        "/F Do",
        &format!("{clips}BT /F1 12 Tf 100 700 Td (HELLO) Tj ET"),
        "",
        "",
        &[],
    );
    let (mut engine, mut session) = session_with(bytes);
    for i in 0..2000 {
        let (x, y) = (20 + (i % 50) * 11, 20 + (i / 50) * 11);
        let reply = op(
            &mut engine,
            &mut session,
            json!({"op": "redact-add", "page": 0, "rect": [x, y, x + 10, y + 10]}),
        );
        assert_eq!(reply["ok"], true, "{reply}");
    }
    let (preview, peak, took) =
        measure(|| op(&mut engine, &mut session, json!({"op": "redact-preview"})));
    eprintln!("p40 preview: {} MB peak, {took:?}", peak / MB);
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(preview["marks"], 2000);
    assert!(peak < 160 * MB, "preview peak {} MB", peak / MB);
    let ((reply, out), peak, took) = measure(|| apply(&mut engine, &mut session));
    eprintln!("p40 apply: {reply}, {} MB peak, {took:?}", peak / MB);
    assert!(peak < 600 * MB, "apply peak {} MB", peak / MB);
    if reply["ok"] == true {
        assert!(!holds(&everything(&out), "HELLO"));
    } else {
        assert_eq!(reply["reason"], "too-large", "{reply}");
    }
}

/// 256 Redact annotations sharing one /QuadPoints array of 1,024 copies of
/// one quad (262,144 rects, the document's cap) over MARKEDWORD, a Form
/// with `clips` page-size clips (`q re W n Q`) and FORMTEXT, and `image`
/// (an image object drawn under the quad when given).
fn file_quads(clips: usize, image: bool) -> Vec<u8> {
    let quads = vec!["100 720 300 720 100 680 300 680"; 1024].join(" ");
    let annots: String = (0..256).map(|i| format!("{} 0 R ", 9 + i)).collect();
    let mut more = vec![
        format!("[{quads}]"),
        stream(
            "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8",
            "@P`p",
        ),
    ];
    more.extend((0..256).map(|_| {
        "<< /Type /Annot /Subtype /Redact /Rect [100 680 300 720] /QuadPoints 7 0 R /P 3 0 R >>"
            .to_string()
    }));
    let picture = match image {
        true => "q 200 0 0 40 100 680 cm /Im Do Q ",
        false => "",
    };
    form_page(
        &format!("/F Do {picture}BT /F1 12 Tf 120 700 Td (MARKEDWORD) Tj ET"),
        &format!(
            "{}BT /F1 12 Tf 100 400 Td (FORMTEXT) Tj ET",
            "q 0 0 612 792 re W n Q\n".repeat(clips)
        ),
        "/Im 8 0 R",
        &format!("/Annots [{annots}]"),
        &more,
    )
}

#[test]
fn file_borne_rects_by_the_hundred_thousand_are_answered_once() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Review probe p41: 262,144 copies of one quad from the file, a Form
    // whose part work falls back to its box (8 clips), and an image under
    // the quad. Before: 74-89 s of preview (every query cloned and sorted
    // the 262,144 rects).
    for (clips, image) in [(10, false), (8, true)] {
        let (mut engine, mut session) = session_with(file_quads(clips, image));
        let (preview, peak, took) =
            measure(|| op(&mut engine, &mut session, json!({"op": "redact-preview"})));
        eprintln!(
            "p41 ({clips} clips, image {image}) preview: {} MB peak, {took:?}",
            peak / MB
        );
        assert_eq!(preview["ok"], true, "{preview}");
        assert_eq!(preview["marks"], 256);
        assert_eq!(preview["shared"], json!([]));
        assert!(took < Duration::from_secs(10), "{took:?}");
        assert!(peak < 200 * MB, "preview peak {} MB", peak / MB);
        let ((reply, out), peak, took) = measure(|| apply(&mut engine, &mut session));
        eprintln!(
            "p41 ({clips} clips, image {image}) apply: {reply}, {} MB peak, {took:?}",
            peak / MB
        );
        assert_eq!(reply["ok"], true, "{reply}");
        assert!(took < Duration::from_secs(30), "{took:?}");
        let all = everything(&out);
        assert!(!holds(&all, "MARKEDWORD"));
        assert!(holds(&all, "FORMTEXT"));
    }
}

/// 256 Redact annotations sharing one /QuadPoints array of 1,024 quads,
/// `quad(i)` (y-up page space), over a page of text; the area under the
/// quads is drawn.
fn page_size_quads(quad: impl Fn(usize) -> [f64; 4]) -> Vec<u8> {
    let quads: String = (0..1024)
        .map(|i| {
            let [x0, y0, x1, y1] = quad(i);
            format!("{x0} {y1} {x1} {y1} {x0} {y0} {x1} {y0} ")
        })
        .collect();
    let annots: String = (0..256).map(|i| format!("{} 0 R ", 8 + i)).collect();
    let mut more = vec![format!("[{quads}]")];
    more.extend((0..256).map(|_| {
        "<< /Type /Annot /Subtype /Redact /Rect [0 0 612 792] /QuadPoints 7 0 R /P 3 0 R >>"
            .to_string()
    }));
    form_page(
        "0.5 g 0 0 612 792 re f 0 g BT /F1 12 Tf 100 700 Td (PAGEWORD) Tj ET",
        "",
        "",
        &format!("/Annots [{annots}]"),
        &more,
    )
}

/// The i-th quad of a /QuadPoints array.
type QuadAt = Box<dyn Fn(usize) -> [f64; 4]>;

#[test]
fn page_size_quads_are_checked_once_each_row_by_row() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // 262,144 copies of one page-size quad (the review's 290 s, 1,287 MB
    // pixel check), then 1,024 distinct page-size quads under 256 marks
    // each (the check walked every pixel of each).
    let cases: [(&str, QuadAt); 2] = [
        ("identical", Box::new(|_| [0.0, 0.0, 612.0, 792.0])),
        (
            "distinct",
            Box::new(|i| {
                let (dx, dy) = ((i % 32) as f64, (i / 32) as f64);
                [dx, dy, 612.0 - dx, 792.0 - dy]
            }),
        ),
    ];
    for (name, quad) in cases {
        let (mut engine, mut session) = session_with(page_size_quads(quad));
        let ((reply, out), peak, took) = measure(|| apply(&mut engine, &mut session));
        eprintln!(
            "{name} page-size quads: apply {} {}, {} MB peak, {took:?}",
            reply["ok"],
            reply["verified"],
            peak / MB
        );
        assert_eq!(reply["ok"], true, "{name}: {reply}");
        assert_eq!(reply["verified"]["pixels"], 256);
        assert!(took < Duration::from_secs(20), "{name}: {took:?}");
        assert!(peak < 300 * MB, "{name}: {} MB", peak / MB);
        assert!(!holds(&everything(&out), "PAGEWORD"));
    }
}

#[test]
fn the_pixel_check_answers_as_a_walk_over_every_pixel_would() {
    use tandem_pdf_engine::app::redact::marks::Mark;
    use tandem_pdf_engine::app::redact::verify;
    use tandem_pdf_engine::view::{RenderOptions, ViewDocument};
    // Boxes of several colours, a thin line and text: marks at random,
    // some repeated, some off the page or without area, compared with the
    // check as it was (every pixel of every rect).
    let bytes = form_page(
        "0 g 50 50 300 300 re f 1 0 0 rg 100 100 20 20 re f 0 0 1 rg 400 400 150 300 re f \
         0.2 g 0 600 612 1 re f 0 g BT /F1 30 Tf 60 700 Td (Hello) Tj ET",
        "",
        "",
        "",
        &[],
    );
    let view = ViewDocument::open(bytes, "").unwrap();
    let image = view.render(0, 1.0, RenderOptions::default()).unwrap();
    let (w, h) = (image.width as usize, image.height as usize);
    let walked = |r: [f64; 4]| {
        let clamp = |v: f64, max: usize| (v.max(0.0) as usize).min(max);
        let (x0, x1) = (clamp(r[0].floor() + 2.0, w), clamp(r[2].ceil() - 2.0, w));
        let (y0, y1) = (clamp(r[1].floor() + 2.0, h), clamp(r[3].ceil() - 2.0, h));
        let mut first: Option<&[u8]> = None;
        for y in y0..y1 {
            for x in x0..x1 {
                let at = (y * w + x) * 4;
                let rgb = &image.pixels[at..at + 3];
                match first {
                    None => first = Some(rgb),
                    Some(f) if f != rgb => return false,
                    Some(_) => {}
                }
            }
        }
        true
    };
    let mut seed = 99u64;
    let mut next = |scale: f64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as f64 / f64::from(1u32 << 31) * scale
    };
    let mut marks = Vec::new();
    for i in 0..600 {
        let size = [4.0, 30.0, 300.0, 900.0][i % 4];
        let (x, y) = (next(700.0) - 50.0, next(900.0) - 50.0);
        let rect = match i % 9 {
            // Inside the black box (page space is y-down: 792 - y).
            0 => {
                let (x, y) = (60.0 + next(200.0), 452.0 + next(200.0));
                [x, y, x + 10.0 + next(30.0), y + 10.0 + next(30.0)]
            }
            1 => [x, y, x, y + size],
            _ => [x, y, x + next(size), y + next(size)],
        };
        let quads = match i % 5 {
            0 => Vec::new(),
            _ => {
                // Two quads, the second sometimes the first again.
                let q = |r: [f64; 4]| [(r[0], r[1]), (r[2], r[1]), (r[0], r[3]), (r[2], r[3])];
                let other = [rect[0] + 3.0, rect[1] + 3.0, rect[2] + 5.0, rect[3] + 2.0];
                vec![q(rect), q(if i % 2 == 0 { rect } else { other })]
            }
        };
        marks.push(Mark {
            page: 0,
            id: None,
            rect,
            quads,
            search: None,
        });
    }
    let failed = verify::pixels(&view, &marks);
    let mut uniform = 0;
    for (index, mark) in marks.iter().enumerate() {
        let expected = mark.rects().into_iter().all(walked);
        uniform += usize::from(expected);
        assert_eq!(
            failed.contains(&index),
            !expected,
            "mark {index}: {:?} {:?}",
            mark.rect,
            mark.quads
        );
    }
    // Both answers are well represented.
    assert!(uniform > 100 && uniform < 500, "{uniform}");
}

#[test]
fn the_rect_index_answers_as_a_scan_would_for_odd_and_repeated_rects() {
    use tandem_pdf_engine::app::redact::marks::Mark;
    use tandem_pdf_engine::app::redact::region::{self, Regions};
    let mut seed = 7u64;
    let mut next = |scale: f64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as f64 / f64::from(1u32 << 31) * scale
    };
    let mut rects: Vec<[f64; 4]> = Vec::new();
    for i in 0..2000 {
        let (x, y) = (next(612.0), next(792.0));
        let size = [2.0, 20.0, 200.0, 2000.0][i % 4];
        let r = match i % 23 {
            // Infinite, inverted, NaN and huge rects, and repeats.
            0 => [f64::NEG_INFINITY, y, x, y + 5.0],
            1 => [x, y, f64::INFINITY, f64::INFINITY],
            2 => [x + 10.0, y, x, y + 10.0],
            3 => [f64::NAN, y, x, y + 1.0],
            4 => [-1e300, -1e300, 1e300, 1e300],
            5 | 6 => rects.last().copied().unwrap_or([0.0; 4]),
            _ => [x, y, x + next(size), y + next(size)],
        };
        rects.push(r);
    }
    let marks: Vec<Mark> = (rects.iter())
        .map(|&rect| Mark {
            page: 0,
            id: None,
            rect,
            quads: Vec::new(),
            search: None,
        })
        .collect();
    let regions = Regions::of(&marks);
    let order = |a: &[f64; 4], b: &[f64; 4]| a.map(f64::to_bits).cmp(&b.map(f64::to_bits));
    let mut distinct = rects.clone();
    distinct.sort_by(order);
    distinct.dedup_by(|a, b| order(a, b).is_eq());
    assert_eq!(regions.on(0).len(), distinct.len());
    for i in 0..4000 {
        let (x, y) = (next(800.0) - 100.0, next(1000.0) - 100.0);
        let size = [0.0, 3.0, 40.0, 700.0][i % 4];
        let mut q = [x, y, x + next(size), y + next(size)];
        if i % 11 == 0 {
            q = [-1e9, y, -1e9 + 1.0, y + 1.0];
        }
        if i % 13 == 0 {
            let r = rects[i % rects.len()];
            q = [r[2], r[1], r[2] + 3.0, r[1] + 1.0];
        }
        let scan = distinct.iter().any(|r| region::intersects(r, &q));
        assert_eq!(regions.touches(0, &q), scan, "{q:?}");
        let inside = distinct.iter().any(|r| {
            r[0] - 0.01 <= q[0] && q[2] <= r[2] + 0.01 && r[1] - 0.01 <= q[1] && q[3] <= r[3] + 0.01
        });
        assert_eq!(regions.contains(0, &q), inside, "{q:?}");
        let meets =
            (distinct.iter()).any(|r| q[0] <= r[2] && q[2] >= r[0] && q[1] <= r[3] && q[3] >= r[1]);
        assert_eq!(regions.meets(0, &q), meets, "{q:?}");
        let mut touching: Vec<[f64; 4]> = (distinct.iter())
            .copied()
            .filter(|r| region::intersects(r, &q))
            .collect();
        let mut found = regions.touching(0, &q);
        touching.sort_by(order);
        found.sort_by(order);
        assert_eq!(
            found
                .iter()
                .map(|r| r.map(f64::to_bits))
                .collect::<Vec<_>>(),
            touching
                .iter()
                .map(|r| r.map(f64::to_bits))
                .collect::<Vec<_>>(),
            "{q:?}"
        );
    }
}

#[test]
fn search_past_the_views_resource_budget_refuses_scan_budget() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // 1,100 pages sharing one /Resources of 4,000 fonts, plain and
    // layered: the search refused with an untagged error (review: it must
    // be the recorded `scan-budget`).
    let pages = 1100;
    let fonts: String = (0..4000).map(|i| format!("/F{i} 5 0 R ")).collect();
    let build = |catalog: &str| {
        let mut objects = vec![
            format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>"),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {pages} >>",
                (0..pages)
                    .map(|i| format!("{} 0 R ", 8 + i))
                    .collect::<String>()
            ),
            format!("<< /Font << {fonts} >> >>"),
            stream("", "BT /F0 12 Tf 100 700 Td (SECRETBIG text) Tj ET"),
            HELV.to_string(),
            "<< /Type /OCG /Name (Layer) >>".to_string(),
            "null".to_string(),
        ];
        objects.extend((0..pages).map(|_| {
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 3 0 R /Contents 4 0 R >>"
                .to_string()
        }));
        classic(&objects)
    };
    let layered = "/OCProperties << /OCGs [6 0 R] /D << /Order [6 0 R] >> >>";
    for catalog in ["", layered] {
        let (mut engine, mut session) = session_with(build(catalog));
        let reply = op(
            &mut engine,
            &mut session,
            json!({"op": "redact-search", "text": "SECRETBIG"}),
        );
        assert_eq!(reply["ok"], false, "{catalog}: {reply}");
        assert_eq!(reply["reason"], "scan-budget", "{catalog}: {reply}");
        let error = reply["error"].as_str().unwrap();
        assert!(
            error.starts_with("redact-search: ") && error.contains("larger than the view's limit"),
            "{catalog}: {error}"
        );
    }
}
