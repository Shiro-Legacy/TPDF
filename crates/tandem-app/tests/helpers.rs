//! The editor's pure helpers: reading and Tab order, the page-transform round trip, gesture,
//! snap, group and duplicate rects, the clipboard chord rule and its announcements, the batch
//! output names and the redaction tool's helpers, plus the wirings the Electron tests pinned by
//! grepping renderer.ts (undo keeps the selection, a secondary click never starts an armed
//! tool, no keydown owns the clipboard chords).
//!
//! Ported from the Electron-era tests/p6-direct.test.mjs (which imported build/renderer/renderer.js
//! in a stubbed DOM), tests/p6-batch.test.mjs (build/policy.js) and tests/redact-box.test.mjs
//! (build/renderer/redact-ui.js).
// Some helpers serve only the Unix tests (modes, links).
#![cfg_attr(not(unix), allow(unused_imports, dead_code))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;
use tandem_app::host::{self, Host, HostConfig};
use tandem_app::platform::{Answers, Scripted};
use tandem_app::session::Session;
use tandem_app::session::clipboard::{
    ActiveElement, ChordContext, clipboard_chord_ok, cut_announcement, paste_announcement,
};
use tandem_app::session::direct::{map_rect, turn_rect, union_rect};
use tandem_app::session::dom_ids;
use tandem_app::session::nav::{
    GestureOptions, editor_box, gesture_rect, pdf_box, reading_order, snap_rect, tab_order,
    view_box,
};
use tandem_app::session::object_menu::menu_duplicate_rect;
use tandem_app::session::redact_ui::{
    SANITIZE_KEYS, group_by_page, sanitize_all, sanitize_from, tint_shapes, whole_page_rect,
};
use tandem_app::session::types::{
    Axis, ObjectFocus, PageSize, RedactBox, RedactInspectRegion, SnapEdge, SnapGuide, TintKind,
    TintShape, UiMode,
};
use tandem_protocol::engine::{FieldItem, FieldKind, FieldValue, RedactSanitize};

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

/// A fresh folder of this test's own under the temp folder.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tandem-t-helpers-{}-{tag}", std::process::id()));
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

/// The TS test's `field(over)`: a text field `a` at [0, 0, 10, 10].
fn field(name: &str, rect: [f64; 4]) -> FieldItem {
    FieldItem {
        id: 1,
        name: name.into(),
        label: name.into(),
        kind: FieldKind::Text,
        value: FieldValue::Text(String::new()),
        rect: rect.to_vec(),
        ..FieldItem::default()
    }
}

fn radio(name: &str, on: &str, checked: bool, rect: [f64; 4]) -> FieldItem {
    FieldItem {
        kind: FieldKind::Radio,
        on: Some(on.into()),
        value: FieldValue::Bool(checked),
        ..field(name, rect)
    }
}

fn names(items: &[FieldItem]) -> Vec<String> {
    items.iter().map(|i| i.name.clone()).collect()
}

fn options(min: f64, lock_aspect: bool) -> GestureOptions {
    GestureOptions {
        min,
        lock_aspect,
        page: [612.0, 792.0],
        within: None,
    }
}

const ALL_EDGES: [SnapEdge; 6] = [
    SnapEdge::X0,
    SnapEdge::X1,
    SnapEdge::Cx,
    SnapEdge::Y0,
    SnapEdge::Y1,
    SnapEdge::Cy,
];

fn guide(axis: Axis, at: f64) -> SnapGuide {
    SnapGuide { axis, at }
}

// ------------------------------------------------------------------ tests/p6-direct.test.mjs

