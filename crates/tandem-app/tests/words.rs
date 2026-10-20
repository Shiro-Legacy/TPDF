//! Plain words (UX audit X7/X8, docs/rust-and-ux-plan.md §2.3, §3.2): the message catalogue
//! (`tandem_app::messages`, the port of src/renderer/messages.ts), the redaction tool's words
//! and the host's native questions, the notice kinds, Verify's dates and the password rule.
//!
//! Ported from the Electron-era tests/p1-ux.test.mjs, tests/integration.test.mjs,
//! tests/redact-box.test.mjs and tests/p2-encrypt.test.mjs (the cases that imported
//! build/renderer/messages.js, build/renderer/redact-ui.js or read src/renderer/*.ts).
//!
//! Wording the Rust app changed on purpose after the port (UX walkthrough, commit bb18ae7) is
//! asserted as the Rust app says it: sentence-case buttons in the pending-boxes question
//! ("Redact now…", "Close without redacting", "Discard the boxes") and "Turn page 4 into an
//! image" for the flatten question's yes button (was "Flatten page 4").
// Some helpers serve only the Unix tests (modes, links).
#![cfg_attr(not(unix), allow(unused_imports, dead_code))]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{Value, json};
use tandem_app::host::{self, Event, Host, HostConfig};
use tandem_app::messages::{
    MessageKind, annotate, banner, editable_copy, filled, general, is_catalogue_text, main, open,
    redact, save, sign, user_text, verify,
};
use tandem_app::platform::{
    Answers, Filter, MessageBox, Pick, Platform, PrintOutcome, Question, SavePurpose, Scripted,
};
use tandem_app::session::Session;
use tandem_app::session::redact_ui::SANITIZE_KEYS;
use tandem_protocol::limits::MAX_REDACT_SOURCE_BYTES;

// ------------------------------------------------------------------ shared helpers

/// The catalogue remembers the last 256 texts its functions produced, process-wide. A test
/// that checks a produced text is left alone holds this lock, so the eviction test's 400 new
/// texts never push another test's text out between producing and checking it.
static MEMORY: Mutex<()> = Mutex::new(());

fn memory() -> MutexGuard<'static, ()> {
    MEMORY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

/// A fresh folder of this test's own under the temp folder.
fn scratch(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tandem-t-words-{}-{n}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A session over a memory-only smoke host whose user-data folder is `dir`.
fn session_in(dir: &Path) -> Session {
    let host = Host::new(
        HostConfig {
            user_data: dir.join("udata"),
            smoke: true,
            echo_stdout: false,
        },
        Box::new(Scripted::new(Answers::default())),
    );
    Session::with_host(host)
}

/// The words of `text`: maximal runs of `[A-Za-z0-9_]` (what `\b` separates).
fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect()
}

/// `/\b[a-z]+-[a-z]+-[a-z]+\b/`: three hyphen-joined lowercase words (an error token).
fn has_error_token(text: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .any(|run| {
            let parts: Vec<&str> = run.split('-').collect();
            parts.windows(3).any(|three| {
                three
                    .iter()
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase()))
            })
        })
}

/// `/^[a-z]+(?:-[a-z]+)*: /`: an operation prefix.
fn has_op_prefix(text: &str) -> bool {
    let Some((name, rest)) = text.split_once(':') else {
        return false;
    };
    rest.starts_with(' ')
        && name
            .split('-')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase()))
}

/// `/P=\d/`.
fn has_p_flag(text: &str) -> bool {
    text.as_bytes()
        .windows(3)
        .any(|w| w[0] == b'P' && w[1] == b'=' && w[2].is_ascii_digit())
}

