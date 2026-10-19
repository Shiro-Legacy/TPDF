//! What the operating system and its native UI answer: file pickers, message
//! boxes, opening a web link, printing, the OS recent-documents list and the
//! window's represented file. Electron's `dialog`, `shell` and `app` calls in
//! `src/main.ts` become calls on a [`Platform`].
//!
//! The Tauri shell implements it with real sheets in a window. A headless run
//! (`--smoke`, `--drive`) uses [`Scripted`]: every picker is answered from the
//! run's pre-answers ([`Answers`]: `--save-to`, a drive line's `to`, `from`,
//! `images`, `folder`, `filled` …) and a missing pre-answer is "cancelled";
//! no native sheet is ever shown.

use std::path::{Path, PathBuf};

/// A file-type filter of a picker (`{name, extensions}` in Electron).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
}

pub const PDF_FILTER: Filter = Filter {
    name: "PDF",
    extensions: &["pdf"],
};
pub const CERTIFICATE_FILTER: Filter = Filter {
    name: "Certificate",
    extensions: &["p12", "pfx", "pem"],
};
pub const TRUST_FILTER: Filter = Filter {
    name: "Certificates",
    extensions: &["pem", "crt", "cer"],
};
pub const IMAGES_FILTER: Filter = Filter {
    name: "Images",
    extensions: &["png", "jpg", "jpeg"],
};

/// Why a single file is being picked (M:437, 987, 1075, 1197).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// File ▸ Open… (filter PDF).
    Open,
    /// Insert Pages from File… and Compare with… (filter PDF).
    InsertSource,
    /// Sign with Certificate… (filter Certificate).
    Certificate,
    /// Verify ▸ Trust certificates (filter Certificates).
    TrustCertificates,
}

/// Why a save sheet is shown (M:881, 1035, 1144, 1343).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavePurpose {
    /// `save-file`: Save a Copy, Save As, Save on Untitled, a filled form's new file.
    Document,
    /// `save-new`: an export, extracted pages, a redacted copy, new from images.
    New,
    /// `sign-file`: the signed copy.
    Signed,
    /// Print with no printer: save the print as a PDF.
    Print,
}

/// The kind of a message box (Electron's `type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxKind {
    Info,
    Question,
    Warning,
    Error,
}

/// Which question a message box asks, so a scripted run can answer it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Question {
    /// "Discard unsaved changes?" (M:95-102).
    Discard,
    /// Unsaved redaction boxes before close/open (M:522-530).
    PendingBoxes,
    /// The first Save of a filled form: over it, or a new file (M:848-858).
    Filled,
    /// "Open <uri> in your browser?" (M:970-976).
    OpenLink,
    /// A certificate not meant for signing documents (M:1104-1110).
    NotForSigning,
}

/// A native message box (`dialog.showMessageBox`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageBox {
    pub question: Question,
    pub kind: BoxKind,
    pub message: String,
    pub detail: Option<String>,
    pub buttons: Vec<String>,
    /// The button Return chooses.
    pub default: usize,
    /// The button Escape (or closing the sheet) chooses.
    pub cancel: usize,
}

/// How a system print ended (`PrintResult` reasons, M:63, M:332-337).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintOutcome {
    Printed,
    Cancelled,
    /// The machine has no printer: the host then offers to save the print as a PDF.
    NoPrinter,
    Failed,
}

