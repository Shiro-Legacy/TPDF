//! Small helpers the host shares: Node's path rules, temp folders, ids,
//! base64, the batch sort order and store writes.

use crate::fsx::OpenMode;
use crate::safe_write::{self, Inject};
use std::cmp::Ordering;
use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf};

/// Node's `path.resolve(text)`: absolute against the process's working
/// folder, then normalised lexically (`.` and `..` folded, trailing and
/// repeated separators dropped). Symbolic links are not followed.
pub fn resolve(text: &str) -> PathBuf {
    let path = Path::new(text);
    let joined = if path.has_root() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from(std::path::MAIN_SEPARATOR_STR))
            .join(path)
    };
    normalize(&joined)
}

/// Folds `.` and `..` lexically (`..` at the root stays at the root). A
/// path without a root is taken from the root. A Windows drive (`C:`) or
/// share prefix is kept.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    let mut parts = path.components().peekable();
    if let Some(Component::Prefix(prefix)) = parts.peek() {
        out.push(prefix.as_os_str());
        parts.next();
    }
    out.push(std::path::MAIN_SEPARATOR_STR);
    for part in parts {
        match part {
            Component::RootDir | Component::Prefix(_) | Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => out.push(name),
        }
    }
    out
}

/// Whether `c` separates path parts here (`/`, and `\` on Windows).
fn is_separator(c: char) -> bool {
    c == '/' || (cfg!(windows) && c == '\\')
}

/// Node's `path.basename(path)`: the last component, trailing separators ignored.
pub fn basename(path: &Path) -> String {
    let text = path.to_string_lossy();
    let trimmed = text.trim_end_matches(is_separator);
    trimmed
        .rsplit(is_separator)
        .next()
        .unwrap_or("")
        .to_string()
}

/// `os.tmpdir()`: `$TMPDIR` without its trailing separator.
pub fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir();
    let text = dir.to_string_lossy().into_owned();
    let trimmed = text.trim_end_matches(is_separator);
    if trimmed.is_empty() || trimmed.ends_with(':') {
        dir
    } else {
        PathBuf::from(trimmed)
    }
}

/// A user-data folder of its own for one memory-only session (unit tests): `tag-{pid}/{n}`
/// under the temp folder. Tests run in parallel; a shared folder let one test's journal or
/// recovery files reach another.
pub fn session_scratch(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    temp_dir()
        .join(format!("{tag}-{}", std::process::id()))
        .join(n.to_string())
}

const NAME_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// `fs.mkdtemp(prefix)`: a new folder named `prefix` plus six random characters, mode 0700.
pub fn mkdtemp(prefix: &Path) -> std::io::Result<PathBuf> {
    for _ in 0..100 {
        let suffix: String = crate::engine::entropy(6)
            .iter()
            .map(|b| NAME_CHARS[usize::from(*b) % NAME_CHARS.len()] as char)
            .collect();
        let mut name = prefix.as_os_str().to_owned();
        name.push(suffix);
        let path = PathBuf::from(name);
        match crate::fsx::create_private_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("mkdtemp: no free name"))
}

/// `crypto.randomUUID()`: a version-4 UUID.
pub fn uuid() -> String {
    let mut b = crate::engine::entropy(16);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        let digits = [n >> 18, n >> 12, n >> 6, n];
        for (i, d) in digits.iter().enumerate() {
            if i <= chunk.len() {
                out.push(B64[(d & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Node's lenient `Buffer.from(text, 'base64')`: skips characters outside the
/// alphabet, stops at the first `=`.
pub fn base64_decode_lenient(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => continue,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

/// `a.localeCompare(b, 'en', {numeric: true, sensitivity: 'base'})`, as far as
/// file names need it: digit runs compare as numbers, letters without case.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n.push(c);
                    x.next();
                }
                let mut m = String::new();
                while let Some(d) = y.peek().copied().filter(char::is_ascii_digit) {
                    m.push(d);
                    y.next();
                }
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let order = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(c), Some(d)) => {
                let order = c.to_lowercase().cmp(d.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// A JavaScript number as JSON: whole values as integers (JS has one number type).
pub fn js_number(v: f64) -> serde_json::Value {
    if v.fract() == 0.0 && v.abs() < 9.007_199_254_740_992e15 {
        serde_json::Value::from(v as i64)
    } else {
        serde_json::Value::from(v)
    }
}

/// Replaces a small store file (recent list, trust list, drawn signature)
/// through `safe_write`: temp beside it, fsync, read back, rename.
/// `private` files are created 0600 (their mode is kept once they exist).
pub fn write_store(
    path: &Path,
    bytes: &[u8],
    private: bool,
    journal: Option<&Path>,
) -> Result<(), safe_write::SaveFail> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| safe_write::classify(&e))?;
    }
    if private && fs::symlink_metadata(path).is_err() {
        // write_new keeps an existing file's mode: start the file 0600.
        let _ = OpenOptions::new()
            .write(true)
            .create_new(true)
            .with_mode(0o600)
            .open(path);
    }
    if bytes.is_empty() {
        // write_new refuses empty files; an empty store is a truncate.
        return OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .with_mode(0o600)
            .open(path)
            .map(|_| ())
            .map_err(|e| safe_write::classify(&e));
    }
    safe_write::write_new(Some(path), bytes, journal, &Inject::default()).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(normalize(Path::new("/a/./b/../c/")), PathBuf::from("/a/c"));
        assert_eq!(normalize(Path::new("/../x")), PathBuf::from("/x"));
        assert!(resolve("x/y").is_absolute());
        assert_eq!(resolve("/p/q/../r"), PathBuf::from("/p/r"));
        assert_eq!(basename(Path::new("/a/b.pdf")), "b.pdf");
        assert_eq!(basename(Path::new("/a/b/")), "b");
        assert_eq!(basename(Path::new("")), "");
        assert!(!temp_dir().to_string_lossy().ends_with('/'));
    }

    #[test]
    fn ids_and_base64() {
        let id = uuid();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        for text in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            let enc = base64_encode(text.as_bytes());
            assert_eq!(base64_decode_lenient(&enc), text.as_bytes());
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_decode_lenient("Zm9v\nYg=="), b"foob");
    }

    #[test]
    fn natural_order() {
        let mut names = vec!["b10.pdf", "B2.pdf", "a.pdf", "b1.pdf", "A.PDF"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, vec!["a.pdf", "A.PDF", "b1.pdf", "B2.pdf", "b10.pdf"]);
        assert_eq!(js_number(3.0), serde_json::json!(3));
        assert_eq!(js_number(2.5), serde_json::json!(2.5));
    }

    #[test]
    fn mkdtemp_makes_fresh_folders() {
        let a = mkdtemp(&temp_dir().join("tandem-util-test-")).unwrap();
        let b = mkdtemp(&temp_dir().join("tandem-util-test-")).unwrap();
        assert_ne!(a, b);
        assert!(a.is_dir());
        fs::remove_dir(&a).unwrap();
        fs::remove_dir(&b).unwrap();
    }
}
