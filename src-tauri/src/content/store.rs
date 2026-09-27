// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! SQLite + FTS5 store for extracted content (alongside the path index).

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use super::extract::Extracted;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS content (
    path        TEXT PRIMARY KEY,
    title       TEXT,
    author      TEXT,
    language    TEXT,
    source      TEXT NOT NULL,
    text        TEXT NOT NULL,
    size_bytes  INTEGER NOT NULL DEFAULT 0,
    extracted_ms INTEGER NOT NULL DEFAULT 0,
    file_modified_ms INTEGER NOT NULL DEFAULT 0,
    ok          INTEGER NOT NULL DEFAULT 1,
    error       TEXT
);
-- Standalone FTS5 (not external-content) so inserts/deletes stay consistent
-- without shadow-table bookkeeping.
CREATE VIRTUAL TABLE IF NOT EXISTS content_fts USING fts5(
    path UNINDEXED,
    title,
    text,
    tokenize = 'porter unicode61'
);
CREATE TABLE IF NOT EXISTS content_failures (
    path TEXT PRIMARY KEY,
    error TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 1,
    last_ms INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_content_ok ON content(ok);
"#;

pub struct ContentStore {
    conn: Connection,
    path: PathBuf,
}

impl ContentStore {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, String> {
        let mut path = dir.into();
        path.push("content.db");
        Self::open_file(path)
    }

    pub fn open_file(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(&path).map_err(|e| e.to_string())?;
        // Throughput tuning (Phase 7): WAL + NORMAL keeps per-file commits
        // cheap during bulk extraction while staying crash-safe for an
        // index that can always be rebuilt from source files.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        // Ensure FTS5 is compiled in: create a throwaway FTS table and MATCH it.
        // (SELECT on an empty content_fts returns no rows, which is not an error.)
        conn.execute_batch("CREATE VIRTUAL TABLE IF NOT EXISTS _fts_probe USING fts5(x);")
            .map_err(|e| format!("fts5 unavailable: {e}"))?;
        let probe: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM _fts_probe WHERE _fts_probe MATCH 'zzzunlikely'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| format!("fts5 unavailable: {e}"))?;
        // 0 hits is fine — the MATCH clause itself must not error.
        let _ = probe;
        let _ = conn.execute_batch("DROP TABLE IF EXISTS _fts_probe;");
        Ok(Self { conn, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn upsert_ok(
        &self,
        path: &str,
        extracted: &Extracted,
        size_bytes: u64,
        file_modified_ms: i64,
        text_clamped: String,
    ) -> Result<(), String> {
        let now = now_ms();
        let title = extracted.title.clone().unwrap_or_default();
        self.conn
            .execute(
                "INSERT INTO content(path, title, author, language, source, text, size_bytes, extracted_ms, file_modified_ms, ok, error)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, NULL)
                 ON CONFLICT(path) DO UPDATE SET
                   title=excluded.title, author=excluded.author, language=excluded.language,
                   source=excluded.source, text=excluded.text, size_bytes=excluded.size_bytes,
                   extracted_ms=excluded.extracted_ms, file_modified_ms=excluded.file_modified_ms,
                   ok=1, error=NULL",
                params![
                    path,
                    title,
                    extracted.author.clone().unwrap_or_default(),
                    extracted.language.clone().unwrap_or_default(),
                    extracted.source,
                    text_clamped,
                    size_bytes as i64,
                    now,
                    file_modified_ms,
                ],
            )
            .map_err(|e| e.to_string())?;
        // Mirror into standalone FTS (delete by path, then insert current row).
        self.conn
            .execute("DELETE FROM content_fts WHERE path = ?1", params![path])
            .map_err(|e| e.to_string())?;
        self.conn
            .execute(
                "INSERT INTO content_fts(path, title, text)
                 SELECT path, title, text FROM content WHERE path = ?1",
                params![path],
            )
            .map_err(|e| e.to_string())?;
        // Clear any prior failure.
        let _ = self.conn.execute(
            "DELETE FROM content_failures WHERE path = ?1",
            params![path],
        );
        Ok(())
    }

    pub fn record_failure(&self, path: &str, error: &str) -> Result<(), String> {
        let now = now_ms();
        self.conn
            .execute(
                "INSERT INTO content_failures(path, error, attempts, last_ms)
                 VALUES(?1, ?2, 1, ?3)
                 ON CONFLICT(path) DO UPDATE SET
                   error=excluded.error,
                   attempts=content_failures.attempts + 1,
                   last_ms=excluded.last_ms",
                params![path, error, now],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Full-text search over extracted content. Returns (path, snippet) pairs.
    pub fn search_fts(&self, query: &str, limit: usize) -> Result<Vec<(String, String)>, String> {
        // Escape FTS5 special chars by wrapping terms in double quotes.
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let fts_query = terms.join(" ");
        let mut stmt = self
            .conn
            .prepare(
                "SELECT f.path, snippet(content_fts, 2, '[', ']', '…', 12)
                 FROM content_fts f
                 JOIN content c ON c.path = f.path
                 WHERE content_fts MATCH ?1 AND c.ok = 1
                 ORDER BY rank
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![fts_query, limit as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn has_content(&self, path: &str) -> Result<bool, String> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM content WHERE path = ?1 AND ok = 1",
                params![path],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    }

    pub fn failure_count(&self) -> Result<u64, String> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM content_failures", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(n as u64)
    }

    pub fn content_count(&self) -> Result<u64, String> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM content WHERE ok = 1", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        Ok(n as u64)
    }

    pub fn remove(&self, path: &str) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM content_fts WHERE path = ?1", params![path])
            .map_err(|e| e.to_string())?;
        self.conn
            .execute("DELETE FROM content WHERE path = ?1", params![path])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Iterate (path, text) for embedding. Skips paths where `needs_embed`
    /// returns false unless `force` is true.
    pub fn text_rows_for_embedding(
        &self,
        force: bool,
        _model: &str,
        needs_embed: impl Fn(&str) -> bool,
    ) -> Result<Vec<(String, String)>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, text FROM content WHERE ok = 1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            let (path, text) = row.map_err(|e| e.to_string())?;
            if force || needs_embed(&path) {
                out.push((path, text));
            }
        }
        Ok(out)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static STORE_N: AtomicU32 = AtomicU32::new(0);

    fn temp_store() -> ContentStore {
        let n = STORE_N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("tpt-content-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        ContentStore::open(&dir).expect("open content store")
    }

    #[test]
    fn fts_roundtrip() {
        let store = temp_store();
        let ex = Extracted {
            text: "The quarterly invoice for Acme Corp is attached.".into(),
            title: Some("Invoice Q1".into()),
            author: Some("Jane".into()),
            created_ms: None,
            language: None,
            source: "text",
        };
        store
            .upsert_ok(r"C:\docs\invoice.txt", &ex, 48, 0, ex.text.clone())
            .unwrap();
        assert!(store.has_content(r"C:\docs\invoice.txt").unwrap());
        let hits = store.search_fts("invoice", 10).unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].0.contains("invoice.txt"));
        assert!(hits[0].1.contains("invoice"));
        let _ = std::fs::remove_dir_all(store.path().parent().unwrap());
    }

    #[test]
    fn failure_recorded() {
        let store = temp_store();
        store.record_failure("/x/y.pdf", "parse error").unwrap();
        store.record_failure("/x/y.pdf", "again").unwrap();
        assert_eq!(store.failure_count().unwrap(), 1);
        let _ = std::fs::remove_dir_all(store.path().parent().unwrap());
    }

    #[test]
    fn remove_clears_fts() {
        let store = temp_store();
        let ex = Extracted {
            text: "uniquezebra token".into(),
            ..Default::default()
        };
        store
            .upsert_ok("/z.txt", &ex, 1, 0, ex.text.clone())
            .unwrap();
        assert!(!store.search_fts("uniquezebra", 5).unwrap().is_empty());
        store.remove("/z.txt").unwrap();
        assert!(store.search_fts("uniquezebra", 5).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(store.path().parent().unwrap());
    }
}