// was tests/p6-direct.test.mjs: 'readingOrder and editorBox match §8' (the child-process run of
// every pure helper; split by helper here). Partly C: session::nav::tests::reading_order_rows_then_columns,
// radio_groups_collapse_to_the_checked_kid, boxes_round_trip.
#[test]
fn reading_order_and_editor_box() {
    let right = field("right", [100.0, 0.0, 140.0, 20.0]);
    let left = field("left", [0.0, 2.0, 40.0, 22.0]);
    let below = field("below", [0.0, 40.0, 40.0, 60.0]);
    assert_eq!(
        names(&reading_order(&[right, below, left])),
        ["left", "right", "below"]
    );
    let small = radio("size", "S", false, [0.0, 0.0, 10.0, 10.0]);
    let medium = radio("size", "M", true, [20.0, 0.0, 30.0, 10.0]);
    let large = radio("size", "L", false, [40.0, 0.0, 50.0, 10.0]);
    let locked = FieldItem {
        read_only: true,
        ..field("fixed", [0.0, 30.0, 40.0, 50.0])
    };
    let sig = FieldItem {
        kind: FieldKind::Signature,
        ..field("sig", [0.0, 60.0, 40.0, 80.0])
    };
    let ordered = reading_order(&[large, sig, small, locked, medium]);
    assert_eq!(names(&ordered), ["size", "fixed"]);
    assert_eq!(ordered[0].on.as_deref(), Some("M"));

    let b = editor_box(&[10.0, 20.0, 110.0, 40.0], 2.0);
    assert_eq!(
        (b.left, b.top, b.width, b.height, b.font_px),
        (20.0, 40.0, 200.0, 40.0, 24.0)
    );
    assert_eq!(editor_box(&[0.0, 0.0, 100.0, 10.0], 1.0).font_px, 10.0);
    assert_eq!(editor_box(&[0.0, 0.0, 50.0, 20.0], 1.0).font_px, 12.0);
    assert_eq!(editor_box(&[0.0, 0.0, 50.0, 1000.0], 1.0).font_px, 400.0);
}

// was tests/p6-direct.test.mjs: 'readingOrder and editorBox match §8' (P6 U3 §1.1/§9.8: the page-transform round trip)
#[test]
fn view_box_and_pdf_box_round_trip() {
    let flip = [1.0, 0.0, 0.0, -1.0, 0.0, 792.0];
    assert_eq!(
        view_box(&[72.0, 600.0, 172.0, 650.0], &flip),
        [72.0, 142.0, 172.0, 192.0]
    );
    assert_eq!(
        pdf_box(&[72.0, 142.0, 172.0, 192.0], &flip).unwrap(),
        [72.0, 600.0, 172.0, 650.0]
    );
    let shifted = [1.0, 0.0, 0.0, -1.0, -10.0, 802.0]; // MediaBox [10 10 622 802]
    assert_eq!(
        view_box(&[20.0, 700.0, 70.0, 750.0], &shifted),
        [10.0, 52.0, 60.0, 102.0]
    );
    assert_eq!(
        pdf_box(&view_box(&[20.5, 700.25, 70.0, 750.0], &shifted), &shifted).unwrap(),
        [20.5, 700.25, 70.0, 750.0]
    );
    let rotated = [0.0, 1.0, 1.0, 0.0, 0.0, 0.0]; // /Rotate 90
    assert_eq!(
        view_box(&[72.0, 600.0, 172.0, 650.0], &rotated),
        [600.0, 72.0, 650.0, 172.0]
    );
    assert_eq!(
        pdf_box(&view_box(&[72.0, 600.0, 172.0, 650.0], &rotated), &rotated).unwrap(),
        [72.0, 600.0, 172.0, 650.0]
    );
    assert_eq!(
        pdf_box(
            &[0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0],
            &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        )
        .unwrap(),
        [0.0, 0.0, 0.3333, 0.6667]
    );
    let err = pdf_box(&[0.0, 0.0, 1.0, 1.0], &[0.0; 6]).unwrap_err();
    assert!(err.message.contains("not invertible"), "{}", err.message);
}

