// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Linux live updates via inotify (through the `notify` crate).
//!
//! Recursive watch over the volume root with these behaviors:
//! - newly created directories are watched automatically (`notify` handles it)
//! - watch-limit exhaustion (`fs.inotify.max_user_watches`, ENOSPC) surfaces as
//!   an event-channel error; we log and keep partial coverage instead of
//!   tearing down live updates entirely
//! - rename pairs (`RenameMode::Both`) map to a single `FsChange::Renamed`

use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};

use notify::event::{ModifyKind, RemoveKind, RenameMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use super::{system_time_ms, BackendError, FsChange, LiveWatch};
use crate::index::model::{FileEntry, VolumeInfo};

/// Start a recursive inotify watch for one volume root.
pub(crate) fn start_inotify_watch(
    volume: &VolumeInfo,
) -> Result<(Receiver<FsChange>, Box<dyn LiveWatch>), BackendError> {
    let root = Path::new(&volume.root);
    if !root.is_dir() {
        return Err(BackendError::Other(format!(
            "not a directory: {}",
            volume.root
        )));
    }

    let (tx_out, rx_out) = channel::<FsChange>();
    let (tx_ev, rx_ev) = channel::<notify::Result<notify::Event>>();

    let mut watcher = notify::recommended_watcher(tx_ev)
        .map_err(|e| BackendError::Other(format!("inotify init: {e}")))?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|e| BackendError::Other(format!("inotify watch {}: {e}", volume.root)))?;

    let volume_id = volume.id;
    std::thread::Builder::new()
        .name("inotify-forward".into())
        .spawn(move || forward_loop(rx_ev, tx_out, volume_id))
        .map_err(|e| BackendError::Other(format!("spawn inotify forwarder: {e}")))?;

    /// Dropping the handle drops the watcher, which closes the notify channel
    /// and lets the forwarder thread exit.
    struct InotifyHandle {
        _watcher: RecommendedWatcher,
    }
    impl LiveWatch for InotifyHandle {
        fn checkpoint(&self) -> Option<u64> {
            None
        }
    }

    Ok((rx_out, Box::new(InotifyHandle { _watcher: watcher })))
}

fn forward_loop(rx: Receiver<notify::Result<notify::Event>>, tx: Sender<FsChange>, volume_id: u32) {
    while let Ok(ev) = rx.recv() {
        match ev {
            Ok(event) => {
                for change in map_event(&event, volume_id) {
                    if tx.send(change).is_err() {
                        return; // receiver gone (watch handle dropped)
                    }
                }
            }
            Err(e) => {
                // Watch-limit exhaustion (ENOSPC) or transient add failures:
                // keep the watches already in place (partial coverage) rather
                // than tearing down live updates for the whole volume.
                eprintln!("inotify error (continuing with partial coverage): {e}");
            }
        }
    }
}

fn map_event(event: &notify::Event, volume_id: u32) -> Vec<FsChange> {
    match event.kind {
        // Reads never change the index.
        EventKind::Access(_) => Vec::new(),

        EventKind::Create(_) => event
            .paths
            .iter()
            .filter_map(|p| entry_change(p, volume_id, true))
            .collect(),

        EventKind::Modify(ModifyKind::Name(ref rename)) => match rename {
            RenameMode::Both if event.paths.len() >= 2 => vec![FsChange::Renamed {
                from: path_str(&event.paths[0]),
                to: path_str(&event.paths[1]),
            }],
            RenameMode::From => event
                .paths
                .iter()
                .map(|p| FsChange::Removed {
                    path: path_str(p),
                    is_dir: false,
                })
                .collect(),
            RenameMode::To => event
                .paths
                .iter()
                .filter_map(|p| entry_change(p, volume_id, true))
                .collect(),
            // Any/Other: fall through to the generic modify handling below.
            _ => event
                .paths
                .iter()
                .filter_map(|p| entry_change(p, volume_id, false))
                .collect(),
        },

        EventKind::Modify(_) => event
            .paths
            .iter()
            .filter_map(|p| entry_change(p, volume_id, false))
            .collect(),

        EventKind::Remove(ref kind) => {
            let is_dir = matches!(kind, RemoveKind::Folder);
            event
                .paths
                .iter()
                .map(|p| FsChange::Removed {
                    path: path_str(p),
                    is_dir,
                })
                .collect()
        }

        // Rescan/Other/Any: no reliable payload (queue overflow etc.).
        EventKind::Any | EventKind::Other => Vec::new(),
    }
}

