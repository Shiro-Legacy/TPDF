#!/usr/bin/env node
/** P3 U6 sizing probe: hand-built AcroForm fields through MuPDF's existing fill API. */
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { performance } from 'node:perf_hooks';
import mupdf from 'mupdf';

const out = path.resolve(process.argv[2] || '.quad/shared/p3/unit6/probe');
mkdirSync(out, { recursive: true });
const RESULTS = { generated: new Date().toISOString(), mupdf: '1.28.1', cases: {} };
const nullish = (o) => !o || o.isNull();
const indirect = (o) => nullish(o) ? null : (() => { try { return o.asIndirect(); } catch { return null; } })();
const value = (o) => {
  if (nullish(o)) return null;
  if (o.isName()) return `/${o.asName()}`;
  if (o.isString()) return o.asString();
  if (o.isBoolean()) return o.asBoolean();
  if (o.isNumber()) return o.asNumber();
  return o.toString();
};
const escapePdf = (s) => s.replace(/([\\()])/g, '\\$1');

function blankDoc() {
  const doc = new mupdf.PDFDocument();
  const page = doc.addPage([0, 0, 612, 792], 0, {}, Buffer.from(''));
  doc.insertPage(-1, page);
  return doc;
}

function ensureAcroForm(doc) {
  const root = doc.getTrailer().get('Root');
  let acro = root.get('AcroForm');
  if (nullish(acro)) {
    const fields = doc.newArray();
    const helv = doc.addObject({ Type: 'Font', Subtype: 'Type1', BaseFont: 'Helvetica', Encoding: 'WinAnsiEncoding' });
    acro = doc.addObject({ Fields: fields, DA: doc.newString('/Helv 12 Tf 0 g'), DR: { Font: { Helv: helv } }, NeedAppearances: false });
    root.put('AcroForm', acro);
    return { acro, fields, helv };
  }
  let fields = acro.get('Fields');
  if (nullish(fields)) { fields = doc.newArray(); acro.put('Fields', fields); }
  let dr = acro.get('DR');
  if (nullish(dr)) { dr = doc.newDictionary(); acro.put('DR', dr); }
  let fonts = dr.get('Font');
  if (nullish(fonts)) { fonts = doc.newDictionary(); dr.put('Font', fonts); }
  let helv = fonts.get('Helv');
  if (nullish(helv)) {
    helv = doc.addObject({ Type: 'Font', Subtype: 'Type1', BaseFont: 'Helvetica', Encoding: 'WinAnsiEncoding' });
    fonts.put('Helv', helv);
  }
  if (nullish(acro.get('DA'))) acro.put('DA', doc.newString('/Helv 12 Tf 0 g'));
  return { acro, fields, helv };
}

function pageAnnots(doc, pageObject) {
  let annots = pageObject.get('Annots');
  if (nullish(annots)) { annots = doc.newArray(); pageObject.put('Annots', annots); }
  return annots;
}

function addMerged(doc, pageIndex, spec, ctx = ensureAcroForm(doc)) {
  const page = doc.findPage(pageIndex);
  const dict = {
    Type: 'Annot', Subtype: 'Widget', FT: spec.ft, T: doc.newString(spec.name),
    Rect: spec.rect, F: 4, P: page, DA: doc.newString('/Helv 12 Tf 0 g'),
  };
  if (spec.ff) dict.Ff = spec.ff;
  if (spec.opt) dict.Opt = spec.opt.map((x) => doc.newString(x));
  if (spec.as) dict.AS = spec.as;
  const ref = doc.addObject(dict);
  ctx.fields.push(ref);
  pageAnnots(doc, page).push(ref);
  return ref;
}

function addTrio(doc, pageIndex = 0, prefix = 'probe') {
  const ctx = ensureAcroForm(doc);
  const specs = [
    { ft: 'Tx', name: `${prefix}_text`, rect: [360, 650, 540, 680] },
    { ft: 'Btn', name: `${prefix}_check`, rect: [360, 600, 380, 620], as: 'Off' },
    { ft: 'Ch', name: `${prefix}_choice`, rect: [360, 550, 540, 580], ff: mupdf.PDFWidget.CH_FIELD_IS_COMBO, opt: ['one', 'two', 'three'] },
  ];
  return { ...ctx, specs, refs: specs.map((spec) => addMerged(doc, pageIndex, spec, ctx)) };
}

