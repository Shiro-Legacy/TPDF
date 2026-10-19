//! The one saved signature (port of `src/sign/drawn.ts`): a drawing (pad CSS
//! px, its bounds' top-left at the origin) or an image (PNG/JPEG bytes as
//! canonical base64 with the decoded pixel size; never a file name). It lives
//! in the user-data folder on this machine, or in memory in a smoke or drive
//! run. Validated on read and on write; never logged.

use super::util::{base64_decode_lenient, base64_encode, js_number, write_store};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

/// The read cap: base64 of a 4 MiB image is about 5.6 MB (sign-merge D2).
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// = engine MAX_STAMP_IMAGE_BYTES; change both together.
const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const JPEG_MAGIC: [u8; 3] = [0xff, 0xd8, 0xff];

fn round2(v: f64) -> f64 {
    (v * 100.0 + 0.5).floor() / 100.0
}

fn finite(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|n| n.is_finite())
}

/// draw-signature D3: a valid drawing (coordinates rounded to 2 decimals), else `None`.
pub fn check_drawn_signature(value: &Value) -> Option<Value> {
    let rec = value.as_object()?;
    if rec.get("v").and_then(Value::as_f64) != Some(1.0) {
        return None;
    }
    let width = finite(rec.get("width"))?;
    let height = finite(rec.get("height"))?;
    if !(8.0..=4096.0).contains(&width) || !(0.0..=4096.0).contains(&height) {
        return None;
    }
    let strokes = rec.get("strokes")?.as_array()?;
    if strokes.is_empty() || strokes.len() > 64 {
        return None;
    }
    // 4096 points in all: an Ink appearance is quadratic in the point count (the pad stops there too).
    let mut total = 0usize;
    let mut out = Vec::with_capacity(strokes.len());
    for stroke in strokes {
        let stroke = stroke.as_array()?;
        total += stroke.len();
        if stroke.len() < 2 || stroke.len() > 2048 || total > 4096 {
            return None;
        }
        let mut points = Vec::with_capacity(stroke.len());
        for point in stroke {
            let pair = point.as_array().filter(|p| p.len() == 2)?;
            let (x, y) = (finite(pair.first())?, finite(pair.get(1))?);
            if x < -0.01 || y < -0.01 || x > width + 0.01 || y > height + 0.01 {
                return None;
            }
            points.push(json!([js_number(round2(x)), js_number(round2(y))]));
        }
        out.push(Value::Array(points));
    }
    Some(
        json!({"v": 1, "width": js_number(round2(width)), "height": js_number(round2(height)), "strokes": out}),
    )
}

fn pixels(v: Option<&Value>) -> Option<i64> {
    let n = v?.as_f64()?;
    (n.fract() == 0.0 && (1.0..=16384.0).contains(&n)).then_some(n as i64)
}

/// sign-merge D2: a drawing (no `kind`) or an image (`kind: "image"`, only its
/// five keys kept), else `None`.
pub fn check_saved_signature(value: &Value) -> Option<Value> {
    let rec = value.as_object()?;
    match rec.get("kind") {
        None => check_drawn_signature(value),
        Some(Value::String(k)) if k == "image" => {
            if rec.get("v").and_then(Value::as_f64) != Some(1.0) {
                return None;
            }
            let width = pixels(rec.get("width"))?;
            let height = pixels(rec.get("height"))?;
            let data = rec.get("data")?.as_str()?;
            if data.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
                return None;
            }
            let bytes = base64_decode_lenient(data);
            if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES || base64_encode(&bytes) != data {
                return None;
            }
            if !bytes.starts_with(&PNG_MAGIC) && !bytes.starts_with(&JPEG_MAGIC) {
                return None;
            }
            let mut out = Map::new();
            out.insert("v".into(), json!(1));
            out.insert("kind".into(), json!("image"));
            out.insert("width".into(), json!(width));
            out.insert("height".into(), json!(height));
            out.insert("data".into(), json!(data));
            Some(Value::Object(out))
        }
        Some(_) => None,
    }
}

#[derive(Debug, Default)]
pub struct SignatureStore {
    path: Option<PathBuf>,
    journal: Option<PathBuf>,
    memory: Option<Value>,
}

impl SignatureStore {
    /// `path: None` keeps it in memory (smoke/drive/tests).
    pub fn new(path: Option<PathBuf>, journal: Option<PathBuf>) -> Self {
        SignatureStore {
            path,
            journal,
            memory: None,
        }
    }

