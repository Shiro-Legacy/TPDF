//! The one saved signature (`host/drawn.rs`, the port of `src/sign/drawn.ts`): a drawing or an
//! image, in memory in a smoke or drive run and in a 0600 file otherwise, checked on read and
//! on write. Ported from tests/draw-signature.test.mjs and tests/sign-merge.test.mjs, which
//! drove `build/sign/drawn.js`. (The store through the host: host/tests.rs
//! `stores_are_files_outside_a_smoke`; the checks' own unit tests: host/drawn.rs tests.)
// Some helpers serve only the Unix tests (modes, links).
#![cfg_attr(not(unix), allow(unused_imports, dead_code))]

use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::mpsc;
#[cfg(unix)]
use std::time::Duration;
use tandem_app::host::drawn::{SignatureStore, check_drawn_signature, check_saved_signature};
use tandem_app::host::util::{mkdtemp, temp_dir};

/// A temp folder of the test's own, removed when the test ends.
struct Work(PathBuf);

impl Work {
    fn new(tag: &str) -> Work {
        Work(mkdtemp(&temp_dir().join(format!("tandem-sigstore-{tag}-"))).unwrap())
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn file_store(path: &Path) -> SignatureStore {
    SignatureStore::new(Some(path.to_path_buf()), None)
}

/// `value` with every number as a float, so 30 and 30.0 compare equal.
fn norm(value: &Value) -> Value {
    match value {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(items) => Value::Array(items.iter().map(norm).collect()),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), norm(v))).collect())
        }
        other => other.clone(),
    }
}

#[track_caller]
fn same(actual: Option<Value>, expected: &Value) {
    let actual = actual.unwrap_or_else(|| panic!("none, expected {expected}"));
    assert_eq!(norm(&actual), norm(expected));
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// Standard base64 with padding, written here so the test does not lean on the app's encoder.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name),
    )
    .unwrap()
}

/// Writes `value` as the store's file and reads it back through a fresh store.
fn read_back(path: &Path, value: &Value) -> Option<Value> {
    fs::write(path, value.to_string()).unwrap();
    file_store(path).get()
}

// was tests/draw-signature.test.mjs: '(b) the signature store: memory and file round trips, 0600,
// rounding, removal, and every bad value refused'
#[test]
fn drawings_round_trip_rounded_and_every_bad_value_is_refused() {
    let good = json!({"v": 1, "width": 200, "height": 40,
        "strokes": [[[0, 0], [40.123456, 30]], [[100, 10], [200, 40]]]});
    let rounded = json!({"v": 1, "width": 200, "height": 40,
        "strokes": [[[0, 0], [40.12, 30]], [[100, 10], [200, 40]]]});

    let mut memory = SignatureStore::new(None, None);
    assert_eq!(memory.get(), None);
    assert!(memory.set(Some(&good)));
    same(memory.get(), &rounded);
    assert!(memory.set(None));
    assert_eq!(memory.get(), None);

    let work = Work::new("drawn");
    let path = work.path("drawn-signature.json");
    let mut store = file_store(&path);
    assert_eq!(store.get(), None, "a missing file reads as none");
    assert!(store.set(Some(&good)));
    #[cfg(unix)]
    assert_eq!(mode(&path), 0o600);
    let on_disk: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    same(Some(on_disk["strokes"][0][1].clone()), &json!([40.12, 30]));
    same(file_store(&path).get(), &rounded);
    assert!(store.set(None));
    assert!(!path.exists(), "set(None) removes the file");
    assert!(store.set(None), "a missing file is not an error");

    fs::write(&path, "{not json").unwrap();
    assert_eq!(store.get(), None, "a corrupt file reads as none");
    let with = |key: &str, value: Value| {
        let mut v = good.clone();
        v[key] = value;
        v
    };
    let bad = [
        with("strokes", json!(vec![json!([[0, 0], [10, 10]]); 65])),
        // 4098 points in all.
        with(
            "strokes",
            json!(vec![
                (0..1366)
                    .map(|i| json!([i % 2 * 10, 5]))
                    .collect::<Vec<_>>();
                3
            ]),
        ),
        // JSON has no NaN: a non-finite coordinate reaches the file as null.
        with("strokes", json!([[[0, null], [10, 10]]])),
        with("strokes", json!([[[0, 0], [250, 10]]])),
        with("v", json!(2)),
        with("width", json!(4)),
        with("strokes", json!([[[0, 0]]])),
        json!("signature"),
    ];
    for value in &bad {
        assert_eq!(check_drawn_signature(value), None, "{value}");
        assert!(!memory.set(Some(value)), "{value}");
        assert_eq!(read_back(&path, value), None, "read as none: {value}");
    }
    assert_eq!(memory.get(), None, "a refused set kept nothing");
}

