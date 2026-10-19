// Which app binary the app-launching tests spawn (test-only).
// By default node_modules' Electron with the repo as its app ('.'). With TANDEM_APP_BIN (or
// TPDF_DRIVE_APP) set, that binary instead (the Rust Mac app: crates/target/*/tandem-pdf), which takes
// no app folder. Tests that only make sense on Electron (CDP, a patched build/, Chromium files) skip.
import { createRequire } from 'node:module';
import { resolve } from 'node:path';

const bin = process.env.TANDEM_APP_BIN || process.env.TPDF_DRIVE_APP || '';
/** True when the tests drive a binary other than Electron. */
export const ALT_APP = bin !== '';
/** The executable to spawn. */
export const appBinary = ALT_APP ? resolve(bin) : createRequire(import.meta.url)('electron');
/** The arguments that name the app: Electron takes the repo folder, a real binary nothing. */
export const appEntry = ALT_APP ? [] : ['.'];
/** A `skip` value for tests that only make sense on Electron. */
export const electronOnly = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : ALT_APP ? `Electron-only (TANDEM_APP_BIN=${bin})` : false;