// was tests/p6-direct.test.mjs: 'readingOrder and editorBox match §8' (the pure gesture rect, §2.3/§9.18 `within`)
// Partly C: session::nav::tests::gesture_rect_rules.
#[test]
fn gesture_rects() {
    let im1 = [72.0, 142.0, 172.0, 192.0];
    assert_eq!(
        gesture_rect(&im1, "se", 20.0, 20.0, &options(1.0, true)),
        [72.0, 142.0, 212.0, 212.0]
    );
    assert_eq!(
        gesture_rect(&im1, "se", 20.0, 20.0, &options(1.0, false)),
        [72.0, 142.0, 192.0, 212.0]
    );
    assert_eq!(
        gesture_rect(&im1, "se", 1000.0, 0.0, &options(1.0, true)),
        [72.0, 142.0, 612.0, 412.0]
    );
    assert_eq!(
        gesture_rect(&im1, "nw", 150.0, 150.0, &options(1.0, true)),
        [170.0, 191.0, 172.0, 192.0]
    );
    assert_eq!(
        gesture_rect(&im1, "w", 200.0, 0.0, &options(8.0, false)),
        [164.0, 142.0, 172.0, 192.0]
    );
    assert_eq!(gesture_rect(&im1, "e", 0.0, 50.0, &options(1.0, true)), im1);
    assert_eq!(
        gesture_rect(
            &[400.0, 52.0, 460.0, 92.0],
            "grip",
            500.0,
            -500.0,
            &options(1.0, false)
        ),
        [552.0, 0.0, 612.0, 40.0]
    );
    // `within` (crop mode) replaces the page clamp and never locks the aspect.
    let im4 = [72.0, 352.0, 132.0, 392.0];
    let full = vec![72.0, 342.0, 172.0, 392.0];
    let crop = |lock_aspect| GestureOptions {
        within: Some(full.clone()),
        ..options(1.0, lock_aspect)
    };
    assert_eq!(gesture_rect(&im4, "w", -20.0, 0.0, &crop(true)), im4);
    assert_eq!(gesture_rect(&im4, "ne", 40.0, -10.0, &crop(true)), full);
    assert_eq!(
        gesture_rect(&im4, "grip", 100.0, 0.0, &crop(false)),
        [112.0, 352.0, 172.0, 392.0]
    );
}

// was tests/p6-direct.test.mjs: 'readingOrder and editorBox match §8' (P6 U4 §2.2: tabOrder per /Tabs)
#[test]
fn tab_order_per_tabs() {
    let at = |name: &str, x: f64, y: f64| field(name, [x, y, x + 120.0, y + 20.0]);
    let grid = vec![
        at("b3", 300.0, 172.0),
        at("a1", 72.0, 72.0),
        at("b1", 300.0, 72.0),
        at("a2", 72.0, 122.0),
        at("b2", 300.0, 122.0),
        at("a3", 72.0, 172.0),
    ];
    let (order, used) = tab_order(&grid, Some("C"));
    assert_eq!(names(&order), ["a1", "a2", "a3", "b1", "b2", "b3"]);
    assert_eq!(used, "C");
    let (rows, used) = tab_order(&grid, Some("R"));
    assert_eq!(names(&rows), ["a1", "b1", "a2", "b2", "a3", "b3"]);
    assert_eq!(used, "R");
    assert_ne!(names(&rows), names(&grid));
    assert_eq!(tab_order(&grid, None), (rows, "R"));
    let row = vec![
        at("t3", 368.0, 72.0),
        at("t1", 72.0, 72.0),
        at("t2", 220.0, 72.0),
    ];
    assert_eq!(names(&tab_order(&row, Some("A")).0), ["t3", "t1", "t2"]);
    assert_eq!(tab_order(&row, Some("W")).1, "W");
    let indexed = |name: &str, x: f64, index: Option<i64>| FieldItem {
        struct_index: index,
        ..at(name, x, 72.0)
    };
    let tagged = vec![
        indexed("s1", 72.0, Some(2)),
        indexed("s2", 220.0, Some(0)),
        indexed("s3", 368.0, Some(1)),
        at("s4", 72.0, 200.0),
    ];
    let (order, used) = tab_order(&tagged, Some("S"));
    assert_eq!(names(&order), ["s2", "s3", "s1", "s4"]);
    assert_eq!(used, "S");
    let bare = vec![at("u2", 220.0, 72.0), at("u1", 72.0, 72.0)];
    assert_eq!(
        tab_order(&bare, Some("S")),
        (vec![bare[1].clone(), bare[0].clone()], "fallback")
    );
    let kids = vec![
        FieldItem {
            kind: FieldKind::Signature,
            ..at("sig", 72.0, 300.0)
        },
        radio("g", "B", true, [300.0, 72.0, 420.0, 92.0]),
        radio("g", "A", false, [72.0, 72.0, 192.0, 92.0]),
        at("x", 72.0, 122.0),
    ];
    let stops = |order: Vec<FieldItem>| -> Vec<String> {
        order
            .iter()
            .map(|i| format!("{}{}", i.name, i.on.as_deref().unwrap_or("")))
            .collect()
    };
    for tabs in [Some("A"), Some("C"), None] {
        assert_eq!(stops(tab_order(&kids, tabs).0), ["gB", "x"], "{tabs:?}");
    }
}

