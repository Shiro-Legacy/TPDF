//! The core thread. One thread owns the document session (and so the
//! engine); the UI, menus, the pixel protocol and window events reach it by
//! sending jobs. The window never waits on the engine: Tauri commands are
//! async and the protocol answers from its own thread.

use std::sync::mpsc::{Sender, channel};
use std::thread;
use tandem_protocol::ui::{UiAction, UiState};

/// RGBA pixels of one page.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// What the shell needs from the application core.
pub trait AppCore: Send + 'static {
    /// The whole state the window draws.
    fn ui_state(&mut self) -> UiState;
    /// One action from the UI.
    fn act(&mut self, action: UiAction);
    /// A menu item (`menu::*` ids).
    fn menu(&mut self, id: &str);
    /// Open a file (Finder, drag and drop, recent files).
    fn open_path(&mut self, path: &std::path::Path);
    /// Page pixels: `kind` is "page" or "thumb", `scale` device pixels per point.
    fn pixels(&mut self, kind: &str, page: usize, scale: f64) -> Result<Pixels, String>;
    /// The window wants to close: true when it may (after any questions).
    fn close_requested(&mut self) -> bool;
}

type Job = Box<dyn FnOnce(&mut dyn AppCore) + Send>;

/// A handle to the core thread; cheap to clone.
#[derive(Clone)]
pub struct CoreHandle {
    tx: Sender<Job>,
}

impl CoreHandle {
    pub fn spawn(make: impl FnOnce() -> Box<dyn AppCore> + Send + 'static) -> CoreHandle {
        let (tx, rx) = channel::<Job>();
        thread::Builder::new()
            .name("tandem-core".into())
            // Large pages and deep content streams want a roomy stack.
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                let mut core = make();
                for job in rx {
                    job(core.as_mut());
                }
            })
            .expect("could not start the core thread");
        CoreHandle { tx }
    }

    /// Runs `work` on the core thread and waits for its result (call from
    /// a worker thread or an async command, never from the main thread).
    pub fn call<R: Send + 'static>(&self, work: impl FnOnce(&mut dyn AppCore) -> R + Send + 'static) -> Option<R> {
        let (tx, rx) = channel();
        self.tx.send(Box::new(move |core| { let _ = tx.send(work(core)); })).ok()?;
        rx.recv().ok()
    }

    /// Runs `work` on the core thread without waiting.
    pub fn post(&self, work: impl FnOnce(&mut dyn AppCore) + Send + 'static) {
        let _ = self.tx.send(Box::new(work));
    }
}