/// Every text the catalogue can show, fixed and built, with representative values.
fn catalogue_texts() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |text: &str| out.push(text.to_string());
    for text in [
        general::DESKTOP_ONLY,
        general::UNEXPECTED,
        general::NOT_PERMITTED,
        general::READ_ONLY,
        open::WRONG_PASSWORD,
        open::NO_DOCUMENT,
        open::SAMPLE,
        save::CANCELLED,
        save::NOTHING_TO_SAVE,
        save::RUNNING,
        save::READ_ONLY,
        filled::DETAIL,
        filled::OVERWRITE,
        filled::NEW_FILE,
        filled::CANCEL,
        banner::LOCKED,
        banner::SIGNED_READ_ONLY,
        banner::READ_ONLY,
        banner::XFA,
        banner::OPEN_ACTION,
        banner::REPAIRED,
        annotate::NOTE_EMPTY,
        annotate::NO_SELECTION,
        annotate::NO_MATCH,
        sign::CHOOSE_CERTIFICATE,
        sign::NEEDS_DOCUMENT,
        sign::NO_DRAWING,
        sign::ROTATED,
        sign::INVALID_INPUT,
        sign::SECURITY_PENDING,
        sign::NOT_FOR_SIGNING_MESSAGE,
        sign::PREPARING,
        editable_copy::BUTTON,
        editable_copy::NOT_SIGNED,
        editable_copy::NEEDS_DOCUMENT,
        editable_copy::SAME_FILE_QUESTION.heading,
        editable_copy::SAME_FILE_QUESTION.text,
        editable_copy::SAME_FILE_QUESTION.yes,
        verify::PARTIAL,
        verify::WHOLE,
        redact::REDACT_CHIP,
        redact::CONFIRM_TEXT,
        redact::OVER_REDACTED,
        redact::SAME_FILE_QUESTION.heading,
        redact::SAME_FILE_QUESTION.text,
        redact::SAME_FILE_QUESTION.yes,
        redact::OTHERS_QUESTION.heading,
        redact::OTHERS_QUESTION.text,
        redact::PENDING_DETAIL,
        redact::SIGNED_QUESTION.heading,
        redact::SIGNED_QUESTION.text,
        redact::SIGNED_QUESTION.yes,
        redact::USAGE_RIGHTS_QUESTION.heading,
        redact::USAGE_RIGHTS_QUESTION.text,
        redact::USAGE_RIGHTS_QUESTION.yes,
        redact::ADDED,
        redact::REMOVED_BOX,
        redact::DRAW_FIRST,
        redact::DRAW_HINT,
        redact::STOPPED,
        redact::ORIGINAL_NOT_SAVED,
        redact::ORIGINAL_UNREADABLE,
        main::DISCARD_MESSAGE,
        main::DISCARD_DETAIL,
    ] {
        push(text);
    }
    for button in main::DISCARD_BUTTONS {
        push(button);
    }
    for (_, label) in redact::SANITIZE_LABELS {
        push(label);
    }
    let name = "form.pdf";
    let mut built: Vec<String> = vec![
        open::too_large(name, None),
        open::too_large(name, Some(1024.0 * 1024.0)),
        open::unreadable(name),
        open::not_pdf(name),
        open::missing(name, true),
        open::missing(name, false),
        open::changed(name),
        save::saved(name),
        save::saved_copy(name),
        save::saved_as(name),
        save::late_shown(name),
        save::late_kept(name),
        filled::question(name),
        banner::unlocked(true, &["printing"], true),
        banner::unlocked(false, &["printing", "copying"], false),
        banner::unlocked::<&str>(false, &[], false),
        banner::attachments(1),
        banner::attachments(2),
        annotate::highlighted_all(1),
        annotate::highlighted_all(3),
        sign::signed_opened(name),
        sign::certificate("x", Some("revoked")),
        editable_copy::suggested("form"),
        editable_copy::opened(name),
        editable_copy::opened_locked(name, 0),
        editable_copy::opened_locked(name, 1),
        editable_copy::opened_locked(name, 3),
        editable_copy::carried(1),
        editable_copy::carried(3),
        editable_copy::saved(name),
        editable_copy::same_file(),
        verify::trust("expired", None),
        verify::trust("trusted", Some("P0B Test CA")),
        verify::trust("trusted", None),
        verify::trust("untrusted", None),
        verify::signed_on(Some("2026-10-19T06:36:20.000Z"), true, false, None),
        verify::signed_on(None, true, false, None),
        main::open_link("https://example.com"),
        redact::areas_text(1),
        redact::areas_text(3),
        redact::boxes_text(1),
        redact::boxes_text(3),
        redact::panel_heading(3),
        redact::redact_button_text(0),
        redact::redact_button_text(1),
        redact::redact_button_text(3),
        redact::count_text(0, 0),
        redact::count_text(3, 2),
        redact::confirm_heading(1),
        redact::confirm_heading(3),
        redact::pages_name(&[3], true),
        redact::pages_name(&[1, 4, 6], false),
        redact::flatten_note(2),
        redact::pending_text(1),
        redact::pending_text(3),
        redact::size_limit_text(None),
        redact::added_page(3),
        redact::added_search(0, "secret"),
        redact::added_search(1, "secret"),
        redact::added_search(2, "secret"),
        redact::saved_kept(name, false),
        redact::saved_kept(name, true),
        redact::saved_opened(name),
        redact::saved(name, false),
        redact::saved(name, true),
        redact::same_file(),
    ];
    for (_, text) in save::reason_table() {
        built.push(text);
    }
    for reason in ["permission", "disk-full", "broken-link"] {
        built.extend(save::new_file(reason));
    }
    let reasons = [
        "bad-password",
        "unsupported-key",
        "certificate-expired",
        "no-certificate",
        "unreadable",
        "too-costly",
        "cancelled",
        "busy",
        "invalid",
        "too-large",
        "too-many",
        "permission",
        "disk-full",
        "broken-link",
        "write-failed",
        "something-new",
    ];
    for reason in reasons {
        built.push(sign::certificate(reason, None));
        built.push(sign::failed(reason, None));
        built.push(editable_copy::failed(reason));
        built.push(redact::not_saved(reason));
    }
    built.push(editable_copy::failed("encrypted"));
    for level in 0..4 {
        built.push(verify::certified(level));
    }
    for token in [
        "no-signing-time",
        "weak-digest",
        "unknown-digest",
        "changes-not-permitted",
        "missing-subfilter",
        "byte-range",
        "digest",
        "signature",
        "signer-cert",
        "malformed",
        "no-signed-attrs",
        "signer-id",
        "time-changed",
        "outdated-security",
        "not-for-signing",
        "unsupported-ca-rule",
        "content-type",
    ] {
        built.push(verify::notes(token, None));
    }
    built.push(verify::notes("no-signing-time weak-digest digest", None));
    for (memory, close) in [(false, true), (true, true), (false, false), (true, false)] {
        let action = if close {
            redact::WindowAction::Close
        } else {
            redact::WindowAction::Open
        };
        let q = redact::pending_question(2, memory, action);
        built.push(q.message);
        built.push(q.detail);
        built.extend(q.buttons);
    }
    for pages in [vec![0], vec![0, 2]] {
        let q = redact::flatten_question(&pages);
        built.extend([q.heading, q.text, q.yes]);
    }
    for extra in [
        redact::BannerExtra::default(),
        redact::BannerExtra {
            flattened: Some(vec![0, 2]),
            unsigned: true,
            usage_rights: true,
            over_redacted: true,
        },
        redact::BannerExtra {
            flattened: Some(vec![3]),
            unsigned: true,
            ..Default::default()
        },
    ] {
        built.push(redact::banner_text(name, 2, &extra));
    }
    for reason in [
        "too-large",
        "read-only",
        "no-match",
        "verify-failed",
        "needs-flatten",
        "signed",
        "scan-budget",
        "too-many",
    ] {
        for detail in [None, Some("too-complex"), Some("raw")] {
            for hidden_kept in [false, true] {
                built.extend(redact::refusal_text(
                    Some(reason),
                    &[0, 1],
                    detail,
                    hidden_kept,
                ));
            }
        }
    }
    for action in [
        redact::PendingAction::Save,
        redact::PendingAction::Export,
        redact::PendingAction::Print,
        redact::PendingAction::Sign,
    ] {
        for memory in [false, true] {
            let w = redact::pending_words(action, memory);
            built.push(w.detail.to_string());
            built.push(w.go.to_string());
        }
        built.push(redact::cancelled(action));
    }
    for b in [
        filled::message_box(name),
        sign::not_for_signing(),
        main::discard(),
    ] {
        built.push(b.message);
        built.extend(b.detail);
        built.extend(b.buttons);
    }
    out.extend(built);
    out
}

