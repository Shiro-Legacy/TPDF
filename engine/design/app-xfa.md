# Dynamic XFA forms (materialized at open)

November 4, 2026; scripts added November 5. Code: `src/xfa/`. Hooks: `app::document::AppDocument::load` (open), `app::ops::fields::relayout` (fill). Tests: `tests/app_xfa.rs`, unit tests in each module. Bench: the owner's `Status_Certificate.pdf` (Adobe Designer 6.5, AEM Forms; a copy and its packets are in `tpdf-wt/scratch/xfa/`; it is not a fixture and never enters the repository).

## The problem

A dynamic XFA form (`/NeedsRendering true`) carries one placeholder page ("requires Adobe Reader 8 or higher") and the real form as XML packets in /AcroForm /XFA: a template (layout, fields, captions, rich text, scripts), datasets (the values), config and locale set. Only Acrobat and Reader lay the template out. Every other viewer shows the placeholder. The app opened such files view-only and showed the placeholder with a banner.

## Decision: materialize, do not render

At open, the app lays the template out itself and writes a new, ordinary PDF: one page per laid-out page, the static content drawn in page content with base 14 fonts, and a widget annotation (an AcroForm field) for every fillable field. That document becomes the session's live document. Everything downstream works unchanged: the view, text and search, the fields panel, fills, appearance synthesis, undo, save. No viewer-side XFA rendering exists; the render path never learns about XFA.

