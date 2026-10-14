// SPDX-License-Identifier: AGPL-3.0-or-later
//! Measure edit-history costs: `history_cost_probe EDITS FILE...` prints one
//! JSON line per file. `wholeSnapshotBytes` is what the earlier design retained
//! (every node's full bytes up to the current one); `retainedBytes` is what the
//! delta history retains. Times are wall-clock milliseconds in this process.
use std::time::Instant;
use tandem_pdf_engine::session::{Session, SessionLimits};

fn main() {
    let mut args = std::env::args().skip(1);
    let edits: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(10);
    for path in args {
        let input = std::fs::read(&path).unwrap();
        let limits = SessionLimits {
            max_history_bytes: usize::MAX,
            ..SessionLimits::default()
        };
        let ms = |start: Instant| start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let Ok(mut s) = Session::open(input.clone(), limits) else {
            println!("{{\"file\":{path:?},\"skipped\":\"strict open refused\"}}");
            continue;
        };
        let open_ms = ms(start);
        let Some(page) = (0..s.capabilities().pages.len())
            .find(|&p| s.text_targets(p).is_ok_and(|t| !t.is_empty()))
        else {
            println!("{{\"file\":{path:?},\"skipped\":\"no text targets\"}}");
            continue;
        };
        let mut whole = input.len();
        let (mut steps, mut commit_ms) = (Vec::new(), Vec::new());
        for step in 0..edits {
            let target = s.text_targets(page).unwrap().remove(0);
            let replacement: String = target.text.chars().rev().collect();
            let replacement = if replacement == target.text {
                String::new()
            } else {
                replacement
            };
            let start = Instant::now();
            if s.replace_text(&target, &replacement).is_err() {
                break;
            }
            commit_ms.push(ms(start));
            whole += s.bytes().len();
            steps.push(format!(
                "{{\"step\":{},\"bytes\":{},\"wholeSnapshotBytes\":{whole},\"retainedBytes\":{}}}",
                step + 1,
                s.bytes().len(),
                s.retained_history_bytes()
            ));
        }
        let (mut undo_ms, mut redo_ms) = (Vec::new(), Vec::new());
        while s.state().can_undo {
            let start = Instant::now();
            s.undo().unwrap();
            undo_ms.push(ms(start));
        }
        assert_eq!(s.bytes(), input.as_slice());
        while s.state().can_redo {
            let start = Instant::now();
            s.redo().unwrap();
            redo_ms.push(ms(start));
        }
        let max = |v: &[f64]| v.iter().copied().fold(0.0, f64::max);
        println!(
            "{{\"file\":{path:?},\"inputBytes\":{},\"page\":{page},\"edits\":{},\"openMs\":{open_ms:.1},\"maxCommitMs\":{:.1},\"maxUndoMs\":{:.1},\"maxRedoMs\":{:.1},\"steps\":[{}]}}",
            input.len(),
            steps.len(),
            max(&commit_ms),
            max(&undo_ms),
            max(&redo_ms),
            steps.join(",")
        );
    }
}
