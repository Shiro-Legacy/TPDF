# Tandem PDF for agents

Tandem PDF is a local desktop PDF editor. An agent drives the real app (headless) one step at a
time: it sends a step, gets back the app's STATE after that step, looks, and decides the next
step. Every edit is the same operation the menus run, is proven or refused with a fixed token,
and is one undo step. Nothing leaves the machine.

## Start

Either register the MCP server (stdio, no dependencies beyond `node`). The installed app carries it:

    claude mcp add tandem-pdf -- node "/Applications/Tandem PDF.app/Contents/Resources/agent/scripts/tandem-mcp.mjs" [--out=/ABS/OUT/DIR]

Claude Desktop (`claude_desktop_config.json`):

    { "mcpServers": { "tandem-pdf": { "command": "node", "args": ["/Applications/Tandem PDF.app/Contents/Resources/agent/scripts/tandem-mcp.mjs"] } } }

On Windows the installed server is `%LOCALAPPDATA%\Programs\Tandem PDF\resources\agent\scripts\tandem-mcp.mjs`
(use its full path in the same two forms; in JSON write each `\` as `\\`). Paths in STATE and in step results
are native there (`C:\…\out.pdf`); a step may give a path with `/` or `\`.

On Linux the installed server is `~/.local/share/tandem-pdf/resources/agent/scripts/tandem-mcp.mjs` (write the
home directory out in full). The app needs a display even headless: with none, start the server under `xvfb-run -a`.

From the repo instead (after `npm run build`), the same server is `node /ABS/PATH/pdf4all/scripts/tandem-mcp.mjs`.

Or, in the repo, talk to a session directly: `node scripts/drive.mjs --session [--out=<dir>]` reads one JSON
step per stdin line and prints one JSON result line per step; at stdin EOF it prints the report
and exits 0 when every `expect` matched. Without `--out`, files saved to `${OUT}` live in a temp folder that is deleted when the app exits. From Node: `import { createSession } from
'./scripts/session.mjs'`; `const s = await createSession({ out }); await s.step({...}); await s.close()`.

A session starts with the sample `hello.pdf` open. It runs until the MCP server stops (or stdin
ends, or `{"cmd":"close"}`); there is no idle timeout, but one step that gets no STATE within
60 s ends the session with `step timeout`. Through the MCP server the next tool call then starts a
fresh session (hello.pdf again): the earlier document and its unsaved edits are gone.

## Tools

- `tandem_new {size?}`: blank one-page `Untitled.pdf`; `"letter"` (612x792 pt, default), `"a4"`
  (595.276x841.89) or `[w, h]` points, each 72–14400 (anything else: `error` "Could not read Untitled.pdf.").
- `tandem_open {path}`: open a PDF (relative to the server's working directory).
- `tandem_do {cmd, contents?, path?, to?, from?, range?, ms?}`: any drive step (see Ops).
- `tandem_state {}`: STATE now, nothing changes.
- `tandem_save {path}`: Save a copy (relative paths go to the server's `--out` folder and missing
  folders under it are created; a relative path must stay inside `--out`; an absolute path needs an
  existing folder, else `save {ok:false, reason:"write-failed"}`).
- `tandem_help {topic?}`: this file, or its `create` / `modify` / `fill` / `ops` section.

Every tool returns the step record `{cmd, …, state}` as JSON text; `isError` is true when the
step has `error` or `state.error` is non-empty. `tandem_do` refuses `close`.
In a raw session the same steps are JSON lines: `{"cmd":"new","contents":{"size":"a4"}}`.
An unknown `cmd` sets `error` `Unknown command "…"` and changes nothing. `open` and `new`
discard unsaved edits without asking; save a copy first.

STATE keys worth reading after each step:
- `doc {name, pageCount, readOnly, needsPassword}`, `pages.count`, `page` (shown page, 0-based)
- `revision` (bumps once per landed edit), `dirty`, `canUndo`, `canRedo`
- `error` ("" when fine; a refusal's text otherwise; it can outlive its step: a bare `state` keeps it,
  `find` and a `content` object clear it, so also check `revision`), `announcement` (last status message)
- `content.edited`, `content.refused [{text, detail}]`, `content.replaced`, `content.lastAdd`
- `fields.values {name: value}`, `fields.defs {name: {kind, rect, …}}` (radio kids also as `name.on`), `fields.last {action, name, detail?}`
- `save {ok, name}` or `{ok:false, reason}`, `search {query, count, index}`

Coordinates are PDF points on the page, origin top-left, y down (a letter page is 612 wide,
792 tall). `page` numbers in steps are 0-based.

## Create

    {"cmd":"new","contents":{"size":"a4"}}
      → state.doc {name:"Untitled.pdf", pageCount:1}, dirty false, revision 0
    {"cmd":"text-add","contents":{"page":0,"point":[72,100],"text":"Invoice 42","font":"Helvetica","size":14,"color":[0,0,0]}}
      → dirty true, revision +1; content.lastAdd.targetId names the new run
    {"cmd":"fields","contents":{"action":"add","kind":"text","name":"customer","rect":[72,140,300,164]}}
      → fields.last {action:"add", name:"customer"}; fields.defs.customer
    {"cmd":"save-to","path":"/abs/out/invoice.pdf"}        (MCP: tandem_save {path})
      → save {ok:true, name:"invoice.pdf"}

`point` is where the text baseline starts. `font` is a base-14 name (`Helvetica`, `Times-Roman`,
`Courier`, with -Bold/-Oblique/-Italic variants); text is WinAnsi (Latin-1 plus € and typographic
quotes), `\n` makes more lines. Field kinds: `text`, `checkbox`, `combo` (a dropdown; `options:[…]`),
`radio` (`kids:[{on, rect}]`), `signature`. More pages: `insert` (see Ops). The file stays
`Untitled.pdf` in a temp folder until you save a copy; keep the path you saved to. `tandem_save
"sub/x.pdf"` creates `sub/` under `--out`. A raw `save-in-place` on Untitled needs `path`
(`{"cmd":"save-in-place","path":"${OUT}/x.pdf"}`): it writes there and the document becomes that
file (`doc.name "x.pdf"`); without `path` it is `save {ok:false, reason:"cancelled"}`.

## Modify

    {"cmd":"open","path":"/abs/report.pdf"}                → doc.name, pages.count, error ""
    {"cmd":"goto","contents":{"page":1}}                   → page 1 (text edits act on the shown page)
    {"cmd":"content","contents":{"target":"Total: 10","text":"Total: 12"}}
      → content.edited {from, to}, revision +1; or content.refused [{text, detail}] and no change
    {"cmd":"content","contents":{"find":"2024","replace":"2025","page":0}}
      → content.replaced {applied, matches, rejected [{text, detail}]}
    {"cmd":"undo"} / {"cmd":"redo"}                        → revision +1 each, dirty follows
    {"cmd":"save-to","path":"/abs/report-edited.pdf"}      → save.ok

`target` must equal one text run on the shown page exactly (a run is what one text-show operator
drew, often a line or part of one). If you do not know the runs, `find` the words first
(`{"cmd":"find","contents":{"query":"Total"}}` → `search.count`, and the page it lands on), or
use `find`/`replace`, which matches inside runs. A `target` that matches no run changes nothing and
sets no `error` (only `announcement` "No text target …"): check `content.edited`. A `content`
object with none of `text`, `find` or `action`, or no `contents` at all, is refused (`error` "content: give
text, find or action"); a `{`-prefixed string that is not valid JSON is refused with `error` "content: <parse error>", and any
other string (`"hello"`) is refused like an empty object.
A `{"password":…}`-only object is the `--edit=content` smoke's dev path: it runs the built-in self-test
edit on the page's first run, so never send it on a document you care about.
Other edits: notes (`{"cmd":"note","contents":"text"}`),
redaction (`{"cmd":"redact","contents":{"search":"secret"}}` then
`{"cmd":"redact-apply","contents":{},"to":"/abs/out.pdf"}`; see Redaction below), page ops on the shown page
(`rotate-right`, `rotate-left`, `move-up`, `move-down`, `delete-page`), headers/footers
(`{"cmd":"header-footer","contents":{"kind":"footer","text":"Confidential"}}`; `{bates}` in the text numbers pages).

The engine's redaction requests (worker protocol, `src/renderer/protocol.ts`; what a drive step
forwards is the renderer's choice) also take boxes without marks: `redact-apply` accepts
`regions: [{page, rect}]` (y-down page points, as `redact`'s rect; applied together with the
document's marks), `flatten: [page]` and `unsign: true`. Its refusals carry a code in `reason`
and data beside it: `needs-flatten` with `pages` (0-based: content under a box that can only be
removed by turning those pages into pictures; resend with them in `flatten`), `signed` (resend
with `unsign: true` for an unsigned copy), `verify-failed` with `detail` `raw` and `pages`.
`redact-inspect {regions}` lists, per box, what apply would remove (glyph quads and the text they
spell, pictures, shapes, comments and fields, `needsFlatten`); a region may name a mark by
`annotId` or ask for the search rule with `search: true`.

## Fill

    {"cmd":"open","path":"/abs/form.pdf"}
      → fields.defs lists every field: {"name": {"kind":"text"|"checkbox"|"combo"|"radio", "rect"}}
    {"cmd":"fields","contents":{"action":"fill","name":"customer","value":"Ada"}}
      → fields.values.customer "Ada", fields.last {action:"fill", name:"customer"}
    {"cmd":"fields","contents":{"action":"fill","name":"agree","value":true}}       (checkbox)
    {"cmd":"fields","contents":{"action":"fill","name":"choice","value":"two"}}     (combo option)
    {"cmd":"fields","contents":{"action":"fill","name":"size","value":"M"}}         (radio: the kid's on-value)
    {"cmd":"save-to","path":"/abs/form-filled.pdf"}

Read the names from `fields.defs` of the STATE you got back, never guess. Fields are resolved on
the shown page: `goto` the page first on a multi-page form. A refused fill leaves the value,
sets `fields.last.detail` and `error`. Rename/move/props: `{"action":"props","name":…,"rename"?,
"rect"?,"label"?,"readOnly"?,"required"?,"default"?,"options"?}`; `{"action":"delete","name":…}`.

A flat form (no fields: `fields.defs` is `{}`) is filled by typing text boxes where the blanks are:

    {"cmd":"fill-text","contents":{"page":0,"x":150,"y":210,"text":"Ada Lovelace"}}
      → annots.FreeText +1, fill.boxes [{id, rect, chars}], fill.last {op:"add", id, rect}
    {"cmd":"save-to","path":"/abs/form-filled.pdf"}

`x` is the text's left edge and `y` the vertical centre of its first line (page points from the
top-left, Helvetica 12 black; `\n` starts a new line); the box is sized to the text and kept on the
page. Take the blank's position from `text-all` or `find` hits. A character outside WinAnsi is
refused (`error` "Cannot draw: …"), a rotated page refuses ("A text box cannot be placed on a
rotated page"). Once a document is filled (`fill.saveAsCopy` true) `save-in-place` needs `path`
(it writes a copy, "<name> (filled).pdf" in the GUI, and rebinds to it); without `path` it is
`save.reason "cancelled"`. `save-to` / `tandem_save` always work.

A signature (a picture, not a certificate signature) is drawn, or taken from one image, once, then placed:

    {"cmd":"sign-draw","contents":{"strokes":[[[0,0],[40,30],[80,0]],[[100,10],[200,20]]]}}
      → fill.signature {saved:true, kind:"drawn", strokes, points, armed:false, pad:false}; no journal, never dirty
    {"cmd":"sign-draw","images":["/abs/signature.png"],"contents":{"image":true}}
      → fill.signature {saved:true, kind:"image", strokes:0, points:0, …}; the pad's "Use an image…" and Save
    {"cmd":"sign-place","contents":{"page":0,"x":300,"y":650,"width":180}}
      → annots.Ink +1 (an image: annots.Stamp +1), fill.signatures [{id, rect, kind}], fill.lastSignature {op:"add", id, rect}

Strokes are pad pixels (any origin; the pad's rules: a 1-point stroke is a dot, points under 2 px from the last are skipped, at most 64 strokes and 4096 points in all, 8 px
wide at least; the JSON may be up to 4 MiB, more is refused in `error`; the step record shows
`contents` as "[redacted]"). `sign-place` fits it inside 200×60 pt (or `width` pt, aspect kept, and
never wider or taller than the page), centred on x/y and kept on the page; `{"clear":true}` removes
it. Give exactly one of `strokes`, `image` or `clear` (two → "sign-draw: give one of strokes, image or
clear"). `image` uses the line's `images` pre-answer: exactly one PNG or JPEG (none, two, or a file the
app cannot decode is refused in `error`; `images` stays set for later lines until another line gives
it); it replaces a saved drawing and the other way round (one saved signature, `fill.signature.kind`
"drawn" or "image"). The app keeps its own upright PNG copy of the picture (long side at most 1000 px),
so a phone photo is placed the way it is shown and none of the file's EXIF or other metadata is kept. An image is placed as one Stamp with
`/Subj` "Signature appearance" (no `/Contents`, no `/T`, its aspect inside 200×60 pt; the background is
not made transparent) and moves and resizes exactly like a drawing; `fill.signatures` lists both kinds
(`kind` per entry). The bare op `sign` (no contents) is the same as `sign-place` without contents.
On the placed signature: `drag` moves, `handle` resizes (aspect kept), `key` arrows nudge, `+`/`-`
resize 10 %, `Delete` removes (`fill.lastSignature.op` "move"/"resize"/"delete"). `sign-place`
without contents arms placement (`fill.signature.armed`); then `click {page,x,y}` places it, `key
Enter` places it at the page centre, `key Escape` disarms. Refused: no drawing ("sign-place: no
signature has been drawn"), a rotated page, a document that forbids annotations. An agent session
never sees the human's saved signature: drives and smokes keep it in memory only.

## Ops

Drive steps are `{"cmd": <op>, "contents"?: object|string, …}`; a step may carry `"expect":{…}`
(a subset match on STATE; the report's `ok` says whether it matched). Main-side steps: `new`,
`open {path}`, `save-to {path}`, `save-in-place`, `print-to {path, range?}`, `wait {ms}`,
`state`, `close` (raw sessions only). Line-level answers to native dialogs: `to` (save target of
redact-apply), `from` (insert source), `images`, `folder`. Everything else goes to the
renderer, including: `undo`, `redo`, `goto`, `zoom`, `find`, `mode`, `text-add`, `content`,
`fields`, `fill-text`, `sign-draw`, `sign-place`, `note`, `annot`, `highlight`, `underline`, `strikeout`, `freetext`, `square`, `circle`,
`line`, `ink`, `stamp`, `redact`, `redact-preview`, `redact-apply`, `redact-confirm`, `redact-page`,
`redact-search`, `redact-apply-open`, `redact-underneath`, `redact-escape`, `redact-original`, `redact-pending`, `insert`, `extract`,
`rotate-right`, `rotate-left`, `move-up`, `move-down`, `delete-page`, `labels`, `watermark`,
`header-footer`, `bookmark`, `link`, `encrypt`, `password`, `sign`, `verify`, `export-txt`,
`export-png`, `batch`, `compare`, `help`. The exact contents of each are in
`.quad/shared/p3/design-agent-drive.md` and the scripts under `tests/drive/`; copy a line from
there rather than guessing keys.

### Redaction

Redaction boxes are Redact annotations ("marks"), so `undo`, `redo`, `drag`, `handle`, `key` and the
clipboard work on them as on other objects (in the `redact` mode). Two ways to redact:

- The legacy op, unchanged: `{"cmd":"redact","contents":{"page":0,"rect":[x0,y0,x1,y1]}}` (or
  `{"search":"text"}`), then `{"cmd":"redact-apply","contents":{"sanitize":{…}},"to":"/abs/out.pdf"}`.
  It writes the redacted copy and **keeps the original open** with its boxes (`redact.last {op:"redact-apply", ok, verified}`).
  `contents.regions [{page, rect}]` adds boxes held in memory for this apply only (page 0-based, rect in page
  points from the top-left, at least 1 by 1 pt): no mark is written, so `revision` and `dirty` do not move.
  `flatten [page]` and `unsign: true` answer the engine's `needs-flatten {pages}` and `signed` refusals up front.
  Refused: `invalid` (bad regions), `too-many` (over 2000 boxes).
- The app's own flow (what a person does): `{"cmd":"mode","contents":{"mode":"redact"}}` turns the tool on
  (`redact.tool` true), boxes are drawn with `drag` (or `redact`, `redact-page`, `redact-search`), then
  `{"cmd":"redact-confirm","contents":{},"to":"/abs/out.pdf"}` presses "Redact and save as…". The copy is
  saved and **opens in the window instead of the original** (`doc.name` is the copy, `dirty` false,
  `canUndo` false, `redact.banner {name, areas, original}`, `redact.last.opened`).
  `contents` may hold `hidden` (false keeps all hidden information), `details` (single switches by the
  sanitize keys: `info`, `xmp`, `attachments`, `outlines`, `annots`, `links`, `fields`, `layers`, `struct`),
  and `flatten`/`unsign` (true answers yes to "turn the page into an image" / "make an unsigned copy";
  unanswered, a step that needs them ends with `redact.last.reason` "cancelled" and nothing is written).
  `redact.last` then also carries `flattened` (0-based pages turned into pictures) and `unsigned`.
  `others` answers "You have other unsaved changes." (asked when the document holds unsaved changes besides
  boxes): `save` saves them (and the boxes, as marks) to the original first, `continue` (the default) puts
  them in the copy only, `cancel` stops. A `to` naming the open file itself is refused (`redact.last.reason`
  "same-file"); nothing is written.

Other redaction steps:
- `redact-page {page?}`: a box over the whole page (the page menu's "Redact whole page"; `menu {page,x,y,item:"redact-page"}` too).
- `redact-search {text}`: the "Find text to redact…" helper; a box over every match (`redact-search` alone opens it).
- `redact-apply-open`: opens the confirmation dialog (`redact.dialog` true).
- `redact-underneath {on}`: the "Show what's underneath" view (`redact.underneath`).
- `redact-escape`: a real Escape key press; with nothing selected it stops the tool (`mode` returns to the one before).
  The plain `key Escape` step still only drops a selection, as before.
- `redact-original`: the banner's "Open original" (reopens the file the copy was made from).
- `redact-pending {choice}`: answers the next "N areas are marked but not redacted yet." question, which every way
  out asks while boxes are unredacted (save, save a copy, `export-*`, `print-to`, signing): `redact`, `marks`
  (go on: save the boxes as marks, or export/print/sign anyway) or `cancel`. `redact.prompt {marks, choice, action}`
  records it; without an answer a drive goes on and records `choice: "unasked"` (scripts never block).

On a signed or certified file the document takes no marks: boxes drawn there are held in memory
(`redact.memory` counts them; no undo) and sent as `regions` when you confirm; the result is an unsigned
copy (`redact-confirm {"unsign":true}`), and the signed original is never changed. `redact.flattenNote`
is set when a box on the shown page can only be redacted by turning the page into a picture.

STATE `redact`: `marks`, `pages`, `rects [{id, rect, search}]` (shown page), `preview`, `last`, `tool`,
`memory`, `unsaved` ("count:memory", the boxes a close or open would lose, as main's prompt names them), `tint` (how many glyph runs, picture parts, drawings and comments the shown page's boxes take,
from the engine's `redact-inspect`), `underneath`, `panel {boxes, button}`, `dialog`, `prompt`, `banner`.

### Refusals you will meet

A refused step changes nothing (revision and dirty stay) and says why:
- drive: `error` `Unknown command "<cmd>"` (a typo'd `cmd`) or "content: give text, find or action" (a `content` step with none of them, or no `contents`).
- `tandem_save`: `isError` true with "tandem_save: path must stay inside the output folder" (a relative path with `..` leaving `--out`); nothing is saved.
- `content.refused[].detail` / `content.replaced.rejected[].detail`: `unsupported-encoding`
  (character not drawable in that font; `missing` lists them), `subset-font`, `overflow` and
  `tail-would-move` (new text too wide for the free space), `ambiguous-text-association` (the same
  text drawn twice at one spot), `shared-or-unproven-sharing`, `unsupported-graphics-state`,
  `unsupported-text-operator`, `type3`, `vertical-writing`, `rtl-or-shaping`, `read-only`,
  `stale-target`, `not-candidate`, `write-failed`, `invalid-selection`.
- open: `error` "Could not read <name>." (missing, not a PDF, bad `new` size) or "<name> is
  larger than the … limit."; an encrypted file shows `doc.needsPassword true` until
  `{"cmd":"password","contents":"…"}`.
- save: `save {ok:false, reason}` with `permission`, `disk-full`, `too-large`, `same-file`,
  `unreadable`, `write-failed`, `busy`, `document-changed`, `source-changed`.
- fields: `fields.last.detail`: `field-read-only`, `scripted`, `xfa`, `signature`, `duplicate-name`,
  `invalid-name`, `too-many`, `stale-target`, `invalid`.
- signed, certified or permission-locked documents refuse edits; a JavaScript action is never run.

### Prefer semantic ops

Edit text by its text (`content {target}` or `find`/`replace`), fields by name (`fields`),
pages by number (`goto`, page ops). `click`, `drag`, `handle`, `key`, `type` and `menu` exist for
parity with the mouse and keyboard; they need page coordinates and the right mode and are the
wrong first choice for an agent.

### Ceilings

- Existing text is edited one run at a time; paragraph reflow only for simple upright,
  left-aligned, single-font paragraphs. New text is unembedded base-14 WinAnsi, no reflow.
- Embedded subset, CID (Identity-H) and symbolic "coded" fonts can only draw characters already
  drawn with that font on that page (`unsupported-encoding` otherwise).
- Print-to-PDF pages (macOS Quartz "Save as PDF": one clipped block per glyph) are edited line by
  line: each line is one text target whose item carries `glyphs` (its glyph count), and
  `content.glyphLines` counts them on the shown page. Same repertoire rule (a space is always
  allowed; spaces are normalised); a font change inside a line splits it into separate lines; an
  edited line is redrawn unclipped; Copy/Cut/Duplicate of such a line are refused (`not-candidate`).
- No OCR: a scanned page has no text to edit or find.
- One document per session; `new` has no templates; sizes are Letter, A4 or custom points.
- Form fields: top-level fields only; no list boxes or push buttons; XFA and scripted fields refused.
- Signing needs a PKCS#12/PEM file and its password (the step's contents are redacted in reports).