// ------------------------------------------------------------------ X7: userText

// was tests/p1-ux.test.mjs: 'final check 4: only counts of a mebibyte or more become MB'
#[test]
fn only_a_mebibyte_or_more_becomes_mb() {
    assert_eq!(
        user_text("save: 1000000 bytes is over the 1048575 byte ceiling"),
        "1000000 bytes is over the 1048575 byte limit"
    );
    assert_eq!(
        user_text("save: 1048576 bytes is over the 1048576 byte ceiling"),
        "1 MB is over the 1 MB limit"
    );
}

// was tests/p1-ux.test.mjs: 'final check 5: the catalogue's fixed texts are never evicted from what userText leaves alone'
#[test]
fn fixed_texts_are_never_evicted() {
    let _memory = memory();
    let first = save::saved("first.pdf");
    for i in 0..400 {
        save::saved(&format!("f{i}.pdf"));
    }
    assert!(
        !is_catalogue_text(&first),
        "old produced texts are evicted (the set is bounded)"
    );
    assert!(is_catalogue_text(&save::saved("f399.pdf")));
    for text in [
        open::SAMPLE.to_string(),
        save::reason("too-large").unwrap(),
        sign::CHOOSE_CERTIFICATE.to_string(),
        banner::REPAIRED.to_string(),
    ] {
        assert!(is_catalogue_text(&text), "{text}");
    }
}

// was tests/p1-ux.test.mjs: 'X7: no catalogue message carries an internal name, a byte count or jargon'
// The TS walked the MSG object; here every catalogue entry is listed (with more argument shapes).
#[test]
fn no_catalogue_message_carries_internal_words() {
    let texts = catalogue_texts();
    assert!(texts.len() > 60, "walked {} texts", texts.len());
    for text in &texts {
        for word in words(text) {
            assert!(
                !matches!(word, "MiB" | "byte" | "bytes"),
                "a byte count: {text}"
            );
        }
        for jargon in [
            "refused",
            "Identity-H",
            "copy-on-write",
            "incremental",
            "show-text",
            "XFA",
            "DocMDP",
            "window.pdf4all",
            "fixture",
        ] {
            assert!(!text.contains(jargon), "{jargon}: {text}");
        }
        assert!(!has_p_flag(text), "a permission flag: {text}");
        assert!(!has_op_prefix(text), "an operation prefix: {text}");
        assert!(!has_error_token(text), "an error token: {text}");
    }
    // Every error that is a full sentence says what to do next, not only what happened.
    for (reason, text) in save::reason_table() {
        if reason == "cancelled" {
            continue;
        }
        let lower = text.to_lowercase();
        let two_sentences = text.ends_with('.')
            && text
                .char_indices()
                .any(|(i, c)| c == '.' && text[i + 1..].starts_with(' ') && i + 3 < text.len());
        assert!(
            two_sentences
                || ["choose", "use ", "wait", "remove", "save ", "try again"]
                    .iter()
                    .any(|next| lower.contains(next)),
            "{reason}: {text}"
        );
    }
}

// ------------------------------------------------------------------ X8: notice kinds

