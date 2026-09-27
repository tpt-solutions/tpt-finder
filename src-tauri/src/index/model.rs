// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

use serde::{Deserialize, Serialize};

/// A single indexed filesystem object (file or directory).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Stable index into the in-memory entry vector (also the public id).
    pub id: u32,
    /// Absolute path (platform separator).
    pub path: String,
    /// Final path component (file or directory name).
    pub name: String,
    /// Lowercase name for ASCII-fast matching.
    pub name_lower: String,
    /// Size in bytes (0 for directories unless known).
    pub size: u64,
    /// Created time, Unix epoch milliseconds (0 if unknown).
    pub created_ms: i64,
    /// Modified time, Unix epoch milliseconds (0 if unknown).
    pub modified_ms: i64,
    /// Accessed time, Unix epoch milliseconds (0 if unknown).
    pub accessed_ms: i64,
    /// Platform attributes (Windows FILE_ATTRIBUTE_*, 0/ignored elsewhere).
    pub attributes: u32,
    /// Index into the engine's volume table.
    pub volume: u32,
    pub is_dir: bool,
    /// Windows file reference number (FRN / MFT id); 0 on other platforms.
    pub frn: u64,
}

impl FileEntry {
    pub fn compute_name_lower(name: &str) -> String {
        name.to_ascii_lowercase()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_meta(
        id: u32,
        path: String,
        size: u64,
        created_ms: i64,
        modified_ms: i64,
        accessed_ms: i64,
        attributes: u32,
        volume: u32,
        is_dir: bool,
        frn: u64,
    ) -> Self {
        let name = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(path.as_str())
            .to_string();
        let name_lower = Self::compute_name_lower(&name);
        Self {
            id,
            path,
            name,
            name_lower,
            size,
            created_ms,
            modified_ms,
            accessed_ms,
            attributes,
            volume,
            is_dir,
            frn,
        }
    }
}

/// A mounted volume that is eligible for indexing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VolumeInfo {
    /// Stable id (Windows: volume serial; Unix: hash of root path).
    pub id: u32,
    /// Root mount path, e.g. `C:\` or `/`.
    pub root: String,
    /// Filesystem name (e.g. `NTFS`, `ext4`).
    pub fs: String,
    /// Total bytes (0 if unknown).
    pub total_bytes: u64,
    /// Free bytes (0 if unknown).
    pub free_bytes: u64,
    /// Whether the backend supports live USN/inotify updates.
    pub live_updates: bool,
}

/// Lightweight item returned to the UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchResultItem {
    pub id: u32,
    pub path: String,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub modified_ms: i64,
    pub attributes: u32,
    /// Relevance score (higher is better).
    pub score: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub items: Vec<SearchResultItem>,
    /// Total matches before `limit` truncation.
    pub total_matched: usize,
    pub took_ms: u64,
    pub query: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_meta_derives_name_and_lower() {
        let e = FileEntry::from_meta(
            1,
            "/home/User/Report.PDF".into(),
            10,
            0,
            0,
            0,
            0,
            0,
            false,
            0,
        );
        assert_eq!(e.name, "Report.PDF");
        assert_eq!(e.name_lower, "report.pdf");
    }

    #[cfg(windows)]
    #[test]
    fn from_meta_windows_separators() {
        let e = FileEntry::from_meta(1, r"C:\Users\a\b.txt".into(), 1, 0, 0, 0, 0, 0, false, 0);
        assert_eq!(e.name, "b.txt");
    }
}
