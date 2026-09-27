// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! In-memory index optimized for instant prefix/substring/fuzzy matching.

use std::collections::{HashMap, HashSet};

use super::model::{FileEntry, SearchResult, SearchResultItem};
use super::query::{contains_ci, match_term, matches_dated, score_kind, MatchKind, ParsedQuery};

/// Backing store for the active index.
///
/// Layout choices for the Everything-style performance bar:
/// - Dense `Vec<FileEntry>` with stable ids (index == id).
/// - `name_index`: lowercase name → entry ids for O(1) exact/prefix jumps.
/// - `path_to_id`: full path → id for O(1) upsert/delete on USN/inotify events.
/// - Deleted slots become `None` and are recycled via `free_ids`.
#[derive(Default)]
pub struct IndexStore {
    entries: Vec<Option<FileEntry>>,
    free_ids: Vec<u32>,
    /// Lowercase name → entry ids (multiple files can share a name).
    name_index: HashMap<String, Vec<u32>>,
    path_to_id: HashMap<String, u32>,
    /// All lowercase names, sorted — enables binary-search prefix ranges.
    sorted_names: Vec<String>,
    sorted_dirty: bool,
    file_count: u64,
    dir_count: u64,
}

impl IndexStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len() - self.free_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn file_count(&self) -> u64 {
        self.file_count
    }

    pub fn dir_count(&self) -> u64 {
        self.dir_count
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.free_ids.clear();
        self.name_index.clear();
        self.path_to_id.clear();
        self.sorted_names.clear();
        self.sorted_dirty = false;
        self.file_count = 0;
        self.dir_count = 0;
    }

    pub fn get(&self, id: u32) -> Option<&FileEntry> {
        self.entries.get(id as usize).and_then(|e| e.as_ref())
    }

    pub fn id_for_path(&self, path: &str) -> Option<u32> {
        self.path_to_id.get(path).copied()
    }

    /// Insert or replace by path. Returns the entry id.
    pub fn upsert(&mut self, mut entry: FileEntry) -> u32 {
        if let Some(&existing) = self.path_to_id.get(&entry.path) {
            self.remove_id(existing);
            entry.id = existing;
            self.insert_with_id(entry);
            return existing;
        }
        let id = match self.free_ids.pop() {
            Some(id) => {
                let idx = id as usize;
                if idx >= self.entries.len() {
                    self.entries.resize(idx + 1, None);
                }
                id
            }
            None => {
                let id = self.entries.len() as u32;
                self.entries.push(None);
                id
            }
        };
        entry.id = id;
        self.insert_with_id(entry);
        id
    }

    fn insert_with_id(&mut self, entry: FileEntry) {
        let id = entry.id;
        let idx = id as usize;
        if idx >= self.entries.len() {
            self.entries.resize(idx + 1, None);
        }

        // Maintain counts when slot was empty.
        let was_empty = self.entries[idx].is_none();
        if was_empty {
            // Remove from free list if present (linear; free lists stay small in practice).
            if let Some(pos) = self.free_ids.iter().position(|&f| f == id) {
                self.free_ids.swap_remove(pos);
            }
        }

        if let Some(prev) = self.entries[idx].take() {
            self.decrement_kind_counts(&prev);
            self.unindex_name(&prev);
            self.path_to_id.remove(&prev.path);
        }

        self.name_index
            .entry(entry.name_lower.clone())
            .or_default()
            .push(id);
        self.path_to_id.insert(entry.path.clone(), id);
        self.sorted_dirty = true;
        self.increment_kind_counts(&entry);
        self.entries[idx] = Some(entry);
    }

    fn increment_kind_counts(&mut self, e: &FileEntry) {
        if e.is_dir {
            self.dir_count += 1;
        } else {
            self.file_count += 1;
        }
    }

    fn decrement_kind_counts(&mut self, e: &FileEntry) {
        if e.is_dir {
            self.dir_count = self.dir_count.saturating_sub(1);
        } else {
            self.file_count = self.file_count.saturating_sub(1);
        }
    }

    fn unindex_name(&mut self, e: &FileEntry) {
        if let Some(ids) = self.name_index.get_mut(&e.name_lower) {
            ids.retain(|&id| id != e.id);
            if ids.is_empty() {
                self.name_index.remove(&e.name_lower);
            }
        }
    }

    pub fn remove_path(&mut self, path: &str) -> bool {
        match self.path_to_id.get(path).copied() {
            Some(id) => self.remove_id(id),
            None => false,
        }
    }

    pub fn remove_id(&mut self, id: u32) -> bool {
        let idx = id as usize;
        if idx >= self.entries.len() {
            return false;
        }
        let Some(prev) = self.entries[idx].take() else {
            return false;
        };
        self.decrement_kind_counts(&prev);
        self.unindex_name(&prev);
        self.path_to_id.remove(&prev.path);
        self.free_ids.push(id);
        true
    }

    /// Bulk load — clears existing data and builds indexes in one pass.
    pub fn bulk_load(&mut self, entries: Vec<FileEntry>) {
        self.clear();
        let cap = entries.len();
        self.entries = Vec::with_capacity(cap);
        self.path_to_id = HashMap::with_capacity_and_hasher(cap, Default::default());
        self.name_index = HashMap::with_capacity_and_hasher(cap, Default::default());
        for mut e in entries {
            let id = self.entries.len() as u32;
            e.id = id;
            self.name_index
                .entry(e.name_lower.clone())
                .or_default()
                .push(id);
            self.path_to_id.insert(e.path.clone(), id);
            if e.is_dir {
                self.dir_count += 1;
            } else {
                self.file_count += 1;
            }
            self.entries.push(Some(e));
        }
        self.sorted_dirty = true;
        self.free_ids.clear();
    }

    fn ensure_sorted_names(&mut self) {
        if !self.sorted_dirty {
            return;
        }
        self.sorted_names.clear();
        self.sorted_names.extend(self.name_index.keys().cloned());
        self.sorted_names.sort_unstable();
        self.sorted_dirty = false;
    }

    /// Candidate entry ids for a term via sorted-name binary search (prefix)
    /// plus a full scan fallback for substring when needed.
    fn candidates_for_term(&mut self, term: &str, fuzzy: bool) -> Vec<(u32, MatchKind)> {
        let mut out: Vec<(u32, MatchKind)> = Vec::new();
        let mut seen: HashSet<u32> = HashSet::new();

        // Exact / prefix via sorted names.
        self.ensure_sorted_names();
        let prefix = term.to_ascii_lowercase();
        let start = self
            .sorted_names
            .partition_point(|n| n.as_str() < prefix.as_str());
        for name in self.sorted_names[start..].iter() {
            if !name.starts_with(&prefix) {
                break;
            }
            if let Some(ids) = self.name_index.get(name.as_str()) {
                let kind = if name.as_str() == prefix.as_str() {
                    MatchKind::ExactName
                } else {
                    MatchKind::Prefix
                };
                for &id in ids {
                    if seen.insert(id) {
                        out.push((id, kind));
                    }
                }
            }
        }

        // Substring / path / fuzzy: scan live entries not already matched.
        // For multi-M indexes this is the cost of substring queries; Everything
        // uses a specialized substring index — acceptable for Phase 1 baseline.
        if term.len() >= 2 || fuzzy {
            for slot in self.entries.iter().flatten() {
                if seen.contains(&slot.id) {
                    continue;
                }
                if let Some(kind) = match_term(
                    &slot.name_lower,
                    &slot.path.to_ascii_lowercase(),
                    term,
                    fuzzy,
                ) {
                    if matches!(
                        kind,
                        MatchKind::Substring | MatchKind::PathSubstring | MatchKind::Fuzzy
                    ) {
                        seen.insert(slot.id);
                        out.push((slot.id, kind));
                    }
                }
            }
        }

        out
    }

    /// Execute a parsed query. `limit` caps returned items; `total_matched`
    /// reflects the pre-limit count.
    pub fn search(&mut self, query: &ParsedQuery, limit: usize) -> SearchResult {
        let started = std::time::Instant::now();
        let q_display = String::new(); // filled by caller via with_query_display if needed

        if query.is_empty() {
            return SearchResult {
                items: Vec::new(),
                total_matched: 0,
                took_ms: started.elapsed().as_millis() as u64,
                query: q_display,
            };
        }

        // Structural-only query: scan all entries.
        if query.terms.is_empty() && query.has_structural_filters() {
            let mut matches = Vec::new();
            for slot in self.entries.iter().flatten() {
                if structural_pass(query, slot) {
                    matches.push((slot.id, 0u32));
                }
            }
            return self.finish_search(matches, limit, started, q_display);
        }

        // Intersect term candidate sets.
        let mut sets: Vec<Vec<(u32, MatchKind)>> = Vec::new();
        for term in &query.terms {
            sets.push(self.candidates_for_term(term, query.fuzzy));
        }

        let mut score_map: HashMap<u32, u32> = HashMap::new();
        if sets.is_empty() {
            // Shouldn't reach (is_empty checked), but handle gracefully.
            for slot in self.entries.iter().flatten() {
                score_map.insert(slot.id, 0);
            }
        } else {
            // Start with first term's matches, then intersect.
            for &(id, kind) in &sets[0] {
                score_map.insert(id, score_kind(kind));
            }
            for set in sets.iter().skip(1) {
                let next: HashMap<u32, u32> = set
                    .iter()
                    .map(|&(id, kind)| (id, score_kind(kind)))
                    .collect();
                score_map.retain(|id, score| {
                    if let Some(&s2) = next.get(id) {
                        *score += s2;
                        true
                    } else {
                        false
                    }
                });
            }
        }

        // Apply structural filters.
        let mut matches: Vec<(u32, u32)> = Vec::new();
        for (id, score) in score_map {
            let Some(entry) = self.get(id) else {
                continue;
            };
            if structural_pass(query, entry) {
                matches.push((id, score));
            }
        }

        self.finish_search(matches, limit, started, q_display)
    }

    fn finish_search(
        &self,
        mut matches: Vec<(u32, u32)>,
        limit: usize,
        started: std::time::Instant,
        query: String,
    ) -> SearchResult {
        // Sort: higher score first, then shorter name (more specific), then name asc.
        matches.sort_by(|a, b| {
            let ea = self.get(a.0);
            let eb = self.get(b.0);
            b.1.cmp(&a.1)
                .then_with(|| {
                    let la = ea.map(|e| e.name.len()).unwrap_or(usize::MAX);
                    let lb = eb.map(|e| e.name.len()).unwrap_or(usize::MAX);
                    la.cmp(&lb)
                })
                .then_with(|| {
                    ea.map(|e| e.name_lower.as_str())
                        .unwrap_or("")
                        .cmp(eb.map(|e| e.name_lower.as_str()).unwrap_or(""))
                })
        });

        let total_matched = matches.len();
        let items: Vec<SearchResultItem> = matches
            .into_iter()
            .take(limit)
            .filter_map(|(id, score)| {
                let e = self.get(id)?;
                Some(SearchResultItem {
                    id: e.id,
                    path: e.path.clone(),
                    name: e.name.clone(),
                    size: e.size,
                    is_dir: e.is_dir,
                    modified_ms: e.modified_ms,
                    attributes: e.attributes,
                    score,
                })
            })
            .collect();

        SearchResult {
            items,
            total_matched,
            took_ms: started.elapsed().as_millis() as u64,
            query,
        }
    }

    /// Snapshot all live entries (for persistence).
    pub fn snapshot_entries(&self) -> Vec<FileEntry> {
        self.entries.iter().flatten().cloned().collect()
    }

    /// Restore from persistence without reusing free-list bookkeeping.
    pub fn from_snapshot(entries: Vec<FileEntry>) -> Self {
        let mut s = Self::new();
        s.bulk_load(entries);
        s
    }

    /// Number of allocated slots including deleted (internal / tests).
    #[allow(dead_code)]
    pub fn slot_count(&self) -> usize {
        self.entries.len()
    }

    /// Names with the given exact lowercase key (tests / debug).
    #[allow(dead_code)]
    pub fn ids_with_name(&self, name_lower: &str) -> Option<&[u32]> {
        self.name_index.get(name_lower).map(|v| v.as_slice())
    }

    /// Prefix range helper used by benchmarks.
    pub fn count_prefix(&mut self, prefix: &str) -> usize {
        self.ensure_sorted_names();
        let prefix = prefix.to_ascii_lowercase();
        let start = self
            .sorted_names
            .partition_point(|n| n.as_str() < prefix.as_str());
        self.sorted_names[start..]
            .iter()
            .take_while(|n| n.starts_with(&prefix))
            .count()
    }

    /// Whether a path is currently indexed.
    pub fn contains_path(&self, path: &str) -> bool {
        self.path_to_id.contains_key(path)
    }
}

