// SPDX-License-Identifier: MIT
// The suite's reader of a document the Rust engine holds, over the engine's
// test-only `inspect` op (engine/src/app/ops/inspect.rs). Tests use it to look
// at what an op wrote: the trailer, page dictionaries, annotation and widget
// dictionaries, streams, the extracted text and its lines. The engine answers
// from its live document; the bytes a save writes are checked by opening them
// again (`engine.reopen`) and by the independent pypdf checks some tests run.
//
// Values come back as JavaScript values: null, booleans and numbers as
// themselves, a name as a `Name`, a string as a `PdfString` (bytes and decoded
// text), an array as a `List`, a dictionary or stream as a `Dict` and an
// unresolved reference as a `Ref`. `Dict.get` and `List.get` follow references;
// `raw` leaves them as `Ref`s, `ref` names the target. An inspector reads one
// document state (the harness opens the session's current bytes in a session
// of its own and hands out a new inspector whenever that state changes); a
// superseded or closed inspector is `stale`, and every read of it throws, so a
// held handle never mixes old and new values.

/** A PDF name, `/Page` as `new Name('Page')`; compares by `name`. */
export class Name {
  constructor(name) {
    this.name = name;
    Object.freeze(this);
  }
  toString() {
    return this.name;
  }
  [Symbol.toPrimitive]() {
    return this.name;
  }
  equals(other) {
    return other instanceof Name ? other.name === this.name : other === this.name;
  }
}

/** A PDF string: its `bytes` as stored and its `text` as a PDF text string. */
export class PdfString {
  constructor(latin1, text) {
    this.bytes = new Uint8Array(Buffer.from(latin1, 'latin1'));
    this.text = text;
    Object.freeze(this);
  }
  toString() {
    return this.text;
  }
  [Symbol.toPrimitive]() {
    return this.text;
  }
}

/** An indirect reference, `12 0 R`, not followed. */
export class Ref {
  constructor(number, generation) {
    this.number = number;
    this.generation = generation;
    Object.freeze(this);
  }
  /** `[number, generation]`, the id `Inspector.object` takes. */
  get id() {
    return [this.number, this.generation];
  }
  toString() {
    return `${this.number} ${this.generation} R`;
  }
}

/** Two object ids, `[number, generation]`, naming the same object. */
export function sameId(a, b) {
  return Array.isArray(a) && Array.isArray(b) && a[0] === b[0] && a[1] === b[1];
}

/** The name of a `Name`, else null. */
export function nameOf(value) {
  return value instanceof Name ? value.name : null;
}

/** A PDF array. `get` follows references; iteration yields resolved items. */
export class List {
  #inspector;
  #items;
  constructor(inspector, items) {
    this.#inspector = inspector;
    this.#items = items;
  }
  get length() {
    return this.#items.length;
  }
  /** Item `index` as stored: a `Ref` stays a `Ref`; undefined past the end. */
  raw(index) {
    return this.#items[index];
  }
  /** Item `index` with references followed; undefined past the end. */
  get(index) {
    return index < this.#items.length ? this.#inspector.resolve(this.#items[index]) : undefined;
  }
  /** `[number, generation]` when item `index` is a reference, else null. */
  ref(index) {
    const item = this.#items[index];
    return item instanceof Ref ? item.id : null;
  }
  /** Every item, references followed. */
  items() {
    return this.#items.map(item => this.#inspector.resolve(item));
  }
  /** Every item as a number (references followed); a non-number is NaN. */
  numbers() {
    return this.items().map(item => (typeof item === 'number' ? item : Number.NaN));
  }
  map(fn) {
    return this.items().map(fn);
  }
  filter(fn) {
    return this.items().filter(fn);
  }
  some(fn) {
    return this.items().some(fn);
  }
  find(fn) {
    return this.items().find(fn);
  }
  *[Symbol.iterator]() {
    yield* this.items();
  }
}

/**
 * A PDF dictionary, or a stream (`isStream`). `id` is `[number, generation]`
 * for an indirect object and null for a dictionary written in place.
 */
