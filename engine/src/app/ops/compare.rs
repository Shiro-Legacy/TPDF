// SPDX-License-Identifier: MIT
//! The compare slot (engine.ts `compare-open`, `compare-close`,
//! `compare-render`, `compare-text`): a second document opened beside the
//! session's, view-only. It is never journaled, needs no main document
//! except for `compare-text`, survives `open` and `close` (unless an `open`
//! would take the pair past `MAX_PDF_BYTES`), and a refused `compare-open`
//! keeps the slot it would have replaced.
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value, json};

use super::view::render_page;
use crate::app::request;
use crate::app::{
    AppDocument, AppError, AppSession, Buffers, LoadError, MAX_PDF_BYTES, info, reply,
};
use crate::view::ViewDocument;

/// engine.ts `MAX_COMPARE_LINES`: the most lines `compare-text` diffs per page.
pub const MAX_COMPARE_LINES: usize = 2000;

/// Numbers every compare document opened by this process, so a test
/// harness can tell a replaced slot from the one it held.
static SERIAL: AtomicU64 = AtomicU64::new(0);

fn plain(message: impl Into<String>) -> AppError {
    AppError::plain(message)
}

fn tagged(reason: &str, message: impl Into<String>) -> AppError {
    AppError::reason(reason, message)
}

/// The view-only second document (engine.ts `Session.compare`).
#[derive(Debug)]
pub struct CompareDocument {
    doc: AppDocument,
    name: String,
    source_bytes: usize,
    serial: u64,
}

impl CompareDocument {
    /// `bytes` opened as `compare-open` opens them (the tolerant load, no
    /// password), named `name`. compare-open's refusals (encrypted, no
    /// pages, the size of the pair) are the op's, not this constructor's.
    pub fn open(name: impl Into<String>, bytes: Vec<u8>) -> Result<Self, LoadError> {
        let source_bytes = bytes.len();
        let doc = AppDocument::load(Arc::new(bytes), b"")?;
        Ok(Self {
            doc,
            name: name.into(),
            source_bytes,
            serial: SERIAL.fetch_add(1, Ordering::Relaxed).wrapping_add(1),
        })
    }

    /// engine.ts `compare.sourceBytes`: the size of the file as given.
    pub fn source_bytes(&self) -> usize {
        self.source_bytes
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn page_count(&self) -> usize {
        self.doc.page_count()
    }

    /// The bytes the document was read from (rebuilt ones after a repair),
    /// for inspection by an independent reader.
    pub fn bytes(&self) -> &[u8] {
        self.doc.input()
    }

    /// The slot's fields for test inspection; `serial` changes with every
    /// successful `compare-open`.
    pub fn debug_json(&self) -> Value {
        json!({"name": self.name, "pageCount": self.doc.page_count(),
            "sourceBytes": self.source_bytes, "serial": self.serial})
    }
}

/// The open compare document, refused as engine.ts refuses an empty slot.
fn opened<'s>(session: &'s mut AppSession, op: &str) -> Result<&'s mut CompareDocument, AppError> {
    session.compare.as_mut().ok_or_else(|| {
        tagged(
            "no-compare-document",
            format!("{op}: no compare document is open"),
        )
    })
}

/// engine.ts `compare-open`: the bytes opened as `open` opens them, refused
/// when encrypted or empty, or when they and the main document's source
/// together pass `MAX_PDF_BYTES` (checked before opening).
pub fn open(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let name = match request::string(request, "name") {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => return Err(plain("compare-open: missing document name")),
    };
    let Some(bytes) = buffers
        .take(request.get("pdf"))
        .filter(|bytes| bytes.len() >= 5)
    else {
        return Err(tagged("invalid", "compare-open: not a PDF (too few bytes)"));
    };
    if bytes.len().saturating_add(session.source_bytes) > MAX_PDF_BYTES {
        return Err(tagged(
            "too-large",
            format!("compare-open: the two files together exceed the {MAX_PDF_BYTES} byte ceiling"),
        ));
    }
    let compare = CompareDocument::open(name.clone(), bytes).map_err(|error| {
        tagged(
            "invalid",
            format!("compare-open: the file could not be read ({error})"),
        )
    })?;
    let doc = &compare.doc;
    let info = doc.info(doc.is_locked());
    if info.needs_password || info.encrypted {
        return Err(tagged(
            "encrypted",
            "compare-open: an encrypted file cannot be compared",
        ));
    }
    let pages = doc.page_state();
    if pages.count() < 1 {
        return Err(tagged("invalid", "compare-open: the file has no pages"));
    }
    session.compare = Some(compare);
    let mut fields = Map::new();
    fields.insert("name".into(), name.into());
    fields.extend(
        pages
            .json()
            .into_iter()
            .filter(|(key, _)| key != "rotations"),
    );
    Ok(reply(request, Value::Object(fields)))
}

