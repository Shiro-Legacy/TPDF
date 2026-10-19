// Which app binary the app-launching tests spawn (test-only): the Rust app, TANDEM_APP_BIN if set, else
// the release build crates/target/release/tandem-pdf (`cd crates && cargo build --release -p tandem-desktop`).
// The same resolver drives scripts/drive.mjs and scripts/session.mjs. A missing binary fails the tests
// that launch it (CI_NO_ELECTRON=1 skips them all, as gates.sh and run-rust.mjs set it for the
// engine-only runs; the name is historical).
import { appBinary as resolveAppBinary } from '../scripts/session.mjs';

/** The executable to spawn. */
export const appBinary = resolveAppBinary();

/**
 * A `skip` value for a test that ran only on the retired Electron app. Only tests/worker-build.test.mjs
 * (the engine worker build, which moves with mobile) still uses it.
 */
export const electronOnly = 'Electron-only (the Electron app is retired)';