// was tests/sign-merge.test.mjs: '(b) the store: an image signature round trips in memory and on
// disk (0600, 8 MiB read cap), drawings without kind still read, bad images refused' (its FIFO
// part is `a_fifo_at_the_path_reads_as_none_without_blocking`)
#[test]
fn an_image_signature_round_trips_and_bad_images_are_refused() {
    let rgb = fixture("img-rgb.png");
    let good = json!({"v": 1, "kind": "image", "width": 300, "height": 200, "data": base64(&rgb)});

    let mut memory = SignatureStore::new(None, None);
    assert!(memory.set(Some(&good)));
    assert_eq!(memory.get(), Some(good.clone()));
    assert!(memory.set(None));
    assert_eq!(memory.get(), None);

    let work = Work::new("image");
    let path = work.path("drawn-signature.json");
    let mut store = file_store(&path);
    assert!(store.set(Some(&good)));
    #[cfg(unix)]
    assert_eq!(mode(&path), 0o600);
    let text = fs::read_to_string(&path).unwrap();
    let mut keys: Vec<String> = serde_json::from_str::<Value>(&text)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(keys, ["data", "height", "kind", "v", "width"]);
    assert!(
        !text.contains("img-rgb") && !text.contains(".png"),
        "no file name in the file"
    );
    assert_eq!(file_store(&path).get(), Some(good.clone()));

    // A 4 MiB image is about 5.6 MB of base64: over a 4 MiB read cap, under the 8 MiB one.
    let mut max = vec![7u8; 4 * 1024 * 1024];
    max[..8].copy_from_slice(&rgb[..8]);
    let big = json!({"v": 1, "kind": "image", "width": 10, "height": 10, "data": base64(&max)});
    assert!(store.set(Some(&big)));
    assert_eq!(
        file_store(&path).get().unwrap()["data"]
            .as_str()
            .unwrap()
            .len(),
        big["data"].as_str().unwrap().len(),
        "a 4 MiB image reads back"
    );

    // A drawing in draw-signature's format (no kind) still reads as a drawing.
    let drawing = json!({"v": 1, "width": 200, "height": 40, "strokes": [[[0, 0], [200, 40]]]});
    same(read_back(&path, &drawing), &drawing);
    same(check_saved_signature(&drawing), &drawing);

    // A JPEG's magic passes; other keys beside kind "image" are dropped.
    let jpeg = base64(&[0xff, 0xd8, 0xff, 0xe0, 1, 2, 3]);
    let small = json!({"v": 1, "kind": "image", "width": 1, "height": 1, "data": jpeg});
    assert_eq!(check_saved_signature(&small), Some(small.clone()));
    let mut extra = good.clone();
    extra["strokes"] = json!([[[0, 0], [1, 1]]]);
    extra["name"] = json!("me.png");
    assert_eq!(
        check_saved_signature(&extra),
        Some(good.clone()),
        "other keys are dropped"
    );

    let mut over = vec![7u8; 4 * 1024 * 1024 + 1];
    over[..8].copy_from_slice(&rgb[..8]);
    let with = |key: &str, value: Value| {
        let mut v = good.clone();
        v[key] = value;
        v
    };
    let unpadded = format!("{}\n", good["data"].as_str().unwrap().trim_end_matches('='));
    let bad = [
        (
            "wrong magic",
            with("data", json!(base64(b"GIF89a-not-a-png"))),
        ),
        ("0 bytes", with("data", json!(""))),
        ("over 4 MiB", with("data", json!(base64(&over)))),
        ("non-canonical base64", with("data", json!(unpadded))),
        ("width 0", with("width", json!(0))),
        ("width 16385", with("width", json!(16385))),
        ("fractional height", with("height", json!(1.5))),
        ("data not a string", with("data", json!(42))),
        ("kind drawn with image data", with("kind", json!("drawn"))),
        ("v 2", with("v", json!(2))),
    ];
    for (label, value) in &bad {
        assert_eq!(check_saved_signature(value), None, "{label}");
        assert!(!memory.set(Some(value)), "{label}");
        assert_eq!(read_back(&path, value), None, "read as none: {label}");
    }
    assert_eq!(memory.get(), None, "a refused set kept nothing");
}