// was tests/p1-ux.test.mjs: 'X8: success and information are never drawn red; STATE error never holds a success'
// The CSS half (#error[data-kind="success"] colours) is the Leptos UI's now: crates/tandem-ui/styles.css
// `.notice.success` / `.notice.info` (checked below); the renderer.ts greps become session behaviour.
#[test]
fn success_and_information_are_never_errors() {
    let dir = scratch("x8");
    let mut s = session_in(&dir);
    s.show_message("Saved a.pdf.", MessageKind::Success);
    let state = s.state_summary();
    assert_eq!(
        state["error"],
        json!(""),
        "STATE error never holds a success"
    );
    assert_eq!(state["message"]["kind"], json!("success"));
    let notice = s.ui_state().view.notice.expect("the notice is shown");
    assert_eq!(notice.kind, "success");
    s.show_message("Nothing to do.", MessageKind::Info);
    let state = s.state_summary();
    assert_eq!(state["error"], json!(""));
    assert_eq!(state["message"]["kind"], json!("info"));
    s.show_error("open: claims.pdf is gone");
    let state = s.state_summary();
    assert_eq!(
        state["error"],
        json!("open: claims.pdf is gone"),
        "STATE keeps the raw error"
    );
    assert_eq!(state["message"]["kind"], json!("error"));
    assert_eq!(state["message"]["text"], json!("Claims.pdf is gone"));
    assert_eq!(s.ui_state().view.notice.unwrap().kind, "error");

    // The late save that reopened the file is a success; one that kept newer edits is information.
    let hello = std::fs::read(fixture("hello.pdf")).unwrap();
    s.dirty = false;
    s.show_late_save("late.pdf", hello, "app://pdf4all/open/late");
    let state = s.state_summary();
    assert_eq!(state["error"], json!(""));
    assert_eq!(state["message"]["kind"], json!("success"));
    assert_eq!(
        state["message"]["text"],
        json!(save::late_shown("late.pdf"))
    );
    s.dirty = true;
    s.show_late_save("late.pdf", Vec::new(), "app://pdf4all/open/late");
    let state = s.state_summary();
    assert_eq!(state["error"], json!(""));
    assert_eq!(state["message"]["kind"], json!("info"));
    drop(s);
    let _ = std::fs::remove_dir_all(dir);

    // The UI draws only `error` in the danger colours.
    let css = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tandem-ui/styles.css"),
    )
    .unwrap();
    for kind in ["success", "info", "warning"] {
        let selector = format!(".notice.{kind} {{");
        let at = css
            .find(&selector)
            .unwrap_or_else(|| panic!("no {selector}"));
        let rule = &css[at..at + css[at..].find('}').unwrap()];
        assert!(!rule.contains("danger"), "{kind} is drawn red: {rule}");
    }
    assert!(
        css.contains(".notice.error { background: var(--danger-soft); color: var(--danger); }")
    );
}

// ------------------------------------------------------------------ the native questions

// was tests/p1-ux.test.mjs: 'review fix 5: the filled-form question defaults to a new file; Esc cancels'
// C (catalogue default only): messages::tests::other_sections. Also pins that the box the host
// really shows (host::filled_box) says the same as the catalogue.
#[test]
fn filled_form_question_defaults_to_a_new_file() {
    let b = filled::message_box("form.pdf");
    assert_eq!(b.buttons[b.default_id.unwrap()], "Save as New File…");
    assert_eq!(b.buttons[b.cancel_id.unwrap()], "Cancel");
    assert_eq!(
        b.buttons[0], "Save Over",
        "main reads answer 0 as Save Over"
    );
    assert_eq!(b.message, "Save over “form.pdf” or as a new file?");
    let shown = host::filled_box("form.pdf");
    assert_eq!(shown.question, Question::Filled);
    assert_eq!(
        (shown.message, shown.detail, shown.buttons),
        (b.message, b.detail, b.buttons)
    );
    assert_eq!(
        (shown.default, shown.cancel),
        (b.default_id.unwrap(), b.cancel_id.unwrap())
    );
}

// was tests/p1-ux.test.mjs: 'review fix 11: a failed signing says why in plain words, by reason'
#[test]
fn a_failed_signing_says_why() {
    let written = "The signed copy could not be written. Try again or choose another folder.";
    for reason in [
        "invalid",
        "certificate-expired",
        "unreadable",
        "unsupported-key",
    ] {
        assert_ne!(sign::failed(reason, None), written, "{reason}");
    }
    assert!(sign::failed("invalid", None).contains("date and time"));
    let expired = sign::failed(
        "certificate-expired",
        Some("this certificate expired on 2026-01-01"),
    );
    let at = expired
        .find("expired on 2026-01-01")
        .expect("the status is named");
    assert!(
        expired[at..].contains("Choose another certificate"),
        "{expired}"
    );
    assert!(sign::failed("unreadable", None).contains("certificate"));
    assert!(sign::failed("permission", None).contains("read-only"));
    assert_eq!(sign::failed("something-new", None), written);
}

// was tests/integration.test.mjs: 'Phase 2 certificate sentences and verifier words go through the catalogue'
// Partly C: messages::tests::other_sections (failed too-costly, notes with words, trust with an
// empty issuer); the rest is here, plus the host's own not-for-signing box.
#[test]
fn certificate_sentences_and_verifier_words() {
    assert!(sign::certificate("unsupported-key", None).contains("P-256 or P-384"));
    assert!(sign::certificate("too-costly", None).contains("takes too long to open"));
    assert_eq!(
        sign::failed("too-costly", None),
        sign::certificate("too-costly", None)
    );
    assert_eq!(
        verify::notes("no-signing-time", Some("no date given")),
        "no date given",
        "the verifier's own words win"
    );
    assert_eq!(
        verify::notes("time-changed", None),
        "the date was edited after signing"
    );
    assert_eq!(verify::trust("untrusted", None), "not in your trusted list");
    assert_eq!(verify::trust("none", None), "not in your trusted list");
    assert_eq!(
        verify::trust("trusted", Some("P0B Test CA")),
        "trusted by P0B Test CA"
    );
    let b = sign::not_for_signing();
    assert_eq!(
        b.buttons[b.default_id.unwrap()],
        "Cancel",
        "Return never signs with it"
    );
    let shown = host::sign::not_for_signing_box();
    assert_eq!(shown.question, Question::NotForSigning);
    assert_eq!(shown.buttons[shown.default], "Cancel");
    assert_eq!(shown.buttons[shown.cancel], "Cancel");
    assert_eq!((shown.message, shown.buttons), (b.message, b.buttons));
}

