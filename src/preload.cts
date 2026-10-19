import { contextBridge, ipcRenderer, webUtils } from 'electron';

type OpenError = { reason: 'too-large' | 'unreadable' | 'cancelled'; name?: string; limit?: number };
type SaveResult =
  | { ok: true; name: string; warning?: 'source-changed' | 'durability-uncertain'; sameFile?: boolean }
  | { ok: false; reason: 'cancelled' | 'too-large' | 'disk-full' | 'permission' | 'unreadable' | 'same-file' | 'document-changed' | 'busy' | 'write-failed' | 'source-changed' | 'broken-link' };
type PrintResult = { ok: true } | { ok: false; reason: 'cancelled' | 'busy' | 'failed' | 'no-printer' };
type InsertSource =
  | { ok: true; name: string; bytes: ArrayBuffer }
  | { ok: false; reason: 'cancelled' | 'too-large' | 'unreadable' };
type SaveNewResult =
  | { ok: true; name: string; url?: string; original?: boolean }
  | { ok: false; reason: 'cancelled' | 'too-large' | 'unreadable' | 'busy' | 'permission' | 'disk-full' | 'broken-link' | 'same-file' };
type PickImages =
  | { ok: true; images: { name: string; bytes: ArrayBuffer }[] }
  | { ok: false; reason: 'cancelled' | 'too-large' | 'too-many' | 'unreadable' };
type SaveKind = 'pdf' | 'png' | 'jpeg' | 'txt';
type PickFolder =
  | { ok: true; folder: string; out: string; files: { name: string; bytes: number }[] }
  | { ok: false; reason: string };
type BatchIo = { ok: true; bytes?: ArrayBuffer; exists?: boolean; name?: string } | { ok: false; reason: string };

const MAX_SAVE_BYTES = 50 * 1024 * 1024; // duplicate of renderer/limits.ts; change both together
const MAX_NOTE_CHARS = 4096; // duplicate of renderer/engine-shared.ts; change both together
const MAX_LINK_URI = 2048;
/** draw-signature review: a drawn signature's JSON is far over MAX_NOTE_CHARS; the store's own 4 MiB read cap bounds it. */
const MAX_SIGN_DRAW_CHARS = 4 * 1024 * 1024;