function addCheckboxAppearance(doc, dict, rect) {
  const w = rect[2] - rect[0], h = rect[3] - rect[1];
  const common = { Type: 'XObject', Subtype: 'Form', BBox: [0, 0, w, h], Resources: {} };
  const off = doc.addStream(Buffer.from(`q 1 g 0 0 ${w} ${h} re f 0 G 1 w .5 .5 ${w - 1} ${h - 1} re S Q\n`), common);
  const yes = doc.addStream(Buffer.from(`q 1 g 0 0 ${w} ${h} re f 0 G 1 w .5 .5 ${w - 1} ${h - 1} re S 3 3 m ${w - 3} ${h - 3} l S 3 ${h - 3} m ${w - 3} 3 l S Q\n`), common);
  dict.put('AP', { N: { Off: off, Yes: yes } });
}

function addTextAppearance(doc, dict, rect, text, helv) {
  const w = rect[2] - rect[0], h = rect[3] - rect[1];
  const contents = `q 1 g 0 0 ${w} ${h} re f 0 G 1 w .5 .5 ${w - 1} ${h - 1} re S /Tx BMC BT /Helv 12 Tf 0 g 3 ${Math.max(3, (h - 12) / 2)} Td (${escapePdf(text)}) Tj ET EMC Q\n`;
  const ap = doc.addStream(Buffer.from(contents), { Type: 'XObject', Subtype: 'Form', BBox: [0, 0, w, h], Resources: { Font: { Helv: helv } } });
  dict.put('AP', { N: ap });
}

function appearance(dict) {
  const ap = dict.get('AP');
  const n = nullish(ap) ? null : ap.get('N');
  if (nullish(n)) return { present: false, kind: null, object: null };
  if (n.isStream()) return { present: true, kind: 'stream', object: indirect(n) };
  if (n.isDictionary()) return { present: true, kind: 'state-dictionary', states: (() => { const x = []; n.forEach((_v, k) => x.push(String(k))); return x.sort(); })() };
  return { present: true, kind: 'other', object: indirect(n) };
}

function widgetFacts(widget) {
  const dict = widget.getObject();
  return {
    name: widget.getName(), type: widget.getFieldType(), value: widget.getValue(),
    V: value(dict.get('V')), AS: value(dict.get('AS')), AP: appearance(dict), rect: value(dict.get('Rect')),
  };
}

function darkPixels(pix, rect, pageWidth = 612, pageHeight = 792) {
  const width = pix.getWidth(), height = pix.getHeight(), n = pix.getNumberOfComponents(), stride = pix.getStride();
  const pixels = pix.getPixels(), sx = width / pageWidth, sy = height / pageHeight;
  const x0 = Math.max(0, Math.floor(rect[0] * sx)), x1 = Math.min(width, Math.ceil(rect[2] * sx));
  const y0 = Math.max(0, Math.floor((pageHeight - rect[3]) * sy)), y1 = Math.min(height, Math.ceil((pageHeight - rect[1]) * sy));
  let dark = 0;
  for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
    const at = y * stride + x * n;
    if (pixels[at] + pixels[at + Math.min(1, n - 1)] + pixels[at + Math.min(2, n - 1)] < 600) dark++;
  }
  return dark;
}

function render(doc, name, specs) {
  const page = doc.loadPage(0);
  const pix = page.toPixmap(mupdf.Matrix.scale(2, 2), mupdf.ColorSpace.DeviceRGB, false);
  writeFileSync(path.join(out, name), pix.asPNG());
  const dark = Object.fromEntries(specs.map((s) => [s.name, darkPixels(pix, s.rect)]));
  page.destroy();
  return dark;
}

function save(doc, name) {
  const data = doc.saveToBuffer({ incremental: false, garbage: true, compress: true }).asUint8Array();
  const file = path.join(out, name);
  writeFileSync(file, data);
  return { file, bytes: data.length, sha256: createHash('sha256').update(data).digest('hex') };
}

function quickLook(pdf, tag, specs) {
  const dir = path.join(out, `ql-${tag}`);
  rmSync(dir, { recursive: true, force: true }); mkdirSync(dir, { recursive: true });
  const run = spawnSync('/usr/bin/qlmanage', ['-t', '-s', '1584', '-o', dir, pdf], { encoding: 'utf8' });
  const png = path.join(dir, `${path.basename(pdf)}.png`);
  if (run.status !== 0 || !existsSync(png)) return { status: run.status, error: (run.stderr || run.stdout).trim() };
  const image = new mupdf.Image(readFileSync(png));
  const pix = image.toPixmap();
  return {
    status: 0, png: path.relative(out, png), width: pix.getWidth(), height: pix.getHeight(),
    dark: Object.fromEntries(specs.map((s) => [s.name, darkPixels(pix, s.rect)])),
  };
}