// ------------------------------------------------------------------ redaction words

// was tests/redact-box.test.mjs: 'redact-ui words follow §2.3 and §3.2 (plain, counted, MB not MiB)'
// Partly C: messages::tests::redaction_words. The flatten question's yes button is "Turn … into
// an image" in the Rust app (was "Flatten page 4").
#[test]
fn redaction_words_are_plain_and_counted() {
    assert_eq!(
        redact::REDACT_CHIP,
        "Drag over anything you want removed. Esc to stop."
    );
    assert_eq!(
        redact::confirm_heading(3),
        "Permanently remove everything inside 3 boxes?"
    );
    assert_eq!(
        redact::confirm_heading(1),
        "Permanently remove everything inside 1 box?"
    );
    assert_eq!(redact::redact_button_text(3), "Redact 3 areas…");
    assert_eq!(redact::redact_button_text(1), "Redact 1 area…");
    assert_eq!(redact::redact_button_text(0), "Redact areas…");
    assert_eq!(redact::panel_heading(3), "Redactions (3)");
    assert_eq!(redact::count_text(3, 2), "3 boxes on 2 pages");
    assert_eq!(redact::count_text(0, 0), "No boxes yet");
    assert_eq!(redact::areas_text(1), "1 area");
    assert_eq!(
        redact::banner_text("contract redacted.pdf", 3, &redact::BannerExtra::default()),
        "You are viewing the redacted copy 'contract redacted.pdf'. 3 areas removed and checked."
    );
    assert_eq!(
        redact::pending_text(3),
        "3 areas are marked but not redacted yet."
    );
    assert_eq!(
        redact::pending_text(1),
        "1 area is marked but not redacted yet."
    );
    // 12 MiB = 12,582,912 bytes: stated in decimal MB, rounded down so the stated size always works.
    assert_eq!(
        redact::size_limit_text(Some(MAX_REDACT_SOURCE_BYTES as f64)),
        "Redaction works on files up to 12.5 MB in this version."
    );
    assert_eq!(
        redact::size_limit_text(None),
        redact::size_limit_text(Some(MAX_REDACT_SOURCE_BYTES as f64))
    );
    assert!(!redact::size_limit_text(None).contains("MiB"));
    assert_eq!(
        redact::refusal_text(Some("too-large"), &[], None, false).as_deref(),
        Some("Redaction works on files up to 12.5 MB in this version. This file is larger.")
    );
    assert_eq!(
        redact::refusal_text(Some("something-else"), &[], None, false),
        None
    );
    let flat = redact::flatten_question(&[3]);
    assert_eq!(
        format!("{} {}", flat.heading, flat.text),
        "Page 4 contains artwork that can't be cut precisely. Redact page 4 by turning it into an image? Its text will no longer be selectable."
    );
    assert_eq!(flat.yes, "Turn page 4 into an image");
    assert_eq!(
        redact::flatten_question(&[1, 4, 6]).yes,
        "Turn pages 2, 5 and 7 into an image"
    );
    assert_eq!(redact::pages_name(&[0, 2], true), "Pages 1 and 3");
    assert_eq!(
        redact::banner_text(
            "a redacted.pdf",
            2,
            &redact::BannerExtra {
                flattened: Some(vec![3]),
                unsigned: true,
                ..Default::default()
            }
        ),
        "You are viewing the redacted copy 'a redacted.pdf'. 2 areas removed and checked. Page 4 was turned into an image. The copy has no signatures."
    );
    // R5 decision: a raw-check failure with hidden information kept says what to do.
    let raw = redact::refusal_text(Some("verify-failed"), &[0], Some("raw"), true).unwrap();
    assert!(
        raw.starts_with("Some of the text inside your boxes is still in the document's comments, form fields or properties"),
        "{raw}"
    );
    assert!(
        raw.contains("Turn on \"Also remove hidden information, other comments and form entries\" and try again"),
        "{raw}"
    );
    assert!(
        redact::refusal_text(Some("verify-failed"), &[0], Some("raw"), false)
            .unwrap()
            .contains("final check on page 1, so nothing was saved")
    );
    assert_eq!(
        redact::sanitize_label("xmp"),
        Some("Extra document details saved by other apps")
    );
    assert_eq!(
        redact::sanitize_label("struct"),
        Some("Descriptions written for screen readers")
    );
    assert_eq!(
        redact::refusal_text(Some("verify-failed"), &[1, 3], None, false).as_deref(),
        Some(
            "The redacted copy did not pass its final check on pages 2 and 4, so nothing was saved. Try a larger box, or redact the whole page."
        )
    );
    // No internals in any switch name (X7): no XMP, no "tagged", no "annotations".
    assert_eq!(
        SANITIZE_KEYS,
        [
            "info",
            "xmp",
            "attachments",
            "outlines",
            "annots",
            "links",
            "fields",
            "layers",
            "struct"
        ]
    );
    let label_keys: Vec<&str> = redact::SANITIZE_LABELS.iter().map(|(k, _)| *k).collect();
    assert_eq!(
        label_keys, SANITIZE_KEYS,
        "the labels follow the switch order"
    );
    for key in SANITIZE_KEYS {
        let label = redact::sanitize_label(key).unwrap();
        for internal in ["XMP", "Tagged", "tagged", "annotation", "MiB"] {
            assert!(!label.contains(internal), "{key}: {label}");
        }
    }
}

