//! MuPDF comparison driver for the view module (password from TPDF_PDF_PASSWORD):
//!   view_probe render FILE PAGE DPI OUT.ppm   (for tools/compare-mupdf-pages.mjs)
//!   view_probe text FILE                      (JSON: per page text, spans text, line summaries)
//!   view_probe search FILE PAGE NEEDLE...     (JSON: quads per hit, truncation)
use tandem_pdf_engine::view::{MAX_SEARCH_QUADS, RenderOptions, ViewDocument};

fn json(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let password = std::env::var("TPDF_PDF_PASSWORD").unwrap_or_default();
    let open = |path: &str| -> Result<ViewDocument, Box<dyn std::error::Error>> {
        Ok(ViewDocument::open(std::fs::read(path)?, &password)?)
    };
    match args.get(1).map(String::as_str) {
        Some("render") if args.len() == 6 => {
            let image = open(&args[2])?.render(
                args[3].parse()?,
                args[4].parse::<f64>()? / 72.0,
                RenderOptions::default(),
            )?;
            let mut out = format!("P6\n{} {}\n255\n", image.width, image.height).into_bytes();
            for px in image.pixels.as_chunks::<4>().0 {
                out.extend_from_slice(&px[..3]);
            }
            std::fs::write(&args[5], out)?;
        }
        Some("text") if args.len() == 3 => {
            let doc = open(&args[2])?;
            let mut pages = Vec::new();
            for page in 0..doc.page_count() {
                let text = doc.structured_text(page)?;
                let lines: Vec<String> = text
                    .line_summaries()
                    .iter()
                    .map(|l| {
                        format!(
                            "{{\"text\":{},\"x\":{},\"y\":{},\"top\":{},\"w\":{},\"h\":{},\"size\":{},\"family\":\"{}\",\"weight\":\"{}\",\"style\":\"{}\"}}",
                            json(&l.text),
                            l.x,
                            l.y,
                            l.top,
                            l.w,
                            l.h,
                            l.size,
                            l.family,
                            l.weight,
                            l.style
                        )
                    })
                    .collect();
                pages.push(format!(
                    "{{\"text\":{},\"spans\":{},\"lines\":[{}]}}",
                    json(&text.as_text()),
                    json(&text.spans_text()),
                    lines.join(",")
                ));
            }
            println!("[{}]", pages.join(",\n"));
        }
        Some("search") if args.len() >= 5 => {
            let text = open(&args[2])?.structured_text(args[3].parse()?)?;
            let mut rows = Vec::new();
            for needle in &args[4..] {
                let result = text.search(needle, MAX_SEARCH_QUADS);
                let hits: Vec<String> = result.hits.iter().map(|hit| format!("{hit:?}")).collect();
                rows.push(format!(
                    "[{},[{}],{}]",
                    json(needle),
                    hits.join(","),
                    result.truncated
                ));
            }
            println!("[{}]", rows.join(",\n"));
        }
        _ => return Err("usage: view_probe render|text|search ...".into()),
    }
    Ok(())
}
