//! Markup from a text selection (renderer.ts 8507–8647): the page text the person selected as
//! markup quads, the three markup buttons, "Highlight all matches", the drive's `select`, and
//! the markup and text-box adds. Also holds lane 4's own state ([`Lane4State`]) and the text
//! layer's character geometry, which the selection and the Comments list both read.
#![allow(dead_code)]

use super::Session;
use super::dom_ids as ids;
use super::form_fields::journal_of;
use super::types::*;
use crate::js::{is_js_space, js_round, js_trim};
use crate::messages;
use serde_json::json;
use tandem_protocol::engine::Line;

/// The text layer's selection (the DOM `Selection` over `#text-layer`'s `<text>` nodes): from
/// `(line, offset)` to `(line, offset)`, offsets in characters, start before end. `text_key` is
/// the `paintedTextKey` the selection was made on: a rebuilt layer drops it, as the DOM did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextSelection {
    pub page: i64,
    pub start: (usize, usize),
    pub end: (usize, usize),
    pub text_key: String,
}

/// `selectionMarkup()`'s answer: markup quads (ul, ur, ll, lr; page pt, y down) and the words.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SelectionMarkup {
    pub page: i64,
    pub quads: Vec<Vec<f64>>,
    pub text: String,
}

/// A page-area drag (`startAreaDrag`'s closures, R:8133): what the rect is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AreaDragKind {
    /// Draw link (R:11126): the rect opens the link dialog.
    #[default]
    Link,
    /// Draw signature field (R:11130).
    SignatureField,
    /// The armed "Redact area" tool (R:11122).
    RedactArmed,
    /// An unarmed drag in Redact mode (R:11135).
    RedactUnarmed,
}

/// The live area drag: start point (page pt, unclamped) and the draft rect it shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AreaDrag {
    pub page: i64,
    pub start: Point,
    pub kind: AreaDragKind,
    /// The `.area-draft` box (page pt), None while hidden (under 3 CSS px).
    pub draft: Option<Vec<f64>>,
}

/// One row of the compare view's line table (R:9134).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompareRow {
    /// "Only left" | "Only right".
    pub side: String,
    pub text: String,
    /// The row's tooltip: `Page n, x0 y0 x1 y1 pt`.
    pub title: String,
}

/// What `paintCompare` put on the three canvases (RGBA): A, B, and B's red overlay.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComparePaint {
    pub a: (i64, i64, Vec<u8>),
    pub b: (i64, i64, Vec<u8>),
    pub overlay: Vec<u8>,
}

/// Lane 4's state the `Session` struct lacks (renderer 7288–9533).
#[derive(Clone, Debug, PartialEq)]
pub struct Lane4State {
    /// The DOM text selection on the page (`window.getSelection()`).
    pub selection: Option<TextSelection>,
    /// The page-area drag in progress (R:8133).
    pub area_drag: Option<AreaDrag>,
    /// `input[name=stamp-icon]:checked` in the stamp dialog (index.html checks Draft).
    pub stamp_icon: String,
    /// The compare view's line table and canvases (`compareListEl`, `compareCanvas*`).
    pub compare_rows: Vec<CompareRow>,
    pub compare_paint: Option<ComparePaint>,
}

impl Default for Lane4State {
    fn default() -> Self {
        Self {
            selection: None,
            area_drag: None,
            stamp_icon: "Draft".into(),
            compare_rows: Vec::new(),
            compare_paint: None,
        }
    }
}

/// `text.replace(/\s+/g, " ")`.
pub(crate) fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if is_js_space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// `Math.round(value * 100) / 100`.
pub(crate) fn r2(value: f64) -> f64 {
    js_round(value * 100.0) / 100.0
}

/// The ascent and descent of a text-layer glyph box, as fractions of the font size. The SVG
/// layer's `getExtentOfChar` used the browser font's metrics; these are a sans-serif's.
const ASCENT: f64 = 0.8;
const DESCENT: f64 = 0.2;

/// One character's box on the text layer (`getExtentOfChar(i)` / zoom): x, y, width, height in
/// page pt. The layer stretches each line to its width (`lengthAdjust="spacingAndGlyphs"`), so
/// the characters share the line's width evenly; `y` is the baseline.
pub(crate) fn char_box(line: &Line, index: usize, count: usize) -> [f64; 4] {
    let advance = if count > 0 {
        line.w / count as f64
    } else {
        0.0
    };
    [
        line.x + advance * index as f64,
        line.y - ASCENT * line.size,
        advance,
        (ASCENT + DESCENT) * line.size,
    ]
}

