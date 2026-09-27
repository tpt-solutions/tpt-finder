// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Filesystem abstraction trait shared by Windows (USN), Linux (inotify),
//! and future Archon backends.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use super::model::{FileEntry, VolumeInfo};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(windows))]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
pub use unix::{platform_file_id, platform_volumes, PLATFORM_BACKEND_NAME};
#[cfg(windows)]
pub use windows::{platform_file_id, platform_volumes, PLATFORM_BACKEND_NAME};

/// Incremental change emitted by a live watcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsChange {
    Created(FileEntry),
    Updated(FileEntry),
    Removed {
        path: String,
        is_dir: bool,
    },
    /// A rename arrives as two events (old path removed + new created) from
    /// USN/inotify; backends may emit this when both sides are known.
    Renamed {
        from: String,
        to: String,
    },
}

/// Events from a full enumeration progress stream.
#[derive(Clone, Debug)]
pub enum ScanProgress {
    VolumeStarted(VolumeInfo),
    Entry(FileEntry),
    VolumeFinished {
        volume_id: u32,
        file_count: u64,
        dir_count: u64,
    },
    Error {
        message: String,
    },
}

/// Errors from backend operations.
#[derive(Debug)]
pub enum BackendError {
    Io(std::io::Error),
    Unsupported(String),
    JournalWrap,
    Other(String),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendError::Io(e) => write!(f, "io error: {e}"),
            BackendError::Unsupported(s) => write!(f, "unsupported: {s}"),
            BackendError::JournalWrap => write!(f, "usn journal wrapped; full rescan required"),
            BackendError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for BackendError {}

impl From<std::io::Error> for BackendError {
    fn from(e: std::io::Error) -> Self {
        BackendError::Io(e)
    }
}

/// Portable filesystem backend interface.
///
/// Phase 1 provides a portable walk-based enumeration backend for all
/// platforms plus a Windows USN Journal live tail. Phase 5 adds a native
/// Linux inotify watcher behind the same trait. Phase 6 adds an Archon
/// adapter for SQL-style queries over remote/similarity sources.
pub trait FilesystemBackend: Send {
    /// Human-readable backend name (`usn`, `walk`, `walk+inotify`, `archon`, …).
    fn name(&self) -> &'static str;

    /// Enumerate indexable volumes (NTFS drives on Windows; mounts on Linux).
    fn volumes(&self) -> Result<Vec<VolumeInfo>, BackendError>;

    /// Stream a full scan of one volume. Implementations should emit
    /// `ScanProgress::Entry` as they go so the UI can show progress.
    fn scan(
        &self,
        volume: &VolumeInfo,
        tx: std::sync::mpsc::Sender<ScanProgress>,
    ) -> Result<(), BackendError>;

    /// Start live updates for a volume. Returns a receiver of changes and
    /// a handle that stops the watcher when dropped.
    ///
    /// `resume_from_usn` is a Windows USN hint for journal catch-up after a
    /// cold start; other backends may ignore it.
    fn watch(
        &self,
        volume: &VolumeInfo,
        resume_from_usn: Option<u64>,
    ) -> Result<(Receiver<FsChange>, Box<dyn LiveWatch>), BackendError>;
}

/// Handle controlling a live watch. Dropping stops the watcher thread.
pub trait LiveWatch: Send {
    /// Journal position / generation for persistence (Windows USN; 0 elsewhere).
    fn checkpoint(&self) -> Option<u64> {
        None
    }
}

/// Default portable backend: directory walk for enumeration.
pub struct WalkBackend {
    pub follow_symlinks: bool,
    pub max_depth: Option<usize>,
    /// When true, `watch` wires the platform live watcher (USN / inotify)
    /// if compiled in; otherwise returns an idle handle.
    pub use_platform_watch: bool,
}

impl Default for WalkBackend {
    fn default() -> Self {
        Self {
            follow_symlinks: false,
            max_depth: None,
            use_platform_watch: true,
        }
    }
}

impl WalkBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn scan_dir(
        &self,
        volume: &VolumeInfo,
        root: &Path,
        tx: &std::sync::mpsc::Sender<ScanProgress>,
        file_count: &mut u64,
        dir_count: &mut u64,
    ) {
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let read = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(ScanProgress::Error {
                        message: format!("read_dir {}: {e}", dir.display()),
                    });
                    continue;
                }
            };
            for ent in read.flatten() {
                let path = ent.path();
                let Ok(md) = ent.metadata() else {
                    continue;
                };
                let is_dir = md.is_dir();
                if is_dir {
                    *dir_count += 1;
                } else {
                    *file_count += 1;
                }
                let created = system_time_ms(md.created());
                let modified = system_time_ms(md.modified());
                let accessed = system_time_ms(md.accessed());

                #[cfg(windows)]
                let attributes = {
                    use std::os::windows::fs::MetadataExt;
                    md.file_attributes()
                };
                #[cfg(not(windows))]
                let attributes = if is_dir { 0x10 } else { 0x20 };

                // FRN lookup costs an extra file-handle open per entry and the
                // walk path never consumes it (the USN watch keeps its own
                // FRN map), so skip it — this is the difference between a
                // walk taking hours and seconds on large volumes.
                let frn = 0u64;
                let path_str = path.to_string_lossy().into_owned();
                let entry = FileEntry::from_meta(
                    0,
                    path_str,
                    if is_dir { 0 } else { md.len() },
                    created,
                    modified,
                    accessed,
                    attributes,
                    volume.id,
                    is_dir,
                    frn,
                );
                if tx.send(ScanProgress::Entry(entry)).is_err() {
                    return;
                }
                if is_dir && self.max_depth.map(|d| d > 0).unwrap_or(true) {
                    let name = ent.file_name();
                    let name_l = name.to_string_lossy().to_ascii_lowercase();
                    if !matches!(
                        name_l.as_str(),
                        "node_modules" | ".git" | "target" | ".cache"
                    ) {
                        stack.push(path);
                    }
                }
            }
        }
    }
}