// was tests/p6-direct.test.mjs: 'readingOrder and editorBox match §8' (P6 U5 §2.2/§9.25: group union, menu duplicate)
// Partly C: session::direct::tests::rect_helpers, session::object_menu::tests::nudges_and_duplicates.
#[test]
fn group_union_and_menu_duplicate() {
    let page = [612.0, 792.0];
    let union = union_rect(&[
        vec![72.0, 142.0, 172.0, 192.0],
        vec![400.0, 52.0, 460.0, 92.0],
    ]);
    assert_eq!(union, [72.0, 52.0, 460.0, 192.0]);
    // A group moves as its union: dy -80 clamps to -52.
    assert_eq!(
        gesture_rect(&union, "grip", 0.0, -80.0, &options(1.0, false)),
        [72.0, 0.0, 460.0, 140.0]
    );
    assert_eq!(
        menu_duplicate_rect(&[400.0, 52.0, 460.0, 92.0], page),
        [410.0, 62.0, 470.0, 102.0]
    );
    assert_eq!(
        menu_duplicate_rect(&[560.0, 760.0, 612.0, 792.0], page),
        [550.0, 750.0, 602.0, 782.0]
    );
    assert_eq!(
        menu_duplicate_rect(&[0.0, 0.0, 612.0, 792.0], page),
        [0.0, 0.0, 612.0, 792.0]
    );
}

// was tests/p6-direct.test.mjs: 'readingOrder and editorBox match §8' (P6 U6 §2.4/§9.19: snapRect)
// Partly C: session::nav::tests::snap_keeps_the_nearest_and_first.
#[test]
fn snap_rects() {
    let size = [612.0, 792.0];
    let im2 = vec![400.0, 42.0, 450.0, 142.0];
    let snap = |rect: [f64; 4], others: &[Vec<f64>], edges: &[SnapEdge]| {
        snap_rect(&rect, others, size, 4.0, edges)
    };
    assert_eq!(
        snap(
            [398.0, 142.0, 498.0, 192.0],
            std::slice::from_ref(&im2),
            &ALL_EDGES
        ),
        (
            vec![400.0, 142.0, 500.0, 192.0],
            vec![guide(Axis::X, 400.0), guide(Axis::Y, 142.0)]
        )
    );
    assert_eq!(
        snap(
            [102.0, 300.0, 152.0, 350.0],
            &[vec![115.0, 500.0, 135.0, 600.0]],
            &ALL_EDGES
        ),
        (
            vec![100.0, 300.0, 150.0, 350.0],
            vec![guide(Axis::X, 125.0)]
        )
    );
    assert_eq!(
        snap([3.0, 300.0, 53.0, 350.0], &[], &ALL_EDGES),
        (vec![0.0, 300.0, 50.0, 350.0], vec![guide(Axis::X, 0.0)])
    );
    assert_eq!(
        snap([4.0, 300.0, 54.0, 350.0], &[], &ALL_EDGES),
        (vec![0.0, 300.0, 50.0, 350.0], vec![guide(Axis::X, 0.0)])
    );
    assert_eq!(
        snap([4.01, 300.0, 54.01, 350.0], &[], &ALL_EDGES),
        (vec![4.01, 300.0, 54.01, 350.0], vec![])
    );
    assert_eq!(
        snap(
            [398.0, 300.0, 448.0, 350.0],
            std::slice::from_ref(&im2),
            &[SnapEdge::X1]
        ),
        (
            vec![400.0, 300.0, 450.0, 350.0],
            vec![guide(Axis::X, 450.0)]
        )
    );
    assert_eq!(
        snap(
            [398.0, 300.0, 448.0, 350.0],
            std::slice::from_ref(&im2),
            &ALL_EDGES
        ),
        (
            vec![400.0, 300.0, 450.0, 350.0],
            vec![guide(Axis::X, 400.0)]
        )
    );
    assert_eq!(
        snap(
            [398.0, 300.0, 448.0, 350.0],
            std::slice::from_ref(&im2),
            &[SnapEdge::Y0, SnapEdge::Y1]
        ),
        (vec![398.0, 300.0, 448.0, 350.0], vec![])
    );
}

