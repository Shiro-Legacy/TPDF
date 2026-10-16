// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';
// Electron hook for engine/tools/op-evidence.mjs: the ops the app's PDF worker
// receives during a drive. Loaded into Electron's main process with
//   NODE_OPTIONS=--require=<this file> TPDF_OP_LOG=<dir> [TPDF_OP_LOG_TAG=<label>]
// It does nothing in plain Node or without TPDF_OP_LOG, and changes no app
// source. In the main process it registers this same file as an extra preload
// on each web contents' session (before the page loads) and appends the page's
// "TPDF_OP {...}" console lines to <dir>/electron-<pid>.jsonl. As a preload it
// runs, before any page script, a main-world wrapper of Worker.prototype.postMessage
// that reports every request carrying an `op` when sent and again when its
// reply arrives (ok, or refused with the reply's reason). The page's own
// messages and the drive's STATE lines are untouched; the wrapper adds one
// 'message' listener per worker.

/** Runs in the page's main world (serialized by contextBridge.executeInMainWorld). */
function patchWorkers() {
  if (globalThis.__tpdfOpLog) return;
  Object.defineProperty(globalThis, '__tpdfOpLog', { value: true });
  const post = Worker.prototype.postMessage;
  const log = console.debug.bind(console);
  const watched = new WeakSet();
  const sent = new Map();
  const str = value => (typeof value === 'string' ? value : undefined);
  Worker.prototype.postMessage = function (message, ...rest) {
    try {
      const op = message && typeof message === 'object' ? str(message.op) : undefined;
      if (op !== undefined) {
        if (!watched.has(this)) {
          watched.add(this);
          this.addEventListener('message', event => {
            try {
              const reply = event.data;
              if (!reply || typeof reply.id !== 'number' || !sent.has(reply.id)) return;
              const request = sent.get(reply.id);
              sent.delete(reply.id);
              const outcome = reply.ok === true ? 'ok' : reply.ok === false ? 'refused' : 'other';
              log('TPDF_OP ' + JSON.stringify({
                t: 'reply', id: reply.id, op: request.op, outcome,
                reason: outcome === 'refused' ? str(reply.reason) : undefined,
                error: outcome === 'refused' ? String(reply.error ?? '').slice(0, 200) : undefined,
              }));
            } catch {}
          });
        }
        const nested = op === 'edit-batch' && Array.isArray(message.items) ? message.items.map(item => (item && str(item.op)) || null) : undefined;
        if (typeof message.id === 'number') sent.set(message.id, { op });
        log('TPDF_OP ' + JSON.stringify({ t: 'sent', id: message.id, op, nested }));
      }
    } catch {}
    return post.call(this, message, ...rest);
  };
}