function fillCreated(doc, added, prefix) {
  const page = doc.loadPage(0);
  const widgets = page.getWidgets();
  const selected = widgets.filter((w) => w.getName().startsWith(`${prefix}_`));
  const results = [];
  for (const widget of selected) {
    const dict = widget.getObject();
    let setResult;
    if (widget.isText()) setResult = widget.setTextValue('abc');
    else if (widget.isCheckbox()) setResult = widget.toggle();
    else setResult = widget.setChoiceValue('two');
    const updateResult = widget.update();
    const beforeFallbackAP = appearance(dict);
    let route = 'mupdf widget.update()', fallbackUpdateResult = null;
    if (!beforeFallbackAP.present) {
      if (widget.isCheckbox()) addCheckboxAppearance(doc, dict, added.specs.find((s) => s.name === widget.getName()).rect);
      else addTextAppearance(doc, dict, added.specs.find((s) => s.name === widget.getName()).rect, widget.getValue(), added.helv);
      fallbackUpdateResult = widget.update();
      route = 'manual /AP fallback';
    }
    results.push({ ...widgetFacts(widget), setResult, updateResult, beforeFallbackAP, fallbackUpdateResult, appearanceRoute: route });
  }
  return { pageWidgetCount: widgets.length, createdWidgetCount: selected.length, widgets: results };
}

function mainCase(tag, source = null) {
  const doc = source ? new mupdf.PDFDocument(readFileSync(source)) : blankDoc();
  const beforeExisting = source ? null : 0;
  const added = addTrio(doc, 0, tag);
  const beforeDark = render(doc, `${tag}-before-mupdf.png`, added.specs);
  const filled = fillCreated(doc, added, tag);
  const afterDark = render(doc, `${tag}-after-mupdf.png`, added.specs);
  const saved = save(doc, `${tag}-filled.pdf`);
  const reopened = new mupdf.PDFDocument(readFileSync(saved.file));
  const reopenedWidgets = reopened.loadPage(0).getWidgets().filter((w) => w.getName().startsWith(`${tag}_`)).map(widgetFacts);
  return {
    source, beforeExisting, fieldsBeforeFill: added.refs.length, fill: filled,
    render: { beforeDark, afterDark, delta: Object.fromEntries(added.specs.map((s) => [s.name, afterDark[s.name] - beforeDark[s.name]])) },
    save: { ...saved, file: path.relative(out, saved.file) }, reopened: { count: reopenedWidgets.length, widgets: reopenedWidgets },
    quickLook: quickLook(saved.file, tag, added.specs), specs: added.specs,
  };
}

function staleCacheCase() {
  const doc = blankDoc();
  let page = doc.loadPage(0);
  const initial = page.getWidgets().length;
  addMerged(doc, 0, { ft: 'Tx', name: 'stale_text', rect: [72, 650, 252, 680] });
  const same = page.getWidgets().length;
  const pageUpdate = page.update();
  const afterUpdate = page.getWidgets().length;
  const reloadStart = performance.now();
  page.destroy(); page = doc.loadPage(0);
  const afterReload = page.getWidgets().length, reloadMs = performance.now() - reloadStart;
  doc.setPageTreeCache(false); page = doc.loadPage(0);
  const afterPageTreeCacheOff = page.getWidgets().length;
  const start = performance.now();
  const saved = doc.saveToBuffer({ incremental: false, garbage: true });
  const reopened = new mupdf.PDFDocument(saved);
  const ms = performance.now() - start;
  return { initial, same, pageUpdate, afterUpdate, destroyAndReload: { count: afterReload, ms: reloadMs }, afterPageTreeCacheOff, saveReopen: { count: reopened.loadPage(0).getWidgets().length, bytes: saved.length, ms } };
}