fn structural_pass(query: &ParsedQuery, e: &FileEntry) -> bool {
    if query.dirs_only && !e.is_dir {
        return false;
    }
    if query.files_only && e.is_dir {
        return false;
    }
    if let Some(ext) = &query.ext {
        // Match extension on files (and dirs with dots, rarely).
        let name = e.name_lower.as_str();
        let matches_ext = name
            .rsplit_once('.')
            .map(|(_, x)| x == ext.as_str())
            .unwrap_or(false);
        if !matches_ext {
            return false;
        }
    }
    if let Some(p) = &query.path {
        if !contains_ci(&e.path.to_ascii_lowercase(), p) {
            return false;
        }
    }
    if let Some(min) = query.size_min {
        if e.size < min {
            return false;
        }
    }
    if let Some(max) = query.size_max {
        if e.size > max {
            return false;
        }
    }
    if let Some(dated) = &query.dated {
        if !matches_dated(dated, e.modified_ms) {
            return false;
        }
    }
    true
}

// Silence unused import warning when only used in tests on some cfgs.
#[allow(unused_imports)]
use super::query::starts_with_ci as _starts_with_ci;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::model::FileEntry;

    fn entry(path: &str, size: u64) -> FileEntry {
        FileEntry::from_meta(
            0,
            path.to_string(),
            size,
            0,
            1_767_225_600_000,
            0,
            0,
            0,
            false,
            0,
        )
    }

    fn dir(path: &str) -> FileEntry {
        FileEntry::from_meta(0, path.to_string(), 0, 0, 0, 0, 0, 0, true, 0)
    }

    #[test]
    fn upsert_and_search_prefix() {
        let mut s = IndexStore::new();
        s.upsert(entry("/docs/Report.pdf", 100));
        s.upsert(entry("/docs/notes.txt", 50));
        s.upsert(dir("/docs"));

        let r = s.search(&ParsedQuery::parse("rep"), 10);
        assert_eq!(r.total_matched, 1);
        assert_eq!(r.items[0].name, "Report.pdf");

        let r = s.search(&ParsedQuery::parse("ext:pdf"), 10);
        assert_eq!(r.total_matched, 1);

        let r = s.search(&ParsedQuery::parse("path:docs"), 10);
        assert_eq!(r.total_matched, 3);
    }

    #[test]
    fn upsert_replaces_by_path() {
        let mut s = IndexStore::new();
        s.upsert(entry("/a.txt", 1));
        s.upsert(entry("/a.txt", 99));
        assert_eq!(s.len(), 1);
        let r = s.search(&ParsedQuery::parse("a.txt"), 10);
        assert_eq!(r.items[0].size, 99);
    }

    #[test]
    fn remove_path() {
        let mut s = IndexStore::new();
        s.upsert(entry("/gone.txt", 1));
        assert!(s.remove_path("/gone.txt"));
        assert!(!s.remove_path("/gone.txt"));
        assert_eq!(s.len(), 0);
        assert_eq!(s.file_count(), 0);
    }

    #[test]
    fn free_id_reuse() {
        let mut s = IndexStore::new();
        s.upsert(entry("/x1.txt", 1));
        s.upsert(entry("/x2.txt", 1));
        let id1 = s.id_for_path("/x1.txt").unwrap();
        s.remove_path("/x1.txt");
        s.upsert(entry("/x3.txt", 1));
        let id3 = s.id_for_path("/x3.txt").unwrap();
        assert_eq!(id1, id3);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn multi_term_and() {
        let mut s = IndexStore::new();
        s.upsert(entry("/invoice_2026.pdf", 10));
        s.upsert(entry("/invoice_2025.pdf", 10));
        s.upsert(entry("/resume.pdf", 10));
        let r = s.search(&ParsedQuery::parse("invoice 2026"), 10);
        assert_eq!(r.total_matched, 1);
        assert!(r.items[0].name.contains("2026"));
    }

    #[test]
    fn dirs_only_filter() {
        let mut s = IndexStore::new();
        s.upsert(entry("/a/file.txt", 1));
        s.upsert(dir("/a/subdir"));
        // Use a term that prefix-matches the dir name; dirs: alone is structural.
        let r = s.search(&ParsedQuery::parse("dirs sub"), 10);
        assert_eq!(r.total_matched, 1);
        assert!(r.items[0].is_dir);
        let r = s.search(&ParsedQuery::parse("dirs"), 10);
        assert_eq!(r.total_matched, 1);
        assert!(r.items[0].is_dir);
    }

    #[test]
    fn bulk_load_counts() {
        let mut s = IndexStore::new();
        s.bulk_load(vec![entry("/f1.txt", 1), entry("/f2.txt", 1), dir("/d1")]);
        assert_eq!(s.file_count(), 2);
        assert_eq!(s.dir_count(), 1);
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn size_filter() {
        let mut s = IndexStore::new();
        s.upsert(entry("/big.bin", 20 * 1024 * 1024));
        s.upsert(entry("/small.bin", 10));
        let r = s.search(&ParsedQuery::parse("size:>1mb"), 10);
        assert_eq!(r.total_matched, 1);
        assert_eq!(r.items[0].name, "big.bin");
    }

    #[test]
    fn fuzzy_flag() {
        let mut s = IndexStore::new();
        s.upsert(entry("/annual_report_final.pdf", 1));
        let r = s.search(&ParsedQuery::parse("fuzzy:anlprt"), 10);
        assert_eq!(r.total_matched, 1);
    }

    #[test]
    fn count_prefix_helper() {
        let mut s = IndexStore::new();
        s.upsert(entry("/a/alpha.txt", 1));
        s.upsert(entry("/a/alpine.txt", 1));
        s.upsert(entry("/a/beta.txt", 1));
        assert_eq!(s.count_prefix("alp"), 2);
    }
}