// was tests/integration.test.mjs: 'the redaction words for shared content and Adobe Reader usage rights'
// The renderer.ts grep (usage rights only when the engine says so) becomes Session::redact_usage_rights.
// The Rust engine's open reply has no `usageRights`, so the TS's second clause has no counterpart.
#[test]
fn redaction_words_for_shared_content_and_usage_rights() {
    let over = "Some content outside your boxes was also removed, because it is shared with other parts of the document. Check the copy.";
    assert_eq!(redact::OVER_REDACTED, over);
    assert_eq!(
        redact::banner_text(
            "a.pdf",
            2,
            &redact::BannerExtra {
                over_redacted: true,
                ..Default::default()
            }
        ),
        format!("You are viewing the redacted copy 'a.pdf'. 2 areas removed and checked. {over}")
    );
    assert!(!redact::banner_text("a.pdf", 2, &redact::BannerExtra::default()).contains("shared"));
    assert_eq!(
        redact::saved("a.pdf", true),
        format!("Saved the redacted copy a.pdf. {over}")
    );
    assert!(
        redact::banner_text(
            "a.pdf",
            1,
            &redact::BannerExtra {
                unsigned: true,
                usage_rights: true,
                ..Default::default()
            }
        )
        .ends_with("no longer has Adobe Reader's special permissions.")
    );
    assert!(
        redact::banner_text(
            "a.pdf",
            1,
            &redact::BannerExtra {
                unsigned: true,
                ..Default::default()
            }
        )
        .ends_with("The copy has no signatures.")
    );
    assert_eq!(
        format!(
            "{} {}",
            redact::USAGE_RIGHTS_QUESTION.heading,
            redact::USAGE_RIGHTS_QUESTION.text
        ),
        "This file has special permissions from Adobe Reader. Redacting removes them. Make a redacted copy without them? The original keeps them."
    );
    // The Reader wording is used only when the engine says so.
    let dir = scratch("usage");
    let s = session_in(&dir);
    assert!(s.redact_usage_rights(&json!({"reason": "signed", "detail": "usage-rights"})));
    assert!(!s.redact_usage_rights(&json!({"reason": "signed", "detail": "timestamp"})));
    assert!(!s.redact_usage_rights(&json!({"reason": "signed"})));
    drop(s);
    let _ = std::fs::remove_dir_all(dir);
}

// was tests/redact-box.test.mjs: 'the redaction dialog and Help say MB, keep the nine switches and drop the stale alternate-text warning'
// The dialog and Help are the Leptos UI's now (crates/tandem-ui/src/dialogs.rs); the switches
// start on (Session::redact_ui_show_confirm sets all nine) and their labels must be the catalogue's.
// Also the index.html half of tests/integration.test.mjs: 'engine c21cec8: the links switch names
// document scripts; a copy too complex to check says so' (the links label in the dialog).
#[test]
fn redaction_dialog_and_help_say_mb() {
    let ui = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tandem-ui/src/dialogs.rs"),
    )
    .unwrap();
    assert!(ui.contains(
        "Also remove hidden information, other comments and form entries (author, document properties, attachments, bookmarks…)"
    ));
    assert!(ui.contains("\"Recommended\""));
    assert!(ui.contains("label=\"Redact and save as…\""));
    for (key, label) in redact::SANITIZE_LABELS {
        assert!(
            ui.contains(&format!("id=\"redact-sanitize-{key}\" label=\"{label}\"")),
            "the dialog's {key} switch says the catalogue's words"
        );
    }
    assert!(
        !ui.contains("not removed by a rect mark"),
        "the stale warning is gone (§2.4)"
    );
    assert!(!ui.contains("redact-inline-alt"));
    assert!(!ui.contains("MiB"), "no MiB in the dialogs or Help");
    // Help states the same limit as the catalogue.
    let limit = redact::size_limit_text(None);
    let mb = limit
        .strip_prefix("Redaction works on files up to ")
        .and_then(|rest| rest.strip_suffix(" in this version."))
        .unwrap();
    assert!(
        ui.contains(&format!("Files up to {mb} can be redacted.")),
        "Help says {mb}"
    );
    // All nine switches start on when the dialog opens.
    let dir = scratch("dialog");
    let mut s = session_in(&dir);
    s.redact_ui_show_confirm(&Default::default());
    let on = s.redact_ui_sanitize();
    assert_eq!(on, tandem_app::session::redact_ui::sanitize_all(true));
    drop(s);
    let _ = std::fs::remove_dir_all(dir);
}

// was tests/redact-box.test.mjs: 'the pending-boxes question names each way out'
// and tests/integration.test.mjs: '"Sign anyway" says what happens to the boxes'
#[test]
fn pending_boxes_question_names_each_way_out() {
    use redact::PendingAction::*;
    assert_eq!(redact::pending_words(Export, false).go, "Export anyway");
    assert_eq!(redact::pending_words(Print, false).go, "Print anyway");
    assert_eq!(redact::pending_words(Sign, false).go, "Sign anyway");
    assert_eq!(redact::pending_words(Save, false).go, "Save marks only");
    assert_eq!(
        redact::pending_words(Save, true).go,
        "Save without the boxes"
    );
    assert_eq!(
        redact::pending_words(Sign, true).detail,
        "These boxes are kept only while the file is open. The signed copy will not have them, and it opens in place of this file."
    );
    assert_eq!(redact::pending_words(Sign, true).go, "Sign anyway");
    assert!(
        redact::pending_words(Sign, false)
            .detail
            .contains("keeps the boxes as marks")
    );
}

