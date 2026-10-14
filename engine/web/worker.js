import init, { RustEngine } from './pkg/tandem_rust_workflow.js';
const ready = init();
let engine = null;
let targetPage = null;
let pendingTicket = null;
let saveSequence = 0;
const info = () => JSON.parse(engine.info());
self.onmessage = async ({ data: request }) => {
  if (!request || typeof request !== 'object') return;
  const { id, op } = request;
  try {
    await ready;
    if (['render','edit-targets','edit-text'].includes(op) &&
        (!Number.isSafeInteger(request.page) || request.page < 0 || request.page > 0xffffffff))
      throw new Error('invalid-page: page must be a nonnegative integer');
    if (op === 'edit-text' && (!Number.isSafeInteger(request.targetId) || request.targetId < 0 || request.targetId > 0xffffffff))
      throw new Error('stale-target: invalid target identifier');
    let reply = {};
    let transfer = [];
    if (op === 'open') {
      const next = new RustEngine(new Uint8Array(request.bytes));
      engine?.free(); engine = next; targetPage = null; pendingTicket = null; reply = info();
    } else if (op === 'close') {
      engine?.free(); engine = null; targetPage = null; pendingTicket = null;
    } else {
      if (!engine) throw new Error('no-document: open a PDF first');
      switch (op) {
        case 'state': reply = { ...info(), nodeAbsent: typeof process === 'undefined' && typeof require === 'undefined' }; break;
        case 'render': {
          const pixels = engine.render(request.page, request.dpi ?? 72);
          reply = { ...info(), page: request.page, pixels: pixels.buffer }; transfer = [pixels.buffer]; break;
        }
        case 'edit-targets': {
          targetPage = null;
          const items = JSON.parse(engine.targets(request.page)); targetPage = request.page;
          reply = { ...info(), page: request.page, items }; break;
        }
        case 'edit-text': {
          const state = info();
          if (request.generation !== state.generation || request.revision !== state.revision || request.page !== targetPage)
            throw new Error('stale-target: document or selection changed');
          reply = JSON.parse(engine.replace(request.targetId, request.text)); targetPage = null; break;
        }
        case 'undo': reply = JSON.parse(engine.undo()); targetPage = null; break;
        case 'redo': reply = JSON.parse(engine.redo()); targetPage = null; break;
        case 'save': {
          const bytes = engine.save(); pendingTicket = `${info().generation}:${++saveSequence}`; reply = { ...info(), ticket: pendingTicket, bytes: bytes.buffer }; transfer = [bytes.buffer]; break;
        }
        case 'validate': {
          reply = { pageCount: RustEngine.validate(new Uint8Array(request.bytes)), bytes: request.bytes };
          transfer = [reply.bytes]; break;
        }
        case 'saved':
          if (!pendingTicket || request.ticket !== pendingTicket) throw new Error('stale-save: incorrect save ticket');
          reply = JSON.parse(engine.saved()); pendingTicket = null; break;
        case 'cancel-save':
          if (!pendingTicket || request.ticket !== pendingTicket) throw new Error('stale-save: incorrect save ticket');
          engine.cancel_save(); pendingTicket = null; reply = info(); break;
        default: throw new Error(`unsupported-operation: ${op}`);
      }
    }
    self.postMessage({ id, ok: true, ...reply }, transfer);
  } catch (error) {
    self.postMessage({ id, ok: false, error: String(error) });
  }
};
ready.then(() => self.postMessage({ ready: true, backend: 'rust-wasm' })).catch(error => self.postMessage({ fatal: String(error) }));
