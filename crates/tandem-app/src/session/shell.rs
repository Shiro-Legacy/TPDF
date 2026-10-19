//! Lane 1: renderer.ts 1–936 (platform wording, edit and sidebar state helpers) and the
//! lane's own state ([`Lane1State`]): the console-line outbox the drive harness drains, the
//! `#viewport` size the fit zooms read, and the deferred work the worker plumbing scheduled.
//!
//! Also the small shared helpers of engine-core.ts / engine-shared.ts the renderer imports
//! (`allowed`, `permissionNames`, `parsePageRange`, `createRenderCache`).
#![allow(dead_code)]

use super::Session;
use super::types::{Culprit, RenderCache};
use crate::dom::Dom;
use crate::js::{is_js_space, js_num};
use serde_json::{Map, Value, json};
use tandem_protocol::engine::{
    AnnotItem, DocumentInfo, DocumentPermissions, PermissionName, RenderReply, ThumbReply,
};
use tandem_protocol::limits::SIGNATURE_SUBJECT;

/// `document.title` (the window title) has no element id; the virtual DOM keeps it under this
/// pseudo-id (text). Written by `openDocument`, `showRestored` and `syncDirty`.
pub const DOCUMENT_TITLE: &str = "document.title";

/// The `#viewport` client size the Electron drive window gave the page, in CSS px.
///
/// Derivation (main.ts:724 `width: 1360, height: 860, useContentSize: true`; styles.css):
/// - width: the app grid is `#page-sidebar` 14.5rem (232 px, border-box) | `#workspace`
///   (1fr) | `#redact-panel` (hidden) | `#inspector` 18.75rem (300 px) → the workspace is
///   1360 − 232 − 300 = 828 px; `#reader figure` pads .75rem (12 px) each side → the
///   viewport box is 804 px; its 1 px borders leave a client width of 802 px (macOS overlay
///   scrollbars, and `.drive-geometry` elsewhere, take no gutter).
/// - height: 860 − title bar 2.25rem (36) − toolbar (two `.bar` rows of a 26 px button plus
///   .3rem padding each, the mode row's 1 px top border and the toolbar's 1 px bottom border:
///   ≈ 73) − status bar 1.7rem (≈ 27) = ≈ 724 px for the app row; the figure's 12 px padding
///   top and bottom and the caption (.4rem margin + one .72rem line ≈ 22.5 px) leave a
///   viewport box of ≈ 677 px, client height ≈ 675 px. The notices row and the form bar are
///   hidden for the drive fixtures. The UI reports the real size with
///   [`Session::set_viewport`].
pub const DRIVE_VIEWPORT: (f64, f64) = (802.0, 675.0);

/// Lane 1's state the `Session` struct lacks.
#[derive(Debug)]
pub struct Lane1State {
    /// Console lines the renderer logged (`P0_RESULT`, `P1_RESULT`, `P2_EDIT`, …), queued for
    /// the drive harness (`Driven::take_lines`).
    pub lines: Vec<(String, Value)>,
    /// `#viewport`'s `clientWidth` / `clientHeight` in CSS px (see [`DRIVE_VIEWPORT`]).
    pub viewport: (f64, f64),
    /// `window.devicePixelRatio`; the headless drive renders at 1.
    pub device_pixel_ratio: f64,
    /// R:1176-1186: an engine that answered `reason: "worker"` (a caught panic) is replaced
    /// "on the next macrotask", after the caller handled the refusal: the culprit waits here
    /// until [`Session::run_pending_tasks`] or the next request runs `workerFailed`.
    pub trap_pending: Option<Culprit>,
    /// R:1257: the restart notice is shown again after the callers' own errors, if the same
    /// restarted engine still has no document: (notice, engine epoch).
    pub reshow_notice: Option<(String, u64)>,
    /// Counts engine restarts (the TS `worker` object identity).
    pub worker_epoch: u64,
    /// `body.starting` (the start screen is up).
    pub starting: bool,
    /// The start screen's recent-files rows (`#start-recent-list`): `(id, name, folder)`.
    pub start_recent: Vec<(String, String, String)>,
}

impl Default for Lane1State {
    fn default() -> Self {
        Lane1State {
            lines: Vec::new(),
            viewport: DRIVE_VIEWPORT,
            device_pixel_ratio: 1.0,
            trap_pending: None,
            reshow_notice: None,
            worker_epoch: 0,
            starting: false,
            start_recent: Vec::new(),
        }
    }
}