impl Session {
    /// The shown page's text layer (`#text-layer`'s `<text>` nodes): the lines of the render it
    /// was painted from, or None when no page is painted (the TS's `zoom > 0` check).
    pub(crate) fn text_layer(&self) -> Option<&[Line]> {
        let doc = self.doc.as_ref()?;
        let page = usize::try_from(self.shown_page).ok()?;
        doc.pages.get(page)?;
        let result = self.last_result.as_ref()?;
        if result.page != self.shown_page || self.shown_zoom <= 0.0 {
            return None;
        }
        Some(&result.lines)
    }

    /// UI entry: the person selected text on the page (None = the selection was cleared).
    /// `start`/`end` are `(line, char offset)` on the shown page's text layer, in either order.
    pub fn on_text_select(&mut self, range: Option<((usize, usize), (usize, usize))>) {
        self.lane4.selection = range.map(|(a, b)| {
            let (start, end) = if a <= b { (a, b) } else { (b, a) };
            TextSelection {
                page: self.shown_page,
                start,
                end,
                text_key: self.painted_text_key.clone(),
            }
        });
        self.sync_markup_buttons();
    }

    /// R:8510: the page text the person selected, as markup quads (ul, ur, ll, lr; page points,
    /// y down), or None.
    pub fn selection_markup(&self) -> Option<SelectionMarkup> {
        let selection = self.lane4.selection.as_ref()?;
        if self.doc.is_none() || self.shown_page < 0 || selection.start == selection.end {
            return None;
        }
        // `overlay.contains(range.commonAncestorContainer)`: the selection is on this layer.
        if selection.page != self.shown_page || selection.text_key != self.painted_text_key {
            return None;
        }
        let lines = self.text_layer()?;
        let mut quads = Vec::new();
        let mut words: Vec<String> = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            // `range.intersectsNode(element)`.
            if index < selection.start.0 || index > selection.end.0 {
                continue;
            }
            let chars: Vec<char> = line.text.chars().collect();
            // An empty `<text>` has no text node.
            if chars.is_empty() {
                continue;
            }
            let length = chars.len();
            let start = if index == selection.start.0 {
                selection.start.1.min(length)
            } else {
                0
            };
            let end = if index == selection.end.0 {
                selection.end.1.min(length)
            } else {
                length
            };
            if end <= start {
                continue;
            }
            let first = char_box(line, start, length);
            let last = char_box(line, end - 1, length);
            let x0 = first[0].min(last[0]);
            let x1 = (first[0] + first[2]).max(last[0] + last[2]);
            let y0 = first[1].min(last[1]);
            let y1 = (first[1] + first[3]).max(last[1] + last[3]);
            if !(x1 - x0 > 0.1 && y1 - y0 > 0.1) {
                continue;
            }
            quads.push(vec![
                r2(x0),
                r2(y0),
                r2(x1),
                r2(y0),
                r2(x0),
                r2(y1),
                r2(x1),
                r2(y1),
            ]);
            words.push(chars[start..end].iter().collect());
        }
        if quads.is_empty() {
            return None;
        }
        let text = js_trim(&collapse_spaces(&words.join(" "))).to_string();
        Some(SelectionMarkup {
            page: self.shown_page,
            quads,
            text,
        })
    }

    /// R:8543: the three markup buttons act on the selection first, else on the search match;
    /// their words say which.
    pub fn sync_markup_buttons(&mut self) {
        let can = self.doc.is_some() && self.can_annotate();
        let selected = can && self.selection_markup().is_some();
        let hit = can && self.current_hit().is_some();
        for (id, kind, _) in ids::MARKUP_BUTTONS {
            let words = match kind {
                "Highlight" => ("Highlight", "Highlight match"),
                "Underline" => ("Underline", "Underline match"),
                _ => ("Strike out", "Strike out match"),
            };
            self.dom.set_disabled(id, !selected && !hit);
            self.dom
                .set_text(id, if !selected && hit { words.1 } else { words.0 });
        }
        self.dom
            .set_disabled(ids::MARKUP_ALL_BUTTON, !can || self.search_hits.is_empty());
    }

    /// R:8556: a markup button or its drive op: the selection if there is one, else the current
    /// search match.
    pub fn mark_selection_or_match(&mut self, kind: &str) -> bool {
        let verb = match kind {
            "Highlight" => "Highlighted",
            "Underline" => "Underlined",
            _ => "Struck out",
        };
        if let Some(selected) = self.selection_markup() {
            let quoted: String = selected.text.chars().take(60).collect();
            let ok = self.add_markup(
                selected.page,
                kind,
                &selected.quads,
                &format!("{verb} “{quoted}”"),
            );
            if ok {
                self.markup_last = Some(MarkupLast {
                    kind: kind.to_string(),
                    page: selected.page,
                    source: MarkupSource::Selection,
                    quads: selected.quads,
                    text: Some(selected.text),
                });
                self.lane4.selection = None;
            }
            return ok;
        }
        let Some(hit) = self.current_hit() else {
            self.announce(messages::annotate::NO_SELECTION);
            return false;
        };
        let ok = self.add_markup(hit.page, kind, &hit.quads, &format!("{verb} match"));
        if ok {
            self.markup_last = Some(MarkupLast {
                kind: kind.to_string(),
                page: hit.page,
                source: MarkupSource::Search,
                quads: hit
                    .quads
                    .iter()
                    .map(|quad| quad.iter().map(|v| r2(*v)).collect())
                    .collect(),
                text: None,
            });
        }
        ok
    }

    /// R:8578: "Highlight all matches": one Highlight per page that has matches (one undo step
    /// per page).
    pub fn highlight_all_matches(&mut self) -> bool {
        if self.doc.is_none() || !self.can_annotate() || self.search_hits.is_empty() {
            return false;
        }
        // A Map: pages in the order of their first match.
        let mut by_page: Vec<(i64, Vec<Vec<f64>>)> = Vec::new();
        for hit in &self.search_hits {
            match by_page.iter_mut().find(|(page, _)| *page == hit.page) {
                Some((_, quads)) => quads.extend(hit.quads.iter().cloned()),
                None => by_page.push((hit.page, hit.quads.clone())),
            }
        }
        let total = self.search_hits.len() as i64;
        let mut count = 0;
        for (page, quads) in &by_page {
            if !self.add_markup(
                *page,
                "Highlight",
                quads,
                &messages::annotate::highlighted_all(total),
            ) {
                return false;
            }
            count += 1;
        }
        self.markup_last = Some(MarkupLast {
            kind: "Highlight".into(),
            page: by_page.first().map_or(0, |(page, _)| *page),
            source: MarkupSource::Search,
            quads: Vec::new(),
            text: None,
        });
        self.announce(&messages::annotate::highlighted_all(total));
        count > 0
    }

    /// R:8593: `select {text, page?, occurrence?}` selects that text on the page as a person
    /// would; `select {clear:true}` clears.
    pub fn drive_select(&mut self, contents: Option<&str>) {
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        let Some(page_count) = self.doc.as_ref().map(|d| d.page_count) else {
            return;
        };
        if wanted.get("clear") == Some(&json!(true)) {
            self.lane4.selection = None;
            self.sync_markup_buttons();
            return;
        }
        let page = match wanted.get("page").and_then(|v| v.as_f64()) {
            Some(page) => page,
            None => self.wanted_page as f64,
        };
        let text = wanted
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if text.is_empty() || page.fract() != 0.0 || page < 0.0 || page >= page_count as f64 {
            self.show_error("select: give text (and page)");
            return;
        }
        let page = page as i64;
        self.go_to(page);
        // The TS waited for the page to be shown; the synchronous drain has shown it.
        self.drain();
        let mut skip = wanted
            .get("occurrence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let needle: Vec<char> = text.chars().collect();
        let found = self.text_layer().and_then(|lines| {
            for (index, line) in lines.iter().enumerate() {
                let hay: Vec<char> = line.text.chars().collect();
                let mut at = find_chars(&hay, &needle, 0);
                while let Some(position) = at {
                    if skip <= 0.0 {
                        break;
                    }
                    skip -= 1.0;
                    at = find_chars(&hay, &needle, position + 1);
                }
                if hay.is_empty() {
                    continue;
                }
                if let Some(position) = at {
                    return Some((index, position));
                }
            }
            None
        });
        if let Some((index, position)) = found {
            self.lane4.selection = Some(TextSelection {
                page: self.shown_page,
                start: (index, position),
                end: (index, position + needle.len()),
                text_key: self.painted_text_key.clone(),
            });
            self.sync_markup_buttons();
            return;
        }
        self.show_error(&format!("select: \"{text}\" is not on page {}", page + 1));
    }

    /// R:8617: markup over the given quads; the comment starts empty and is edited from the list.
    pub fn add_markup(
        &mut self,
        page: i64,
        kind: &str,
        quads: &[Vec<f64>],
        announcement: &str,
    ) -> bool {
        if self.doc.is_none() || !self.can_annotate() {
            self.show_error("This document does not permit annotations.");
            return false;
        }
        let quads: Vec<serde_json::Value> =
            quads.iter().map(|q| super::form_fields::nums(q)).collect();
        let request =
            json!({"op": "annot-add", "page": page, "type": kind, "quads": quads, "contents": ""});
        match self.request(request, vec![]) {
            Ok(reply) => {
                self.apply_edit(&journal_of(&reply), announcement);
                true
            }
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                false
            }
        }
    }

    /// R:8633: adds a 160x40 text box whose top-left is `point` (PDF points, y down).
    pub fn add_text_box_at(&mut self, page: i64, point: Point, contents: &str) -> bool {
        if self.doc.is_none() || !self.can_annotate() {
            self.show_error("This document does not permit annotations.");
            return false;
        }
        let point = super::form_fields::nums(&point);
        let request = json!({"op": "annot-add", "page": page, "type": "FreeText", "point": point, "contents": contents});
        match self.request(request, vec![]) {
            Ok(reply) => {
                self.apply_edit(&journal_of(&reply), "Added text box");
                true
            }
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                false
            }
        }
    }
}