/// Build a `Created`/`Updated` change by reading the path's metadata now.
fn entry_change(path: &Path, volume_id: u32, created: bool) -> Option<FsChange> {
    // lstat semantics (no symlink following) to match the walk-based scan.
    let md = std::fs::symlink_metadata(path).ok()?;
    let is_dir = md.is_dir();
    let attributes = if is_dir { 0x10 } else { 0x20 };
    let entry = FileEntry::from_meta(
        0,
        path_str(path),
        if is_dir { 0 } else { md.len() },
        system_time_ms(md.created()),
        system_time_ms(md.modified()),
        system_time_ms(md.accessed()),
        attributes,
        volume_id,
        is_dir,
        super::platform_file_id(path).unwrap_or(0),
    );
    Some(if created {
        FsChange::Created(entry)
    } else {
        FsChange::Updated(entry)
    })
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn test_volume(root: &Path) -> VolumeInfo {
        VolumeInfo {
            id: 4242,
            root: root.to_string_lossy().into_owned(),
            fs: "test".into(),
            total_bytes: 0,
            free_bytes: 0,
            live_updates: true,
        }
    }

    fn wait_for<F: Fn(&FsChange) -> bool>(rx: &Receiver<FsChange>, pred: F) -> Option<FsChange> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match rx.try_recv() {
                Ok(c) if pred(&c) => return Some(c),
                Ok(_) => continue,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
            }
        }
        None
    }

    #[test]
    fn watches_create_modify_remove() {
        let dir = std::env::temp_dir().join(format!(
            "tpt-inotify-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let vol = test_volume(&dir);
        let (rx, _handle) = start_inotify_watch(&vol).expect("start watch");

        // inotify watches are registered synchronously by `watch()`; give the
        // backend a beat before exercising it.
        std::thread::sleep(Duration::from_millis(200));

        let file = dir.join("live.txt");
        std::fs::write(&file, b"one").unwrap();
        let created = wait_for(
            &rx,
            |c| matches!(c, FsChange::Created(e) if e.path.contains("live.txt")),
        );
        assert!(created.is_some(), "expected Created for live.txt");

        std::fs::write(&file, b"two-two").unwrap();
        let updated = wait_for(
            &rx,
            |c| matches!(c, FsChange::Updated(e) if e.path.contains("live.txt")),
        );
        assert!(updated.is_some(), "expected Updated for live.txt");

        std::fs::remove_file(&file).unwrap();
        let removed = wait_for(
            &rx,
            |c| matches!(c, FsChange::Removed { path, .. } if path.contains("live.txt")),
        );
        assert!(removed.is_some(), "expected Removed for live.txt");

        drop(_handle);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_missing_root() {
        let vol = test_volume(Path::new("/definitely/not/here/xyz"));
        assert!(start_inotify_watch(&vol).is_err());
    }

    #[test]
    fn map_event_rename_both() {
        let ev = notify::Event {
            kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            paths: vec![PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b.txt")],
            attrs: Default::default(),
        };
        let out = map_event(&ev, 1);
        assert_eq!(
            out,
            vec![FsChange::Renamed {
                from: "/tmp/a.txt".into(),
                to: "/tmp/b.txt".into(),
            }]
        );
    }

    use std::path::PathBuf;
}