impl Session {
    /// `console.log(`${prefix} ${JSON.stringify(payload)}`)`: queued for the drive harness.
    pub fn console_line(&mut self, prefix: &str, payload: Value) {
        self.lane1.lines.push((prefix.to_string(), payload));
    }

    /// The UI reports `#viewport`'s client size (the ResizeObserver of R:11303+); the fit
    /// zooms read it.
    pub fn set_viewport(&mut self, width: f64, height: f64) {
        self.lane1.viewport = (width, height);
    }

    /// The UI reports `window.devicePixelRatio`.
    pub fn set_device_pixel_ratio(&mut self, ratio: f64) {
        self.lane1.device_pixel_ratio = if ratio.is_finite() && ratio > 0.0 {
            ratio
        } else {
            1.0
        };
    }

    /// `element.setAttribute("aria-pressed", String(on))` (kept in both the `checked` flag and
    /// the attribute, as `init_dom` declares the pressed buttons).
    pub(crate) fn set_pressed(&mut self, id: &str, on: bool) {
        set_pressed(&mut self.dom, id, on);
    }

    /// R:845 `signFromNotes`: a placed signature stamp on the shown page, as `(name, subject)`.
    pub fn sign_from_notes(&self) -> Option<(String, String)> {
        let signed = self
            .notes
            .iter()
            .find(|item| item.r#type == "Stamp" && item.subject == SIGNATURE_SUBJECT)?;
        let name = signed_by(&signed.contents)?;
        Some((name, signed.subject.clone()))
    }
}

/// `aria-pressed` on a button, in both places the virtual DOM keeps it.
pub(crate) fn set_pressed(dom: &mut Dom, id: &str, on: bool) {
    dom.set_checked(id, on);
    dom.set_attr(id, "aria-pressed", if on { "true" } else { "false" });
}

/// `/^Signed by (.*) on \d{4}-\d{2}-\d{2}$/.exec(contents)?.[1]` (`.` is not a line terminator).
fn signed_by(contents: &str) -> Option<String> {
    let rest = contents.strip_prefix("Signed by ")?;
    // " on " + 10 ASCII date characters.
    if rest.len() < 14 || !rest.is_char_boundary(rest.len() - 14) {
        return None;
    }
    let (name, tail) = rest.split_at(rest.len() - 14);
    let tail = tail.as_bytes();
    let digits = |range: std::ops::Range<usize>| tail[range].iter().all(u8::is_ascii_digit);
    let date_ok = &tail[..4] == b" on "
        && digits(4..8)
        && tail[8] == b'-'
        && digits(9..11)
        && tail[11] == b'-'
        && digits(12..14);
    let line_end = |c: char| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    (date_ok && !name.contains(line_end)).then(|| name.to_string())
}

/// R:204 `platformWording`: index.html is written with the macOS keys; elsewhere the page
/// names this platform's (⌥⌘K → Ctrl+Alt+K, ⌥-click → Alt-click, "this Mac" → "this
/// computer"). Applied off macOS only (`IS_MAC`), by the UI to the strings it draws.
pub fn platform_wording(text: &str) -> String {
    let glyph = |c: char| matches!(c, '⌘' | '⌥' | '⇧');
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !glyph(chars[i]) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && glyph(chars[i]) {
            i += 1;
        }
        let keys = &chars[start..i];
        let names: Vec<&str> = [('⌘', "Ctrl"), ('⌥', "Alt"), ('⇧', "Shift")]
            .iter()
            .filter(|(g, _)| keys.contains(g))
            .map(|(_, name)| *name)
            .collect();
        out.push_str(&names.join("+"));
        // `(?=([\s-]|$)?)`: the group takes part (no "+") before a space, "-" or the end.
        let bare = match chars.get(i) {
            None => true,
            Some(&c) => is_js_space(c) || c == '-',
        };
        if !bare {
            out.push('+');
        }
    }
    out.replace('⌫', "Backspace")
        .replace("Cmd/Ctrl", "Ctrl")
        .replace("this Mac", "this computer")
}

/// R:202 `cmdKey`: ⌘ on macOS; Ctrl (or Meta) elsewhere.
pub fn cmd_key(meta: bool, ctrl: bool) -> bool {
    meta || (!super::consts::IS_MAC && ctrl)
}