// was tests/p6-direct.test.mjs: 'readingOrder and editorBox match §8' (§2.1/§2.2: turn about the union centre, map by the union affine)
// Partly C: session::direct::tests::rect_helpers.
#[test]
fn turn_and_map_rects() {
    assert_eq!(
        turn_rect(&[72.0, 142.0, 172.0, 192.0], "rotateCw", 186.0, 167.0),
        [161.0, 53.0, 211.0, 153.0]
    );
    assert_eq!(
        turn_rect(&[200.0, 142.0, 300.0, 192.0], "rotateCw", 186.0, 167.0),
        [161.0, 181.0, 211.0, 281.0]
    );
    assert_eq!(
        turn_rect(&[72.0, 142.0, 172.0, 192.0], "rotateCcw", 122.0, 167.0),
        [97.0, 117.0, 147.0, 217.0]
    );
    assert_eq!(
        turn_rect(&[72.0, 142.0, 172.0, 192.0], "flipH", 186.0, 167.0),
        [200.0, 142.0, 300.0, 192.0]
    );
    assert_eq!(
        map_rect(
            &[200.0, 142.0, 300.0, 192.0],
            &[72.0, 142.0, 300.0, 192.0],
            &[72.0, 142.0, 528.0, 192.0]
        ),
        [328.0, 142.0, 528.0, 192.0]
    );
}

// was tests/p6-direct.test.mjs: 'readingOrder and editorBox match §8' (P6 U7 §10.30: the clipboard chord rule)
// Partly C: session::clipboard::tests::chords_stay_out_of_text_entry.
#[test]
fn clipboard_chords_belong_to_the_page_selection_only() {
    let ctx = || ChordContext {
        mode: UiMode::Edit,
        layer_live: true,
        dialog_open: false,
        menu_open: false,
        editor_open: false,
        active: None,
        collapsed: true,
        selection: 1,
        clipboard: true,
        editable: true,
    };
    let active = |tag: &str, kind: Option<&str>| ActiveElement {
        tag: tag.into(),
        kind: kind.map(str::to_string),
        ..ActiveElement::default()
    };
    assert!(
        clipboard_chord_ok("copy", &ctx()),
        "#page / body focus copies"
    );
    let with = |a: ActiveElement| ChordContext {
        active: Some(a),
        ..ctx()
    };
    assert!(
        clipboard_chord_ok("copy", &with(active("BUTTON", Some("button")))),
        "a thumbnail button copies"
    );
    assert!(
        clipboard_chord_ok("copy", &with(active("INPUT", Some("radio")))),
        "a radio input copies"
    );
    assert!(
        !clipboard_chord_ok("copy", &with(active("INPUT", Some("text")))),
        "a text input keeps native copy"
    );
    assert!(!clipboard_chord_ok(
        "copy",
        &with(ActiveElement {
            inline_editor: true,
            ..active("INPUT", Some("text"))
        })
    ));
    assert!(!clipboard_chord_ok(
        "paste",
        &with(active("TEXTAREA", None))
    ));
    assert!(!clipboard_chord_ok(
        "copy",
        &with(ActiveElement {
            content_editable: true,
            ..active("DIV", None)
        })
    ));
    let not = |kind: &str, c: ChordContext| assert!(!clipboard_chord_ok(kind, &c), "{kind} {c:?}");
    not(
        "copy",
        ChordContext {
            dialog_open: true,
            ..ctx()
        },
    );
    not(
        "copy",
        ChordContext {
            collapsed: false,
            ..ctx()
        },
    );
    not(
        "copy",
        ChordContext {
            menu_open: true,
            ..ctx()
        },
    );
    not(
        "cut",
        ChordContext {
            editor_open: true,
            ..ctx()
        },
    );
    not(
        "copy",
        ChordContext {
            selection: 0,
            ..ctx()
        },
    );
    assert!(
        clipboard_chord_ok(
            "paste",
            &ChordContext {
                selection: 0,
                ..ctx()
            }
        ),
        "paste needs no selection"
    );
    not(
        "paste",
        ChordContext {
            clipboard: false,
            ..ctx()
        },
    );
    not(
        "paste",
        ChordContext {
            editable: false,
            ..ctx()
        },
    );
    not(
        "copy",
        ChordContext {
            mode: UiMode::Read,
            ..ctx()
        },
    );
    not(
        "copy",
        ChordContext {
            layer_live: false,
            ..ctx()
        },
    );
}

