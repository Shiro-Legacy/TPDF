//! Comments the window can find again: the sidebar's document-wide list (every page's
//! comments, grouped by page) and the popover that opens a comment beside its annotation.
//!
//! The list comes from the engine's `comments` op, one call for every page, read again only
//! when the document changes (its open generation or journal revision moves: an edit, an
//! undo, a page op), so moving between pages costs nothing.

use crate::session::Session;
use serde_json::json;
use tandem_protocol::engine::{AnnotItem, AnnotsReply, CommentItem, CommentsReply};
use tandem_protocol::ui::{CommentPopover, CommentRow, NoteMark, Reveal, css_hex, note_icon_rect};

/// The most characters of a comment a list row shows.
const ROW_CHARS: usize = 80;

/// The comment open beside its annotation: which one, on which page, what it said when it
/// opened, what has been typed, and where it sits (it stays open while its page is in view,
/// whichever page is current).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Popover {
    pub id: i64,
    pub page: i64,
    pub draft: String,
    pub opened: String,
    pub rect: [f64; 4],
    pub kind: String,
    pub editable: bool,
    /// A note's anchor (its icon's top left): the popover sits beside its icon, whose size on
    /// the page follows the zoom.
    pub anchor: Option<[f64; 2]>,
    pub color: Option<String>,
}

/// The sidebar's rows as last built, what they were built from, and their revision.
#[derive(Clone, Debug, Default)]
pub struct RowCache {
    pub key: Option<(Option<CommentsKey>, Option<i64>)>,
    pub rows: Vec<CommentRow>,
    pub rev: u64,
    /// The revision the window was last sent.
    pub sent: u64,
}

/// The kinds a person adds as comments: only adding one of these introduces the sidebar tab.
const ADDED_KINDS: [&str; 6] = [
    "Text",
    "FreeText",
    "Highlight",
    "Underline",
    "StrikeOut",
    "Squiggly",
];

/// Whether an action can be a person adding a comment (a note or text box placed and
/// finished, text marked): a page inserted, a paste or a menu op that is not one cannot.
pub fn may_add_comment(action: &tandem_protocol::ui::UiAction) -> bool {
    use tandem_protocol::ui::UiAction;
    match action {
        UiAction::Pointer { phase, .. } => phase == "up",
        UiAction::EditorCommit | UiAction::Key { .. } => true,
        UiAction::Edit { op, .. } => matches!(
            op.as_str(),
            "highlight" | "underline" | "strikeout" | "squiggly" | "note" | "freetext"
        ),
        _ => false,
    }
}

/// The document a comment list was read from: (open generation, journal revision, name).
pub type CommentsKey = (i64, i64, String);

/// "Note", "Highlight", "Squiggly underline": what a comment is, in plain words.
fn kind_name(item: &AnnotItem) -> &'static str {
    if item.r#type == "Squiggly" {
        return "Squiggly underline";
    }
    crate::session::pages::annot_name(item).unwrap_or("Comment")
}

fn as_annot(c: &CommentItem) -> AnnotItem {
    AnnotItem {
        id: c.id,
        r#type: c.r#type.clone(),
        rect: c.rect.clone(),
        contents: c.contents.clone(),
        subject: c.subject.clone(),
        editable: c.editable,
        search: None,
        color: c.color.clone(),
        anchor: c.anchor.clone(),
        ..AnnotItem::default()
    }
}

/// A note's anchor in page points: the corner its icon hangs from (the engine's `anchor`, its
/// /Rect's user-space top left), else its box's top left. None for anything but a note (Text), or one with no place on the page.
pub fn note_anchor(item: &AnnotItem) -> Option<[f64; 2]> {
    if item.r#type != "Text" {
        return None;
    }
    match (item.anchor.as_deref(), item.rect.as_slice()) {
        (Some([x, y]), _) if x.is_finite() && y.is_finite() => Some([*x, *y]),
        (_, [x0, y0, _, _]) => Some([*x0, *y0]),
        _ => None,
    }
}

