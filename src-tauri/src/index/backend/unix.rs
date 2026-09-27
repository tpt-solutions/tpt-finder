// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Linux/macOS platform helpers: volume enumeration via `/proc/mounts` or
//! `/` fallback, file IDs via `st_ino`, and (on Linux) a recursive inotify
//! live watch.

use std::path::Path;

use crate::index::model::VolumeInfo;

use super::BackendError;

#[cfg(target_os = "linux")]
pub const PLATFORM_BACKEND_NAME: &str = "walk+inotify";
#[cfg(not(target_os = "linux"))]
pub const PLATFORM_BACKEND_NAME: &str = "walk";

pub fn platform_volumes() -> Result<Vec<VolumeInfo>, BackendError> {
    let mut out = Vec::new();

    #[cfg(target_os = "linux")]
    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
        for line in mounts.lines() {
            let mut it = line.split_whitespace();
            let _src = it.next();
            let Some(target) = it.next() else { continue };
            let Some(fstype) = it.next() else { continue };
            // Index common local filesystems; skip network/proc/sys/virtual.
            let indexable = matches!(
                fstype,
                "ext4" | "ext3" | "ext2" | "xfs" | "btrfs" | "f2fs" | "ntfs" | "fuseblk"
            );
            if !indexable {
                continue;
            }
            // Decode octal escapes in mount paths (e.g. \040 for space).
            let root = unescape_mount(target);
            if out.iter().any(|v: &VolumeInfo| v.root == root) {
                continue;
            }
            let id = stable_volume_id(&root);
            out.push(VolumeInfo {
                id,
                root,
                fs: fstype.to_string(),
                total_bytes: 0,
                free_bytes: 0,
                // Linux: inotify live watch (macOS stays scan-only for now).
                live_updates: cfg!(target_os = "linux"),
            });
        }
    }

    if out.is_empty() {
        out.push(VolumeInfo {
            id: 1,
            root: "/".into(),
            fs: "unknown".into(),
            total_bytes: 0,
            free_bytes: 0,
            live_updates: cfg!(target_os = "linux"),
        });
    }
    Ok(out)
}

/// Start the platform live watcher (Linux inotify; idle elsewhere).
pub(crate) fn start_platform_watch(
    volume: &VolumeInfo,
) -> Result<
    (
        std::sync::mpsc::Receiver<super::FsChange>,
        Box<dyn super::LiveWatch>,
    ),
    BackendError,
> {
    #[cfg(target_os = "linux")]
    {
        super::linux::start_inotify_watch(volume)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = volume;
        Ok(super::idle_watch())
    }
}

#[cfg(target_os = "linux")]
fn unescape_mount(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let hex: String = chars.by_ref().take(3).collect();
            if let Ok(v) = u32::from_str_radix(&hex, 8) {
                if let Some(ch) = char::from_u32(v) {
                    out.push(ch);
                    continue;
                }
            }
            out.push('\\');
            out.push_str(&hex);
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
fn unescape_mount(s: &str) -> String {
    s.to_string()
}

fn stable_volume_id(root: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in root.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(0x01000193);
    }
    h.max(1)
}

/// Inode as FRN surrogate (unique per filesystem).
pub fn platform_file_id(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    let md = std::fs::metadata(path).ok()?;
    Some(md.ino())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volumes_exist() {
        let v = platform_volumes().unwrap();
        assert!(!v.is_empty());
        assert!(!v[0].root.is_empty());
    }

    #[test]
    fn file_id_root() {
        // Root always exists on unix.
        let id = platform_file_id(Path::new("/")).or_else(|| platform_file_id(Path::new(".")));
        if let Some(id) = id {
            assert!(id > 0);
        }
    }
}