contextBridge.exposeInMainWorld('pdf4all', {
  openFile: (): Promise<void> => ipcRenderer.invoke('open-file') as Promise<void>,
  // UX audit X4: the start screen. Main keeps the paths; an entry is opened by its index.
  newBlank: (): Promise<void> => ipcRenderer.invoke('new-blank') as Promise<void>,
  recentFiles: (): Promise<{ id: string; name: string; folder: string }[]> => ipcRenderer.invoke('recent-list') as Promise<{ id: string; name: string; folder: string }[]>,
  clearRecent: (): Promise<void> => ipcRenderer.invoke('recent-clear') as Promise<void>,
  openRecent: (id: string): Promise<void> => (typeof id === 'string' && id.length <= 64 ? ipcRenderer.invoke('recent-open', id) as Promise<void> : Promise.resolve()),
  openDropped: (file: File): Promise<void> => {
    try {
      const path = webUtils.getPathForFile(file);
      if (!path) return Promise.resolve();
      return ipcRenderer.invoke('open-path', path) as Promise<void>;
    } catch {
      return Promise.resolve();
    }
  },
  saveFile: (bytes: ArrayBuffer, documentUrl: string, options?: { inPlace?: boolean; filled?: boolean; rebind?: boolean; suggestedName?: string }): Promise<SaveResult> => {
    if (bytes instanceof ArrayBuffer && bytes.byteLength > MAX_SAVE_BYTES) {
      return Promise.resolve({ ok: false, reason: 'too-large' });
    }
    return ipcRenderer.invoke('save-file', bytes, documentUrl, options) as Promise<SaveResult>;
  },
  pickInsertSource: (): Promise<InsertSource> => ipcRenderer.invoke('insert-source') as Promise<InsertSource>,
  saveNew: (bytes: ArrayBuffer, suggestedName: string, kind: SaveKind, options?: { open?: boolean; redacted?: boolean }): Promise<SaveNewResult> => {
    if (!(bytes instanceof ArrayBuffer) || bytes.byteLength > MAX_SAVE_BYTES) {
      return Promise.resolve({ ok: false, reason: 'too-large' });
    }
    if (kind !== 'pdf' && kind !== 'png' && kind !== 'jpeg' && kind !== 'txt') {
      return Promise.resolve({ ok: false, reason: 'unreadable' });
    }
    return ipcRenderer.invoke('save-new', bytes, suggestedName, kind, options) as Promise<SaveNewResult>;
  },
  pickImages: (): Promise<PickImages> => ipcRenderer.invoke('pick-images') as Promise<PickImages>,
  pickFolder: (): Promise<PickFolder> => ipcRenderer.invoke('pick-folder') as Promise<PickFolder>,
  batchIo: (name: string, action: 'read' | 'write' | 'exists', bytes?: ArrayBuffer): Promise<BatchIo> => {
    if (action === 'write' && (!(bytes instanceof ArrayBuffer) || bytes.byteLength > MAX_SAVE_BYTES)) {
      return Promise.resolve({ ok: false, reason: 'too-large' });
    }
    return ipcRenderer.invoke('batch-io', name, action, bytes) as Promise<BatchIo>;
  },
  setDirty: (dirty: boolean): void => {
    if (typeof dirty !== 'boolean') return;
    ipcRenderer.send('dirty', dirty);
  },
  /** Redaction redesign §2.3: how many boxes are marked but not redacted (main's close prompt names them). */
  setRedactPending: (count: number, memory: boolean): void => {
    if (!Number.isInteger(count) || count < 0) return;
    ipcRenderer.send('redact-pending', count, memory === true);
  },
  /** §2.3 step 6: the renderer opened the copy (or the original) it was handed; main binds the window to it now. */
  bindOpen: (url: string): Promise<boolean> => typeof url === 'string' ? ipcRenderer.invoke('bind-open', url) as Promise<boolean> : Promise.resolve(false),
  /** §2.3 step 6: reopen the file the open redacted copy was made from. */
  openOriginal: (): Promise<{ ok: true; name: string; url: string } | { ok: false; reason: string }> =>
    ipcRenderer.invoke('open-original') as Promise<{ ok: true; name: string; url: string } | { ok: false; reason: string }>,
  onDocument: (cb: (doc: { name: string; size: number; url: string; untitled?: boolean }) => void): void => {
    ipcRenderer.on('document', (_event, payload: { name?: unknown; size?: unknown; url?: unknown; untitled?: unknown }) => {
      if (typeof payload?.name !== 'string' || typeof payload?.size !== 'number' || typeof payload?.url !== 'string') return;
      cb({ name: payload.name, size: payload.size, url: payload.url, ...(payload.untitled === true ? { untitled: true } : {}) });
    });
  },
  onOpenError: (cb: (err: OpenError) => void): void => {
    ipcRenderer.on('open-error', (_event, err: OpenError) => cb(err));
  },
  onSaveRequest: (cb: (payload: { inPlace?: boolean; as?: boolean }) => void): void => {
    ipcRenderer.on('save-request', (_event, payload: unknown) => {
      if (payload == null) {
        cb({});
        return;
      }
      if (typeof payload !== 'object' || Array.isArray(payload)) return;
      const inPlace = (payload as { inPlace?: unknown }).inPlace;
      // UX item 7: `as` is Save As (the window then edits the new file); neither flag is Save a copy.
      const as = (payload as { as?: unknown }).as;
      if (as === true) cb({ as: true });
      else if (inPlace === undefined) cb({});
      else if (typeof inPlace === 'boolean') cb({ inPlace });
    });
  },
  onEditRequest: (cb: (payload: { op: string; contents?: string }) => void): void => {
    ipcRenderer.on('edit-request', (_event, payload: unknown) => {
      if (payload == null || typeof payload !== 'object' || Array.isArray(payload)) return;
      const op = (payload as { op?: unknown }).op;
      if (typeof op !== 'string' || !op) return;
      const contents = (payload as { contents?: unknown }).contents;
      if (contents === undefined) {
        cb({ op });
        return;
      }
      if (typeof contents === 'string' && contents.length <= (op === 'sign-draw' ? MAX_SIGN_DRAW_CHARS : MAX_NOTE_CHARS)) cb({ op, contents });
      // An over-cap sign-draw is refused in STATE error (empty contents), never dropped into a step timeout.
      else if (op === 'sign-draw') cb({ op, contents: '' });
    });
  },
  openLink: (uri: string): Promise<void> => {
    if (typeof uri !== 'string' || !uri || uri.length > MAX_LINK_URI) return Promise.resolve();
    return ipcRenderer.invoke('open-link', uri) as Promise<void>;
  },
  pickCertificate: (opts?: { path?: string }): Promise<{ ok: true; name: string } | { ok: false; reason: string }> => {
    if (opts !== undefined && (opts === null || typeof opts !== 'object' || (opts.path !== undefined && typeof opts.path !== 'string'))) {
      return Promise.resolve({ ok: false, reason: 'unreadable' });
    }
    return ipcRenderer.invoke('pick-certificate', opts) as Promise<{ ok: true; name: string } | { ok: false; reason: string }>;
  },
  certificateInfo: (password: string): Promise<{ ok: true; cn: string; notAfter: string; certs: number } | { ok: false; reason: string; date?: string; status?: string }> => {
    if (typeof password !== 'string' || password.length > 1024) return Promise.resolve({ ok: false, reason: 'unreadable' });
    return ipcRenderer.invoke('certificate-info', password) as Promise<{ ok: true; cn: string; notAfter: string; certs: number } | { ok: false; reason: string; date?: string; status?: string }>;
  },
  certificateClear: (): Promise<{ ok: boolean }> => ipcRenderer.invoke('certificate-clear') as Promise<{ ok: boolean }>,
  verifySignatures: (bytes: ArrayBuffer, entries: unknown[]): Promise<{ ok: true; verdicts: unknown[]; trusted: { cn: string; notAfter: string }[] } | { ok: false; reason: string }> => {
    if (!(bytes instanceof ArrayBuffer) || bytes.byteLength > MAX_SAVE_BYTES) {
      return Promise.resolve({ ok: false, reason: bytes instanceof ArrayBuffer ? 'too-large' : 'unreadable' });
    }
    if (!Array.isArray(entries)) return Promise.resolve({ ok: false, reason: 'unreadable' });
    return ipcRenderer.invoke('verify-signatures', bytes, entries) as Promise<{ ok: true; verdicts: unknown[]; trusted: { cn: string; notAfter: string }[] } | { ok: false; reason: string }>;
  },
  trustAdd: (opts?: { path?: string }): Promise<{ ok: boolean; reason?: string; added?: number; total?: number }> => {
    if (opts !== undefined && (opts === null || typeof opts !== 'object' || (opts.path !== undefined && typeof opts.path !== 'string'))) {
      return Promise.resolve({ ok: false, reason: 'unreadable' });
    }
    return ipcRenderer.invoke('trust-add', opts) as Promise<{ ok: boolean; reason?: string; added?: number; total?: number }>;
  },
  trustClear: (): Promise<{ ok: boolean }> => ipcRenderer.invoke('trust-clear') as Promise<{ ok: boolean }>,
  /** draw-signature D3: the saved drawn signature (pad px strokes), or null; set null removes it. */
  signatureGet: (): Promise<{ ok: boolean; signature: unknown }> => ipcRenderer.invoke('signature-get') as Promise<{ ok: boolean; signature: unknown }>,
  signatureSet: (value: unknown): Promise<{ ok: boolean }> => ipcRenderer.invoke('signature-set', value) as Promise<{ ok: boolean }>,
  signFile: (bytes: ArrayBuffer, offsets: { contentsAt: number; contentsLen: number; byteRangeAt: number }, when: number, suggestedName: string): Promise<{ ok: true; name: string; signer: string; opened?: boolean } | { ok: false; reason: string }> => {
    if (!(bytes instanceof ArrayBuffer) || bytes.byteLength > MAX_SAVE_BYTES) {
      return Promise.resolve({ ok: false, reason: bytes instanceof ArrayBuffer ? 'too-large' : 'unreadable' });
    }
    if (!offsets || typeof offsets !== 'object' || typeof when !== 'number' || typeof suggestedName !== 'string') {
      return Promise.resolve({ ok: false, reason: 'unreadable' });
    }
    return ipcRenderer.invoke('sign-file', bytes, offsets, when, suggestedName) as Promise<{ ok: true; name: string; signer: string } | { ok: false; reason: string }>;
  },
  print: (): Promise<PrintResult> => ipcRenderer.invoke('print') as Promise<PrintResult>,
  onPrintRequest: (cb: (payload: { range?: string }) => void): void => {
    ipcRenderer.on('print-request', (_event, payload: unknown) => {
      if (payload == null || typeof payload !== 'object') return;
      const range = (payload as { range?: unknown }).range;
      if (range === undefined) cb({});
      else if (typeof range === 'string') cb({ range });
    });
  },
});