// was tests/draw-signature.test.mjs: '(review) the store: a stale 0644 temp never loosens the file
// mode'. The Rust store writes through safe_write (a fresh, randomly named 0600 temp made with
// create_new), so the TS temp name and safe_write's own fixed-name pattern are both planted.
#[cfg(unix)]
#[test]
fn a_stale_loose_temp_never_loosens_the_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let work = Work::new("stale");
    let path = work.path("drawn-signature.json");
    for stale in [
        format!("drawn-signature.json.{}.tmp", std::process::id()),
        "drawn-signature.json.tmp".to_string(),
        ".drawn-signature.json.0123456789abcdef01234567.tmp".to_string(),
    ] {
        let stale = work.path(&stale);
        fs::write(&stale, "stale").unwrap();
        fs::set_permissions(&stale, fs::Permissions::from_mode(0o644)).unwrap();
    }
    let good = json!({"v": 1, "width": 200, "height": 40, "strokes": [[[0, 0], [200, 40]]]});
    assert!(file_store(&path).set(Some(&good)));
    assert_eq!(
        mode(&path),
        0o600,
        "the file is 0600 even beside stale 0644 temps"
    );
    same(file_store(&path).get(), &good);
}

// Beyond the JS: src/sign/drawn.ts renamed a fresh 0600 temp over the file, so a signature file
// that was 0644 became 0600 on the next save. The Rust store does the same (host/util.rs
// `write_store`, private: safe_write `write_private`).
#[cfg(unix)]
#[test]
fn an_existing_loose_signature_file_is_made_private_on_save() {
    use std::os::unix::fs::PermissionsExt;
    let work = Work::new("loose");
    let path = work.path("drawn-signature.json");
    fs::write(&path, "{}").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let good = json!({"v": 1, "width": 200, "height": 40, "strokes": [[[0, 0], [200, 40]]]});
    assert!(file_store(&path).set(Some(&good)));
    assert_eq!(mode(&path), 0o600);
}

// was tests/draw-signature.test.mjs: '(review) the store: a failed write leaves no temp behind'
#[test]
fn a_failed_write_leaves_no_temp_behind() {
    let work = Work::new("blocked");
    // A folder at the path makes the rename fail: the save is refused and no temp is left.
    let blocked = work.path("blocked");
    fs::create_dir_all(blocked.join("inside")).unwrap();
    let good = json!({"v": 1, "width": 200, "height": 40, "strokes": [[[0, 0], [200, 40]]]});
    assert!(!file_store(&blocked).set(Some(&good)));
    let temps: Vec<String> = fs::read_dir(&work.0)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(temps.is_empty(), "no temp left behind: {temps:?}");
    assert!(blocked.join("inside").is_dir(), "the folder is untouched");
    assert_eq!(file_store(&blocked).get(), None);
}

// was tests/draw-signature.test.mjs: '(review) the store: a FIFO at the path reads as none without
// blocking'; also the FIFO part of sign-merge's '(b) the store: an image signature round trips…'
#[cfg(unix)]
#[test]
fn a_fifo_at_the_path_reads_as_none_without_blocking() {
    let work = Work::new("fifo");
    let fifo = work.path("fifo.json");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if !made.is_ok_and(|s| s.success()) {
        eprintln!("skipped: mkfifo is missing");
        return;
    }
    let (send, receive) = mpsc::channel();
    let at = fifo.clone();
    std::thread::spawn(move || {
        let _ = send.send(file_store(&at).get());
    });
    match receive.recv_timeout(Duration::from_secs(5)) {
        Ok(got) => assert_eq!(got, None),
        Err(_) => {
            // Unblock the reader before failing.
            let _ = fs::OpenOptions::new().write(true).open(&fifo);
            panic!("get() on a FIFO blocked");
        }
    }
    assert_eq!(file_store(&work.path("missing.json")).get(), None);
}

#[test]
fn the_test_base64_is_standard() {
    for (text, want) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(base64(text.as_bytes()), want);
    }
}