/// engine.ts `compare-close`: empties the slot; never fails.
pub fn close(session: &mut AppSession, request: &Value) -> Value {
    session.compare = None;
    reply(request, json!({}))
}

/// engine.ts `compare-render`: `render`'s drawing of a compare page, with
/// its plain `render:` errors and no `revision`.
pub fn render(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let compare = opened(session, "compare-render")?;
    let mut fields = Map::new();
    fields.insert("page".into(), request.get("page").cloned().into());
    fields.extend(render_page(|| compare.doc.view(), request, buffers)?);
    Ok(reply(request, Value::Object(fields)))
}

/// One line of a page as `compare-text` reports it (engine.ts `stextLines`):
/// the preserve-spans line and its bbox, y down, in whole page points.
#[derive(Clone, Debug, PartialEq)]
struct Line {
    text: String,
    bbox: [i64; 4],
}

impl Line {
    fn json(&self) -> Value {
        json!({"text": self.text, "bbox": self.bbox})
    }
}

fn page_lines(view: &ViewDocument, page: usize) -> Result<Vec<Line>, AppError> {
    let text =
        (view.structured_text(page)).map_err(|error| plain(format!("compare-text: {error}")))?;
    let lines = text.line_summaries().into_iter().map(|line| {
        let (x, y) = (i64::from(line.x), i64::from(line.top));
        Line {
            bbox: [x, y, x + i64::from(line.w), y + i64::from(line.h)],
            text: line.text,
        }
    });
    Ok(lines.collect())
}

/// engine.ts `checkPage` on the compare document.
fn compare_page(value: Option<&Value>, count: usize) -> Result<usize, AppError> {
    value
        .and_then(request::as_int)
        .and_then(|page| usize::try_from(page).ok())
        .filter(|&page| page < count)
        .ok_or_else(|| {
            tagged(
                "invalid",
                format!(
                    "compare-text: page must be an integer in 0..{}",
                    count.saturating_sub(1)
                ),
            )
        })
}

/// engine.ts `compare-text`: the line diff of main page `pageA` against
/// compare page `pageB`, when the main document permits copying.
pub fn text(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let page_a = {
        let Some(doc) = &session.doc else {
            return Err(tagged("no-document", "compare-text: no document open"));
        };
        if session.locked {
            return Err(tagged(
                "read-only",
                "compare-text: document is encrypted, a password is required",
            ));
        }
        if !info::permissions(doc.document()).copy {
            return Err(tagged(
                "read-only",
                "compare-text: this document does not permit copying",
            ));
        }
        opened(session, "compare-text")?;
        session.check_page(request.get("pageA"), "compare-text")?
    };
    let AppSession {
        doc: Some(doc),
        compare: Some(compare),
        revision,
        ..
    } = session
    else {
        return Err(plain("compare-text: no document open"));
    };
    let page_b = compare_page(request.get("pageB"), compare.doc.page_count())?;
    let view = doc
        .view()
        .map_err(|error| plain(format!("compare-text: {error}")))?;
    let a = page_lines(view, page_a)?;
    let view = (compare.doc.view()).map_err(|error| plain(format!("compare-text: {error}")))?;
    let b = page_lines(view, page_b)?;
    if a.len() > MAX_COMPARE_LINES || b.len() > MAX_COMPARE_LINES {
        return Err(tagged(
            "too-many",
            format!("compare-text: a page has more than {MAX_COMPARE_LINES} lines"),
        ));
    }
    let diff = diff_lines(&a, &b);
    let json = |lines: Vec<&Line>| lines.into_iter().map(Line::json).collect::<Vec<_>>();
    Ok(reply(
        request,
        json!({"pageA": page_a, "pageB": page_b, "revision": *revision, "same": diff.same,
            "removed": json(diff.removed), "added": json(diff.added), "moved": 0}),
    ))
}

