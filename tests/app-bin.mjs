// Which app binary the app-launching tests spawn (test-only): the Rust app, TANDEM_APP_BIN if set, else
// the release build crates/target/release/tandem-pdf (`cd crates && cargo build --release -p tandem-desktop`).
// The same resolver drives scripts/drive.mjs and scripts/session.mjs. A missing binary fails the tests
// that launch it (TANDEM_NO_APP=1 skips them all, as gates.sh and run-rust.mjs set it for the
// engine-only runs).
import { appBinary as resolveAppBinary } from '../scripts/session.mjs';

/** The executable to spawn. */
export const appBinary = resolveAppBinary();
