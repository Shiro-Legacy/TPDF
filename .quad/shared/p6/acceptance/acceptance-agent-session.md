# Acceptance — agent-session (2026-09-25, fable51)

**Decision: accepted on the non-GUI gate; the real-keystroke proof `checks/shot-agent.mjs` (D8, ⌘N → Untitled.pdf, PNG `review-out/agent-new.png`) is DEFERRED until the human is away.**

Unit: contract `.quad/shared/p6/contract-agent-session.md` D1–D8. Build: one Opus 5.5 implementer via the Agent tool (`agent-session-build.md`, 7 declared departures, all grounded: Help pins forbid a 12th matrix row; STATE has `doc.name`/`doc.pageCount`/`pages.count`, no page-size slice; content list carries no text so `find` is the check; `close` is never a step; `text-add` needs no mode). Review: one Opus 5.5 reviewer, refute-first (`review-agent-session.md`, 13 findings: 4 fixed — help topic regex injection, doc traps added (silent no-op on unmatched target/unknown cmd, drive-mode discard, session restart after timeout, the destructive `content {}` self-test), test cleanup on failure, new test (f) for queued steps + secret redaction; 6 left as leftovers; 3 info).

Gate `.quad/scratch/coded/accept-unit-nogui.sh agent-session agent-session shot-agent.mjs 'agent-*.png' tests/agent-session.test.mjs 584` → `acceptance/accept-agent-session-nogui.out`: tsc-ok, build-ok, suite 591/590/0/1 (need ≥ 590, 6 new), red-before log 5 not ok, agent-session drive ×3 green, 23 existing drives green (26 × exit 0, 0 red), secrets none, fixtures.sha256 9 OK, `agent-session-hash-after` = `hash-before` (56 lines, identical), nothing new under fixtures, no straggler Electron, package-ok (asar 09:39). Result fail=0.

fable51's own checks: build note and `docs/agent.md` read end to end (honest; recipes verified by the reviewer through `drive.mjs --session`, 23 lines); reviewer's raw MCP exchange (initialize echo, -32700/-32600/-32602, in-order replies, EOF → exit 0, no `--drive=` left).

Backups: `.quad/backup/pre-agent-session-20260925.tgz`, `pre-agent-session-review-20260925.tgz`. Baseline after this unit: 591 tests / 590 pass / 0 fail / 1 skip.

**GUI proof (2026-09-25 09:47, human ordered "run the shot now"):** `node .quad/shared/p6/checks/shot-agent.mjs` under the lock, sandbox disabled → exit 0, `ok File ▸ New… opened a blank Untitled.pdf ["Untitled.pdf",1,1,false,""]`, `PASS all`; `review-out/agent-new.png` (111 KB) viewed: title Untitled.pdf, blank page "Page 1 of 1", Save disabled / Save As… highlighted; fixtures.sha256 9 OK before and after; no straggler Electron. Log `acceptance/agent-session-shot.log`. **Unit fully accepted.**
