//! File details that differ between Unix (macOS, Linux) and Windows, in one place.
//!
//! - Modes: Unix permission bits as they are. Windows has ACLs instead; a file's "mode" there
//!   is its read-only flag (`0o444` read-only, else `0o666`) and setting a mode sets only that
//!   flag. Private files and folders (`0o600`/`0o700` on Unix) need nothing extra on Windows:
//!   they live under the person's profile (`%APPDATA%`, `%TEMP%`), whose ACLs already keep
//!   other users out.
//! - File identity ([`file_key`]): device and inode on Unix. On Windows the file index is not
//!   available in stable std without native calls, so the key is the creation time; with the
//!   size and modification time beside it (`safe_write::FileId`) a replaced file still shows.

use std::fs::{File, Metadata, OpenOptions};
use std::io;
use std::path::Path;

/// `mode` for files this process creates (Unix); nothing on Windows.
pub trait OpenMode {
    fn with_mode(&mut self, mode: u32) -> &mut Self;
}

impl OpenMode for OpenOptions {
    #[cfg(unix)]
    fn with_mode(&mut self, mode: u32) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt;
        self.mode(mode)
    }
    #[cfg(not(unix))]
    fn with_mode(&mut self, _mode: u32) -> &mut Self {
        self
    }
}

/// The permission bits of a file (`0o777` mask applied by callers).
#[cfg(unix)]
pub fn mode_of(meta: &Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode()
}

/// The permission bits of a file: its read-only flag on Windows.
#[cfg(not(unix))]
pub fn mode_of(meta: &Metadata) -> u32 {
    if meta.permissions().readonly() {
        0o444
    } else {
        0o666
    }
}

/// Sets a path's permission bits (on Windows: read-only when no write bit is set).
pub fn set_mode(path: &Path, bits: u32) -> io::Result<()> {
    std::fs::set_permissions(path, permissions(&std::fs::metadata(path)?, bits))
}

/// [`set_mode`] on an open file.
pub fn set_file_mode(file: &File, bits: u32) -> io::Result<()> {
    file.set_permissions(permissions(&file.metadata()?, bits))
}

#[cfg(unix)]
fn permissions(_meta: &Metadata, bits: u32) -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(bits)
}

#[cfg(not(unix))]
fn permissions(meta: &Metadata, bits: u32) -> std::fs::Permissions {
    let mut p = meta.permissions();
    p.set_readonly(bits & 0o222 == 0);
    p
}

/// What tells two files apart beyond their path: `(device, inode)` on Unix,
/// `(0, creation time)` on Windows.
#[cfg(unix)]
pub fn file_key(meta: &Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(windows)]
pub fn file_key(meta: &Metadata) -> (u64, u64) {
    use std::os::windows::fs::MetadataExt;
    (0, meta.creation_time())
}

#[cfg(not(any(unix, windows)))]
pub fn file_key(_meta: &Metadata) -> (u64, u64) {
    (0, 0)
}

/// The modification time in nanoseconds since 1970 (negative before).
pub fn mtime_ns(meta: &Metadata) -> i128 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128
    }
    #[cfg(not(unix))]
    {
        match meta.modified() {
            Ok(t) => match t.duration_since(std::time::UNIX_EPOCH) {
                Ok(d) => d.as_nanos() as i128,
                Err(e) => -(e.duration().as_nanos() as i128),
            },
            Err(_) => 0,
        }
    }
}

/// Creates one folder, private to this user (`0o700` on Unix).
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::DirBuilder::new().create(path)
    }
}

/// The process umask (Unix), measured once by creating a scratch file; 0 on Windows.
pub fn umask() -> u32 {
    #[cfg(unix)]
    {
        use std::sync::OnceLock;
        static MASK: OnceLock<u32> = OnceLock::new();
        *MASK.get_or_init(|| {
            let probe = std::env::temp_dir().join(format!(
                ".tandem-umask-{}-{}",
                std::process::id(),
                crate::engine::entropy(8)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            ));
            let mask = OpenOptions::new()
                .write(true)
                .create_new(true)
                .with_mode(0o777)
                .open(&probe)
                .and_then(|f| f.metadata())
                .map(|m| !mode_of(&m) & 0o777)
                .unwrap_or(0o022);
            let _ = std::fs::remove_file(&probe);
            mask
        })
    }
    #[cfg(not(unix))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_round_trip() {
        let dir = crate::host::util::session_scratch("fsx");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, b"x").unwrap();
        set_mode(&file, 0o444).unwrap();
        assert_eq!(mode_of(&std::fs::metadata(&file).unwrap()) & 0o222, 0);
        set_mode(&file, 0o644).unwrap();
        assert_ne!(mode_of(&std::fs::metadata(&file).unwrap()) & 0o200, 0);
        let sub = dir.join("private");
        create_private_dir(&sub).unwrap();
        #[cfg(unix)]
        assert_eq!(mode_of(&std::fs::metadata(&sub).unwrap()) & 0o777, 0o700);
        let a = std::fs::metadata(&file).unwrap();
        assert_eq!(file_key(&a), file_key(&std::fs::metadata(&file).unwrap()));
        assert!(mtime_ns(&a) > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
