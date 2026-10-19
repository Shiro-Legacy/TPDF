//! Single-instance hand-over: a second launch of the window app on the same user-data folder
//! (a command line, Finder/Explorer "Open with", a file association) passes its files to the
//! running window and quits, instead of quitting with nothing done.
//!
//! The authority is still [`SingletonLock`] (one app per user-data folder; the headless
//! `--smoke`/`--drive` runs keep exiting 3 on a held lock). The app that holds the lock also
//! listens on a loopback port and writes `<port> <token>` to `<userData>/SingletonHandover`
//! (private to the user); a second launch that finds the lock held reads it, connects, sends
//! the token and its files as one JSON line, and waits for `ok`. Plain std on every OS, so the
//! same code runs on macOS, Windows and Linux. The token keeps other users' processes (who
//! cannot read the user-data folder) and web pages (which cannot read files at all) out.

use super::SingletonLock;
use super::util;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The file in the user-data folder that names the running app's port and token.
pub const FILE: &str = "SingletonHandover";
/// The most a hand-over may carry (a long list of paths is still far below this).
const MAX_MESSAGE: u64 = 256 * 1024;

/// What a launch found.
#[derive(Debug)]
pub enum Claim {
    /// This process is the app: it holds the lock; [`Listener::listen`] takes hand-overs.
    First(SingletonLock, Listener),
    /// The running app took the files (or, with none, was asked to come to the front).
    HandedOver,
    /// Another process holds the folder and did not answer (a headless run, or an app that
    /// is still starting or has hung).
    Busy,
}

/// Takes the user-data folder, or hands `files` to the app that has it. A launch that finds
/// the lock held while the app is still starting (no hand-over file yet) retries for `wait`.
pub fn claim(user_data: &Path, files: &[PathBuf], wait: Duration) -> io::Result<Claim> {
    let until = Instant::now() + wait;
    loop {
        if let Some(lock) = SingletonLock::acquire(user_data)? {
            let listener = Listener::bind(user_data)?;
            return Ok(Claim::First(lock, listener));
        }
        if send(user_data, files).is_ok() {
            return Ok(Claim::HandedOver);
        }
        if Instant::now() >= until {
            return Ok(Claim::Busy);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The running app's end: a loopback listener and its token.
#[derive(Debug)]
pub struct Listener {
    listener: TcpListener,
    token: String,
}

impl Listener {
    fn bind(user_data: &Path) -> io::Result<Listener> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
        let token = util::uuid().replace('-', "") + &util::uuid().replace('-', "");
        let port = listener.local_addr()?.port();
        util::write_store(
            &user_data.join(FILE),
            format!("{port} {token}\n").as_bytes(),
            true,
            None,
        )
        .map_err(|e| io::Error::other(format!("{e:?}")))?;
        Ok(Listener { listener, token })
    }

    /// Takes hand-overs on a thread of its own for the life of the process: `on` gets each
    /// launch's files (absolute paths; empty when it named none).
    pub fn listen(self, on: impl Fn(Vec<PathBuf>) + Send + 'static) {
        let _ = std::thread::Builder::new()
            .name("tandem-handover".into())
            .spawn(move || {
                for stream in self.listener.incoming().flatten() {
                    if let Some(files) = receive(&stream, &self.token) {
                        let _ = (&stream).write_all(b"ok\n");
                        on(files);
                    }
                }
            });
    }
}

/// One incoming hand-over: the files, or `None` for anything that is not one.
fn receive(stream: &TcpStream, token: &str) -> Option<Vec<PathBuf>> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut lines = BufReader::new(stream.take(MAX_MESSAGE)).lines();
    let given = lines.next()?.ok()?;
    if !same(given.trim().as_bytes(), token.as_bytes()) {
        return None;
    }
    let body: serde_json::Value = serde_json::from_str(&lines.next()?.ok()?).ok()?;
    let files = body.get("files")?.as_array()?;
    Some(
        files
            .iter()
            .filter_map(|f| f.as_str())
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .collect(),
    )
}

/// Compares the whole token whatever the first difference (no early exit).
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Hands `files` to the app named in the folder's hand-over file and waits for its `ok`.
fn send(user_data: &Path, files: &[PathBuf]) -> io::Result<()> {
    let text = std::fs::read_to_string(user_data.join(FILE))?;
    let (port, token) = text
        .trim()
        .split_once(' ')
        .ok_or_else(|| io::Error::other("bad hand-over file"))?;
    let port: u16 = port.parse().map_err(io::Error::other)?;
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        Duration::from_secs(1),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let absolute: Vec<String> = files
        .iter()
        .map(|f| {
            util::resolve(&f.to_string_lossy())
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let body = serde_json::json!({ "files": absolute });
    stream.write_all(format!("{token}\n{body}\n").as_bytes())?;
    let mut reply = String::new();
    stream.take(16).read_to_string(&mut reply)?;
    if reply.trim() == "ok" {
        Ok(())
    } else {
        Err(io::Error::other("the running app did not take the files"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn a_second_launch_hands_its_files_over() {
        let dir = util::session_scratch("handover");
        let Ok(Claim::First(lock, listener)) = claim(&dir, &[], Duration::ZERO) else {
            panic!("the first launch takes the folder");
        };
        let (tx, rx) = channel();
        listener.listen(move |files| {
            let _ = tx.send(files);
        });
        let file = dir.join("a.pdf");
        assert!(matches!(
            claim(&dir, std::slice::from_ref(&file), Duration::ZERO),
            Ok(Claim::HandedOver)
        ));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), vec![file]);
        // No files: the app is asked to come to the front.
        assert!(matches!(
            claim(&dir, &[], Duration::ZERO),
            Ok(Claim::HandedOver)
        ));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Vec::<PathBuf>::new()
        );
        drop(lock);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_token_is_ignored() {
        let dir = util::session_scratch("handover-token");
        let Ok(Claim::First(_lock, listener)) = claim(&dir, &[], Duration::ZERO) else {
            panic!("the first launch takes the folder");
        };
        let (tx, rx) = channel();
        listener.listen(move |files| {
            let _ = tx.send(files);
        });
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let (port, _) = text.trim().split_once(' ').unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap())).unwrap();
        stream
            .write_all(b"not-the-token\n{\"files\":[\"/x/evil.pdf\"]}\n")
            .unwrap();
        let mut reply = String::new();
        let _ = stream.read_to_string(&mut reply);
        assert_eq!(reply, "");
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_held_folder_without_a_listener_is_busy() {
        let dir = util::session_scratch("handover-busy");
        // A headless run holds the lock and takes no hand-overs.
        let _lock = SingletonLock::acquire(&dir).unwrap().unwrap();
        let started = Instant::now();
        assert!(matches!(
            claim(&dir, &[], Duration::from_millis(300)),
            Ok(Claim::Busy)
        ));
        assert!(started.elapsed() >= Duration::from_millis(300));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tokens_compare_whole() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"ab"));
    }
}
