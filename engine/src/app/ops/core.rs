// SPDX-License-Identifier: AGPL-3.0-or-later
//! Document lifecycle and session ops.
use crate::app::{AppEngine, AppSession, reply};
use serde_json::{Value, json};

/// engine.ts `close`: never fails, allocates a new generation and keeps the
/// clipboard, compare slot and batch mode.
pub fn close(engine: &mut AppEngine, session: &mut AppSession, request: &Value) -> Value {
    let AppSession {
        clipboard,
        compare,
        batch,
        ..
    } = std::mem::take(session);
    *session = AppSession {
        clipboard,
        compare,
        batch,
        generation: engine.next_generation(),
        ..AppSession::default()
    };
    reply(request, json!({}))
}
