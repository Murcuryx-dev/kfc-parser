use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

mod io;
mod track;

const BUILD_ID: &str = env!("BUILD_ID");

#[derive(Debug, Serialize, Deserialize)]
pub struct FileStateCache {
    build_id: String,
    /// What is compared between runs: a time for game files, a content
    /// fingerprint for mod files.
    files: HashMap<PathBuf, u64>,
    /// Size and time of each mod file when its content was fingerprinted.
    #[serde(default)]
    stamps: HashMap<PathBuf, FileStamp>,
    /// Fingerprints of an earlier run, reused for files whose size and time are
    /// unchanged, so a file is only read again when it may have changed.
    #[serde(skip)]
    known: HashMap<PathBuf, (FileStamp, u64)>,
}

/// Size and modification time of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStamp {
    size: u64,
    /// Nanoseconds since the Unix epoch.
    modified: u64,
}

impl FileStamp {

    pub fn of(path: &std::path::Path) -> Option<Self> {
        let metadata = path.metadata().ok()?;
        let modified = metadata.modified().ok()?
            .duration_since(std::time::UNIX_EPOCH).ok()?;

        Some(Self {
            size: metadata.len(),
            modified: modified.as_secs()
                .wrapping_mul(1_000_000_000)
                .wrapping_add(modified.subsec_nanos() as u64),
        })
    }

}

impl Default for FileStateCache {
    fn default() -> Self {
        Self {
            build_id: BUILD_ID.to_string(),
            files: HashMap::new(),
            stamps: HashMap::new(),
            known: HashMap::new(),
        }
    }
}

impl FileStateCache {

    pub fn error() -> Self {
        Self {
            build_id: String::new(),
            files: HashMap::new(),
            stamps: HashMap::new(),
            known: HashMap::new(),
        }
    }

    /// Lets this cache take content fingerprints from `previous` for every mod
    /// file whose size and time have not changed since. Call before tracking.
    pub fn reuse_fingerprints(
        &mut self,
        previous: &Self,
    ) {
        for (path, stamp) in &previous.stamps {
            if let Some(fingerprint) = previous.files.get(path) {
                self.known.insert(path.clone(), (*stamp, *fingerprint));
            }
        }
    }

    /// Records a mod file by its content. The content is read unless an earlier
    /// run fingerprinted the file at the same size and time.
    pub(super) fn track_by_content(
        &mut self,
        path: &std::path::Path,
        fingerprint: impl FnOnce(&std::path::Path) -> u64,
    ) {
        let stamp = FileStamp::of(path);
        let value = match (stamp, self.known.get(path)) {
            (Some(stamp), Some((known_stamp, known))) if *known_stamp == stamp => *known,
            _ => fingerprint(path),
        };

        self.files.insert(path.to_path_buf(), value);

        if let Some(stamp) = stamp {
            self.stamps.insert(path.to_path_buf(), stamp);
        }
    }

    pub fn track(
        &mut self,
        path: impl Into<PathBuf>,
    ) {
        let path = path.into();
        let timestamp = path.metadata()
            .map_or(0, |m| m.modified()
                .map_or(0, |t| t.duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs())));

        self.files.insert(path, timestamp);
    }

}

pub struct CacheDiff {
    build_id_changed: bool,
    files_changed: bool,
}

#[allow(dead_code)]
impl CacheDiff {

    pub fn new_dirty() -> Self {
        Self {
            build_id_changed: true,
            files_changed: true,
        }
    }

    pub fn build_id_changed(&self) -> bool {
        self.build_id_changed
    }

    pub fn files_changed(&self) -> bool {
        self.files_changed
    }

    pub fn is_none(&self) -> bool {
        !self.is_some()
    }

    pub fn is_some(&self) -> bool {
        self.build_id_changed || self.files_changed
    }

}

impl FileStateCache {

    pub fn diff(&self, other: &Self) -> CacheDiff {
        CacheDiff {
            build_id_changed: !self.eq_build_id(other),
            files_changed: !self.eq_files(other),
        }
    }

    fn eq_build_id(&self, other: &Self) -> bool {
        self.build_id == other.build_id
    }

    fn eq_files(&self, other: &Self) -> bool {
        for (path, timestamp) in &other.files {
            match self.files.get(path) {
                Some(cached_timestamp) => {
                    if cached_timestamp != timestamp {
                        return false;
                    }
                },
                None => {
                    return false;
                }
            }
        }

        for path in self.files.keys() {
            if !other.files.contains_key(path) {
                return false;
            }
        }

        true
    }

}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn content_is_only_read_again_when_size_or_time_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.manm");
        std::fs::write(&path, b"first").unwrap();

        let reads = Cell::new(0);
        let fingerprint = |path: &std::path::Path| {
            reads.set(reads.get() + 1);
            std::fs::read(path).unwrap().iter().map(|b| *b as u64).sum()
        };

        let mut first = FileStateCache::default();
        first.track_by_content(&path, fingerprint);
        assert_eq!(reads.get(), 1);

        // unchanged file: the fingerprint is taken over, the file is not read
        let mut second = FileStateCache::default();
        second.reuse_fingerprints(&first);
        second.track_by_content(&path, fingerprint);
        assert_eq!(reads.get(), 1);
        assert!(second.diff(&first).is_none());

        // other size: read again, and the caches differ
        std::fs::write(&path, b"second!").unwrap();
        let mut third = FileStateCache::default();
        third.reuse_fingerprints(&second);
        third.track_by_content(&path, fingerprint);
        assert_eq!(reads.get(), 2);
        assert!(third.diff(&second).is_some());
    }
}
