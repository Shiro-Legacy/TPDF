//! The headless Tandem PDF application core (docs/rust-and-ux-plan.md §1.3).
//!
//! - [`engine`]: the PDF engine called natively.
//! - [`session`]: the document editor: every piece of state and logic the
//!   Electron renderer had (`src/renderer/renderer.ts`), ported to Rust.
//! - [`host`]: what Electron's main process did: files, saving, printing,
//!   recent files, the trust list, the drawn-signature store.
//! - [`drive`]: the `--smoke` / `--drive` JSON-lines protocol.
//! - [`mcp`]: the `--mcp` server for agents (stdio JSON-RPC) over the same drive.
//! - [`platform`]: native dialogs and the OS, answered by the Tauri shell in
//!   a window and by pre-answers in a headless run.

pub mod autosave;
pub mod dialog_ids;
pub mod dom;
pub mod drive;
pub mod engine;
pub mod fsx;
pub mod host;
pub mod imaging;
pub mod jpeg;
pub mod js;
pub mod mcp;
pub mod messages;
pub mod platform;
pub mod printing;
pub mod safe_write;
pub mod session;
pub mod ui;