/// JavaScript's `\s` (and `String.prototype.trim`) whitespace.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// engine.ts `normaliseLine`: trimmed, each whitespace run one space.
fn normalise(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split(js_space).filter(|word| !word.is_empty()) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

struct Diff<'a> {
    same: usize,
    removed: Vec<&'a Line>,
    added: Vec<&'a Line>,
}

/// engine.ts `diffLines`: a longest-common-subsequence walk over the
/// normalised lines; a moved line is one removed and one added.
fn diff_lines<'a>(a: &'a [Line], b: &'a [Line]) -> Diff<'a> {
    let ka: Vec<String> = a.iter().map(|line| normalise(&line.text)).collect();
    let kb: Vec<String> = b.iter().map(|line| normalise(&line.text)).collect();
    let (n, m) = (ka.len(), kb.len());
    let w = m + 1;
    // Suffix table: at(i, j) is the LCS length of ka[i..] and kb[j..].
    let mut table = vec![0u16; (n + 1) * w];
    let at = |table: &[u16], i: usize, j: usize| table.get(i * w + j).copied().unwrap_or(0);
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let value = if ka.get(i) == kb.get(j) {
                at(&table, i + 1, j + 1).saturating_add(1)
            } else {
                at(&table, i + 1, j).max(at(&table, i, j + 1))
            };
            if let Some(cell) = table.get_mut(i * w + j) {
                *cell = value;
            }
        }
    }
    let mut diff = Diff {
        same: 0,
        removed: Vec::new(),
        added: Vec::new(),
    };
    let (mut i, mut j) = (0, 0);
    while let (Some(line_a), Some(line_b)) = (a.get(i), b.get(j)) {
        if ka.get(i) == kb.get(j) {
            diff.same += 1;
            i += 1;
            j += 1;
        } else if at(&table, i + 1, j) >= at(&table, i, j + 1) {
            diff.removed.push(line_a);
            i += 1;
        } else {
            diff.added.push(line_b);
            j += 1;
        }
    }
    diff.removed.extend(a.iter().skip(i));
    diff.added.extend(b.iter().skip(j));
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(texts: &[&str]) -> Vec<Line> {
        (texts.iter())
            .map(|text| Line {
                text: text.to_string(),
                bbox: [0; 4],
            })
            .collect()
    }

    #[test]
    fn diff_walks_the_lcs_as_engine_ts() {
        let a = lines(&["alpha", "beta", "gamma"]);
        let b = lines(&["alpha", "gamma", "delta"]);
        let diff = diff_lines(&a, &b);
        assert_eq!(diff.same, 2);
        assert_eq!(diff.removed, [&a[1]]);
        assert_eq!(diff.added, [&b[2]]);
        // A swapped pair: one kept, the other removed and added.
        let (a, b) = (lines(&["x", "y"]), lines(&["y", "x"]));
        let diff = diff_lines(&a, &b);
        assert_eq!(
            (diff.same, diff.removed, diff.added),
            (1, vec![&a[0]], vec![&b[1]])
        );
        let empty = diff_lines(&[], &b);
        assert_eq!(
            (empty.same, empty.removed.len(), empty.added.len()),
            (0, 0, 2)
        );
    }

    #[test]
    fn lines_compare_by_normalised_text() {
        assert_eq!(normalise("  a \t\u{3000} b\u{A0}"), "a b");
        assert_eq!(normalise(" \n "), "");
        let (a, b) = (lines(&["a  b"]), lines(&["a b"]));
        assert_eq!(diff_lines(&a, &b).same, 1);
    }
}