export class Dict {
  #inspector;
  #entries;
  #stream;
  constructor(inspector, entries, id = null, stream = null) {
    this.#inspector = inspector;
    this.#entries = entries;
    this.#stream = stream;
    this.id = id;
  }
  /** The object number of an indirect object, else null (`number(key)` reads a numeric entry). */
  get objectNumber() {
    return this.id ? this.id[0] : null;
  }
  get isStream() {
    return this.#stream !== null;
  }
  get keys() {
    return Object.keys(this.#entries);
  }
  has(key) {
    return Object.hasOwn(this.#entries, key);
  }
  /** The entry as stored (a `Ref` stays a `Ref`); null when absent or null. */
  raw(key) {
    return Object.hasOwn(this.#entries, key) ? this.#entries[key] : null;
  }
  /** The entry with references followed; null when absent or null. */
  get(key) {
    return Object.hasOwn(this.#entries, key) ? this.#inspector.resolve(this.#entries[key]) : null;
  }
  /** `[number, generation]` when the entry is a reference, else null. */
  ref(key) {
    const value = this.raw(key);
    return value instanceof Ref ? value.id : null;
  }
  /** Typed reads, null when the entry is absent or of another kind. */
  name(key) {
    return nameOf(this.get(key));
  }
  number(key) {
    const value = this.get(key);
    return typeof value === 'number' ? value : null;
  }
  boolean(key) {
    const value = this.get(key);
    return typeof value === 'boolean' ? value : null;
  }
  /** A string entry's decoded text. */
  string(key) {
    const value = this.get(key);
    return value instanceof PdfString ? value.text : null;
  }
  array(key) {
    const value = this.get(key);
    return value instanceof List ? value : null;
  }
  dict(key) {
    const value = this.get(key);
    return value instanceof Dict ? value : null;
  }
  /** The numbers of an array entry (`/Rect`, `/MediaBox`), else null. */
  numbers(key) {
    return this.array(key)?.numbers() ?? null;
  }
  /** `[[key, value]]` with references followed. */
  entries() {
    return this.keys.map(key => [key, this.get(key)]);
  }
  /** The stream's bytes with every filter decoded. */
  stream() {
    return this.#inspector.streamData(this.#streamId(), 'decoded');
  }
  /** The stream's bytes as stored. */
  rawStream() {
    return this.#inspector.streamData(this.#streamId(), 'raw');
  }
  /** The stored byte count of a stream, else null. */
  get streamLength() {
    return this.#stream?.length ?? null;
  }
  #streamId() {
    if (!this.isStream || !this.id) throw new Error(`inspect: object ${this.id?.join(' ') ?? '(direct)'} is not a stream`);
    return this.id;
  }
  /** Debug view: the entries with references left as `Ref`s. */
  toJSON() {
    return Object.fromEntries(this.keys.map(key => [key, this.raw(key)]));
  }
}

/** A dictionary's JSON entries as a Dict; `id` for an indirect object. */
function dictOf(inspector, json, id, stream = null) {
  return new Dict(inspector, Object.fromEntries(Object.entries(json).map(([key, value]) => [key, wrap(inspector, value)])), id, stream);
}

/** A wire value as a JavaScript value (see the file comment). */
function wrap(inspector, json, id = null) {
  if (json === null || typeof json !== 'object') return json;
  if (Array.isArray(json)) return new List(inspector, json.map(item => wrap(inspector, item)));
  if (Object.hasOwn(json, 'dict')) return dictOf(inspector, json.dict, id);
  if (Object.hasOwn(json, 'stream')) return dictOf(inspector, json.stream, id, { length: json.length });
  if (Object.hasOwn(json, 'ref')) return new Ref(json.ref[0], json.ref[1]);
  if (Object.hasOwn(json, 'name')) return new Name(json.name);
  if (Object.hasOwn(json, 'string')) return new PdfString(json.string, json.text);
  // `{deep: true}`: nesting past the engine's limit, which no test reaches.
  return null;
}

/**
 * The reader of one document state. `send(request)` dispatches an `inspect`
 * request (the op and, for the compare slot, `compare: true` are added) and
 * returns the reply; a refused read throws its error.
 */
export class Inspector {
  #send;
  #summary = null;
  #pages = new Map();
  #objects = new Map();
  /** True once the harness has let the document go (superseded or closed): reads throw from then on. */
  stale = false;

  constructor(send, { compare = false, owner = null } = {}) {
    this.#send = request => {
      this.#fresh();
      const reply = send(compare ? { ...request, op: 'inspect', compare: true } : { ...request, op: 'inspect' });
      if (reply?.ok !== true) throw new Error(reply?.error ?? 'inspect: no reply');
      return reply;
    };
    /** The hidden session of an `engine.reopen` document, closed by `engine.closeDocument`. */
    this.owner = owner;
  }

  #fresh() {
    if (this.stale) throw new Error('inspect: this reader is stale (the document changed or was closed); read session.doc again');
  }

  #summarize() {
    this.#fresh();
    this.#summary ??= this.#send({});
    return this.#summary;
  }

