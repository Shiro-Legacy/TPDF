# P2 unit 10 §A — encryption and permissions (ds2, 2026-09-16)

Owner: ds2. Files: `src/renderer/engine.ts`, `src/renderer/renderer.ts`, `src/renderer/index.html`,
`src/renderer/styles.css`, `tests/p2-encrypt.test.mjs`. `npm test` **188/188** at hand-off (was 175; +10 mine, the rest grok's §B).
Gate scripts are in `.stage/`: `gate-u10.mjs`, `gate-u10-owner.mjs`, `gate-u10-trailer.mjs`,
`gate-u10-opts.mjs`, `gate-u10-auth2.mjs`, `gate-u10-pypdf.py` (pypdf via my throwaway
`.stage/gate-venv`, pypdf 6.18.1 + cryptography 50.0.1), `gui-u10-check.mjs`, `u10-timing.mjs`.

## 1. Gate result (mupdf 1.28.1 wasm) — every question answered

| Question | Answer (proved) |
|---|---|
| `authenticatePassword` return | `0` = refused, `2` = user, `4` = owner (`level & 4`). Never throws. Wrong/empty → `0`. |
| `saveToBuffer("incremental=no,encrypt=keep")` | **Works.** Authenticated RC4-128 and AES-256 documents re-encrypt with their original `/V /R /Length`, **both original passwords still open the copy**, `/P` is preserved, and pending mutations survive. On a plain document it writes a plain file. RC4 *is* re-producible → the contract's "refuse Save for RC4" fallback is **not needed**. |
| `encrypt=aes-256,user-password=…,owner-password=…,permissions=<int>` | Produces **Standard V5 R6 256-bit AES** (`/V 5 /R 6 /Length 256 /CFM AESV3`). `permissions` is the raw `/P` bit sum (print 4, edit 8, copy 16, annotate 32, form 256); mupdf supplies the mandatory bits itself. pypdf 6.18.1 opens it with the **user** password, refuses the wrong and the empty one, and reports exactly the flags asked for. |
| `encrypt=none` | Strips `/Encrypt`; pypdf reports `is_encrypted false`, all permissions. |
| `garbage=yes` vs `garbage=compact` | Both drop orphaned streams; only `compact` renumbers and breaks cross-save undo. Chose `yes` (§4). |
| Owner vs `/P` | An owner-authenticated mupdf session reports **all five** permissions (`hasPermission` is not bound by `/P`), as the contract wants. pypdf still reports the raw `/P`, so `validate` compares the `/P` bits (`permissionBitsOf`), not `hasPermission`. |

Three hazards found and handled:

1. **`needsPassword()` re-locks an authenticated document** and drops the owner's full permissions back
   to `/P`. Every engine path passes the session's known value to `detect(doc, session.locked)`; the flag
   is also why `detect()` is never called bare on a live authenticated session (see `p1-worker` note 3).
2. **A comma in a password is not representable**: mupdf parses the option string as comma-separated
   `key=value` and throws `Unused pdf arguments found`. `security-set` refuses a comma up front with an
   honest message. `=` , spaces, UTF-8 and 1024 chars all work; the ≤1024 bound is enforced.
3. **`garbage=compact` renumbers the object table** (addendum A): mupdf then marks the document clean
   (`hasUnsavedChanges()` false) and journal steps from *before* the save no longer replay — `undo()`
   reports success and changes nothing. `garbage=yes` collects the same orphans without breaking undo,
   so that is what the save paths use. See §4.

## 2. What was built

- `engine.ts`: `session.auth ("none"|"user"|"owner")`, `session.password` (worker memory only, cleared on
  open/close), `session.security` (pending `keep|none|aes-256`), `session.securityDirty`.
  `DocumentInfo` gains `algorithm` (`none|rc4-40|rc4-128|aes-128|aes-256`, read from `/Encrypt` even while
  locked) and `permissions` (`{print,copy,edit,annotate,form}`).
- `serializeDocument` builds `incremental=no,garbage=yes[,encrypt=keep|none|aes-256,…]`; refuses a
  locked document (`reason:"encrypted"`), and signed/certified (`"read-only"`).
- `validateDocument` reopens the copy with the session's own password (a fresh `needsPassword()` first,
  then authenticate), checks `/Encrypt` presence/absence against the pending mode, page count, repair and
  the other `sameSaveShape` keys, and the `/P` bits. A mismatch is `"invalid"`.
- `requireMutable(session, op)` is the one gate every mutation routes through; it maps op → permission:
  annotate (annotations, links), edit (pages, outlines, insert, watermark, labels), form (field fill);
  undo/redo need any of the three. Refusals are `reason:"read-only"` with a message naming the missing
  permission.