// was tests/redact-box.test.mjs: 'main asks before unsaved boxes are lost; a signed file's boxes say they live only while it is open'
// Buttons are sentence case in the Rust app. Partly C: host::tests::pending_boxes_and_discard_in_a_window
// (the host's box). Also pins that the host's box and the catalogue agree.
#[test]
fn pending_boxes_question_before_boxes_are_lost() {
    let kept = redact::pending_question(3, false, redact::WindowAction::Close);
    assert_eq!(kept.message, "3 areas are marked but not redacted yet.");
    assert_eq!(
        kept.buttons,
        ["Redact now…", "Close without redacting", "Cancel"]
    );
    assert!(
        kept.detail
            .starts_with("Closing now discards the boxes and any other unsaved changes")
    );
    let memory = redact::pending_question(1, true, redact::WindowAction::Open);
    assert_eq!(memory.message, "1 area is marked but not redacted yet.");
    assert_eq!(
        memory.buttons,
        ["Redact now…", "Discard the boxes", "Cancel"]
    );
    assert!(memory.detail.starts_with(
        "These boxes are kept only while the file is open. Opening another file now discards them."
    ));
    for count in [1, 3] {
        for memory in [false, true] {
            for close in [false, true] {
                let action = if close {
                    redact::WindowAction::Close
                } else {
                    redact::WindowAction::Open
                };
                let words = redact::pending_question(count, memory, action);
                let shown = host::pending_question(count as u32, memory, close);
                assert_eq!(shown.question, Question::PendingBoxes);
                assert_eq!(shown.message, words.message);
                assert_eq!(shown.detail.as_deref(), Some(words.detail.as_str()));
                assert_eq!(shown.buttons, words.buttons);
                assert_eq!(
                    (shown.default, shown.cancel),
                    (0, 2),
                    "Return redacts; Esc cancels"
                );
            }
        }
    }
}

/// A window's platform: queued message-box answers, the questions asked recorded.
#[derive(Default)]
struct Asked {
    replies: Vec<usize>,
    asked: Vec<Question>,
}

struct Window(Arc<Mutex<Asked>>);

