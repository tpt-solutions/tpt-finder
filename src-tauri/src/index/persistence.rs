// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Index persistence for fast cold-start (avoid full rescan every launch).
//!
//! Format: JSON Lines header + bincode body would be ideal, but we keep a
//! simple single-file bincode blob with a magic/version header so we can
//! evolve the layout without silent corruption.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::model::{FileEntry, VolumeInfo};
use super::store::IndexStore;

const MAGIC: &[u8; 8] = b"TPTIDX01";
const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct IndexSnapshot {
    pub version: u32,
    pub saved_at_ms: i64,
    pub backend: String,
    pub volumes: Vec<VolumeInfo>,
    pub entries: Vec<FileEntry>,
    /// Windows USN resume points per volume root.
    pub usn_checkpoints: Vec<UsnCheckpoint>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct UsnCheckpoint {
    pub root: String,
    pub usn: u64,
}

pub struct Persistence {
    path: PathBuf,
}

impl Persistence {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let mut path = dir.into();
        path.push("index.bin");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn save(
        &self,
        store: &IndexStore,
        volumes: &[VolumeInfo],
        backend: &str,
        usn: &[UsnCheckpoint],
    ) -> Result<(), String> {
        let snapshot = IndexSnapshot {
            version: FORMAT_VERSION,
            saved_at_ms: super::query::now_ms(),
            backend: backend.to_string(),
            volumes: volumes.to_vec(),
            entries: store.snapshot_entries(),
            usn_checkpoints: usn.to_vec(),
        };
        let tmp = self.path.with_extension("bin.tmp");
        if let Some(parent) = tmp.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let data = bincode::serialize(&snapshot).map_err(|e| e.to_string())?;
        {
            let mut f = BufWriter::new(File::create(&tmp).map_err(|e| e.to_string())?);
            f.write_all(MAGIC).map_err(|e| e.to_string())?;
            f.write_all(&FORMAT_VERSION.to_le_bytes())
                .map_err(|e| e.to_string())?;
            f.write_all(&data).map_err(|e| e.to_string())?;
            f.flush().map_err(|e| e.to_string())?;
        }
        fs::rename(&tmp, &self.path).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn load(&self) -> Result<Option<IndexSnapshot>, String> {
        if !self.path.exists() {
            return Ok(None);
        }
        let mut f = BufReader::new(File::open(&self.path).map_err(|e| e.to_string())?);
        let mut magic = [0u8; 8];
        f.read_exact(&mut magic).map_err(|e| e.to_string())?;
        if &magic != MAGIC {
            return Err("index file magic mismatch".into());
        }
        let mut ver = [0u8; 4];
        f.read_exact(&mut ver).map_err(|e| e.to_string())?;
        let version = u32::from_le_bytes(ver);
        if version != FORMAT_VERSION {
            return Err(format!("unsupported index version {version}"));
        }
        let mut rest = Vec::new();
        f.read_to_end(&mut rest).map_err(|e| e.to_string())?;
        let snapshot: IndexSnapshot = bincode::deserialize(&rest).map_err(|e| e.to_string())?;
        Ok(Some(snapshot))
    }

    pub fn restore_store(snapshot: &IndexSnapshot) -> IndexStore {
        IndexStore::from_snapshot(snapshot.entries.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::model::FileEntry;

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join(format!("tpt-finder-test-{}", std::process::id()));
        let p = Persistence::new(&dir);
        let mut store = IndexStore::new();
        store.upsert(FileEntry::from_meta(
            0,
            "/tmp/hello.txt".into(),
            42,
            1,
            2,
            3,
            0,
            0,
            false,
            9,
        ));
        let vols = vec![VolumeInfo {
            id: 1,
            root: "/".into(),
            fs: "test".into(),
            total_bytes: 0,
            free_bytes: 0,
            live_updates: false,
        }];
        p.save(&store, &vols, "walk", &[]).unwrap();

        let snap = p.load().unwrap().expect("snapshot");
        assert_eq!(snap.entries.len(), 1);
        assert_eq!(snap.entries[0].path, "/tmp/hello.txt");
        assert_eq!(snap.backend, "walk");
        let restored = Persistence::restore_store(&snap);
        assert_eq!(restored.len(), 1);
        assert_eq!(restored.file_count(), 1);

        let _ = fs::remove_dir_all(&dir);
    }
}