- `security-set` stores the pending setting, bumps `revision`, sets the dirty flag (a flag, not
  `savedPosition`: mupdf's journal cannot express a trailer-only change), and refuses unless `auth ===
  "owner"` or the document is unencrypted.
- Renderer: `permitted(name)` / `canAnnotate()` / `canEdit()` / `canForm()`, `editable()` = unlocked and
  at least one editing permission, `syncControls` disables each group and sets `title` ("This document
  does not permit changes"), `#banner` names the algorithm, who opened it and which permissions apply.
  `#security` button in `#context-organize` (+ `aria-keyshortcuts="Alt+Control+S Alt+Meta+S"`) and
  `<dialog id="security-dialog">` with the contract's exact ids; status text goes to the footer
  `#edit-status`; pending state is announced as "Security will change when you save".
- Smoke seam (for grok): `edit-request {op:'security'}` opens the dialog; `{op:'encrypt',contents:
  '{"user","owner","permissions":[…]}}'` applies AES-256 and saves a copy; `{op:'encrypt-keep',contents:
  '{"password","note"}'}` unlocks (worker-only), adds one note and saves a copy; `{op:'password',
  contents}` unlocks. `P2_EDIT` gains `security:{mode,algorithm,permissions[],auth}`.
- Passwords: the only carriers are the `authenticate` and `security-set` requests. No reply, log,
  `P1_*`/`P2_*` line, journal or file contains one; the dialog clears both fields on submit and on close.

## 3. Tests and real-app check

`tests/p2-encrypt.test.mjs` (10 tests): keep-save round-trip (both passwords, algorithm, the new note,
locked refused); AES-256 set → validate → reopen with the right permissions and refused wrong/empty;
second save keeps AES-256; remove → no `/Encrypt`, password dropped; user session cannot change security
and cannot annotate/rotate/fill when `/P` forbids, owner can; `security-set` validation; too-large;
`garbage=yes` orphan test; passwords absent from every serialized reply and from every `console.log`
line in the renderer; `detect()` algorithm/permissions.

`.stage/gui-u10-check.mjs` (CDP, real app, not in `npm test`) — three parts, all PASS:
1. A user-password AES-256 document with `permissions=4` → banner `Detected: encrypted (256-bit AES),
   opened with the user password, permits print.`, note/page-op/extract disabled with the permission title,
   print enabled, `#security` disabled with the owner reason.
2. hello.pdf → dialog, AES-256 + print/annotate, Apply → `#edit-status` "Security will change when you
   save", dirty chip on, banner "Security will change to AES-256 when you save", password fields empty.
3. **End to end on a copy of `fixtures/encrypted.pdf`**: unlock with `pdf4all-user`, add a note, click
   Save (in place) → `Saved u10-encrypted-copy.pdf`, document clean, banner no longer pending and now
   says `encrypted (128-bit RC4, weak encryption), opened with the user password, every permission.`;
   the rewritten file still needs the original password, opens with it, and carries the note. The
   in-place banner/hand-off after the write is `markSecuritySaved()` (a Save a copy deliberately leaves
   the pending change pending: the open file was not rewritten).
Screenshots `.quad/shared/p2/u10-permissions.png`, `u10-dialog.png`.

## 4. Addendum A — `garbage=yes`, not `compact` (fable51 m89 gate)

`SAVE_OPTIONS = "incremental=no,garbage=yes"`, and every unit 10 save string is built on it.
Evidence (`u10-orphan.mjs`, `.stage/u10-journal2.mjs`, `u10-journal3.mjs`): delete page 1 of `nav.pdf`
→ plain save keeps the deleted page's stream (`one` present, 2219 B); `garbage=compact` drops it
(1662 B, `countVersions()==1`) **but renumbers the object table**, which makes mupdf mark the document
clean (`hasUnsavedChanges()` false) and makes `undo()` of any pre-save step report success while
changing nothing; `garbage=yes` drops the orphan the same way (`one` absent, 1750 B, one revision) and
keeps the journal intact — cross-save undo still replays, `hasUnsavedChanges()` stays true. Timing,
`large.pdf` (500 pages, median of 3), untouched / after a page delete: plain 4 ms / 3 ms,
`garbage=yes` 4 ms / 3 ms (identical), `garbage=compact` 5–6 ms / 3 ms. So `yes` costs nothing and the
`undoFloor` guard added for `compact` is gone (no engine or test change survives from it).

## 5. Layout fix found by the same review (fable51 ask 2)

The new `#security` button made the Organize row wider than the window, and the page overflowed:
CDP showed `document.documentElement.scrollWidth` 1449 vs 1360, `main.app` 1449 wide, the whole row
stretched instead of the list scrolling. Root cause: the header's min-content (the nowrap mode row)
set the implicit `auto` column of `body`'s grid, so `main.app` grew past the viewport. Two CSS lines:
`body { grid-template-columns: minmax(0, 1fr) }` and `.bar-mode .ctx { flex: 1 1 0 }`. Measured after
(`gui-u10-check.mjs` `checkOrganizeOverflow`): 1360 → `scrollWidth 1360`, row `923/1012` (the row is
what scrolls); 900 → `scrollWidth 900`, row `231/1012`; both tab strips fit; `scripts/gui-shell-check.mjs`
still PASS (including its 900x600 no-overflow check). Screenshot `u10-permissions.png` shows the row at
1360 with the Security button visible.

## 6. Deviations, ceilings, notes

- `permissions` for a locked document are mupdf's `/P` read (all true for `P=-4`); the banner only claims
  permissions when the document is unlocked.
- A blank owner password is written as the user password (contract). Then anyone who can open the file
  can change permissions — the dialog says so in one line, and `validate` compares `/P`, not
  `hasPermission`, because mupdf reports all five for such a session.
- Deferred and stated: RC4/AES-128 as new encryption, public-key encryption, per-object crypt filters,
  password strength meter, remembering passwords, unlocking via keychain.
- CSS: two lines in `styles.css` (`body` grid column, `.bar-mode .ctx` flex) as §5; no shell id or control changed.
- Test files touched outside my own: `tests/p1-worker.test.mjs` (grok's) — the three assertions that
  encoded "encrypted documents cannot be saved/edited" (`:62` info literal, `:584` refusals, `:872`
  unlocked save) were updated to the new contract and reported to grok; `tests/p2-edit.test.mjs`,
  `tests/p2-pages.test.mjs`, `tests/p2-nav.test.mjs`, `tests/p2-assemble.test.mjs` (mine) follow the
  unlocked-encrypted behaviour (`garbage=yes` keeps cross-save undo, so those tests are unchanged).
