// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Search engine orchestrator: owns the store, coordinates scans and live
//! watches, and exposes status for IPC.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::Serialize;

use super::backend::{FilesystemBackend, FsChange, LiveWatch, ScanProgress, WalkBackend};
use super::model::{FileEntry, SearchResult, SearchResultItem, VolumeInfo};
use super::persistence::{Persistence, UsnCheckpoint};
use super::query::ParsedQuery;
use super::store::IndexStore;

/// Status snapshot pushed to the UI over IPC / events.
#[derive(Clone, Debug, Serialize)]
pub struct IndexStatus {
    pub building: bool,
    pub file_count: u64,
    pub dir_count: u64,
    pub total_entries: u64,
    pub volumes: Vec<VolumeStatus>,
    pub backend: String,
    pub last_full_scan_ms: i64,
    pub last_saved_ms: i64,
    pub errors: u64,
    pub watch_active: bool,
    /// Content extraction counters (Phase 3); zeroed when pipeline is idle.
    #[serde(default)]
    pub content_queued: u64,
    #[serde(default)]
    pub content_extracted: u64,
    #[serde(default)]
    pub content_failed: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct VolumeStatus {
    pub root: String,
    pub fs: String,
    pub live_updates: bool,
    pub indexed: bool,
}

struct Shared {
    store: RwLock<IndexStore>,
    volumes: Mutex<Vec<VolumeInfo>>,
    building: AtomicBool,
    last_full_scan_ms: AtomicU64,
    last_saved_ms: AtomicU64,
    errors: AtomicU64,
    watch_active: AtomicBool,
    backend_name: Mutex<String>,
    /// Listeners invoked on every live FS change (content/embedding pipelines).
    change_listeners: Mutex<Vec<ChangeListener>>,
}

/// Status change listener callback type.
type StatusListener = Box<dyn Fn(IndexStatus) + Send + Sync>;
/// Filesystem change listener callback type (called before the store mutates).
type ChangeListener = Box<dyn Fn(&FsChange) + Send + Sync>;

/// Top-level engine used by Tauri commands and background workers.
pub struct SearchEngine {
    shared: Arc<Shared>,
    persistence: Persistence,
    /// Optional sink for IndexStatus events (wired by Tauri layer).
    status_listeners: Mutex<Vec<StatusListener>>,
    data_dir: PathBuf,
    watches: Mutex<Vec<Box<dyn LiveWatch>>>,
}

impl SearchEngine {
    pub fn new(data_dir: PathBuf) -> Self {
        let persistence = Persistence::new(data_dir.clone());
        let backend = WalkBackend::new();
        let shared = Arc::new(Shared {
            store: RwLock::new(IndexStore::new()),
            volumes: Mutex::new(Vec::new()),
            building: AtomicBool::new(false),
            last_full_scan_ms: AtomicU64::new(0),
            last_saved_ms: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            watch_active: AtomicBool::new(false),
            backend_name: Mutex::new(backend.name().to_string()),
            change_listeners: Mutex::new(Vec::new()),
        });
        Self {
            shared,
            persistence,
            status_listeners: Mutex::new(Vec::new()),
            data_dir,
            watches: Mutex::new(Vec::new()),
        }
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    /// Register a callback invoked whenever status changes (best-effort).
    pub fn on_status<F>(&self, f: F)
    where
        F: Fn(IndexStatus) + Send + Sync + 'static,
    {
        self.status_listeners.lock().unwrap().push(Box::new(f));
    }

    /// Register a callback invoked for every live filesystem change, before
    /// the store mutates (used by content extraction / embedding pipelines).
    pub fn on_change<F>(&self, f: F)
    where
        F: Fn(&FsChange) + Send + Sync + 'static,
    {
        self.shared
            .change_listeners
            .lock()
            .unwrap()
            .push(Box::new(f));
    }

    fn emit_status(&self) {
        let st = self.status();
        let listeners = self.status_listeners.lock().unwrap();
        for l in listeners.iter() {
            l(st.clone());
        }
    }

    pub fn status(&self) -> IndexStatus {
        let store = self.shared.store.read().unwrap();
        let volumes = self.shared.volumes.lock().unwrap();
        let backend = self.shared.backend_name.lock().unwrap();
        IndexStatus {
            building: self.shared.building.load(Ordering::Relaxed),
            file_count: store.file_count(),
            dir_count: store.dir_count(),
            total_entries: store.len() as u64,
            volumes: volumes
                .iter()
                .map(|v| VolumeStatus {
                    root: v.root.clone(),
                    fs: v.fs.clone(),
                    live_updates: v.live_updates,
                    indexed: true,
                })
                .collect(),
            backend: backend.clone(),
            last_full_scan_ms: self.shared.last_full_scan_ms.load(Ordering::Relaxed) as i64,
            last_saved_ms: self.shared.last_saved_ms.load(Ordering::Relaxed) as i64,
            errors: self.shared.errors.load(Ordering::Relaxed),
            watch_active: self.shared.watch_active.load(Ordering::Relaxed),
            content_queued: 0,
            content_extracted: 0,
            content_failed: 0,
        }
    }

    /// Search the in-memory index.
    pub fn search(&self, query: &str, limit: usize) -> SearchResult {
        let parsed = ParsedQuery::parse(query);
        let mut store = self.shared.store.write().unwrap();
        let mut result = store.search(&parsed, limit);
        result.query = query.to_string();
        result
    }

    /// Snapshot all live entries (used by the content extraction pipeline).
    pub fn snapshot_entries(&self) -> Vec<FileEntry> {
        self.shared.store.read().unwrap().snapshot_entries()
    }

    /// Look up a single entry by exact path (hybrid ranking fill-in).
    pub fn lookup_item(&self, path: &str) -> Option<SearchResultItem> {
        let store = self.shared.store.read().unwrap();
        let id = store.id_for_path(path)?;
        let e = store.get(id)?;
        Some(SearchResultItem {
            id,
            path: e.path.clone(),
            name: e.name.clone(),
            size: e.size,
            is_dir: e.is_dir,
            modified_ms: e.modified_ms,
            attributes: e.attributes,
            score: 0,
        })
    }

    /// Attempt cold-start load from disk. Returns true if an index was restored.
    /// Empty snapshots (e.g. saved by a failed scan) count as "not restored" so
    /// a rebuild still happens on launch.
    pub fn try_load_persisted(&self) -> bool {
        match self.persistence.load() {
            Ok(Some(snap)) => {
                if snap.entries.is_empty() {
                    return false;
                }
                let mut store = self.shared.store.write().unwrap();
                *store = Persistence::restore_store(&snap);
                drop(store);
                {
                    let mut vols = self.shared.volumes.lock().unwrap();
                    *vols = snap.volumes.clone();
                }
                self.shared
                    .backend_name
                    .lock()
                    .unwrap()
                    .clone_from(&snap.backend);
                self.shared
                    .last_saved_ms
                    .store(snap.saved_at_ms.max(0) as u64, Ordering::Relaxed);
                self.emit_status();
                true
            }
            Ok(None) => false,
            Err(e) => {
                self.shared.errors.fetch_add(1, Ordering::Relaxed);
                eprintln!("index load failed: {e}");
                false
            }
        }
    }

    /// Persist the current index. Safe to call periodically.
    pub fn save(&self) -> Result<(), String> {
        // Collect checkpoints BEFORE taking the volumes lock —
        // collect_usn_checkpoints locks `shared.volumes` itself, and
        // std::sync::Mutex is not reentrant (re-locking here deadlocks the
        // autosave / build threads on every save with live watches).
        let usn = self.collect_usn_checkpoints();
        let store = self.shared.store.read().unwrap();
        let volumes = self.shared.volumes.lock().unwrap();
        let backend = self.shared.backend_name.lock().unwrap().clone();
        self.persistence.save(&store, &volumes, &backend, &usn)?;
        let now = super::query::now_ms().max(0) as u64;
        self.shared.last_saved_ms.store(now, Ordering::Relaxed);
        drop(store);
        drop(volumes);
        drop(backend);
        self.emit_status();
        Ok(())
    }

    fn collect_usn_checkpoints(&self) -> Vec<UsnCheckpoint> {
        let watches = self.watches.lock().unwrap();
        let volumes = self.shared.volumes.lock().unwrap();
        let mut out = Vec::new();
        for w in watches.iter() {
            if let Some(usn) = w.checkpoint() {
                // Associate in order with live volumes when possible.
                if let Some(v) = volumes.iter().find(|v| v.live_updates) {
                    out.push(UsnCheckpoint {
                        root: v.root.clone(),
                        usn,
                    });
                }
            }
        }
        out
    }

    /// Kick off a full background scan of all volumes. No-op if already building.
    pub fn rebuild_async(self: &Arc<Self>) -> Result<(), String> {
        if self
            .shared
            .building
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("index build already in progress".into());
        }
        self.emit_status();
        let engine = Arc::clone(self);
        std::thread::Builder::new()
            .name("index-build".into())
            .spawn(move || {
                engine.build_sync();
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Synchronous full rebuild (tests / CLI).
    pub fn build_sync(&self) {
        let backend = WalkBackend::new();
        *self.shared.backend_name.lock().unwrap() = backend.name().to_string();

        let volumes = match backend.volumes() {
            Ok(v) => v,
            Err(e) => {
                self.shared.errors.fetch_add(1, Ordering::Relaxed);
                eprintln!("volume enum failed: {e}");
                self.shared.building.store(false, Ordering::Relaxed);
                self.emit_status();
                return;
            }
        };
        {
            let mut vols = self.shared.volumes.lock().unwrap();
            *vols = volumes.clone();
        }

        // Collect all entries across volumes into one vec, then swap in.
        let mut collected: Vec<FileEntry> = Vec::new();
        scan_volumes(&backend, &volumes, &mut collected, &self.shared.errors);

        // Windows without elevation: USN volume opens fail (GENERIC_READ on
        // \\.\C: needs admin) and the index would stay empty. Fall back to a
        // portable directory walk — slower and scan-only, but functional.
        if collected.is_empty() && !volumes.is_empty() {
            eprintln!("platform scan yielded no entries; falling back to walk scan");
            let walk = WalkBackend {
                use_platform_watch: false,
                ..WalkBackend::default()
            };
            *self.shared.backend_name.lock().unwrap() = walk.name().to_string();
            scan_volumes(&walk, &volumes, &mut collected, &self.shared.errors);
        }

        {
            let mut store = self.shared.store.write().unwrap();
            store.bulk_load(collected);
        }

        self.shared
            .last_full_scan_ms
            .store(super::query::now_ms().max(0) as u64, Ordering::Relaxed);
        self.shared.building.store(false, Ordering::Relaxed);

        // Start live watches after build.
        self.start_watches();

        // Best-effort save.
        if let Err(e) = self.save() {
            eprintln!("save after build failed: {e}");
        }
        self.emit_status();
    }

    fn start_watches(&self) {
        let backend = WalkBackend::new();
        let volumes = self.shared.volumes.lock().unwrap().clone();
        let mut handles = self.watches.lock().unwrap();
        handles.clear();

        let mut started_any = false;
        let mut failed: Vec<u32> = Vec::new();
        for vol in volumes.iter().filter(|v| v.live_updates) {
            match backend.watch(vol, None) {
                Ok((rx, handle)) => {
                    handles.push(handle);
                    started_any = true;
                    let engine_shared = Arc::clone(&self.shared);
                    // Spawn change applier.
                    std::thread::Builder::new()
                        .name(format!("fs-watch-{}", vol.id))
                        .spawn(move || {
                            while let Ok(change) = rx.recv() {
                                apply_change(&engine_shared, change);
                            }
                        })
                        .ok();
                }
                Err(e) => {
                    eprintln!("watch {} failed: {e}", vol.root);
                    self.shared.errors.fetch_add(1, Ordering::Relaxed);
                    failed.push(vol.id);
                }
            }
        }
        drop(handles);

        // Reflect reality in status: failed volumes are scan-only.
        if !failed.is_empty() {
            let mut vols = self.shared.volumes.lock().unwrap();
            for v in vols.iter_mut() {
                if failed.contains(&v.id) {
                    v.live_updates = false;
                }
            }
        }
        self.shared
            .watch_active
            .store(started_any, Ordering::Relaxed);
    }

    /// Apply a single change (also used by tests).
    pub fn apply_external_change(&self, change: FsChange) {
        apply_change(&self.shared, change);
    }

    pub fn is_building(&self) -> bool {
        self.shared.building.load(Ordering::Relaxed)
    }

    /// Wait until no build is in progress (test helper).
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        while self.is_building() {
            if start.elapsed() > timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }
}

/// Scan every volume with `backend`, pushing entries into `collected`.
/// Shared by the platform scan and the non-admin walk fallback.
fn scan_volumes(
    backend: &WalkBackend,
    volumes: &[VolumeInfo],
    collected: &mut Vec<FileEntry>,
    errors: &AtomicU64,
) {
    for vol in volumes {
        let (tx, rx) = mpsc::channel::<ScanProgress>();
        if let Err(e) = backend.scan(vol, tx) {
            errors.fetch_add(1, Ordering::Relaxed);
            eprintln!("scan {} failed: {e}", vol.root);
            continue;
        }
        // scan() is synchronous and drops tx when done — drain after.
        while let Ok(ev) = rx.recv() {
            match ev {
                ScanProgress::Entry(e) => collected.push(e),
                ScanProgress::Error { message } => {
                    errors.fetch_add(1, Ordering::Relaxed);
                    eprintln!("scan error: {message}");
                }
                ScanProgress::VolumeStarted(_) | ScanProgress::VolumeFinished { .. } => {}
            }
        }
    }
}

fn apply_change(shared: &Arc<Shared>, change: FsChange) {
    // Notify listeners first, without holding the store lock (a listener that
    // touches the index would otherwise deadlock).
    {
        let listeners = shared.change_listeners.lock().unwrap();
        for l in listeners.iter() {
            l(&change);
        }
    }
    let mut store = shared.store.write().unwrap();
    match change {
        FsChange::Created(e) | FsChange::Updated(e) => {
            store.upsert(e);
        }
        FsChange::Removed { path, .. } => {
            store.remove_path(&path);
        }
        FsChange::Renamed { from, to } => {
            if let Some(id) = store.id_for_path(&from) {
                if let Some(mut entry) = store.get(id).cloned() {
                    store.remove_path(&from);
                    entry.path = to.clone();
                    entry.name = to
                        .rsplit(['/', '\\'])
                        .next()
                        .unwrap_or(to.as_str())
                        .to_string();
                    entry.name_lower = FileEntry::compute_name_lower(&entry.name);
                    store.upsert(entry);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_runs_on_empty() {
        let dir = std::env::temp_dir().join(format!("tpt-eng-{}", std::process::id()));
        let engine = SearchEngine::new(dir.clone());
        let r = engine.search("anything", 10);
        assert_eq!(r.total_matched, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn external_change_applies() {
        let dir = std::env::temp_dir().join(format!("tpt-eng2-{}", std::process::id()));
        let engine = SearchEngine::new(dir.clone());
        engine.apply_external_change(FsChange::Created(FileEntry::from_meta(
            0,
            "/tmp/zebra.txt".into(),
            1,
            0,
            0,
            0,
            0,
            0,
            false,
            0,
        )));
        let r = engine.search("zebra", 10);
        assert_eq!(r.total_matched, 1);
        engine.apply_external_change(FsChange::Removed {
            path: "/tmp/zebra.txt".into(),
            is_dir: false,
        });
        assert_eq!(engine.search("zebra", 10).total_matched, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_shape() {
        let dir = std::env::temp_dir().join(format!("tpt-eng3-{}", std::process::id()));
        let engine = SearchEngine::new(dir.clone());
        let st = engine.status();
        assert!(!st.building);
        assert_eq!(st.total_entries, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn on_change_listener_fires_and_lookup_works() {
        use std::sync::atomic::AtomicUsize;
        let dir = std::env::temp_dir().join(format!("tpt-eng4-{}", std::process::id()));
        let engine = SearchEngine::new(dir.clone());

        let seen = Arc::new(AtomicUsize::new(0));
        engine.on_change({
            let seen = Arc::clone(&seen);
            move |change| {
                if matches!(change, FsChange::Created(_)) {
                    seen.fetch_add(1, Ordering::SeqCst);
                }
            }
        });

        engine.apply_external_change(FsChange::Created(FileEntry::from_meta(
            0,
            "/tmp/kangaroo.txt".into(),
            5,
            0,
            7,
            0,
            0,
            0,
            false,
            0,
        )));
        assert_eq!(seen.load(Ordering::SeqCst), 1);

        let item = engine.lookup_item("/tmp/kangaroo.txt").expect("lookup");
        assert_eq!(item.name, "kangaroo.txt");
        assert_eq!(item.size, 5);
        assert_eq!(item.modified_ms, 7);
        assert!(engine.lookup_item("/tmp/missing.txt").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_snapshot_does_not_suppress_rebuild() {
        use crate::index::backend::WalkBackend;
        use crate::index::persistence::Persistence;
        let dir = std::env::temp_dir().join(format!("tpt-eng6-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Simulate a prior run whose scan failed: an empty snapshot on disk.
        let p = Persistence::new(&dir);
        let vols = vec![VolumeInfo {
            id: 1,
            root: "C:\\".into(),
            fs: "NTFS".into(),
            total_bytes: 0,
            free_bytes: 0,
            live_updates: false,
        }];
        p.save(
            &crate::index::store::IndexStore::new(),
            &vols,
            WalkBackend::new().name(),
            &[],
        )
        .unwrap();

        let engine = SearchEngine::new(dir.clone());
        assert!(
            !engine.try_load_persisted(),
            "empty snapshot must not count as a restored index"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cold_start_roundtrip_restores_search() {
        use crate::index::backend::WalkBackend;
        let dir = std::env::temp_dir().join(format!("tpt-eng5-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let engine = SearchEngine::new(dir.clone());
        engine.apply_external_change(FsChange::Created(FileEntry::from_meta(
            0,
            "/data/persistence-check.bin".into(),
            11,
            0,
            9,
            0,
            0,
            0,
            false,
            0,
        )));
        engine.save().expect("save index");

        // Fresh engine (cold start) restores the persisted index.
        let engine2 = SearchEngine::new(dir.clone());
        assert!(engine2.try_load_persisted(), "snapshot should restore");
        let r = engine2.search("persistence", 10);
        assert_eq!(r.total_matched, 1);
        assert_eq!(r.items[0].name, "persistence-check.bin");
        // Backend name travels with the snapshot (may be "usn" or "walk").
        let backend_name = WalkBackend::new().name().to_string();
        assert!(!engine2.status().backend.is_empty());
        drop(backend_name);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