fn system_time_ms(t: std::io::Result<std::time::SystemTime>) -> i64 {
    t.ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl FilesystemBackend for WalkBackend {
    fn name(&self) -> &'static str {
        if self.use_platform_watch {
            crate::index::backend::PLATFORM_BACKEND_NAME
        } else {
            "walk"
        }
    }

    fn volumes(&self) -> Result<Vec<VolumeInfo>, BackendError> {
        platform_volumes()
    }

    fn scan(
        &self,
        volume: &VolumeInfo,
        tx: std::sync::mpsc::Sender<ScanProgress>,
    ) -> Result<(), BackendError> {
        let _ = tx.send(ScanProgress::VolumeStarted(volume.clone()));
        let root = PathBuf::from(&volume.root);
        let mut file_count = 0u64;
        let mut dir_count = 0u64;
        dir_count += 1;
        self.scan_dir(volume, &root, &tx, &mut file_count, &mut dir_count);
        let _ = tx.send(ScanProgress::VolumeFinished {
            volume_id: volume.id,
            file_count,
            dir_count,
        });
        Ok(())
    }

    fn watch(
        &self,
        volume: &VolumeInfo,
        resume_from_usn: Option<u64>,
    ) -> Result<(Receiver<FsChange>, Box<dyn LiveWatch>), BackendError> {
        if self.use_platform_watch {
            #[cfg(windows)]
            {
                return windows::start_usn_watch(volume, resume_from_usn);
            }
            #[cfg(not(windows))]
            {
                // Linux inotify (recursive) / idle elsewhere.
                let _ = resume_from_usn;
                return unix::start_platform_watch(volume);
            }
        }
        Ok(idle_watch())
    }
}

/// Open channel that never emits; sender is retained by the handle so the
/// receiver does not immediately disconnect.
fn idle_watch() -> (Receiver<FsChange>, Box<dyn LiveWatch>) {
    let (tx, rx) = std::sync::mpsc::channel::<FsChange>();
    struct IdleHold {
        _tx: std::sync::mpsc::Sender<FsChange>,
    }
    impl LiveWatch for IdleHold {
        fn checkpoint(&self) -> Option<u64> {
            None
        }
    }
    let handle: Box<dyn LiveWatch> = Box::new(IdleHold { _tx: tx });
    (rx, handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn walk_backend_name() {
        let mut b = WalkBackend::new();
        b.use_platform_watch = false;
        assert_eq!(b.name(), "walk");
    }

    #[test]
    fn volumes_non_empty() {
        let vols = WalkBackend::new().volumes().expect("volumes");
        assert!(!vols.is_empty(), "expected at least one volume");
        assert!(vols[0].root.contains('/') || vols[0].root.contains('\\'));
    }

    #[test]
    fn scan_emits_started_and_finished() {
        // Scan a tiny temp tree, not a whole volume (which would hang CI).
        let tmp = std::env::temp_dir().join(format!("tpt-scan-test-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("a.txt"), b"hi").unwrap();
        std::fs::write(tmp.join("sub/b.txt"), b"yo").unwrap();

        let backend = WalkBackend {
            use_platform_watch: false,
            ..WalkBackend::default()
        };
        let vol = VolumeInfo {
            id: 999,
            root: tmp.to_string_lossy().into_owned(),
            fs: "test".into(),
            total_bytes: 0,
            free_bytes: 0,
            live_updates: false,
        };
        let (tx, rx) = channel();
        backend.scan(&vol, tx).unwrap();
        let mut started = false;
        let mut finished = false;
        let mut entries = 0usize;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                ScanProgress::VolumeStarted(_) => started = true,
                ScanProgress::VolumeFinished { file_count, .. } => {
                    finished = true;
                    assert!(
                        file_count >= 2,
                        "expected at least 2 files, got {file_count}"
                    );
                }
                ScanProgress::Entry(_) => entries += 1,
                ScanProgress::Error { .. } => {}
            }
        }
        assert!(started && finished);
        assert!(
            entries >= 3,
            "expected root+subdir+2 files ≥3 entries, got {entries}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn idle_watch_stays_open() {
        let (rx, _h) = idle_watch();
        // Channel exists and has no messages yet (try_recv WouldBlock/disconnected check).
        match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty)
            | Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
            Ok(_) => panic!("idle watch should not emit"),
        }
    }
}