// was tests/p6-direct.test.mjs: 'readingOrder and editorBox match §8' (§10.33 paste and §9.10 Cut announcements)
// Partly C: session::clipboard::tests::announcements_are_exact. A pasted redaction mark is
// "redaction box" in the Rust app (UX walkthrough, commit 3bf8cc8; was "redaction mark").
#[test]
fn paste_and_cut_announcements() {
    let kinds = |pairs: &[(&str, i64)]| -> BTreeMap<String, i64> {
        pairs.iter().map(|(k, n)| (k.to_string(), *n)).collect()
    };
    let list = |names: &[&str]| -> Vec<String> { names.iter().map(|n| n.to_string()).collect() };
    assert_eq!(
        paste_announcement(&kinds(&[("image", 1)]), 1, 0, UiMode::Edit, &list(&["PA1"])),
        "Pasted image PA1"
    );
    assert_eq!(
        paste_announcement(
            &kinds(&[("image", 1), ("form", 1)]),
            2,
            0,
            UiMode::Edit,
            &list(&["PA1", "PA2"])
        ),
        "Pasted 2 objects"
    );
    assert_eq!(
        paste_announcement(&kinds(&[("link", 2)]), 1, 1, UiMode::Annotate, &[]),
        "Pasted 1 item; 1 link skipped (its page is not in this document)"
    );
    assert_eq!(
        paste_announcement(
            &kinds(&[("image", 1), ("field", 2), ("mark", 1)]),
            4,
            0,
            UiMode::Edit,
            &list(&["PA1"])
        ),
        "Pasted 4 items; 2 fields shown in Fill forms; 1 redaction box shown in Redact"
    );
    assert_eq!(
        paste_announcement(&kinds(&[("text", 1)]), 1, 0, UiMode::Forms, &[]),
        "Pasted 1 item; 1 text run shown in Edit text"
    );
    assert_eq!(
        cut_announcement(1, 1),
        "Cut 1 item; 1 text run stays (it cannot be deleted)"
    );
    assert_eq!(
        cut_announcement(2, 2),
        "Cut 2 items; 2 text runs stay (they cannot be deleted)"
    );
    assert_eq!(cut_announcement(3, 0), "Cut 3 items");
}

// was tests/p6-direct.test.mjs: 'U5 review: undo keeps the selection, Inspector follows the anchor, Escaped gesture adds no text'
// (the §9.28 part: undoEdit/redoEdit never clear the page selection). The Escape part is C by
// session::direct::tests::a_gesture_needs_three_pixels_then_commits; the Inspector part is not tested.
#[test]
fn undo_and_redo_keep_the_selection() {
    let dir = scratch("undo");
    let mut s = session_in(&dir);
    let member = ObjectFocus {
        kind: "image".into(),
        name: "Im1".into(),
        target_id: "7:Im1".into(),
        rect: vec![72.0, 142.0, 172.0, 192.0],
        ..ObjectFocus::default()
    };
    s.direct_selection = vec![member.clone()];
    s.direct_focused_object = Some(member.clone());
    s.can_undo = false;
    s.can_redo = false;
    s.undo_edit();
    assert_eq!(
        s.direct_selection,
        vec![member.clone()],
        "undo keeps the selection"
    );
    s.redo_edit();
    assert_eq!(s.direct_selection, vec![member], "redo keeps the selection");
    drop(s);
    let _ = std::fs::remove_dir_all(dir);
}