    /// The saved signature, or `None` (none saved, or a file that is not a valid one).
    pub fn get(&self) -> Option<Value> {
        let Some(path) = &self.path else {
            return self.memory.clone();
        };
        // A regular file only: a FIFO or device at the path would block or flood the process.
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            return None;
        }
        let text = std::fs::read_to_string(path).ok()?;
        check_saved_signature(&serde_json::from_str(&text).ok()?)
    }

    /// Replaces (or with `None` removes) the saved signature: `false` when the
    /// value is not a valid signature (nothing changes) or the file cannot be written.
    pub fn set(&mut self, value: Option<&Value>) -> bool {
        let checked = match value {
            None => None,
            Some(v) => match check_saved_signature(v) {
                Some(c) => Some(c),
                None => return false,
            },
        };
        let Some(path) = &self.path else {
            self.memory = checked;
            return true;
        };
        let Some(checked) = checked else {
            return match std::fs::remove_file(path) {
                Ok(()) => true,
                Err(e) => e.kind() == std::io::ErrorKind::NotFound,
            };
        };
        serde_json::to_string(&checked).is_ok_and(|text| {
            write_store(path, text.as_bytes(), true, self.journal.as_deref()).is_ok()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawings_are_checked_and_rounded() {
        let ok = json!({"v": 1, "width": 100.004, "height": 40, "strokes": [[[0, 0], [10.126, 20.5], [100.01, 40]]]});
        let got = check_drawn_signature(&ok).unwrap();
        assert_eq!(
            got,
            json!({"v": 1, "width": 100, "height": 40, "strokes": [[[0, 0], [10.13, 20.5], [100.01, 40]]]})
        );
        for bad in [
            json!({"v": 2, "width": 100, "height": 40, "strokes": [[[0, 0], [1, 1]]]}),
            json!({"v": 1, "width": 7, "height": 40, "strokes": [[[0, 0], [1, 1]]]}),
            json!({"v": 1, "width": 100, "height": 40, "strokes": []}),
            json!({"v": 1, "width": 100, "height": 40, "strokes": [[[0, 0]]]}),
            json!({"v": 1, "width": 100, "height": 40, "strokes": [[[0, 0], [101, 1]]]}),
            json!({"v": 1, "width": 100, "height": 40, "strokes": [[[0, 0], [1, "1"]]]}),
            json!(null),
        ] {
            assert_eq!(check_saved_signature(&bad), None, "{bad}");
        }
        let many = vec![json!([1, 1]); 2049];
        assert_eq!(
            check_drawn_signature(&json!({"v": 1, "width": 10, "height": 10, "strokes": [many]})),
            None
        );
    }

    #[test]
    fn images_must_be_canonical_png_or_jpeg() {
        let png = base64_encode(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0]);
        let img = json!({"v": 1, "kind": "image", "width": 20, "height": 10.0, "data": png, "name": "secret.png"});
        let got = check_saved_signature(&img).unwrap();
        assert_eq!(
            got,
            json!({"v": 1, "kind": "image", "width": 20, "height": 10, "data": png})
        );
        let gif = base64_encode(b"GIF89a....");
        assert_eq!(
            check_saved_signature(
                &json!({"v": 1, "kind": "image", "width": 1, "height": 1, "data": gif})
            ),
            None
        );
        let loose = format!("{}\n", png);
        assert_eq!(
            check_saved_signature(
                &json!({"v": 1, "kind": "image", "width": 1, "height": 1, "data": loose})
            ),
            None
        );
        assert_eq!(
            check_saved_signature(
                &json!({"v": 1, "kind": "image", "width": 1.5, "height": 1, "data": png})
            ),
            None
        );
        assert_eq!(
            check_saved_signature(&json!({"v": 1, "kind": "stamp"})),
            None
        );
    }

    #[test]
    fn store_round_trip() {
        let dir =
            super::super::util::mkdtemp(&super::super::util::temp_dir().join("tandem-drawn-"))
                .unwrap();
        let file = dir.join("drawn-signature.json");
        let mut store = SignatureStore::new(Some(file.clone()), None);
        assert_eq!(store.get(), None);
        let sig = json!({"v": 1, "width": 50, "height": 20, "strokes": [[[1, 1], [2, 2]]]});
        assert!(store.set(Some(&sig)));
        assert_eq!(store.get(), Some(sig.clone()));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!store.set(Some(&json!({"v": 1}))));
        assert_eq!(store.get(), Some(sig.clone()));
        std::fs::write(&file, b"{broken").unwrap();
        assert_eq!(store.get(), None);
        assert!(store.set(None));
        assert!(!file.exists());
        assert!(store.set(None));
        let mut mem = SignatureStore::new(None, None);
        assert!(mem.set(Some(&sig)));
        assert_eq!(mem.get(), Some(sig));
        assert!(mem.set(None));
        assert_eq!(mem.get(), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
