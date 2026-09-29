use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{Conversation, Error, Result};

/// One JSON file per conversation: `<dir>/<id>.json`.
#[derive(Clone, Debug)]
pub struct ConversationStore {
    dir: PathBuf,
}

impl ConversationStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Loads every conversation, newest first. Unreadable files are skipped with a warning.
    pub fn load_all(&self) -> Result<Vec<Conversation>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::io(&self.dir)(e)),
        };
        let mut conversations: Vec<Conversation> = entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .filter_map(|path| {
                read_json(&path)
                    .inspect_err(|e| tracing::warn!(%e, "skipping conversation file"))
                    .ok()
            })
            .collect();
        conversations.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
        Ok(conversations)
    }

    pub fn save(&self, conversation: &Conversation) -> Result<()> {
        write_json(&self.path(&conversation.id), conversation)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.path(id);
        match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(Error::io(path)(e)),
            _ => Ok(()),
        }
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).map_err(Error::io(path))?;
    serde_json::from_slice(&bytes).map_err(Error::json(path))
}

/// Writes via a temp file + rename so a crash never leaves a half-written file.
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(Error::io(parent))?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(Error::json(path))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(Error::io(&tmp))?;
    fs::rename(&tmp, path).map_err(Error::io(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;

    #[test]
    fn round_trips_and_sorts_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(dir.path().join("c"));
        let mut old = Conversation::new();
        old.updated_at = 1;
        let mut new = Conversation::new();
        new.updated_at = 2;
        new.messages.push(Message::user("hello"));
        store.save(&old).unwrap();
        store.save(&new).unwrap();

        let loaded = store.load_all().unwrap();
        assert_eq!(loaded, vec![new.clone(), old.clone()]);

        store.delete(&new.id).unwrap();
        store.delete(&new.id).unwrap();
        assert_eq!(store.load_all().unwrap(), vec![old]);
    }

    #[test]
    fn missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(dir.path().join("none"));
        assert!(store.load_all().unwrap().is_empty());
    }

    #[test]
    fn corrupt_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("bad.json"), "{").unwrap();
        let store = ConversationStore::new(dir.path());
        assert!(store.load_all().unwrap().is_empty());
    }
}