if (process.versions.electron && process.type === 'renderer') {
  // The preload: registered only by the main-process half below.
  try {
    require('electron').contextBridge.executeInMainWorld({ func: patchWorkers });
  } catch (err) {
    console.debug('TPDF_OP ' + JSON.stringify({ t: 'error', error: `preload: ${err && err.message}` }));
  }
} else if (process.env.TPDF_OP_LOG && process.versions.electron && process.type === 'browser') {
  const { appendFileSync, mkdirSync } = require('node:fs');
  const { join } = require('node:path');
  const dir = process.env.TPDF_OP_LOG;
  mkdirSync(dir, { recursive: true });
  const file = join(dir, `electron-${process.pid}.jsonl`);
  const tag = process.env.TPDF_OP_LOG_TAG || null;
  const write = entry => {
    try {
      appendFileSync(file, JSON.stringify({ tag, pid: process.pid, ...entry }) + '\n');
    } catch {
      // The log must never take the app down.
    }
  };
  // The drive script this Electron replays: scripts/drive.mjs hands it a packed copy, which
  // op-evidence.mjs matches by hash against packDrive() of each tests/drive script.
  const drive = process.argv.find(arg => arg.startsWith('--drive='))?.slice('--drive='.length) ?? null;
  let scriptSha = null;
  try {
    if (drive && drive !== '-') scriptSha = require('node:crypto').createHash('sha256').update(require('node:fs').readFileSync(drive)).digest('hex');
  } catch {
    // Unreadable: the run is reported without a script.
  }
  write({ t: 'start', drive, scriptSha, smoke: process.argv.includes('--smoke') });
  /** The paths where `actual` differs from a drive expect (main.ts matches objects key by key). */
  const mismatches = (expected, actual, path = '') => {
    if (expected && typeof expected === 'object' && !Array.isArray(expected)) {
      if (!actual || typeof actual !== 'object') return [{ path: path || '.', expected, actual: actual ?? null }];
      return Object.keys(expected).flatMap(key => mismatches(expected[key], actual[key], `${path}.${key}`));
    }
    return JSON.stringify(expected) === JSON.stringify(actual) ? [] : [{ path: path || '.', expected, actual: actual ?? null }];
  };
  const clip = value => {
    const text = JSON.stringify(value);
    return text !== undefined && text.length > 300 ? `${text.slice(0, 300)}...` : value;
  };
  /** From the --report main.ts has written by the time it calls app.exit: the step counts and each failing step. */
  const verdict = () => {
    try {
      const path = process.argv.find(arg => arg.startsWith('--report='))?.slice('--report='.length);
      if (!path) return {};
      const report = JSON.parse(require('node:fs').readFileSync(path, 'utf8'));
      const steps = Array.isArray(report.steps) ? report.steps : [];
      return {
        ok: report.ok, error: report.error, steps: steps.length, expects: steps.filter(step => step.expect !== undefined).length,
        failed: steps.flatMap((step, index) => (step.ok === false || step.error ? [{
          index, cmd: step.cmd, error: step.error,
          diff: step.expect ? mismatches(step.expect, step.state).slice(0, 8).map(item => ({ path: item.path, expected: clip(item.expected), actual: clip(item.actual) })) : undefined,
        }] : [])),
      };
    } catch (err) {
      return { reportError: String(err && err.message) };
    }
  };
  // The drive's own verdict: main.ts ends a drive with app.exit(0) iff every expect matched.
  let exited = false;
  process.on('exit', code => {
    if (!exited) write({ t: 'exit', code, via: 'process' });
  });
  const sessions = new WeakSet();

  function watch(_event, contents) {
    // Registered before the first navigation of this contents, so the preload runs before its page scripts.
    if (!sessions.has(contents.session)) {
      sessions.add(contents.session);
      try {
        contents.session.registerPreloadScript({ type: 'frame', filePath: __filename });
      } catch (err) {
        write({ t: 'error', error: `registerPreloadScript: ${err && err.message}` });
      }
    }
    contents.on('console-message', (details, _level, legacy) => {
      const message = typeof details?.message === 'string' ? details.message : legacy;
      if (typeof message !== 'string' || !message.startsWith('TPDF_OP ')) return;
      try {
        write({ t: 'op', ...JSON.parse(message.slice('TPDF_OP '.length)) });
      } catch {
        write({ t: 'error', error: 'unparseable TPDF_OP line' });
      }
    });
  }

  // A --require'd module runs before Electron has set up its own module: wait a turn for `app`.
  // The app's first window comes after the 'ready' event, well after this.
  const attach = tries => {
    const { app } = require('electron');
    if (app) {
      const exit = app.exit;
      app.exit = function (code, ...rest) {
        exited = true;
        write({ t: 'exit', code: code ?? 0, via: 'app.exit', ...verdict() });
        return Reflect.apply(exit, this, [code, ...rest]);
      };
      return void app.on('web-contents-created', watch);
    }
    if (tries < 100) setImmediate(attach, tries + 1);
    else write({ t: 'error', error: 'electron app never became available' });
  };
  setImmediate(attach, 0);
}