/// A note's colour as the window draws it (CSS hex), None for anything but a note.
pub fn note_color(item: &AnnotItem) -> Option<String> {
    (item.r#type == "Text").then(|| css_hex(item.color.as_deref()))
}

/// A row's words: the first line, cut at a word near `ROW_CHARS`, or a stand-in.
pub fn row_text(item: &AnnotItem) -> String {
    let first = item
        .contents
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first.is_empty() {
        return match item.r#type.as_str() {
            "Text" => "Empty note",
            "FreeText" => "Empty text box",
            _ => "No words",
        }
        .into();
    }
    let more = item.contents.trim().chars().count() > first.chars().count();
    if first.chars().count() <= ROW_CHARS {
        return if more {
            format!("{first} …")
        } else {
            first.to_string()
        };
    }
    let cut: String = first.chars().take(ROW_CHARS).collect();
    let at = cut
        .rfind(' ')
        .filter(|i| *i > ROW_CHARS / 2)
        .unwrap_or(cut.len());
    format!("{}…", cut[..at].trim_end())
}

/// A PDF date ("D:20261104093000Z", "D:2026") in plain words: "4 Nov 2026, 09:30", or
/// "Nov 2026" / "2026" for the parts it has; empty when it is not a date.
pub fn plain_date(pdf: &str) -> String {
    let digits: String = pdf
        .trim()
        .trim_start_matches("D:")
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let part = |from: usize, len: usize| -> Option<u32> {
        digits.get(from..from + len).and_then(|s| s.parse().ok())
    };
    let Some(year) = part(0, 4) else {
        return String::new();
    };
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = part(4, 2).filter(|m| (1..=12).contains(m));
    let day = part(6, 2).filter(|d| (1..=31).contains(d));
    let time = part(8, 2)
        .zip(part(10, 2))
        .filter(|(h, m)| *h < 24 && *m < 60);
    match (month, day) {
        (Some(m), Some(d)) => {
            let date = format!("{d} {} {year}", MONTHS[m as usize - 1]);
            match time {
                Some((h, min)) => format!("{date}, {h:02}:{min:02}"),
                None => date,
            }
        }
        (Some(m), None) => format!("{} {year}", MONTHS[m as usize - 1]),
        _ => year.to_string(),
    }
}

/// Whether a single click on the annotation opens its popover: a note always, any other
/// comment when it has words (a highlight without a comment is only picked; Return opens
/// it). A text box shows its words on the page, and a placed signature has none to show.
pub fn opens_on_click(item: &AnnotItem) -> bool {
    match item.r#type.as_str() {
        "Text" => true,
        "FreeText" | "Redact" | "Link" | "Widget" | "Popup" => false,
        _ => !crate::session::direct::is_signature(item) && !item.contents.trim().is_empty(),
    }
}

impl Session {
    /// The document's key now, or None without a readable document.
    fn ui_comments_key(&self) -> Option<CommentsKey> {
        let doc = self.doc.as_ref()?;
        (!doc.info.needs_password).then(|| (self.generation, self.revision, doc.name.clone()))
    }

    /// Reads every page's comments again when the document changed since the last read.
    pub(crate) fn ui_refresh_doc_comments(&mut self) {
        let key = self.ui_comments_key();
        if key == self.ui.doc_comments_key {
            return;
        }
        self.ui.doc_comments_key = key.clone();
        self.ui.doc_comments.clear();
        if key.is_none() {
            return;
        }
        if let Ok(reply) = self.request(json!({ "op": "comments" }), Vec::new()) {
            let reply: CommentsReply = crate::session::parse_reply_or_default(&reply.value);
            // A placed signature is a picture of a name, not a comment.
            self.ui.doc_comments = reply
                .items
                .into_iter()
                .filter(|c| !crate::session::direct::is_signature(&as_annot(c)))
                .collect();
        }
    }

    /// The document key before an action that may add a comment, with the list read for it.
    pub(crate) fn ui_comments_before(&mut self) -> Option<CommentsKey> {
        self.ui_refresh_doc_comments();
        self.ui.doc_comments_key.clone()
    }