/// `String#indexOf(needle, from)` over characters.
fn find_chars(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&at| hay[at..at + needle.len()] == *needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_whitespace_like_js() {
        assert_eq!(collapse_spaces("a \n\t b  c"), "a b c");
        assert_eq!(collapse_spaces("  a"), " a");
    }

    #[test]
    fn char_boxes_share_the_line_width() {
        let line = Line {
            text: "abcd".into(),
            x: 10.0,
            y: 100.0,
            w: 40.0,
            size: 10.0,
            ..Line::default()
        };
        assert_eq!(char_box(&line, 0, 4), [10.0, 92.0, 10.0, 10.0]);
        assert_eq!(char_box(&line, 3, 4)[0], 40.0);
    }

    #[test]
    fn finds_occurrences() {
        let hay: Vec<char> = "one two one".chars().collect();
        let needle: Vec<char> = "one".chars().collect();
        assert_eq!(find_chars(&hay, &needle, 0), Some(0));
        assert_eq!(find_chars(&hay, &needle, 1), Some(8));
        assert_eq!(find_chars(&hay, &needle, 9), None);
    }

    #[test]
    fn selection_on_a_painted_line_gives_one_quad() {
        let mut session = Session::new();
        session.doc = Some(OpenDoc {
            page_count: 1,
            pages: vec![PageSize {
                width: 200.0,
                height: 200.0,
            }],
            ..OpenDoc::default()
        });
        session.shown_page = 0;
        session.shown_zoom = 1.0;
        let line = Line {
            text: "PDF4All here".into(),
            x: 0.0,
            y: 50.0,
            w: 120.0,
            size: 10.0,
            ..Line::default()
        };
        session.last_result = Some(RenderReply {
            page: 0,
            lines: vec![line],
            ..RenderReply::default()
        });
        session.lane4.selection = Some(TextSelection {
            page: 0,
            start: (0, 0),
            end: (0, 7),
            text_key: String::new(),
        });
        let marked = session.selection_markup().expect("a selection");
        assert_eq!(marked.text, "PDF4All");
        assert_eq!(
            marked.quads,
            vec![vec![0.0, 42.0, 70.0, 42.0, 0.0, 52.0, 70.0, 52.0]]
        );
        session.lane4.selection.as_mut().unwrap().end = (0, 0);
        assert!(session.selection_markup().is_none());
    }
}