/// A headless run's dialog pre-answers (main.ts's `saveToArg`, `insertFromArg`,
/// `imagesArg`, `batchDirArg`, `filledAnswer`, `printToArg`). The drive harness
/// rewrites them per line exactly as M:2214-2220 does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answers {
    /// Every save sheet (`--save-to`, a drive line's `to`/`path`); per line in a drive.
    pub save_to: Option<PathBuf>,
    /// The insert/compare source (`--insert-from`, `from`; sticky).
    pub insert_from: Option<PathBuf>,
    /// Comma-joined image paths (`--images`, `images`; sticky). Kept as the joined
    /// text, as main kept it: `pick-images` splits it again.
    pub images: Option<String>,
    /// The batch folder (`--batch-dir`, `folder`; sticky).
    pub batch_dir: Option<PathBuf>,
    /// The filled-form question: true = "Save Over" (`filled: "over"`, per line).
    pub filled_over: bool,
    /// The smoke print target (`--print-to`, a drive `print-to` step's `path`).
    pub print_to: Option<PathBuf>,
}

/// Everything the OS or its native UI answers for the host.
pub trait Platform: Send {
    /// A single-file open sheet; `None` = cancelled.
    fn pick_file(&mut self, pick: Pick, filters: &[Filter]) -> Option<PathBuf>;
    /// The multi-select image sheet (filter Images); empty = cancelled.
    fn pick_images(&mut self) -> Vec<PathBuf>;
    /// A folder sheet; `None` = cancelled.
    fn pick_folder(&mut self) -> Option<PathBuf>;
    /// A save sheet with a suggested name or path; `None` = cancelled.
    fn pick_save(
        &mut self,
        purpose: SavePurpose,
        default_path: &Path,
        filters: &[Filter],
    ) -> Option<PathBuf>;
    /// A message box; returns the index of the chosen button.
    fn ask(&mut self, question: &MessageBox) -> usize;
    /// Opens an allowed link in the default browser or mail app.
    fn open_external(&mut self, uri: &str);
    /// Prints a PDF through the system print dialog.
    fn print(&mut self, pdf: &[u8], job_name: &str) -> PrintOutcome;
    /// `app.addRecentDocument`.
    fn add_recent_document(&mut self, path: &Path);
    /// `app.clearRecentDocuments`.
    fn clear_recent_documents(&mut self);
    /// The window's represented file (macOS proxy icon) and title; `None` = no file.
    fn set_represented_file(&mut self, path: Option<&Path>, title: &str);
    /// `setDocumentEdited` (the macOS dirty dot) and the Save menu items:
    /// `save` = File ▸ Save, `save_as` = Save As… and Save a Copy… (M:388-401).
    fn set_document_state(&mut self, edited: bool, save: bool, save_as: bool);
    /// The pre-answers, when this platform is scripted (a headless run).
    fn answers(&mut self) -> Option<&mut Answers> {
        None
    }
}

/// The headless platform: pre-answers only, never a sheet.
#[derive(Debug, Default)]
pub struct Scripted {
    pub answers: Answers,
    /// Every question asked, for tests and for a report.
    pub asked: Vec<Question>,
    /// Links handed to `open_external` (never opened).
    pub opened_links: Vec<String>,
    /// The last represented file and title, and the document state.
    pub title: String,
    pub represented: Option<PathBuf>,
    pub edited: bool,
    pub save_enabled: bool,
    pub save_as_enabled: bool,
}

impl Scripted {
    pub fn new(answers: Answers) -> Self {
        Scripted {
            answers,
            ..Scripted::default()
        }
    }
}

fn non_empty(path: &Option<PathBuf>) -> Option<PathBuf> {
    path.clone().filter(|p| !p.as_os_str().is_empty())
}

impl Platform for Scripted {
    fn pick_file(&mut self, pick: Pick, _filters: &[Filter]) -> Option<PathBuf> {
        match pick {
            // M:983: `smoke && insertFromArg` (a non-empty pre-answer), else the
            // native sheet, which a headless run answers "cancelled" (spec §2.4).
            Pick::InsertSource => non_empty(&self.answers.insert_from),
            // Open without a path, and the certificate pickers without `opts.path`:
            // Electron showed a sheet on the hidden window; headless = cancelled.
            Pick::Open | Pick::Certificate | Pick::TrustCertificates => None,
        }
    }

