// SPDX-License-Identifier: MIT
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
            by_area: quads.is_empty(),
            quads,
            search: None,
        });
    }
    let failed = verify::pixels(&view, &marks).expect("the page draws at full scale");
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
            by_area: true,
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

/// Two pages placing the Form `form_dict` (object 8) holding `clips`
/// page-size clips, LEFTPART and OFFPAGE: at `300 600 cm` on page 1, where
/// OFFPAGE lies past the right edge, and at `50 300 cm` on page 2, where
/// it shows.
fn placed_twice(form_dict: &str, clips: usize) -> Vec<u8> {
    let form = format!(
        "{}BT /F1 20 Tf 10 40 Td (LEFTPART) Tj ET BT /F1 20 Tf 330 40 Td (OFFPAGE) Tj ET",
        "q 0 0 612 792 re W n Q\n".repeat(clips)
    );
    let page = |contents: usize| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> /XObject << /F 8 0 R >> >> /Contents {contents} 0 R >>"
        )
    };
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        page(5),
        page(7),
        stream(
            "",
            "q 1 0 0 1 300 600 cm /F Do Q BT /F1 12 Tf 72 300 Td (BODYTEXT) Tj ET",
        ),
        HELV.to_string(),
        stream(
            "",
            "q 1 0 0 1 50 300 cm /F Do Q BT /F1 12 Tf 72 200 Td (PAGETWO) Tj ET",
        ),
        stream(
            &format!(
                "/Type /XObject /Subtype /Form {form_dict} /Resources << /Font << /F1 6 0 R >> >>"
            ),
            &form,
        ),
    ])
}

#[test]
fn a_form_without_a_bbox_keeps_text_past_the_edge_shown_elsewhere_when_its_box_decides() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Re-review of round 3 (probe r3-bboxless): past the part budget (5,000
    // clips), a Form without a usable /BBox was decided by the walk's
    // stand-in box (the clip or the page), which stops at the page edge. The
    // mark on page 1's right edge takes OFFPAGE, which page 2 shows; preview
    // said `shared` [] and apply removed OFFPAGE from page 2.
    for dict in ["", "/BBox [0 0 420]", "/BBox [0 0 612 792]"] {
        for clips in [0, 5000] {
            let (mut engine, mut session) = session_with(placed_twice(dict, clips));
            for (page, rect) in [(0, [550, 120, 612, 170]), (1, [300, 420, 362, 470])] {
                let reply = op(
                    &mut engine,
                    &mut session,
                    json!({"op": "redact-add", "page": page, "rect": rect}),
                );
                assert_eq!(reply["ok"], true, "{reply}");
            }
            let preview = op(&mut engine, &mut session, json!({"op": "redact-preview"}));
            assert_eq!(preview["ok"], true, "{dict} {clips}: {preview}");
            assert_ne!(preview["shared"], json!([]), "{dict} {clips}: {preview}");
            // R3: the placement on page 1 gets its own copy; page 2 still
            // shows OFFPAGE (before R3 apply refused `shared-content`).
            let (reply, bytes) = apply(&mut engine, &mut session);
            assert_eq!(reply["ok"], true, "{dict} {clips}: {reply}");
            let text = |bytes: Vec<u8>| {
                let view = tandem_pdf_engine::view::ViewDocument::open(bytes, "").unwrap();
                view.structured_text(1).unwrap().as_text()
            };
            assert_eq!(
                text(placed_twice(dict, clips)),
                text(bytes.clone()),
                "{dict} {clips}"
            );
            // The original Form, which page 2 draws, keeps OFFPAGE.
            assert!(holds(&everything(&bytes), "OFFPAGE"), "{dict} {clips}");
        }
    }
}