function createAnnotationCase() {
  const one = (prescan) => {
    const doc = blankDoc(), page = doc.loadPage(0);
    if (prescan) page.getWidgets();
    const ctx = ensureAcroForm(doc), annot = page.createAnnotation('Widget'), dict = annot.getObject();
    dict.put('FT', 'Tx'); dict.put('T', doc.newString(`native_${prescan}`)); dict.put('Rect', [72, 650, 252, 680]);
    dict.put('F', 4); dict.put('DA', doc.newString('/Helv 12 Tf 0 g')); ctx.fields.push(dict);
    const widgets = page.getWidgets();
    return { prescan, createdType: annot.getType(), listed: widgets.length, names: widgets.map((w) => w.getName()) };
  };
  return [one(false), one(true)];
}

function radioCase() {
  const doc = blankDoc(), ctx = ensureAcroForm(doc), pageObject = doc.findPage(0), annots = pageAnnots(doc, pageObject);
  const parent = doc.createObject();
  const makeKid = (state, rect) => {
    const w = rect[2] - rect[0], h = rect[3] - rect[1];
    const common = { Type: 'XObject', Subtype: 'Form', BBox: [0, 0, w, h], Resources: {} };
    const off = doc.addStream(Buffer.from(`q 1 g 0 0 ${w} ${h} re f 0 G 1 w .5 .5 ${w - 1} ${h - 1} re S Q\n`), common);
    const on = doc.addStream(Buffer.from(`q 1 g 0 0 ${w} ${h} re f 0 G 1 w .5 .5 ${w - 1} ${h - 1} re S 0 g 5 5 ${w - 10} ${h - 10} re f Q\n`), common);
    return doc.addObject({ Type: 'Annot', Subtype: 'Widget', Parent: parent, Rect: rect, F: 4, P: pageObject, AS: 'Off', AP: { N: { Off: off, [state]: on } } });
  };
  const a = makeKid('A', [72, 650, 92, 670]), b = makeKid('B', [110, 650, 130, 670]);
  parent.writeObject({ FT: 'Btn', T: doc.newString('group'), Ff: mupdf.PDFWidget.BTN_FIELD_IS_RADIO, Kids: [a, b], V: 'Off' });
  ctx.fields.push(parent); annots.push(a); annots.push(b);
  const widgets = doc.loadPage(0).getWidgets(), specs = [{ name: 'radio_A', rect: [72, 650, 92, 670] }, { name: 'radio_B', rect: [110, 650, 130, 670] }];
  const names = widgets.map((w) => w.getName());
  const firstResult = widgets[0].toggle(), firstUpdate = widgets[0].update();
  const afterFirst = { parentV: value(parent.get('V')), states: widgets.map((w) => value(w.getObject().get('AS'))), dark: render(doc, 'radio-after-first.png', specs) };
  const secondResult = widgets[1].toggle(), secondUpdate = widgets[1].update();
  const afterSecond = { parentV: value(parent.get('V')), states: widgets.map((w) => value(w.getObject().get('AS'))), dark: render(doc, 'radio-after-second.png', specs) };
  const saved = save(doc, 'radio-pair.pdf');
  const reopened = new mupdf.PDFDocument(readFileSync(saved.file)).loadPage(0).getWidgets();
  return { listed: widgets.length, names, uniqueNames: new Set(names).size, firstResult, firstUpdate, afterFirst, secondResult, secondUpdate, afterSecond, reopened: reopened.map(widgetFacts), quickLook: quickLook(saved.file, 'radio', specs), save: { ...saved, file: path.relative(out, saved.file) } };
}

function mutationCase() {
  const doc = blankDoc(), ctx = ensureAcroForm(doc);
  const spec = { ft: 'Tx', name: 'mutable', rect: [72, 650, 252, 680] };
  const ref = addMerged(doc, 0, spec, ctx), page = doc.loadPage(0), widget = page.getWidgets()[0], dict = widget.getObject();
  widget.setTextValue('abc'); widget.update();
  const apBefore = appearance(dict), rectBefore = [...widget.getRect()];
  const oldSpec = { name: 'oldRect', rect: spec.rect }, newSpec = { name: 'newRect', rect: [300, 500, 480, 530] };
  const beforeMoveDark = render(doc, 'mutation-before-move.png', [oldSpec, newSpec]);
  dict.put('T', doc.newString('renamed')); dict.put('Rect', newSpec.rect);
  const moveUpdate = widget.update(), apAfter = appearance(dict), rectAfter = [...widget.getRect()];
  const afterMoveDark = render(doc, 'mutation-after-move.png', [oldSpec, newSpec]);
  const renameVisible = widget.getName();
  doc.findPage(0).put('Annots', doc.newArray()); ctx.acro.put('Fields', doc.newArray());
  const rawAfterDelete = { annots: doc.findPage(0).get('Annots').length, fields: ctx.acro.get('Fields').length };
  const cachedWidgets = page.getWidgets().length;
  page.destroy();
  const reloadedWidgets = doc.loadPage(0).getWidgets().length;
  const saved = save(doc, 'renamed-moved-deleted.pdf');
  const reopened = new mupdf.PDFDocument(readFileSync(saved.file));
  return { ref: indirect(ref), renameVisible, rectBefore, rectAfter, moveUpdate, apBefore, apAfter, render: { beforeMoveDark, afterMoveDark }, rawAfterDelete, cachedWidgets, reloadedWidgets, reopenedWidgets: reopened.loadPage(0).getWidgets().length, save: { ...saved, file: path.relative(out, saved.file) } };
}