    /// After an action that may have added a comment: the first one added this session opens
    /// the sidebar's Comments tab with its row current, so one sees where comments live.
    pub(crate) fn ui_introduce_comments(&mut self, before_key: &Option<CommentsKey>) {
        let now = self.ui_comments_key();
        // Nothing changed (most actions): nothing to compare.
        if now == *before_key {
            return;
        }
        let same_doc = matches!(
            (before_key, &now),
            (Some(a), Some(b)) if a.0 == b.0 && a.2 == b.2
        );
        if !same_doc {
            return;
        }
        let before: std::collections::HashSet<i64> =
            self.ui.doc_comments.iter().map(|c| c.id).collect();
        self.ui_refresh_doc_comments();
        let added = self
            .ui
            .doc_comments
            .iter()
            .find(|c| !before.contains(&c.id) && ADDED_KINDS.contains(&c.r#type.as_str()))
            .map(|c| c.id);
        let Some(id) = added else {
            return;
        };
        self.ui.comment_current = Some(id);
        if self.ui.comments_introduced {
            return;
        }
        self.ui.comments_introduced = true;
        self.ui.sidebar_tab = "comments".into();
        if !self.sidebar_open {
            self.set_thumb_sidebar(true);
        }
    }

    /// Builds the sidebar's rows again when the list or the current comment changed; the
    /// revision moves only when they did.
    pub(crate) fn ui_sync_comment_rows(&mut self) {
        self.ui_refresh_doc_comments();
        let key = Some((self.ui.doc_comments_key.clone(), self.ui.comment_current));
        if key == self.ui.comment_rows.key {
            return;
        }
        let current = self.ui.comment_current;
        let rows: Vec<CommentRow> = self
            .ui
            .doc_comments
            .iter()
            .map(|c| {
                let item = as_annot(c);
                let page = c.page.max(0) as usize;
                CommentRow {
                    id: c.id.to_string(),
                    page,
                    page_label: self
                        .labels
                        .get(page)
                        .filter(|l| !l.is_empty())
                        .cloned()
                        .unwrap_or_else(|| (page + 1).to_string()),
                    kind: kind_name(&item).into(),
                    text: row_text(&item),
                    author: c.author.trim().to_string(),
                    date: plain_date(&c.date),
                    current: current == Some(c.id),
                    located: c.rect.len() == 4,
                    editable: c.editable,
                    color: note_color(&item),
                }
            })
            .collect();
        let cache = &mut self.ui.comment_rows;
        cache.key = key;
        if rows != cache.rows {
            cache.rows = rows;
            cache.rev += 1;
        }
    }

    /// Goes to a comment's page when it is on another one; its annotation on the shown page
    /// as listed there, if it is there.
    fn ui_comment_page(&mut self, id: i64) -> Option<AnnotItem> {
        self.ui_refresh_doc_comments();
        let page = self
            .ui
            .doc_comments
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.page);
        if let Some(page) = page
            && page != self.wanted_page
        {
            self.edit("goto", Some(json!({ "page": page })));
        }
        self.notes.iter().find(|n| n.id == id).cloned()
    }

    /// A row of the sidebar's Comments list: "select" goes to the comment, brings it into
    /// view, picks it and flashes it; "edit" opens it beside the annotation; "delete".
    pub(crate) fn ui_doc_comment_row(&mut self, id: i64, action: &str) {
        self.ui_close_popover(true);
        let Some(note) = self.ui_comment_page(id) else {
            return;
        };
        self.ui.comment_current = Some(id);
        match action {
            "edit" => self.ui_open_popover(id),
            "delete" => {
                self.ui.annot_selected = None;
                self.on_note_list_action("delete", id);
            }
            _ => {
                if self.ui.tool == "select" {
                    self.ui_pick_annot(Some(id));
                } else {
                    self.lane2.note_row_current = Some(id);
                }
                // A comment with no place on the page (no /Rect) brings the page's top in; a
                // note flashes its icon.
                let rect = match (note_anchor(&note), note.rect.as_slice()) {
                    (Some(anchor), _) => note_icon_rect(anchor, self.ui_zoom()),
                    (None, [a, b, c, d]) => [*a, *b, *c, *d],
                    _ => [0.0, 0.0, 24.0, 24.0],
                };
                self.ui.reveal_seq += 1;
                self.ui.reveal = Some(Reveal {
                    page: self.wanted_page.max(0) as usize,
                    rect,
                    seq: self.ui.reveal_seq,
                });
            }
        }
    }

    /// Opens the comment beside its annotation on the shown page, with its words to change.
    pub(crate) fn ui_open_popover(&mut self, id: i64) {
        let Some(note) = self.notes.iter().find(|n| n.id == id).cloned() else {
            return;
        };
        let [a, b, c, d] = note.rect.as_slice() else {
            return;
        };
        let anchor = note_anchor(&note);
        let rect = anchor.map_or([*a, *b, *c, *d], |a| note_icon_rect(a, self.ui_zoom()));
        self.ui.popover = Some(Popover {
            id,
            page: self.wanted_page,
            draft: note.contents.clone(),
            opened: note.contents.clone(),
            rect,
            kind: kind_name(&note).into(),
            editable: note.editable && self.can_annotate(),
            anchor,
            color: note_color(&note),
        });
        self.ui.comment_current = Some(id);
        if self.ui.tool == "select" {
            self.ui_pick_annot(Some(id));
        } else {
            self.lane2.note_row_current = Some(id);
        }
    }

    /// Closes the popover; `keep` saves what was typed when it differs from what it opened
    /// with (Esc, Save, a click outside); a deleted comment's is not kept. Returns whether
    /// one was open.
    pub(crate) fn ui_close_popover(&mut self, keep: bool) -> bool {
        let Some(p) = self.ui.popover.take() else {
            return false;
        };
        if keep {
            self.ui_save_popover(&p);
        }
        true
    }

    /// Saves the popover's words as one undoable edit, when they changed and may be changed.
    /// Compared with what it opened with: a change made meanwhile (by an agent) is not
    /// written over by words no one typed.
    fn ui_save_popover(&mut self, p: &Popover) {
        if !p.editable || p.draft == p.opened || !self.can_annotate() {
            return;
        }
        // The note must still be there, on its page (an agent or another step may have
        // removed it meanwhile): if it is not, nothing is written and nothing is said.
        let there = self
            .request(json!({ "op": "annots", "page": p.page }), Vec::new())
            .ok()
            .map(|r| crate::session::parse_reply_or_default::<AnnotsReply>(&r.value))
            .is_some_and(|r| r.items.iter().any(|n| n.id == p.id));
        if !there {
            return;
        }
        let text = crate::session::decorate::slice_utf16(
            &p.draft,
            tandem_protocol::limits::MAX_NOTE_CHARS,
        );
        self.edit_note_on(p.page, p.id, &text);
    }

    /// The popover's buttons and typing (`UiAction::Comment`). Anything but "open" acts only
    /// on the comment open now: a late "close" for one that was already closed (the window's
    /// click-outside after the page's own press opened another) does nothing.
    pub(crate) fn ui_comment_action(&mut self, action: &str, id: &str, text: Option<String>) {
        let num = id.parse::<i64>().unwrap_or(-1);
        if action == "open" {
            self.ui_close_popover(true);
            self.ui_open_popover(num);
            return;
        }
        if self.ui.popover.as_ref().is_none_or(|p| p.id != num) {
            return;
        }
        if let (Some(p), Some(text)) = (self.ui.popover.as_mut(), text) {
            p.draft = text;
        }
        match action {
            "input" => {}
            "delete" => {
                let Some(p) = self.ui.popover.take() else {
                    return;
                };
                if !p.editable || !self.can_annotate() {
                    return;
                }
                if p.page != self.wanted_page {
                    self.edit("goto", Some(json!({ "page": p.page })));
                }
                self.ui.annot_selected = None;
                self.on_note_list_action("delete", p.id);
            }
            // "save", "close" (Esc, a click outside): what was typed is kept.
            _ => {
                self.ui_close_popover(true);
            }
        }
    }

    /// After each action: the popover stays while its page is in view (scrolling to the next
    /// page does not lose what was typed); once its page is gone from view it closes, saving,
    /// as a click outside does. On the current page, an annotation that is gone (undone,
    /// deleted elsewhere) takes it along.
    pub(crate) fn ui_popover_follow(&mut self) {
        let Some(p) = self.ui.popover.as_ref() else {
            return;
        };
        if p.page == self.wanted_page {
            if self.shown_page == self.wanted_page && !self.notes.iter().any(|n| n.id == p.id) {
                self.ui.popover = None;
            }
            return;
        }
        if !self.ui.visible.contains(&(p.page.max(0) as usize)) {
            self.ui_close_popover(true);
        }
    }

    /// The popover as the window draws it.
    pub(crate) fn ui_popover_view(&mut self) -> Option<CommentPopover> {
        let p = self.ui.popover.clone()?;
        self.ui_refresh_doc_comments();
        let listed = self.ui.doc_comments.iter().find(|c| c.id == p.id);
        let author = listed
            .map(|c| c.author.trim().to_string())
            .unwrap_or_default();
        let date = listed.map(|c| plain_date(&c.date)).unwrap_or_default();
        // Beside a note's icon as it is drawn now, at this zoom.
        let rect = p
            .anchor
            .map_or(p.rect, |a| note_icon_rect(a, self.ui_zoom()));
        Some(CommentPopover {
            id: p.id.to_string(),
            page: p.page.max(0) as usize,
            rect,
            kind: p.kind,
            text: p.opened,
            draft: p.draft,
            author,
            date,
            editable: p.editable,
            color: p.color,
        })
    }

    /// The zoom the window draws at (CSS px per point): the one of the last paint.
    pub(crate) fn ui_zoom(&self) -> f64 {
        if self.shown_zoom > 0.0 {
            self.shown_zoom
        } else {
            self.zoom_now(self.wanted_page).max(0.01)
        }
    }

    /// The notes on the pages in view (and the current page), each drawn as the note icon:
    /// from the document-wide comment list, which leaves out hidden notes as the page did.
    /// Picked (Select), or open beside its icon: drawn with its ring.
    pub(crate) fn ui_note_marks(&mut self) -> Vec<NoteMark> {
        self.ui_refresh_doc_comments();
        let current = self.wanted_page.max(0) as usize;
        let open = self.ui.popover.as_ref().map(|p| p.id);
        let picked = self.ui.annot_selected;
        let shown = |page: usize| page == current || self.ui.visible.contains(&page);
        self.ui
            .doc_comments
            .iter()
            // A reply sits on its parent: the parent's icon is drawn, not a second one.
            .filter(|c| c.page >= 0 && !c.reply && shown(c.page as usize))
            .filter_map(|c| {
                let item = as_annot(c);
                Some(NoteMark {
                    id: c.id.to_string(),
                    page: c.page as usize,
                    anchor: note_anchor(&item)?,
                    color: note_color(&item)?,
                    selected: open == Some(c.id) || picked == Some(c.id),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_plainly() {
        assert_eq!(plain_date("D:20261104093000Z"), "4 Nov 2026, 09:30");
        assert_eq!(plain_date("D:20260101"), "1 Jan 2026");
        assert_eq!(plain_date("D:202603"), "Mar 2026");
        assert_eq!(plain_date("D:2026"), "2026");
        assert_eq!(plain_date("not a date"), "");
        assert_eq!(plain_date(""), "");
    }

    #[test]
    fn rows_show_the_first_line_cut_at_a_word() {
        let note = |text: &str| AnnotItem {
            r#type: "Text".into(),
            contents: text.into(),
            ..AnnotItem::default()
        };
        assert_eq!(row_text(&note("Short")), "Short");
        assert_eq!(row_text(&note("First\nsecond")), "First …");
        assert_eq!(row_text(&note("")), "Empty note");
        let long = "word ".repeat(30);
        let row = row_text(&note(&long));
        assert!(
            row.ends_with('…') && row.chars().count() <= ROW_CHARS + 1,
            "{row}"
        );
        assert!(!row.contains("wo…"), "cut at a word: {row}");
    }

    #[test]
    fn a_click_opens_notes_and_comments_with_words() {
        let item = |kind: &str, text: &str| AnnotItem {
            r#type: kind.into(),
            contents: text.into(),
            ..AnnotItem::default()
        };
        assert!(opens_on_click(&item("Text", "")));
        assert!(opens_on_click(&item("Highlight", "check")));
        assert!(!opens_on_click(&item("Highlight", "")));
        assert!(!opens_on_click(&item("Square", "")));
        assert!(!opens_on_click(&item("FreeText", "words")));
    }
}