/// engine-core.ts:208 `allowed(info, need)`; `need` is a permission name, "any" or
/// "edit+annotate".
pub fn allowed(info: &DocumentInfo, need: &str) -> bool {
    if !info.encrypted {
        return true;
    }
    let p = &info.permissions;
    match need {
        "any" => p.annotate || p.edit || p.form,
        "edit+annotate" => p.edit && p.annotate,
        "print" => p.print,
        "edit" => p.edit,
        "copy" => p.copy,
        "annotate" => p.annotate,
        "form" => p.form,
        _ => false,
    }
}

/// engine-core.ts:20 `permissionNames`: in the fixed `PERMISSION_BITS` order print (4),
/// edit (8), copy (16), annotate (32), form (256).
pub fn permission_names(permissions: &DocumentPermissions) -> Vec<PermissionName> {
    [
        (PermissionName::Print, permissions.print),
        (PermissionName::Edit, permissions.edit),
        (PermissionName::Copy, permissions.copy),
        (PermissionName::Annotate, permissions.annotate),
        (PermissionName::Form, permissions.form),
    ]
    .into_iter()
    .filter(|(_, on)| *on)
    .map(|(name, _)| name)
    .collect()
}

/// engine-shared.ts:189 `parsePageRange`: "1-3, 5" → `[0, 1, 2, 4]` (sorted, unique); blank
/// → every page; anything malformed or out of range → None.
pub fn parse_page_range(text: &str, page_count: i64) -> Option<Vec<i64>> {
    if page_count < 0 {
        return None;
    }
    let trimmed = crate::js::js_trim(text);
    if trimmed.is_empty() {
        return Some((0..page_count).collect());
    }
    let number = |digits: &str| -> f64 { digits.parse::<f64>().unwrap_or(f64::INFINITY) };
    let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let mut wanted = std::collections::BTreeSet::new();
    for item in trimmed.split(',') {
        let token = crate::js::js_trim(item);
        // /^(\d*)\s*-\s*(\d*)$/
        let range = token.find('-').and_then(|dash| {
            let left = token[..dash].trim_end_matches(is_js_space);
            let right = token[dash + 1..].trim_start_matches(is_js_space);
            let ok =
                (left.is_empty() || all_digits(left)) && (right.is_empty() || all_digits(right));
            ok.then_some((left, right))
        });
        let (mut first, mut last) = match range {
            Some((left, right)) if !left.is_empty() || !right.is_empty() => (
                if left.is_empty() { 1.0 } else { number(left) },
                if right.is_empty() {
                    page_count as f64
                } else {
                    number(right)
                },
            ),
            _ if all_digits(token) => (number(token), number(token)),
            _ => return None,
        };
        if first > last {
            std::mem::swap(&mut first, &mut last);
        }
        if first < 1.0 || last > page_count as f64 {
            return None;
        }
        let mut page = first as i64;
        while page as f64 <= last {
            wanted.insert(page - 1);
            page += 1;
        }
    }
    Some(wanted.into_iter().collect())
}

/// A JS number as JSON: an integral value is written as an integer (`3`, not `3.0`), `-0`
/// as `0`, NaN/Infinity as `null` (what `JSON.stringify` writes).
pub fn js_number(value: f64) -> Value {
    if !value.is_finite() {
        return Value::Null;
    }
    if value == value.trunc() && value.abs() < 9.007_199_254_740_992e15 {
        return json!(value as i64);
    }
    json!(value)
}

/// `String(value)` for a JSON value (`undefined`/`null` handled by the caller's `?? ""`).
pub fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => js_num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// `String(wanted[key] ?? "")`.
pub fn js_string_or_empty(map: &Map<String, Value>, key: &str) -> String {
    match map.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(value) => js_string(value),
    }
}

/// `text.slice(0, n)` in UTF-16 code units (a split surrogate pair is dropped whole).
pub fn js_slice16(text: &str, n: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for c in text.chars() {
        units += c.len_utf16();
        if units > n {
            break;
        }
        out.push(c);
    }
    out
}

/// `Number.isInteger(value)` for a JSON value, as the integer.
pub fn js_integer(value: Option<&Value>) -> Option<i64> {
    let n = value?.as_f64()?;
    (n.is_finite() && n == n.trunc()).then_some(n as i64)
}

// ------------------------------------------------------------------ createRenderCache

/// engine-shared.ts `CachedRender`: what the cache counts of an entry.
pub trait CachedRender {
    fn pixel_bytes(&self) -> usize;
}

impl CachedRender for RenderReply {
    fn pixel_bytes(&self) -> usize {
        self.pixels.len()
    }
}

impl CachedRender for ThumbReply {
    fn pixel_bytes(&self) -> usize {
        self.pixels.len()
    }
}

