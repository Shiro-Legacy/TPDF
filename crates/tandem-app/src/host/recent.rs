//! The start screen's recent files (port of `src/recent.ts`, UX audit X4):
//! the last few PDFs opened, newest first. The host owns the list and the
//! paths; the session only ever sees a file name, its folder's name and an
//! opaque id per entry, and opens an entry by that id (an index would name
//! another file once an entry is dropped). A smoke or drive run keeps the list
//! in memory only (no file).

use super::util::{basename, uuid, write_store};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const MAX_RECENT: usize = 8;

/// One row of the start screen's list.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct RecentEntry {
    pub id: String,
    pub name: String,
    pub folder: String,
}

#[derive(Debug, Default)]
pub struct RecentStore {
    file: Option<PathBuf>,
    journal: Option<PathBuf>,
    paths: Vec<PathBuf>,
    /// Ids live for this run only: a path keeps its id while it stays listed.
    ids: HashMap<PathBuf, String>,
}

impl RecentStore {
    /// `file: None` keeps the list in memory. A missing or unreadable file starts empty.
    pub fn new(file: Option<PathBuf>, journal: Option<PathBuf>) -> Self {
        let mut paths = Vec::new();
        if let Some(file) = &file
            && let Ok(text) = std::fs::read_to_string(file)
            && let Ok(serde_json::Value::Array(items)) = serde_json::from_str(&text)
        {
            paths = items
                .iter()
                .filter_map(|v| v.as_str())
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .take(MAX_RECENT)
                .collect();
        }
        RecentStore {
            file,
            journal,
            paths,
            ids: HashMap::new(),
        }
    }

    fn id_of(&mut self, path: &Path) -> String {
        self.ids
            .entry(path.to_path_buf())
            .or_insert_with(uuid)
            .clone()
    }

    /// What the start screen shows, newest first.
    pub fn list(&mut self) -> Vec<RecentEntry> {
        let paths = self.paths.clone();
        paths
            .iter()
            .map(|path| RecentEntry {
                id: self.id_of(path),
                name: basename(path),
                folder: path.parent().map(basename).unwrap_or_default(),
            })
            .collect()
    }

    /// The path behind entry `id`, or `None` (an id from an older list).
    pub fn path(&self, id: &str) -> Option<PathBuf> {
        self.paths
            .iter()
            .find(|p| self.ids.get(*p).is_some_and(|i| i == id))
            .cloned()
    }

    /// Moves `path` to the front (adds it when new); the oldest past MAX_RECENT drops off.
    pub fn add(&mut self, path: &Path) {
        if !path.is_absolute() {
            return;
        }
        self.paths.retain(|p| p != path);
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(MAX_RECENT);
        self.persist();
    }

    /// Whether `path` is listed.
    pub fn contains(&self, path: &Path) -> bool {
        self.paths.iter().any(|p| p == path)
    }

    /// Forgets `path` (a file that could not be opened any more).
    pub fn remove(&mut self, path: &Path) {
        let before = self.paths.len();
        self.paths.retain(|p| p != path);
        self.ids.remove(path);
        if self.paths.len() != before {
            self.persist();
        }
    }

    /// Forgets every entry.
    pub fn clear(&mut self) {
        self.paths.clear();
        self.ids.clear();
        self.persist();
    }

    /// The list is a convenience: a failed write loses nothing else.
    fn persist(&self) {
        let Some(file) = &self.file else { return };
        let body: Vec<String> = self
            .paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        if let Ok(text) = serde_json::to_string(&body) {
            let _ = write_store(file, text.as_bytes(), true, self.journal.as_deref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_capped_and_persisted() {
        let dir =
            super::super::util::mkdtemp(&super::super::util::temp_dir().join("tandem-recent-"))
                .unwrap();
        let file = dir.join("recent-files.json");
        let mut store = RecentStore::new(Some(file.clone()), None);
        for i in 0..10 {
            store.add(&dir.join(format!("f{i}/doc{i}.pdf")));
        }
        store.add(Path::new("relative.pdf"));
        store.add(&dir.join("f5/doc5.pdf"));
        let list = store.list();
        assert_eq!(list.len(), MAX_RECENT);
        assert_eq!(list[0].name, "doc5.pdf");
        assert_eq!(list[0].folder, "f5");
        assert_eq!(list[1].name, "doc9.pdf");
        assert_eq!(store.path(&list[1].id), Some(dir.join("f9/doc9.pdf")));
        assert_eq!(store.path("nope"), None);
        // Ids are stable while listed.
        assert_eq!(store.list()[0].id, list[0].id);
        let reread = RecentStore::new(Some(file.clone()), None);
        assert_eq!(reread.paths, store.paths);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        store.remove(&dir.join("f9/doc9.pdf"));
        assert_eq!(store.path(&list[1].id), None);
        store.clear();
        assert!(RecentStore::new(Some(file), None).paths.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn memory_only_and_bad_files() {
        let mut store = RecentStore::new(None, None);
        store.add(Path::new("/a/b.pdf"));
        assert_eq!(store.list().len(), 1);
        let dir =
            super::super::util::mkdtemp(&super::super::util::temp_dir().join("tandem-recent-"))
                .unwrap();
        let file = dir.join("recent-files.json");
        std::fs::write(&file, b"{not json").unwrap();
        assert!(RecentStore::new(Some(file.clone()), None).paths.is_empty());
        std::fs::write(&file, br#"["/x.pdf", 3, "rel.pdf"]"#).unwrap();
        assert_eq!(
            RecentStore::new(Some(file), None).paths,
            vec![PathBuf::from("/x.pdf")]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