impl Platform for Window {
    fn pick_file(&mut self, _pick: Pick, _filters: &[Filter]) -> Option<PathBuf> {
        None
    }
    fn pick_images(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn pick_folder(&mut self) -> Option<PathBuf> {
        None
    }
    fn pick_save(
        &mut self,
        _purpose: SavePurpose,
        _default_path: &Path,
        _filters: &[Filter],
    ) -> Option<PathBuf> {
        None
    }
    fn ask(&mut self, question: &MessageBox) -> usize {
        let mut a = self.0.lock().unwrap();
        a.asked.push(question.question);
        if a.replies.is_empty() {
            question.cancel
        } else {
            a.replies.remove(0)
        }
    }
    fn open_external(&mut self, _uri: &str) {}
    fn print(&mut self, _pdf: &[u8], _job_name: &str) -> PrintOutcome {
        PrintOutcome::NoPrinter
    }
    fn add_recent_document(&mut self, _path: &Path) {}
    fn clear_recent_documents(&mut self) {}
    fn set_represented_file(&mut self, _path: Option<&Path>, _title: &str) {}
    fn set_document_state(&mut self, _edited: bool, _save: bool, _save_as: bool) {}
}

// was tests/integration.test.mjs: 'main asks about boxes before the signed copy opens only when they are a signed file's boxes in memory'
// The pure rule: C by session::redact_ui::tests::boxes_on_open (count 0 added here). The main.ts
// grep (deliverOpen uses it) becomes the host's real open in a window: Host::deliver_open with
// skip_discard (the signed copy's reopen) asks only for memory boxes.
#[test]
fn signed_copy_open_asks_only_for_memory_boxes() {
    use tandem_app::session::redact_ui::asks_boxes_on_open;
    assert!(!asks_boxes_on_open(false, None), "no boxes, no question");
    assert!(!asks_boxes_on_open(false, Some((0, false))));
    assert!(
        asks_boxes_on_open(false, Some((2, false))),
        "opening another file asks"
    );
    assert!(
        !asks_boxes_on_open(true, Some((2, false))),
        "the signed copy keeps marks"
    );
    assert!(
        asks_boxes_on_open(true, Some((2, true))),
        "the signed copy does not have memory boxes"
    );

    let dir = scratch("boxes-open");
    let path = dir.join("hello.pdf");
    std::fs::copy(fixture("hello.pdf"), &path).unwrap();
    let rec = Arc::new(Mutex::new(Asked::default()));
    let mut host = Host::new(
        HostConfig {
            user_data: dir.join("udata"),
            smoke: false,
            echo_stdout: false,
        },
        Box::new(Window(rec.clone())),
    );
    let opened = |events: Vec<Event>| matches!(events.as_slice(), [Event::Document(_)]);
    host.deliver_open(Some(&path), false);
    assert!(opened(host.take_events()));
    // The signed copy over marks: no question, it opens.
    host.set_redact_pending(2, false);
    host.deliver_open(Some(&path), true);
    assert!(opened(host.take_events()));
    assert!(rec.lock().unwrap().asked.is_empty());
    // The signed copy over a signed file's boxes in memory: asked; Cancel keeps the window.
    host.set_redact_pending(2, true);
    host.deliver_open(Some(&path), true);
    assert!(matches!(
        host.take_events().as_slice(),
        [Event::OpenError(e)] if e.reason == "cancelled"
    ));
    assert_eq!(rec.lock().unwrap().asked, vec![Question::PendingBoxes]);
    // "Discard the boxes" opens it.
    rec.lock().unwrap().replies = vec![1];
    host.deliver_open(Some(&path), true);
    assert!(opened(host.take_events()));
    assert_eq!(
        rec.lock().unwrap().asked,
        vec![Question::PendingBoxes, Question::PendingBoxes]
    );
    drop(host);
    let _ = std::fs::remove_dir_all(dir);
}

// ------------------------------------------------------------------ Verify's dates

// was tests/integration.test.mjs: 'Verify shows the signing time as a plain local date; STATE keeps the ISO time'
// The pure formatting: C by messages::tests::signed_on_formats_like_intl (en-US and en-GB in UTC;
// the Rust port has no time-zone database, so the TS's America/New_York case cannot be ported).
// The renderer.ts greps become a real Verify of fixtures/p0b/signed-intact.pdf in a session.
#[test]
fn verify_shows_a_plain_date_and_state_keeps_the_iso_time() {
    let _memory = memory();
    let iso = "2026-10-19T06:36:20.000Z";
    for missing in [None, Some(""), Some("not a date")] {
        assert_eq!(verify::signed_on(missing, true, false, None), "No date");
    }
    assert_eq!(
        verify::signed_on(Some(iso), true, true, None),
        "No date",
        "a \"no-signing-time\" verdict has no date to show"
    );
    let local = verify::signed_on(Some(iso), true, false, None);
    assert!(
        local.starts_with("Dated ") && local.contains("2026"),
        "{local}"
    );
    assert!(!local.contains("T06:36") && !local.ends_with('Z'));
    assert_eq!(user_text(&local), local);

    let dir = scratch("verify");
    let mut s = session_in(&dir);
    let bytes = std::fs::read(fixture("p0b/signed-intact.pdf")).unwrap();
    s.open_document("signed-intact.pdf", bytes, None, false);
    s.refresh_verify();
    let state = s.state_summary();
    let signatures = state["verify"]["last"]["signatures"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(!signatures.is_empty(), "verified: {state:#}");
    let time = signatures[0]["signingTime"]
        .as_str()
        .expect("STATE keeps the verifier's signing time")
        .to_string();
    assert!(
        time.len() >= 20 && time.as_bytes()[10] == b'T',
        "an ISO time: {time}"
    );
    let rows = &s.lane5.verify_rows;
    assert_eq!(rows.len(), signatures.len());
    let date = rows[0].cells[2].clone();
    assert_ne!(date, "No date");
    assert!(
        !date.contains(&time) && !date.contains('T') && !date.ends_with('Z'),
        "{date}"
    );
    assert!(date.contains(&time[..4]), "the year is shown: {date}");
    assert_eq!(user_text(&date), date);
    drop(s);
    let _ = std::fs::remove_dir_all(dir);
}

// ------------------------------------------------------------------ passwords

// was tests/p2-encrypt.test.mjs: 'passwords never appear in a reply or in a renderer log line'
// (only the renderer.ts half: every console line, and the P2_EDIT security slice names only
// mode/algorithm/permissions/auth). The engine-reply half stays in that JS test.
#[test]
fn passwords_never_reach_a_log_line_or_state() {
    const USER: &str = "pdf4all-user";
    const OWNER: &str = "pdf4all-owner";
    const NEW_USER: &str = "gate-user-9f3";
    const NEW_OWNER: &str = "gate-owner-9f3";
    let dir = scratch("passwords");
    let mut seen: Vec<Value> = Vec::new();

    // A new password set on hello.pdf (the smoke's encrypt path logs P2_EDIT).
    let mut s = session_in(&dir.join("a"));
    s.open_document(
        "hello.pdf",
        std::fs::read(fixture("hello.pdf")).unwrap(),
        None,
        false,
    );
    s.smoke_encrypt(Some(&format!(
        r#"{{"user": "{NEW_USER}", "owner": "{NEW_OWNER}", "permissions": ["print"]}}"#
    )));
    let edit = s.edit_summary();
    let security = edit["security"]
        .as_object()
        .expect("P2_EDIT has the security slice");
    let mut keys: Vec<&str> = security.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["algorithm", "auth", "mode", "permissions"]);
    seen.push(edit);
    seen.push(s.state_summary());
    seen.push(serde_json::to_value(s.ui_state()).unwrap());
    let lines = std::mem::take(&mut s.lane1.lines);
    assert!(
        lines.iter().any(|(prefix, _)| prefix == "P2_EDIT"),
        "{lines:?}"
    );
    seen.extend(
        lines
            .into_iter()
            .map(|(prefix, payload)| json!([prefix, payload])),
    );
    drop(s);

    // The password-protected fixture, unlocked through the password form.
    let mut s = session_in(&dir.join("b"));
    s.open_document(
        "encrypted.pdf",
        std::fs::read(fixture("encrypted.pdf")).unwrap(),
        None,
        false,
    );
    s.on_password_submit(USER);
    assert!(
        s.doc.as_ref().is_some_and(|d| !d.info.needs_password),
        "unlocked"
    );
    s.log_edit();
    seen.push(s.edit_summary());
    seen.push(s.state_summary());
    seen.push(serde_json::to_value(s.ui_state()).unwrap());
    let lines = std::mem::take(&mut s.lane1.lines);
    seen.extend(
        lines
            .into_iter()
            .map(|(prefix, payload)| json!([prefix, payload])),
    );
    drop(s);

    let wire = serde_json::to_string(&seen).unwrap();
    for secret in [NEW_USER, NEW_OWNER, USER, OWNER] {
        assert!(!wire.contains(secret), "a line or STATE carried {secret}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