#[test]
fn a_shading_in_a_form_is_compared_where_it_paints_at_every_placement() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Found auditing the case above: a shading's part was cut to the walk's
    // bounds, the clip at that placement (the page without one), while apply
    // cuts it by each rect's whole reach. The Form (no /BBox) paints an axial
    // shading over all of page 2; the mark on page 1's right edge cut it past
    // that edge, so page 2 lost a band right of its own mark.
    let shading = "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 600 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>";
    for (dict, clips) in [("", 0), ("", 5000), ("/BBox [0 0 420]", 0)] {
        let form = format!("{}/Sh sh", "q 0 0 612 792 re W n Q\n".repeat(clips));
        let page = |contents: usize| {
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /F 8 0 R >> >> /Contents {contents} 0 R >>"
            )
        };
        let bytes = classic(&[
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
            page(5),
            page(7),
            stream("", "q 1 0 0 1 300 600 cm /F Do Q"),
            "null".to_string(),
            stream("", "q 1 0 0 1 50 300 cm /F Do Q"),
            stream(
                &format!(
                    "/Type /XObject /Subtype /Form {dict} /Resources << /Shading << /Sh {shading} >> >>"
                ),
                &form,
            ),
        ]);
        let (mut engine, mut session) = session_with(bytes);
        for (page, rect) in [(0, [550, 120, 612, 170]), (1, [300, 420, 362, 470])] {
            let reply = op(
                &mut engine,
                &mut session,
                json!({"op": "redact-add", "page": page, "rect": rect}),
            );
            assert_eq!(reply["ok"], true, "{reply}");
        }
        let preview = op(&mut engine, &mut session, json!({"op": "redact-preview"}));
        assert_eq!(preview["ok"], true, "{dict} {clips}: {preview}");
        assert_ne!(preview["shared"], json!([]), "{dict} {clips}: {preview}");
        // R3: page 1's placement is copied; page 2 keeps its shading right
        // of its own mark (before R3 apply refused `shared-content`).
        let source = {
            let request = json!({"id": 2, "op": "save", "entropy": {"$buffer": 0}});
            engine
                .dispatch(&mut session, request, vec![vec![1u8; 32]])
                .1
                .remove(0)
        };
        let (reply, bytes) = apply(&mut engine, &mut session);
        assert_eq!(reply["ok"], true, "{dict} {clips}: {reply}");
        let pixel = |bytes: Vec<u8>| {
            let view = tandem_pdf_engine::view::ViewDocument::open(bytes, "").unwrap();
            let image = view
                .render(1, 1.0, tandem_pdf_engine::view::RenderOptions::default())
                .unwrap();
            let at = (445 * image.width as usize + 400) * 4;
            image.pixels[at..at + 3].to_vec()
        };
        assert_eq!(pixel(source), pixel(bytes), "{dict} {clips}");
    }
}

#[test]
fn page_size_clips_around_every_run_share_their_regions() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Re-review of round 3 (probe r1-clipruns): every text run in its own
    // page-size clip, a search for a common word. Each clip held its own
    // copy of every hit's region, so 3 pages of 600 runs (1.08 million
    // regions) passed the planned-region cap and refused `too-large`, where
    // round 2 applied. The clips on a page share one list now.
    for (runs, pages) in [(600, 3), (1100, 1)] {
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            String::new(),
            HELV.to_string(),
        ];
        let mut kids = Vec::new();
        for _ in 0..pages {
            let content: Vec<String> = (0..runs)
                .map(|i| {
                    let (x, y) = (
                        20 + (i % 10) * 58,
                        780.0 - (i / 10) as f64 * (760.0 / (runs / 10) as f64),
                    );
                    format!("q 0 0 612 792 re W n BT /F1 5 Tf {x} {y:.2} Td (the w{i}) Tj ET Q")
                })
                .collect();
            let n = objects.len() + 1;
            kids.push(format!("{n} 0 R"));
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
                n + 1
            ));
            objects.push(stream("", &content.join("\n")));
        }
        objects[1] = format!(
            "<< /Type /Pages /Kids [{}] /Count {pages} >>",
            kids.join(" ")
        );
        let (mut engine, mut session) = session_with(classic(&objects));
        let found = op(
            &mut engine,
            &mut session,
            json!({"op": "redact-search", "text": "the"}),
        );
        assert_eq!(found["added"], runs * pages, "{found}");
        let ((reply, out), peak, took) = measure(|| apply(&mut engine, &mut session));
        eprintln!(
            "{runs} runs x {pages} pages: apply {} {}, {} MB peak, {took:?}",
            reply["ok"],
            reply["error"],
            peak / MB
        );
        assert_eq!(reply["ok"], true, "{reply}");
        let view = tandem_pdf_engine::view::ViewDocument::open(out, "").unwrap();
        for page in 0..pages {
            let text = view.structured_text(page).unwrap().as_text();
            assert!(!text.contains("the"), "{text}");
            assert!(text.contains("w599") || text.contains("w1099"), "{text}");
        }
        assert!(peak < 400 * MB, "{} MB", peak / MB);
    }
}

