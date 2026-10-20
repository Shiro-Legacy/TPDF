//! The bytes of a one-page blank PDF (port of `src/blank-pdf.ts`, byte for
//! byte): built by hand so the host can write it without the engine. The page
//! has an empty /Resources and an empty content stream; every xref offset is
//! computed from the bytes.

use serde_json::Value;

/// Letter, the default size, in points.
pub const LETTER: (f64, f64) = (612.0, 792.0);
/// A4 in points.
pub const A4: (f64, f64) = (595.276, 841.89);
pub const MIN_PAGE_PT: f64 = 72.0;
pub const MAX_PAGE_PT: f64 = 14400.0;

/// `pageSizeOf` (blank-pdf.ts:11): `None` (JS `undefined`) or `"letter"` →
/// Letter, `"a4"` → A4, `[w, h]` with each side a finite number in 72…14400;
/// anything else (including `null`) → `None`, which the caller reports as an
/// unreadable `Untitled.pdf`.
pub fn page_size_of(size: Option<&Value>) -> Option<(f64, f64)> {
    let Some(size) = size else {
        return Some(LETTER);
    };
    match size {
        Value::String(s) if s == "letter" => Some(LETTER),
        Value::String(s) if s == "a4" => Some(A4),
        Value::Array(items) if items.len() == 2 => {
            let ok = |v: &Value| {
                v.as_f64()
                    .filter(|n| n.is_finite() && (MIN_PAGE_PT..=MAX_PAGE_PT).contains(n))
            };
            Some((ok(&items[0])?, ok(&items[1])?))
        }
        _ => None,
    }
}

/// `String(Math.round(v * 1000) / 1000)`.
fn num(v: f64) -> String {
    let r = (v * 1000.0 + 0.5).floor() / 1000.0;
    if r.fract() == 0.0 && r.abs() < 1e15 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

/// `blankPdf(width, height)` (blank-pdf.ts:24).
pub fn blank_pdf(width: f64, height: f64) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << >> /Contents 4 0 R >>",
            num(width),
            num(height)
        ),
        "<< /Length 0 >>\nstream\n\nendstream".to_string(),
    ];
    let mut text = String::from("%PDF-1.7\n");
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(text.len());
        text.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
    }
    let xref = text.len();
    text.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for offset in offsets {
        text.push_str(&format!("{offset:010} 00000 n \n"));
    }
    text.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    text.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::{Digest, Sha256};

    fn hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn bytes_match_blank_pdf_ts() {
        // SHA-256 of build/blank-pdf.js `blankPdf(w, h)` output (node, Oct 19 2026).
        let cases = [
            (
                612.0,
                792.0,
                "c7ea3b0ee4dee499f5c57e48470bfe7084103c41fbb03aa193bd3e5a58072a2e",
            ),
            (
                595.276,
                841.89,
                "9cf1c2d5b1cbef7d4568c943159b6930e0052b87a80e9e13c076e0311ea6f217",
            ),
            (
                72.0004,
                14400.0,
                "074d5a86bb4f9f35678f18169f1a6fd67d27bdd0f0c7ba46f3a7ce986741d186",
            ),
            (
                100.5,
                200.25,
                "33ffe51d387bcf7cfcfa69b7764ff4ca9ce206158e7c42f8f19058af33755894",
            ),
        ];
        for (w, h, want) in cases {
            assert_eq!(hex(&blank_pdf(w, h)), want, "{w}x{h}");
        }
        assert!(blank_pdf(612.0, 792.0).starts_with(b"%PDF-1.7\n1 0 obj\n"));
    }

    #[test]
    fn sizes() {
        assert_eq!(page_size_of(None), Some(LETTER));
        assert_eq!(page_size_of(Some(&json!("letter"))), Some(LETTER));
        assert_eq!(page_size_of(Some(&json!("a4"))), Some(A4));
        assert_eq!(
            page_size_of(Some(&json!([72, 14400]))),
            Some((72.0, 14400.0))
        );
        assert_eq!(page_size_of(Some(&json!([71.9, 100]))), None);
        assert_eq!(page_size_of(Some(&json!([100]))), None);
        assert_eq!(page_size_of(Some(&json!(null))), None);
        assert_eq!(page_size_of(Some(&json!("A4"))), None);
        assert_eq!(page_size_of(Some(&json!(["100", 100]))), None);
    }
}