The materialized document keeps the XFA packets (the `form` packet, Acrobat's cached form DOM, dropped) and `/NeedsRendering true`. So:

- Acrobat and Reader ignore our pages and lay the form out from the template and datasets, as before.
- Every other viewer shows our pages and our widgets.
- A fill here is written to the widget's /V and, in the same journaled step, into the datasets packet (`sync`), so Acrobat shows what was filled here, and undo restores both.
- A reopen of a saved file sees the catalog marker `/TPDFXfa 1` and keeps the pages as they are (annotations and fills survive); it does not lay out again.

## Scripts: run where Acrobat runs them

Designer forms carry JavaScript in `event` elements and in `variables/script` objects; the bench form has 377 of them and 205 `presence="hidden"` nodes, and a choice of condominium type reveals whole sections. `src/xfa/script/` runs the subset such forms use (`parse` reads the JavaScript; `eval` runs it against the bound form; `script/mod.rs` picks the events):

- **At open** (`materialize`), after binding and before layout: every `initialize` event in document order (a node before its children), then the `ready` events (`$form`, then `$layout`). That is what Acrobat fires at open, and Designer forms reproduce their saved state from the data in those events (the bench form keeps an `initFld` flag and switches to its `*ShowHide` functions once set). `change`/`click` handlers do not run at open: Acrobat does not run them, and in the bench form they reset other fields (`NotCommon()` clears Section 6's boxes), so running them for every stored value would lose fills.
- **On a fill** (`field-set` on a materialized form), after the datasets packet took the value: the form is built again from the updated packets, `initialize` and `ready` run as at open, then the filled node's own handlers (`click` and `change` for a check box or a radio group and its chosen button; `change` and `exit` for anything else, `xfa.event.newText` the value). The result replaces the live document whole (`Transaction::replace_with`: every object put or removed, the trailer's /Root moved) in the fill's journaled step, so undo brings the previous pages and widgets back and redo the new ones. Widget object numbers change when that happens; the fields panel refetches after every edit.
- **When nothing the layout shows changed**, the pages stay: the emitter writes `/TPDFXfaSig`, a hash (`xfa::signature`) of every node's presence and access, every draw's value and the values of non-fillable and floating fields. A fill whose rebuilt state has the same signature, and whose scripts set no fillable field, is `Materialized::Unchanged` and keeps the document as it is (typing a name costs no relayout and no new widget numbers).
- **Values scripts set** on bound fields (`initFld`, the floating "unit"/"Parcel" words) are written into the datasets packet of the materialized document (`emit::Extra::datasets`), as Acrobat's data DOM would hold them, so the next rebuild and Acrobat see them.
- **What runs**: `this`/`xfa.event.target`, `rawValue`/`value`, `presence`, `access`, `isNull`, `name`, `parent`, `index`, `instanceManager` (its methods do nothing), `xfa.resolveNode` with `name`, `name[n]`, `#class[n]`, `parent`, `xfa.form`, `$form`; names resolved as XFA scopes them (the node's children, then each ancestor's children and variables, unnamed subforms transparent; a script object's functions resolve from the subform holding its `variables`); `var`, `if`/`else`, `switch`, `while`, `do`, `for`, `return`, functions, the arithmetic, comparison and logical operators with JavaScript's conversions, a few string and number methods. `xfa.host` members are functions that return `undefined`. FormCalc scripts and events without a JavaScript `contentType` are skipped.
- **What does not stop a script**: a construct the reader does not know (regular expressions, `for ... in`, bit operators, object and array literals) costs the statement that holds it, not the script; calling a non-function, a missing node from `resolveNode`, or a member a value lacks is `undefined`, never an exception, so the presence lines after a failed validation call still run. A missing `;` at a line end is inserted as JavaScript inserts it.
- **Bounds**: 4,000,000 evaluations per materialization, 400,000 per script, 20 calls deep, 48 levels of nesting, 1 MiB strings, 4,096 locals per frame, scripts of 256 KiB, 64 events per node, 256 variables per subform. A bound stops that script where it is; what it did stands, the layout proceeds.

The source's /Perms (usage rights, which any change breaks), /Names /JavaScript (the "needs Reader" alerts), /OpenAction and /AA are left out of the materialized document.

Failure falls back: when any step refuses (no template, unreadable XML, an unsupported layout, a limit), the document opens as it did before, placeholder and banner. Materialization never makes an open fail.

## What the renderer does and does not do

Done in this round:

- Template model (`types`): subform, field, draw, exclGroup, area, pageSet, pageArea, contentArea, proto/use, margins, borders (edges, fills, corners), fonts, paragraphs, captions, items, bind, occur, keep, breaks, presence, access, tooltips.
- Binding (`bind`): instances from `occur` and from the data (a repeating subform gets one instance per matching data group), values from the datasets, SOM names with indexes, data paths.
- Layout (`layout`): positioned, tb, lr, table and row layouts; fixed and content-sized boxes (`minH`/`maxH`); captions by placement and reserve; pagination through content areas and page masters; keep and break; hidden presence; page-master statics.
- Text (`text`, `richtext`): base 14 faces for any family (Arial, Calibri, Myriad Pro and every other sans to Helvetica; serif names to Times; monospace to Courier), metrics from `builtin_metrics`, wrapping, alignment, rich text runs (bold, italic, underline, size, family, colour, paragraphs, breaks, floating fields).
- Emission (`emit`): pages, content, fonts, the field tree, widgets with /FT, /Ff, /V, /Opt, /TU, /MK-less (XFA draws borders in content), /DA, `TPDFData`, /NeedAppearances true, `/TPDFXfaSig`.
- Sync (`sync`): datasets updated on each fill, and with the values scripts set.
- Scripts (`script`): the JavaScript subset above, `initialize`/`ready` at open, the filled node's handlers on a fill, with a relayout when they changed what is shown.

Not done (recorded as limits, each a later round):

- **Scripts beyond presence and values.** FormCalc does not run (calculated fields written in it stay empty); `calculate` and `validate` events are not fired; `instanceManager.addInstance`/`removeInstance` do nothing, so add-row and delete-row buttons do nothing; `xfa.host.messageBox` and the other host calls are silent; regular expressions, arrays and object literals are not evaluated. A handler's effect that the form's `initialize` events do not reproduce from the data is lost at the next fill (the form is rebuilt from the datasets each time), as it is lost at an Acrobat reopen.
- **Pictures.** Date, time and number pictures are kept but values are shown as typed. A date field is a text field.
- **Fonts.** Base 14 only, so Arial and Calibri render as Helvetica and widths differ slightly; no embedded fonts, no non-Latin text (WinAnsi, `?` otherwise).
- **Images, barcodes, signatures** are not drawn (their boxes are).
- **Rotation**, `lr`/`rl` beyond left-to-right wrapping, and `hand` other than inside are approximations.
- **Static XFA** (no `/NeedsRendering`): untouched; its AcroForm pages already render.
- **Reopen after an Acrobat fill.** A file this app materialized and Acrobat later filled shows our widgets' values, not the datasets'.

## Module contracts

The signatures in `src/xfa/*.rs` are the contract between the four implementers; `types.rs` is the shared model. In brief:

- `xml::parse(&str) -> Result<Element, XmlError>`, `xml::serialize(&Element) -> String`; `Element` keeps qualified names, `local()` strips prefixes.
- `packets::read(&Document) -> Result<Packets, XfaError>`, `packets::is_dynamic(&Document) -> bool`.
- `template::read(&str) -> Result<Form, XfaError>`: one instance of each subform, protos resolved.
- `bind::bind(&mut Form, Option<&Element>) -> Result<(), XfaError>`: instances, SOM names, data paths, values.
- `text::block`, `text::lay_out(&Block, Option<Pt>) -> LaidBlock`, `text::face_for`, `text::advance`, `text::win_ansi_code`; `richtext::parse`, `richtext::plain_text`.
- `layout::lay_out(&Form) -> Result<Vec<Page>, XfaError>`: paint-ordered items in page points, y-down.
- `emit::materialize(&Document, &Packets, &Form, &[Page]) -> Result<Vec<u8>, XfaError>`; `emit::materialize_with(..., &Extra)` writes a replacement datasets packet and the signature too.
- `sync::update_datasets(&str, &[(DataPath, String)]) -> Result<String, XfaError>`.
- `script::Runner::new(&mut Form)`, `.initialize() -> Report`, `.fire(&DataPath, &str) -> Report`, `.run(&str, &NodeRef, EventInfo) -> Result<(), Stopped>`; `bind::record_ids(&mut Form)` rebuilds the floating-field values after scripts.
- `xfa::is_dynamic`, `xfa::materialize(&Document) -> Result<Vec<u8>, XfaError>`, `xfa::materialize_after_fill(&Document, &DataPath, &str) -> Result<Materialized, XfaError>`, `xfa::signature(&Form) -> u64`.
- `app::document::Transaction::replace_with(&Document)`: the live document made another, in one journaled step.

Coordinates: XFA and the layout are y-down in points from the page's top-left; PDF content is y-up, so the emitter maps `y_pdf = height - y - h`. The forms module's page space is also y-down (`forms::PageSpace`), so widget rects pass through `to_pdf`.

Field names: the SOM name with indexes (`form1[0].StatusCertificate[0].Section1[0].CCName[0]`), as Acrobat names an XFA-backed AcroForm's fields, one /T per step. Depth stays under `forms::MAX_FIELD_DEPTH` for real forms; a deeper path is flattened at the top (the first steps joined into one /T with the dots replaced by `_`).

Check buttons: the on state's name is the on item's save value (`/1`), so /V and /AS carry what the datasets store. Radio buttons: one AcroForm radio field per `exclGroup`, kids named by their on values. Choice lists: /Opt as `[export display]` pairs.

## Limits

`xfa::limits`: 64 MiB of packets, 2,000,000 XML elements, 512 levels, 200,000 form nodes after expansion, 2,000 pages, 1,000,000 items, 1 MiB of characters per text block. Every reader and the layout stop at them with `XfaError::Limit`, never a panic; hostile templates (cycles in `use`, unbounded `occur` with data, a content area of zero height) are tests. Scripts have their own bounds (above); a runaway script (`while (true)`, unbounded recursion, a string doubled forty times) is a test too.

## Measurements

November 4, 2026, release build, M-series, the owner's bench form (Designer 6.5; 209 fields, 265 draws, 273 subforms, 71 `tb` flows, one table, 205 hidden nodes, one Letter page master with one content area):

- Open of the source: 1.4 ms. Materialize (packets, template, bind, layout, emit): 19–29 ms for a 240 KB document of 14 pages. Render of a page at 72 dpi: 6–10 ms (hayro, with the widgets' synthesized appearances).
- `fields` lists 209 widgets: text (single and multi-line), check boxes with their XFA on values as state names, two radio groups (`CondoType`, with on values 0–4), choice lists. Captions, rich text (bold, italic, floating fields), tables, check-box rows and the signature block lay out where Designer puts them; U+2029 line separators in plain values break lines.
- Fill, save, reopen: `field-set` on the corporation name and a radio kid wrote `/V` and, in the same step, `<CCName>` and `<CondoType>1</CondoType>` in the datasets packet; the saved file reopens with `xfaRendered` true, the values shown, and no second layout (the marker short-circuits).
- Tests: 109 `xfa::` unit tests, 8 in `tests/app_xfa.rs`; fmt and clippy (`--all-targets -D warnings`) clean.

Open against Acrobat (the owner's view): the blank stretches on pages 2 and 3 are tall text fields whose XFA borders are hidden (Designer draws nothing there either; Acrobat shows its field highlight).

November 5, 2026, scripts, release build, the same form:

- Open: 69 `initialize` scripts run in 1.2 ms (none skipped, none stopped). They hide Section 5's "any condominium corporation" paragraph (`Section5_1`) and Section 6's certificate block (`DefaultCheck` is 0), as Acrobat shows the form before a choice: 13 pages, 122 widgets.
- Choosing "Standard condominium corporation" (`CondoType` = 1) through `field-set`: 22 ms for the fill, the datasets and the relayout. The state diff against the open state is exactly what `Standard()` and `NotCommon()` write: Section 3's instructions hidden, `Section5_1` shown and `Section5_2` hidden, the "not applicable" lines of Sections 25 to 30 shown and their paragraphs hidden, the floating fields of Sections 5 and 7 set to "unit" (page 1 reads "The owner of Unit ... Level ... Suite number" and "for the unit"). Nothing else changes. The saved file reopens with the sections shown. `xfa_probe state FILE [PATH=VALUE]` prints that state.
- Typing a name (`CCName`): 16 ms, `Materialized::Unchanged`, the same widget numbers as at open.
- Tests: 124 `xfa::` unit tests (15 of them for `script`), 10 in `tests/app_xfa.rs`; fmt and clippy (`--all-targets -D warnings`, stable and 1.92) clean.