function refusalCase() {
  const makeFlagged = (tag, key, setting) => {
    const doc = blankDoc(), ctx = ensureAcroForm(doc); ctx.acro.put(key, setting);
    addMerged(doc, 0, { ft: 'Tx', name: `${tag}_text`, rect: [72, 650, 252, 680] }, ctx);
    const listed = doc.loadPage(0).getWidgets().length, saved = save(doc, `${tag}.pdf`);
    return { flag: value(ctx.acro.get(key)), libraryAllowedCreate: listed === 1, save: { ...saved, file: path.relative(out, saved.file) } };
  };
  const xfaDoc = blankDoc(), xfaCtx = ensureAcroForm(xfaDoc); xfaCtx.acro.put('XFA', xfaDoc.newString('stub'));
  addMerged(xfaDoc, 0, { ft: 'Tx', name: 'xfa_text', rect: [72, 650, 252, 680] }, xfaCtx);
  const xfaSave = save(xfaDoc, 'xfa.pdf');
  const xfa = { flag: value(xfaCtx.acro.get('XFA')), libraryAllowedCreate: xfaDoc.loadPage(0).getWidgets().length === 1, save: { ...xfaSave, file: path.relative(out, xfaSave.file) } };
  const need = makeFlagged('need-appearances', 'NeedAppearances', true);
  let certified;
  try {
    const doc = new mupdf.PDFDocument(readFileSync('fixtures/p0b/certified-p2.pdf')), ctx = ensureAcroForm(doc);
    const before = ctx.fields.length;
    addMerged(doc, 0, { ft: 'Tx', name: 'certified_probe', rect: [360, 650, 540, 680] }, ctx);
    const listed = doc.loadPage(0).getWidgets().some((w) => w.getName() === 'certified_probe');
    const saved = save(doc, 'certified-created.pdf');
    certified = { fieldsBefore: before, libraryAllowedCreate: listed, saveAllowed: true, save: { ...saved, file: path.relative(out, saved.file) } };
  } catch (error) { certified = { libraryAllowedCreate: false, saveAllowed: false, error: String(error) }; }
  return { xfa, needAppearances: need, certified };
}

function journalCase() {
  const doc = blankDoc(), ctx = ensureAcroForm(doc), pageObject = doc.findPage(0);
  pageAnnots(doc, pageObject);
  doc.enableJournal();
  doc.beginOperation('create field');
  addMerged(doc, 0, { ft: 'Tx', name: 'journal_text', rect: [72, 650, 252, 680] }, ctx);
  doc.endOperation();
  const counts = () => ({ fields: ctx.acro.get('Fields').length, annots: pageObject.get('Annots').length, widgets: doc.loadPage(0).getWidgets().length });
  const afterCreate = { ...counts(), journal: doc.getJournal() };
  doc.undo();
  const afterUndo = { ...counts(), canRedo: doc.canRedo() };
  doc.redo();
  const afterRedo = counts();
  const saved = save(doc, 'journal-redo.pdf'), reopened = new mupdf.PDFDocument(readFileSync(saved.file));
  return { afterCreate, afterUndo, afterRedo, reopenedWidgets: reopened.loadPage(0).getWidgets().length, save: { ...saved, file: path.relative(out, saved.file) } };
}