  get pageCount() {
    return this.#summarize().pageCount;
  }
  /** The object holding each page, `[number, generation]` or null, in page order. */
  get pageIds() {
    return this.#summarize().pages;
  }
  /** Every object id, ascending. */
  get objectIds() {
    return this.#summarize().objects;
  }
  /** The effective trailer dictionary. */
  get trailer() {
    return dictOf(this, this.#summarize().trailer, null);
  }
  /** The catalog (`/Root`). */
  get root() {
    return this.trailer.get('Root');
  }
  /** `DocumentInfo` as the `open` reply reports it: encryption, signatures, permissions, ... */
  get info() {
    return this.#summarize().info;
  }
  /** The header version, `1.7`. */
  get version() {
    return this.#summarize().version;
  }
  /** Whether the open repaired the file. */
  get repaired() {
    return this.#summarize().repaired;
  }
  /** Still waiting for a password. */
  get locked() {
    return this.#summarize().locked;
  }

  /** Page `index`'s own dictionary (no inheritance), with its `id`. */
  page(index) {
    this.#fresh();
    if (!this.#pages.has(index)) {
      const reply = this.#send({ page: index });
      this.#pages.set(index, wrap(this, reply.value, reply.object));
    }
    return this.#pages.get(index);
  }

  /** The object `number generation`, or null when there is none. */
  object(number, generation = 0) {
    this.#fresh();
    const key = `${number} ${generation}`;
    if (!this.#objects.has(key)) {
      const reply = this.#send({ object: [number, generation] });
      this.#objects.set(key, wrap(this, reply.value, reply.object));
    }
    return this.#objects.get(key);
  }

  /** `value` with references followed (a chain of them too). */
  resolve(value) {
    let seen = 0;
    while (value instanceof Ref && seen++ < 32) value = this.object(value.number, value.generation);
    return value instanceof Ref ? null : value;
  }

  /** A stream's bytes, `kind` `decoded` or `raw`; `id` is `[number, generation]`. */
  streamData(id, kind) {
    const reply = this.#send({ object: id, data: kind });
    return new Uint8Array(reply.data);
  }

  /** The decoded bytes of a page's content: one stream or the array joined, with a newline between parts. */
  contents(index) {
    const contents = this.page(index).get('Contents');
    if (contents instanceof Dict) return contents.stream();
    if (!(contents instanceof List)) return new Uint8Array(0);
    const parts = contents.items().filter(part => part instanceof Dict).map(part => part.stream());
    const out = [];
    parts.forEach((part, at) => {
      out.push(...part);
      if (at < parts.length - 1) out.push(10);
    });
    return new Uint8Array(out);
  }

  /** The page's annotation dictionaries, in `/Annots` order; each indirect one carries its `id`. */
  annots(index) {
    const annots = this.page(index).get('Annots');
    return annots instanceof List ? annots.items().filter(annot => annot instanceof Dict) : [];
  }
  /** The page's annotations of `/Subtype` `subtype`. */
  annotsOf(index, subtype) {
    return this.annots(index).filter(annot => annot.name('Subtype') === subtype);
  }
  /** The page's widgets. */
  widgets(index) {
    return this.annotsOf(index, 'Widget');
  }
  /** The widget named `name` (its own or an ancestor's `/T`), else undefined. */
  widget(index, name) {
    return this.widgets(index).find(widget => fieldName(widget) === name);
  }

  /** The page's extracted text: one line per line, a blank line after each block. */
  text(index) {
    return this.#send({ text: index }).text;
  }
  /** The page's lines, `{text, x, y, top, w, h, size, family, weight, style}`, integer points, y down. */
  lines(index) {
    return this.#send({ text: index }).lines;
  }
  /** Every character of the page, `{c, x, y, size, quad}`: baseline origin in points, y down. */
  chars(index) {
    return this.#send({ text: index, chars: true }).chars;
  }
}

/** A field's fully qualified name: its `/T` and its ancestors', joined by dots. */
export function fieldName(widget) {
  const parts = [];
  for (let node = widget, depth = 0; node instanceof Dict && depth < 32; node = node.get('Parent'), depth++) {
    const part = node.string('T');
    if (part !== null) parts.unshift(part);
  }
  return parts.join('.');
}

/** A field's value (`/V`, inherited through `/Parent`) as text: a name's name, a string's text, else null. */
export function fieldValue(widget) {
  for (let node = widget, depth = 0; node instanceof Dict && depth < 32; node = node.get('Parent'), depth++) {
    if (!node.has('V')) continue;
    const value = node.get('V');
    if (value instanceof Name) return value.name;
    if (value instanceof PdfString) return value.text;
    return value;
  }
  return null;
}

/** An inherited page attribute (`/Rotate`, `/MediaBox`, `/Resources`), read up the `/Parent` chain. */
export function inherited(page, key) {
  for (let node = page, depth = 0; node instanceof Dict && depth < 64; node = node.get('Parent'), depth++) {
    if (node.has(key)) return node.get(key);
  }
  return null;
}

/** A `/Rect`-style array as a normalised `[x0, y0, x1, y1]`, else null. */
export function rectOf(value) {
  const numbers = value instanceof List ? value.numbers() : Array.isArray(value) ? value : null;
  if (!numbers || numbers.length !== 4 || numbers.some(n => !Number.isFinite(n))) return null;
  const [a, b, c, d] = numbers;
  return [Math.min(a, c), Math.min(b, d), Math.max(a, c), Math.max(b, d)];
}

/** The bytes of a stream or buffer as latin1 text, for content-stream assertions. */
export function latin1(bytes) {
  return Buffer.from(bytes.buffer ?? bytes, bytes.byteOffset ?? 0, bytes.byteLength).toString('latin1');
}
