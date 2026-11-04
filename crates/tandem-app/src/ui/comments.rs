//! Comments the window can find again: the sidebar's document-wide list (every page's
//! comments, grouped by page) and the popover that opens a comment beside its annotation.
//!
//! The list comes from the engine's `comments` op, one call for every page, read again only
//! when the document changes (its open generation or journal revision moves: an edit, an
//! undo, a page op), so moving between pages costs nothing.

use crate::session::Session;
use serde_json::json;
use tandem_protocol::engine::{AnnotItem, CommentItem, CommentsReply};
use tandem_protocol::ui::{CommentPopover, CommentRow, Reveal};

/// The most characters of a comment a list row shows.
const ROW_CHARS: usize = 80;

/// The comment open beside its annotation: which one, on which page, and what has been typed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Popover {
    pub id: i64,
    pub page: i64,
    pub draft: String,
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
    }
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

    /// After an action that was not Undo or Redo: the first comment added this session opens
    /// the sidebar's Comments tab with its row current, so one sees where comments live.
    pub(crate) fn ui_introduce_comments(
        &mut self,
        before: &[i64],
        before_key: &Option<CommentsKey>,
    ) {
        self.ui_refresh_doc_comments();
        let same_doc = matches!(
            (before_key, &self.ui.doc_comments_key),
            (Some(a), Some(b)) if a.0 == b.0 && a.2 == b.2
        );
        if !same_doc {
            return;
        }
        let added = self
            .ui
            .doc_comments
            .iter()
            .find(|c| !before.contains(&c.id))
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

    /// The sidebar's rows: every comment, in page order, the current one marked.
    pub(crate) fn ui_doc_comment_rows(&mut self) -> Vec<CommentRow> {
        self.ui_refresh_doc_comments();
        let current = self.ui.comment_current;
        self.ui
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
                }
            })
            .collect()
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
                if note.rect.len() == 4 {
                    self.ui.reveal_seq += 1;
                    self.ui.reveal = Some(Reveal {
                        page: self.wanted_page.max(0) as usize,
                        rect: [note.rect[0], note.rect[1], note.rect[2], note.rect[3]],
                        seq: self.ui.reveal_seq,
                    });
                }
            }
        }
    }

    /// Opens the comment beside its annotation on the shown page, with its words to change.
    pub(crate) fn ui_open_popover(&mut self, id: i64) {
        let Some(note) = self.notes.iter().find(|n| n.id == id).cloned() else {
            return;
        };
        if note.rect.len() != 4 {
            return;
        }
        self.ui.popover = Some(Popover {
            id,
            page: self.wanted_page,
            draft: note.contents.clone(),
        });
        self.ui.comment_current = Some(id);
        if self.ui.tool == "select" {
            self.ui_pick_annot(Some(id));
        } else {
            self.lane2.note_row_current = Some(id);
        }
    }

    /// Closes the popover; `keep` saves what was typed when it differs (a click outside),
    /// else it is dropped (Esc). Returns whether one was open.
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
    fn ui_save_popover(&mut self, p: &Popover) {
        if p.page != self.wanted_page || !self.can_annotate() {
            return;
        }
        let Some(note) = self.notes.iter().find(|n| n.id == p.id).cloned() else {
            return;
        };
        if !note.editable || note.contents == p.draft {
            return;
        }
        let text = crate::session::decorate::slice_utf16(
            &p.draft,
            tandem_protocol::limits::MAX_NOTE_CHARS,
        );
        self.edit_note(p.id, &text);
    }

    /// The popover's buttons and typing (`UiAction::Comment`).
    pub(crate) fn ui_comment_action(&mut self, action: &str, id: &str, text: Option<String>) {
        let num = id.parse::<i64>().unwrap_or(-1);
        match action {
            "open" => {
                self.ui_close_popover(true);
                self.ui_open_popover(num);
            }
            "input" => {
                if let (Some(p), Some(text)) = (self.ui.popover.as_mut(), text) {
                    p.draft = text;
                }
            }
            "save" => {
                if let (Some(p), Some(text)) = (self.ui.popover.as_mut(), text) {
                    p.draft = text;
                }
                self.ui_close_popover(true);
            }
            "delete" => {
                let Some(p) = self.ui.popover.take() else {
                    return;
                };
                if p.page == self.wanted_page && self.can_annotate() {
                    self.ui.annot_selected = None;
                    self.on_note_list_action("delete", p.id);
                }
            }
            "cancel" => {
                self.ui_close_popover(false);
            }
            _ => {
                if let (Some(p), Some(text)) = (self.ui.popover.as_mut(), text) {
                    p.draft = text;
                }
                self.ui_close_popover(true);
            }
        }
    }

    /// The popover as the window draws it, while its annotation is on the shown page.
    pub(crate) fn ui_popover_view(&mut self) -> Option<CommentPopover> {
        let p = self.ui.popover.clone()?;
        let note = (p.page == self.wanted_page)
            .then(|| self.notes.iter().find(|n| n.id == p.id).cloned())
            .flatten()
            .filter(|n| n.rect.len() == 4);
        let Some(note) = note else {
            self.ui.popover = None;
            return None;
        };
        self.ui_refresh_doc_comments();
        let listed = self.ui.doc_comments.iter().find(|c| c.id == p.id);
        let author = listed
            .map(|c| c.author.trim().to_string())
            .unwrap_or_default();
        let date = listed.map(|c| plain_date(&c.date)).unwrap_or_default();
        Some(CommentPopover {
            id: p.id.to_string(),
            page: p.page.max(0) as usize,
            rect: [note.rect[0], note.rect[1], note.rect[2], note.rect[3]],
            kind: kind_name(&note).into(),
            text: note.contents.clone(),
            draft: p.draft,
            author,
            date,
            editable: note.editable && self.can_annotate(),
        })
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