/// engine-shared.ts:114 `createRenderCache`.
impl<T: CachedRender> RenderCache<T> {
    pub fn get(&self, key: &str) -> Option<&T> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn size(&self) -> usize {
        self.entries.len()
    }

    pub fn total_bytes(&self) -> usize {
        self.bytes
    }

    /// Evicts oldest-first; the newest entry always stays, even alone over the byte budget.
    pub fn set(&mut self, key: &str, value: T) {
        if let Some(at) = self.entries.iter().position(|(k, _)| k == key)
            && let Some((_, previous)) = self.entries.remove(at)
        {
            self.bytes -= previous.pixel_bytes();
        }
        self.bytes += value.pixel_bytes();
        self.entries.push_back((key.to_string(), value));
        while self.entries.len() > 1
            && (self.entries.len() > self.limit_entries || self.bytes > self.limit_bytes)
        {
            if let Some((_, evicted)) = self.entries.pop_front() {
                self.bytes -= evicted.pixel_bytes();
            }
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
}

/// The notes' signature stamp test, for callers holding a list other than `self.notes`.
pub fn sign_from_items(items: &[AnnotItem]) -> Option<(String, String)> {
    let signed = items
        .iter()
        .find(|item| item.r#type == "Stamp" && item.subject == SIGNATURE_SUBJECT)?;
    Some((signed_by(&signed.contents)?, signed.subject.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wording_off_mac() {
        assert_eq!(platform_wording("⌥⌘K"), "Ctrl+Alt+K");
        assert_eq!(platform_wording("⌥-click"), "Alt-click");
        assert_eq!(platform_wording("Press ⇧⌘ to"), "Press Ctrl+Shift to");
        assert_eq!(platform_wording("⌘"), "Ctrl");
        assert_eq!(
            platform_wording("⌫ on this Mac, Cmd/Ctrl"),
            "Backspace on this computer, Ctrl"
        );
    }

    #[test]
    fn signature_contents() {
        assert_eq!(
            signed_by("Signed by Ann Lee on 2026-10-19").as_deref(),
            Some("Ann Lee")
        );
        assert_eq!(signed_by("Signed by  on 2026-10-19").as_deref(), Some(""));
        assert_eq!(signed_by("Signed by A\nB on 2026-10-19"), None);
        assert_eq!(signed_by("Signed by A on 2026-1-19"), None);
    }

    #[test]
    fn page_ranges() {
        assert_eq!(parse_page_range("", 3), Some(vec![0, 1, 2]));
        assert_eq!(parse_page_range("3-1, 2", 3), Some(vec![0, 1, 2]));
        assert_eq!(parse_page_range("-2", 5), Some(vec![0, 1]));
        assert_eq!(parse_page_range("4-", 5), Some(vec![3, 4]));
        assert_eq!(parse_page_range("0", 5), None);
        assert_eq!(parse_page_range("6", 5), None);
        assert_eq!(parse_page_range("a", 5), None);
        assert_eq!(parse_page_range("-", 5), None);
        assert_eq!(parse_page_range("2 - 3", 5), Some(vec![1, 2]));
    }

    #[test]
    fn permissions_and_numbers() {
        let mut info = DocumentInfo::default();
        assert!(allowed(&info, "edit"));
        info.encrypted = true;
        info.permissions.form = true;
        assert!(allowed(&info, "any") && !allowed(&info, "edit+annotate"));
        info.permissions.print = true;
        assert_eq!(
            permission_names(&info.permissions),
            vec![PermissionName::Print, PermissionName::Form]
        );
        assert_eq!(js_number(3.0), json!(3));
        assert_eq!(js_number(-0.0), json!(0));
        assert_eq!(js_number(1.5), json!(1.5));
        assert_eq!(js_string(&json!(2.0)), "2");
        assert_eq!(js_slice16("a😀b", 2), "a");
    }

    #[test]
    fn cache_evicts_oldest() {
        let mut cache: RenderCache<RenderReply> = RenderCache::new(2, 10);
        let r = |n: usize| RenderReply {
            pixels: vec![0; n],
            ..RenderReply::default()
        };
        cache.set("a", r(4));
        cache.set("b", r(4));
        cache.set("c", r(4));
        assert!(cache.get("a").is_none() && cache.size() == 2 && cache.total_bytes() == 8);
        cache.set("d", r(20));
        assert_eq!(cache.size(), 1);
        assert_eq!(cache.total_bytes(), 20);
    }
}