    fn pick_images(&mut self) -> Vec<PathBuf> {
        // M:1230-1234: split on commas, trimmed, empties dropped; none = cancelled.
        match &self.answers.images {
            Some(list) => list
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect(),
            None => Vec::new(),
        }
    }

    fn pick_folder(&mut self) -> Option<PathBuf> {
        // M:1265-1268: no (or an empty) pre-answer = cancelled.
        non_empty(&self.answers.batch_dir)
    }

    fn pick_save(
        &mut self,
        purpose: SavePurpose,
        _default_path: &Path,
        _filters: &[Filter],
    ) -> Option<PathBuf> {
        match purpose {
            // M:875, 1031, 1140: `smoke && saveToArg !== undefined` → that path, even
            // an empty one (which then fails as unreadable). No pre-answer = cancelled.
            SavePurpose::Document | SavePurpose::New | SavePurpose::Signed => {
                self.answers.save_to.clone()
            }
            // A smoke print writes `--print-to` itself and never reaches the fallback.
            SavePurpose::Print => None,
        }
    }

    fn ask(&mut self, question: &MessageBox) -> usize {
        self.asked.push(question.question);
        match question.question {
            // M:850: a drive line's `filled` pre-answers it: 0 = over, 1 = new file.
            Question::Filled => usize::from(!self.answers.filled_over),
            // The other questions are skipped in a smoke run before they are asked
            // (the host checks `smoke`); if one is asked anyway, Return answers it.
            _ => question.default,
        }
    }

    fn open_external(&mut self, uri: &str) {
        self.opened_links.push(uri.to_string());
    }

    fn print(&mut self, _pdf: &[u8], _job_name: &str) -> PrintOutcome {
        // A headless run without `--print-to` has no print sheet to show.
        PrintOutcome::Cancelled
    }

    fn add_recent_document(&mut self, _path: &Path) {}

    fn clear_recent_documents(&mut self) {}

    fn set_represented_file(&mut self, path: Option<&Path>, title: &str) {
        self.represented = path.map(Path::to_path_buf);
        self.title = title.to_string();
    }

    fn set_document_state(&mut self, edited: bool, save: bool, save_as: bool) {
        self.edited = edited;
        self.save_enabled = save;
        self.save_as_enabled = save_as;
    }

    fn answers(&mut self) -> Option<&mut Answers> {
        Some(&mut self.answers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(question: Question) -> MessageBox {
        MessageBox {
            question,
            kind: BoxKind::Question,
            message: String::new(),
            detail: None,
            buttons: vec!["a".into(), "b".into(), "c".into()],
            default: 1,
            cancel: 2,
        }
    }

    #[test]
    fn scripted_answers() {
        let mut p = Scripted::default();
        assert_eq!(
            p.pick_save(SavePurpose::Document, Path::new("x.pdf"), &[PDF_FILTER]),
            None
        );
        assert_eq!(p.pick_file(Pick::InsertSource, &[PDF_FILTER]), None);
        assert!(p.pick_images().is_empty());
        assert_eq!(p.ask(&boxed(Question::Filled)), 1);
        p.answers.filled_over = true;
        assert_eq!(p.ask(&boxed(Question::Filled)), 0);
        assert_eq!(p.ask(&boxed(Question::Discard)), 1);
        p.answers.save_to = Some(PathBuf::from("/tmp/a.pdf"));
        p.answers.images = Some(" /a.png, ,/b.jpg ".into());
        p.answers.insert_from = Some(PathBuf::new());
        assert_eq!(
            p.pick_save(SavePurpose::New, Path::new("x"), &[]),
            Some(PathBuf::from("/tmp/a.pdf"))
        );
        assert_eq!(
            p.pick_images(),
            vec![PathBuf::from("/a.png"), PathBuf::from("/b.jpg")]
        );
        assert_eq!(p.pick_file(Pick::InsertSource, &[]), None);
        assert!(p.answers().is_some());
    }
}
