//! The real core: the ported document session behind the window.

use crate::core::{AppCore, Pixels};
use crate::platform::TauriPlatform;
use std::path::Path;
use tandem_app::drive::Driven;
use tandem_app::host::{Host, HostConfig};
use tandem_app::session::Session;
use tandem_protocol::ui::{UiAction, UiState};
use tauri::AppHandle;

pub struct SessionCore {
    session: Session,
}

impl SessionCore {
    pub fn new(app: AppHandle, user_data: std::path::PathBuf) -> Self {
        let recovered = tandem_app::autosave::pending(&user_data);
        let config = HostConfig { user_data, smoke: false, echo_stdout: false };
        let host = Host::new(config, Box::new(TauriPlatform::new(app)));
        let mut session = Session::with_host(host);
        session.autosave.enabled = true;
        session.ui.recovered = recovered;
        // The start screen (no sample document), as a normal Electron launch.
        session.start(None);
        session.ui_pump();
        SessionCore { session }
    }
}

impl AppCore for SessionCore {
    fn ui_state(&mut self) -> UiState {
        self.session.ui_state()
    }

    fn act(&mut self, action: UiAction) {
        self.session.ui_act(action);
    }

    fn menu(&mut self, id: &str) {
        self.session.ui_menu(id);
    }

    fn open_path(&mut self, path: &Path) {
        self.session.ui_act(UiAction::Open { path: Some(path.to_string_lossy().into_owned()) });
    }

    fn pixels(&mut self, kind: &str, page: usize, scale: f64) -> Result<Pixels, String> {
        let (width, height, rgba) = self.session.ui_pixels(kind, page, scale)?;
        Ok(Pixels { width, height, rgba })
    }

    fn close_requested(&mut self) -> bool {
        let ok = self.session.host.close_requested();
        if ok {
            // Closing without saving was the person's choice: nothing to restore next time.
            self.session.autosave_forget();
        } else {
            self.session.ui_pump();
        }
        ok
    }
}
