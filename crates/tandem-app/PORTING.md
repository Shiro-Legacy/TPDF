# Porting the Electron renderer and main process into `tandem-app`

The Rust Mac app keeps the `--drive`/`--smoke` protocol and the STATE shape exactly
(tests compare STATE values). The surest way to do that is a **faithful port** of the
Electron logic: `src/renderer/renderer.ts` (with `redact-ui.ts`, `recovery.ts`,
`messages.ts`) becomes `crates/tandem-app/src/session/`, `src/main.ts` becomes
`crates/tandem-app/src/host/`. The new Leptos UI then draws from this core and calls it.

Specs written from the TypeScript (read the part for your range first):
`$WORKTREES/scratch/rust-mac/spec-drive-state.md` (CLI, drive, STATE),
`parts/A.md` (renderer 1–2768), `parts/B.md` (2768–5347), `parts/C.md` (5347–7288),
`parts/D.md` (7288–9600, forms/annots/nav/compare/export), `parts/E.md` (rest), `parts/M.md`
(main.ts, menus, IPC, messages, recovery, CSS tokens), `spec-tests.md` (the tests).
The TypeScript itself is the source of truth when a spec and the code disagree.

## Shape

- **One struct, many files.** `session::Session` holds every top-level `let` of
  `renderer.ts` (plus the module state of `redact-ui.ts` and `recovery.ts`) as a field with
  the same name in snake_case. Each part of the port is an `impl Session { … }` block in
  its own file under `src/session/`. A field a part needs and that does not exist yet goes
  in that part's own `#[derive(Default)] pub struct <Part>State` (in its file), held by
  `Session` as one field, so two parts never edit the same lines.
- **Functions keep their names**: `fooBar(a, b)` → `fn foo_bar(&mut self, a, b)` (or
  `&self` when it only reads). Same parameter order. Free helpers → free `fn`s in the same file.
- **No async.** The engine is in-process and synchronous: `await request({...})` →
  `self.engine.request(json!({...}), inputs)?` (or a `match`). `Promise.all`, gates, latest-
  guards and generation checks that only existed because of the worker collapse to plain
  sequential code; keep their *effects* (what STATE says) unchanged.
- **Engine calls**: `crate::engine::EngineHost::request(Value, Vec<Vec<u8>>) -> Result<Reply, Refusal>`.
  Binary request fields are `{"$buffer": i}` placeholders into the inputs vector; reply
  bytes come out with `reply.take_bytes("pixels")`. `Refusal` has `message`, `reason`,
  `detail`, `reply` — the renderer's `refusalOf(err)`. A data key starting with `$` must be
  built with `engine::esc_key`. Parse typed replies with the serde types in
  `tandem_protocol::engine` (`serde_json::from_value`), or read fields off the `Value`.
- **The DOM is virtual.** Where the renderer wrote an element and later read it back, or
  STATE reports it (`statusEl.textContent`, `dialog.open`, a button's `disabled`/text, an
  input's value, `aria-pressed`, `hidden`), use `self.dom` (`crate::dom::Dom`) keyed by the
  element's id in `src/renderer/index.html` (`session/dom_ids.rs` maps the TS constant names
  to ids). Pure presentation (classes, styles, canvas painting, focus, scrolling) is
  dropped, *unless* STATE or later logic reads it: e.g. `cssBox()` of an editor is computed
  from page points × zoom (see the view part), hit-layer target counts come from the
  target lists, not from DOM queries.
- **Events become methods.** A click/keydown/pointer handler becomes a method the UI and the
  drive both call (`fn on_search_input(&mut self, text)`), with the DOM event's data as
  plain arguments. Drive gestures (`click`, `drag`, `type`, `key`, `handle`) go through the
  same methods with page points.
- **Native dialogs and the main process** (open/save pickers, message boxes, saving files,
  printing, recent files, trust list, drawn signature store, opening links) are calls on
  `self.host` (`crate::host::Host`) — never `std::fs` from the session directly, except
  where the renderer itself had the bytes (it never touched the disk).
- **Messages**: `crate::messages` is the port of `messages.ts`; use it exactly as the TS
  does (`MSG.save.saved(name)` → `messages::save::saved(name)`), and `messages::user_text`.
- **STATE**: `fn state_summary(&self) -> Value` / `edit_summary` build the same JSON with
  `serde_json::json!`. Optional keys are omitted exactly where the TS spreads
  `...(x ? {k} : {})`. Numbers: JS has one number type; write integers as integers
  (`json!(3)`), and round where the TS rounds. `-0` must serialise as `0`.
- **Time and randomness**: `engine::now_ms()`, `engine::entropy(n)`, `engine::local_date()`.
- **No `unsafe`, no new dependencies** without asking the lead. `unwrap()`/`expect()` only
  where the TS could not fail either; prefer `?` and the TS's own fallbacks.
- **Comments**: keep the TS's explanatory comments (they record decisions and test
  pins), shortened; add the TS line reference for each function (`// R:1234`).

## Building and testing (8 GB Mac)

Every cargo command goes through the slot: `$WORKTREES/tools/slot cargo …`
run from `crates/` (the workspace). Use `cargo check -p tandem-app` while porting and
`cargo test -p tandem-app --lib <filter>` for unit tests. Do not build `tandem-desktop`
(Tauri) or `tandem-ui` unless your task says so. Never `cargo clean`. Never run more than one
cargo command at a time yourself.

The drive tests run against the headless binary `target/debug/tandem-headless`
(`cargo build -p tandem-app --bin tandem-headless`) with `TANDEM_APP_BIN` set; see
`scripts/drive.mjs`. Example:
`TANDEM_APP_BIN=$PWD/crates/target/debug/tandem-headless $WORKTREES/tools/slot node scripts/drive.mjs tests/drive/nav-zoom-find.jsonl`
(from the worktree root).