function pypdf(files) {
  const py = `
import json,sys
from pypdf import PdfReader
from pypdf.generic import DictionaryObject, IndirectObject
out={}
for f in sys.argv[1:]:
 r=PdfReader(f); fields=r.get_fields() or {}; links=[]
 for generation, ids in r.xref.items():
  for number in ids:
   try:
    obj=r.get_object(IndirectObject(number,generation,r))
    if isinstance(obj,DictionaryObject) and (obj.get('/Subtype')=='/Widget' or obj.get('/FT')) and ('/Parent' in obj or '/Kids' in obj): links.append(number)
   except Exception: pass
 out[f]={"pages":len(r.pages),"fields":{k:{"FT":str(v.get('/FT')),"V":str(v.get('/V')),"kids":len(v.get('/Kids',[]))} for k,v in fields.items()},"fieldTreeLinks":links}
print(json.dumps(out))
`;
  const python = existsSync('.venv-qa/bin/python') ? '.venv-qa/bin/python' : 'python3';
  const run = spawnSync(python, ['-c', py, ...files], { encoding: 'utf8' });
  if (run.status !== 0) return { error: (run.stderr || run.stdout).trim() };
  return JSON.parse(run.stdout);
}

RESULTS.cases.blank = mainCase('blank');
RESULTS.cases.form2 = mainCase('form2', 'fixtures/form2.pdf');
RESULTS.cases.staleCache = staleCacheCase();
RESULTS.cases.createAnnotation = createAnnotationCase();
RESULTS.cases.radio = radioCase();
RESULTS.cases.renameMoveDelete = mutationCase();
RESULTS.cases.refusals = refusalCase();
RESULTS.cases.journal = journalCase();
const pdfs = ['blank-filled.pdf', 'form2-filled.pdf', 'radio-pair.pdf', 'renamed-moved-deleted.pdf', 'journal-redo.pdf'].map((x) => path.join(out, x));
RESULTS.pypdf = pypdf(pdfs);
RESULTS.tokenScan = {};
for (const file of [...pdfs, path.join(out, 'xfa.pdf'), path.join(out, 'need-appearances.pdf'), path.join(out, 'certified-created.pdf')].filter(existsSync)) {
  const text = readFileSync(file).toString('latin1');
  RESULTS.tokenScan[path.basename(file)] = { AA: (text.match(/\/AA\b/g) || []).length, JS: (text.match(/\/JS\b/g) || []).length };
}
for (const source of ['fixtures/form2.pdf', 'fixtures/p0b/certified-p2.pdf']) {
  const text = readFileSync(source).toString('latin1');
  RESULTS.tokenScan[source] = { AA: (text.match(/\/AA\b/g) || []).length, JS: (text.match(/\/JS\b/g) || []).length };
}
const check = (ok, message) => { if (!ok) throw new Error(`probe check failed: ${message}`); };
for (const name of ['blank', 'form2']) {
  const c = RESULTS.cases[name];
  check(c.fill.createdWidgetCount === 3 && c.reopened.count === 3, `${name} create/list/reopen`);
  check(c.fill.widgets.every((w) => w.setResult === 1 && w.AP.present), `${name} fill/AP`);
  check(c.quickLook.status === 0 && Object.values(c.quickLook.dark).every((n) => n > 0), `${name} Quick Look`);
}
check(RESULTS.cases.staleCache.destroyAndReload.count === 1, 'page wrapper reload clears widget cache');
check(RESULTS.cases.radio.afterSecond.parentV === '/B' && RESULTS.cases.radio.afterSecond.states.join() === '/Off,/B', 'radio exclusivity');
check(RESULTS.cases.renameMoveDelete.reloadedWidgets === 0 && RESULTS.cases.renameMoveDelete.reopenedWidgets === 0, 'delete');
check(RESULTS.cases.journal.afterUndo.fields === 0 && RESULTS.cases.journal.afterRedo.fields === 1, 'journal undo/redo');
check(!RESULTS.pypdf.error, 'pypdf');
const deletedPy = Object.entries(RESULTS.pypdf).find(([file]) => file.endsWith('renamed-moved-deleted.pdf'))?.[1];
check(deletedPy && Object.keys(deletedPy.fields).length === 0 && deletedPy.fieldTreeLinks.length === 0, 'pypdf deleted field tree');
check(RESULTS.tokenScan['blank-filled.pdf'].AA === 0 && RESULTS.tokenScan['blank-filled.pdf'].JS === 0, 'no scripts');
writeFileSync(path.join(out, 'results.json'), `${JSON.stringify(RESULTS, null, 2)}\n`);
console.log(`wrote ${path.join(out, 'results.json')}`);
