// SPDX-License-Identifier: AGPL-3.0-or-later
//! Op routing: one module per op group.
pub mod core;

use super::{AppEngine, AppError, AppSession, Buffers, request::js_string};
use serde_json::Value;

/// The engine.ts dispatch switch.
pub(crate) fn route(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: &Value,
    _buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let op = request.get("op");
    match op.and_then(Value::as_str) {
        Some("close") => Ok(core::close(engine, session, request)),
        _ => Err(AppError::plain(format!("unknown op: {}", js_string(op)))),
    }
}