/// 4,096 distinct tiny file-borne rects clustered in one index cell (and
/// two at the page corners), and `glyphs` zero-width glyphs (`0 Tz`) in one
/// run in that cell under none of them, on the page or in a Form.
fn crowded_cell(glyphs: usize, in_form: bool) -> Vec<u8> {
    let mut seed = 1u64;
    let mut next = || {
        seed = (seed * 1_103_515_245 + 12_345) % 2_147_483_648;
        seed as f64 / 2_147_483_648.0
    };
    let mut quads = vec![[0.0, 0.0, 1.0, 1.0], [611.0, 791.0, 612.0, 792.0]];
    for _ in 0..4096 {
        let (x, y) = (100.0 + next() * 10.0, 100.0 + next() * 10.0);
        quads.push([x, y, x + 0.01, y + 0.01]);
    }
    let run = format!("BT /F1 1 Tf 0 Tz 112 105 Td ({}) Tj ET", "x".repeat(glyphs));
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        String::new(),
        stream("", if in_form { "/Fm Do" } else { &run }),
        HELV.to_string(),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>",
            &run,
        ),
    ];
    let mut annots = Vec::new();
    for chunk in quads.chunks(1024) {
        let points: Vec<String> = (chunk.iter())
            .map(|[x0, y0, x1, y1]| {
                format!("{x0:.4} {y1:.4} {x1:.4} {y1:.4} {x0:.4} {y0:.4} {x1:.4} {y0:.4}")
            })
            .collect();
        annots.push(format!("{} 0 R", objects.len() + 1));
        objects.push(format!(
            "<< /Type /Annot /Subtype /Redact /Rect [0 0 612 792] /QuadPoints [{}] >>",
            points.join(" ")
        ));
    }
    objects[2] = format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /XObject << /Fm 6 0 R >> >> /Contents 4 0 R /Annots [{}] >>",
        annots.join(" ")
    );
    classic(&objects)
}

#[test]
fn glyph_queries_past_their_budget_refuse() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Re-reviews of round 3 (probes r2-cluster, r2c): every glyph's query
    // tests the 4,096 rects of its cell. 100,000 glyphs made 409 million
    // tests (3.4 s) charged to nothing; then, with the budget checked only
    // between draws, one run of 1,000,000 glyphs took 25.3 s to refuse in
    // apply and 13.7 s in a Form's preview. Checked per glyph now.
    for (glyphs, in_form) in [(100_000, false), (1_000_000, false), (1_000_000, true)] {
        let (mut engine, mut session) = session_with(crowded_cell(glyphs, in_form));
        let (preview, _, took) =
            measure(|| op(&mut engine, &mut session, json!({"op": "redact-preview"})));
        eprintln!(
            "cluster {glyphs} (form {in_form}): preview {} {} {took:?}",
            preview["ok"], preview["reason"]
        );
        assert!(took < Duration::from_secs(6), "{took:?}");
        if in_form {
            assert_eq!(preview["reason"], "scan-budget", "{preview}");
        }
        let ((reply, _), _, took) = measure(|| apply(&mut engine, &mut session));
        eprintln!(
            "cluster {glyphs} (form {in_form}): apply {} {took:?}",
            reply["reason"]
        );
        assert_eq!(reply["reason"], "scan-budget", "{reply}");
        assert!(took < Duration::from_secs(8), "{took:?}");
    }
}