// was tests/p6-direct.test.mjs: 'U7: no keydown handler owns the clipboard chords'
// ⌘C/⌘X/⌘V reach the session only as clipboard events (the Edit menu's roles): the document
// keydown leaves them alone.
#[test]
fn no_keydown_owns_the_clipboard_chords() {
    let dir = scratch("chords");
    let mut s = session_in(&dir);
    s.open_document(
        "hello.pdf",
        std::fs::read(fixture("hello.pdf")).unwrap(),
        None,
        false,
    );
    for key in ["c", "x", "v", "C", "X", "V"] {
        for (meta, ctrl) in [(true, false), (false, true)] {
            for shift in [false, true] {
                assert!(
                    !s.on_document_keydown(key, meta, ctrl, shift, false, false, false),
                    "keydown took {key} meta={meta} ctrl={ctrl} shift={shift}"
                );
            }
        }
    }
    drop(s);
    let _ = std::fs::remove_dir_all(dir);
}

// was tests/p6-direct.test.mjs: 'U7 review: a secondary or Control-click never starts an armed area gesture'
#[test]
fn a_secondary_or_control_click_never_starts_an_armed_tool() {
    let dir = scratch("secondary");
    let mut s = session_in(&dir);
    s.open_document(
        "hello.pdf",
        std::fs::read(fixture("hello.pdf")).unwrap(),
        None,
        false,
    );
    for armed in [
        dom_ids::REDACT_AREA_BUTTON,
        dom_ids::LINK_DRAW_BUTTON,
        dom_ids::SIGFIELD_DRAW_BUTTON,
    ] {
        s.dom.set_attr(armed, "aria-pressed", "true");
        assert!(
            !s.on_page_pointerdown([100.0, 100.0], 2, false, false, false),
            "{armed}: a secondary click"
        );
        assert!(
            !s.on_page_pointerdown([100.0, 100.0], 0, true, false, false),
            "{armed}: a Control-click"
        );
        s.dom.set_attr(armed, "aria-pressed", "false");
    }
    // The primary press does start the armed tool (so the refusals above are the guard's).
    s.dom
        .set_attr(dom_ids::REDACT_AREA_BUTTON, "aria-pressed", "true");
    assert!(s.on_page_pointerdown([100.0, 100.0], 0, false, false, false));
    drop(s);
    let _ = std::fs::remove_dir_all(dir);
}

// ------------------------------------------------------------------ tests/p6-batch.test.mjs

// was tests/p6-batch.test.mjs: 'batchOutName: only listed names and report.txt, never a path'
// Partly C: host::tests::names (../a.pdf, report.txt, unlisted). A non-string name (the TS's 42)
// cannot reach the Rust function.
#[test]
fn batch_out_name_only_listed_names_and_the_report() {
    let listed: Vec<String> = vec!["a.pdf".into(), "nav.pdf".into()];
    let with = |extra: &str| -> Vec<String> {
        let mut names = listed.clone();
        names.push(extra.into());
        names
    };
    assert_eq!(host::batch_out_name("../x.pdf", &listed), None);
    assert_eq!(host::batch_out_name("a/b.pdf", &listed), None);
    assert_eq!(host::batch_out_name("a\\b.pdf", &with("a\\b.pdf")), None);
    assert_eq!(host::batch_out_name("..", &with("..")), None);
    assert_eq!(host::batch_out_name(".", &with(".")), None);
    assert_eq!(host::batch_out_name("", &with("")), None);
    assert_eq!(host::batch_out_name("x.pdf", &listed), None, "unlisted");
    assert_eq!(host::batch_out_name("nav.pdf", &listed), Some("nav.pdf"));
    assert_eq!(
        host::batch_out_name("report.txt", &listed),
        Some("report.txt")
    );
    assert_eq!(host::batch_out_name("a\0.pdf", &with("a\0.pdf")), None);
    let long = format!("{}.pdf", "x".repeat(252));
    assert_eq!(host::batch_out_name(&long, &with(&long)), None, "over 255");
}