/// One page of `fills` page-size fills, each under its own matrix, and 2,000
/// distinct file-borne quads, zero-width (`area` false) or 0.1 pt wide.
fn fills_under_quads(fills: usize, area: bool) -> Vec<u8> {
    let quads: Vec<String> = (0..2000)
        .map(|i| {
            let x = 100.0 + f64::from(i) * 0.2;
            let x1 = if area { x + 0.1 } else { x };
            format!("{x:.2} 401 {x1:.2} 401 {x:.2} 400 {x1:.2} 400")
        })
        .collect();
    let content: Vec<String> = (0..fills)
        .map(|i| {
            format!(
                "q 1 0 0 1 {:.3} 0 cm 0.5 g 0 0 612 792 re f Q",
                i as f64 * 0.001
            )
        })
        .collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [5 0 R 6 0 R] >>"
            .to_string(),
        stream("", &content.join("\n")),
    ];
    for chunk in quads.chunks(1000) {
        objects.push(format!(
            "<< /Type /Annot /Subtype /Redact /Rect [0 0 612 792] /QuadPoints [{}] >>",
            chunk.join(" ")
        ));
    }
    classic(&objects)
}

#[test]
fn region_list_keys_are_charged_and_empty_lists_not_kept() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Second re-review of round 3 (probe r6w): each fill, under its own
    // matrix, kept a key of the 2,000 rects it touches (32 bytes each) and
    // charged only its regions, none for zero-width rects: about 62 KB a
    // fill, 1,493 MB at 24,000 fills. An empty list is no longer kept, and a
    // kept key's rects are charged: 2,000 0.1-pt quads under 2,000 fills
    // refuse at the cap (4,000 charged per fill) instead of growing.
    let mut grew = Vec::new();
    for fills in [500, 2000] {
        let (mut engine, mut session) = session_with(fills_under_quads(fills, false));
        let ((reply, _), peak, took) = measure(|| apply(&mut engine, &mut session));
        eprintln!(
            "{fills} fills, zero-width quads: apply {} {} MB peak, {took:?}",
            reply["ok"],
            peak / MB
        );
        assert_eq!(reply["ok"], true, "{reply}");
        grew.push(peak);
    }
    // 1,500 more fills: well under 62 KB a fill (93 MB).
    assert!(grew[1].saturating_sub(grew[0]) < 32 * MB, "{grew:?}");
    // Past that, the rects each fill's queries test reach the query budget
    // (`scan-budget`) before memory grows: 24,000 fills refuse.
    let (mut engine, mut session) = session_with(fills_under_quads(24_000, false));
    let ((reply, _), peak, took) = measure(|| apply(&mut engine, &mut session));
    eprintln!(
        "24000 fills, zero-width quads: apply {}, {} MB peak, {took:?}",
        reply["reason"],
        peak / MB
    );
    assert_eq!(reply["reason"], "scan-budget", "{reply}");
    assert!(peak < 200 * MB, "{} MB", peak / MB);
    let (mut engine, mut session) = session_with(fills_under_quads(2000, true));
    let ((reply, _), peak, took) = measure(|| apply(&mut engine, &mut session));
    eprintln!(
        "2000 fills, 0.1-pt quads: apply {}, {} MB peak, {took:?}",
        reply["reason"],
        peak / MB
    );
    assert_eq!(reply["reason"], "too-large", "{reply}");
    assert!(peak < 200 * MB, "{} MB", peak / MB);
}