// ------------------------------------------------------------------ tests/redact-box.test.mjs

// was tests/redact-box.test.mjs: 'redact-ui helpers: boxes by page, whole page, sanitize switches'
// Partly C: session::redact_ui::tests::groups_pages_then_top_to_bottom, sanitize_switches, tint_and_keys.
#[test]
fn redact_ui_helpers() {
    let b = |page, id, rect: [f64; 4], search| RedactBox {
        page,
        id,
        rect: rect.to_vec(),
        search,
    };
    let boxes = [
        b(2, 9, [10.0, 300.0, 50.0, 320.0], false),
        b(0, 4, [10.0, 500.0, 50.0, 520.0], true),
        b(0, 3, [200.0, 100.0, 250.0, 120.0], false),
        b(0, 5, [20.0, 100.0, 60.0, 120.0], false),
    ];
    let groups: Vec<(i64, Vec<i64>)> = group_by_page(&boxes)
        .into_iter()
        .map(|g| (g.page, g.boxes.iter().map(|b| b.id).collect()))
        .collect();
    assert_eq!(groups, [(0, vec![5, 3, 4]), (2, vec![9])]);
    assert!(group_by_page(&[]).is_empty());
    assert_eq!(
        whole_page_rect(&PageSize {
            width: 595.2756,
            height: 841.8898
        }),
        [0.0, 0.0, 595.28, 841.89]
    );
    let none = sanitize_all(false);
    assert_eq!(none, RedactSanitize::default());
    assert_eq!(
        sanitize_from(None, None),
        sanitize_all(true),
        "all on by default (§2.3 step 5)"
    );
    assert_eq!(
        sanitize_from(
            Some(&json!(false)),
            Some(&json!({"xmp": true, "bogus": true}))
        ),
        RedactSanitize {
            xmp: true,
            ..sanitize_all(false)
        }
    );
    assert_eq!(
        sanitize_from(
            Some(&json!(true)),
            Some(&json!({"struct": false, "info": "no"}))
        ),
        RedactSanitize {
            r#struct: false,
            ..sanitize_all(true)
        }
    );
    let keys = serde_json::to_value(sanitize_all(true)).unwrap();
    let keys: Vec<&str> = keys
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, SANITIZE_KEYS, "the nine switches, in order");
}

// was tests/redact-box.test.mjs: 'redact-inspect replies become tint shapes; a truncated box is tinted whole'
#[test]
fn inspect_replies_become_tint_shapes() {
    let region: RedactInspectRegion = serde_json::from_value(json!({
        "page": 0, "rect": [10, 10, 100, 40], "text": "abc", "needsFlatten": false, "truncated": false,
        "glyphs": [[10, 10, 50, 10, 50, 20, 10, 20], [1, 2, 3]],
        "images": [{"rect": [20, 20, 30, 30], "whole": true}],
        "shapes": [{"rect": [0, 0, 5, 5], "kind": "path", "whole": false}],
        "annots": [{"kind": "widget", "rect": [60, 10, 90, 30], "label": "name"}],
    }))
    .unwrap();
    let kinds: Vec<TintKind> = tint_shapes(std::slice::from_ref(&region))
        .into_iter()
        .map(|s| s.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            TintKind::Glyph,
            TintKind::Image,
            TintKind::Shape,
            TintKind::Annot
        ],
        "a malformed quad is skipped"
    );
    let truncated = RedactInspectRegion {
        truncated: true,
        ..region
    };
    assert_eq!(
        tint_shapes(&[truncated]),
        [TintShape {
            kind: TintKind::Shape,
            points: vec![10.0, 10.0, 100.0, 40.0]
        }]
    );
}
