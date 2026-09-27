// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Vector store: SQLite table of f32 embedding blobs + brute-force cosine top-k.
//!
//! Fine for v1 (millions of short chunks fit comfortably); swap for HNSW in
//! Phase 7 if profiling says so.

use std::path::PathBuf;

use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct VectorHit {
    pub path: String,
    pub chunk_index: u32,
    pub score: f32,
    pub snippet: String,
}

pub struct VectorStore {
    conn: Connection,
    path: PathBuf,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS embeddings (
    path         TEXT NOT NULL,
    chunk_index  INTEGER NOT NULL,
    model        TEXT NOT NULL,
    dims         INTEGER NOT NULL,
    vector       BLOB NOT NULL,
    snippet      TEXT NOT NULL DEFAULT '',
    created_ms   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (path, chunk_index, model)
);
CREATE INDEX IF NOT EXISTS idx_emb_model ON embeddings(model);
"#;

impl VectorStore {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, String> {
        let mut path = dir.into();
        path.push("vectors.db");
        Self::open_file(path)
    }

    pub fn open_file(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(&path).map_err(|e| e.to_string())?;
        // Same throughput tuning as the content store (Phase 7): WAL +
        // NORMAL sync; the vector index is always rebuildable from text.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        Ok(Self { conn, path })
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Replace all chunks for a path under `model`.
    pub fn put_path(
        &self,
        path: &str,
        model: &str,
        vectors: &[Vec<f32>],
        snippets: &[String],
    ) -> Result<(), String> {
        let now = now_ms();
        self.conn
            .execute(
                "DELETE FROM embeddings WHERE path = ?1 AND model = ?2",
                params![path, model],
            )
            .map_err(|e| e.to_string())?;
        for (i, v) in vectors.iter().enumerate() {
            if v.is_empty() {
                continue;
            }
            let blob = f32s_to_blob(v);
            let snippet = snippets.get(i).cloned().unwrap_or_default();
            self.conn
                .execute(
                    "INSERT INTO embeddings(path, chunk_index, model, dims, vector, snippet, created_ms)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![path, i as i64, model, v.len() as i64, blob, snippet, now],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn has_embeddings(&self, path: &str, model: &str) -> Result<bool, String> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM embeddings WHERE path = ?1 AND model = ?2",
                params![path, model],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    }

    pub fn count(&self, model: Option<&str>) -> Result<u64, String> {
        let n: i64 = match model {
            Some(m) => self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM embeddings WHERE model = ?1",
                    params![m],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?,
            None => self
                .conn
                .query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))
                .map_err(|e| e.to_string())?,
        };
        Ok(n as u64)
    }

    /// Brute-force cosine top-k over a model's embeddings.
    pub fn search(
        &self,
        query: &[f32],
        model: &str,
        limit: usize,
        path_prefix: Option<&str>,
    ) -> Result<Vec<VectorHit>, String> {
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = if path_prefix.is_some() {
            self.conn
                .prepare(
                    "SELECT path, chunk_index, dims, vector, snippet
                     FROM embeddings WHERE model = ?1 AND path LIKE ?2",
                )
                .map_err(|e| e.to_string())?
        } else {
            self.conn
                .prepare(
                    "SELECT path, chunk_index, dims, vector, snippet
                     FROM embeddings WHERE model = ?1",
                )
                .map_err(|e| e.to_string())?
        };

        let q_norm = l2_norm(query);
        if q_norm == 0.0 {
            return Ok(Vec::new());
        }

        let mut hits: Vec<VectorHit> = Vec::new();
        let mapper =
            |r: &rusqlite::Row<'_>| -> rusqlite::Result<(String, i64, i64, Vec<u8>, String)> {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            };

        let rows = match path_prefix {
            Some(p) => stmt.query_map(params![model, format!("{p}%")], mapper),
            None => stmt.query_map(params![model], mapper),
        }
        .map_err(|e| e.to_string())?;

        for row in rows {
            let (path, chunk_index, dims, blob, snippet) = row.map_err(|e| e.to_string())?;
            let v = blob_to_f32s(&blob);
            if v.len() as i64 != dims || v.is_empty() {
                continue;
            }
            let score = cosine(query, &v) / (q_norm * l2_norm(&v)).max(1e-9);
            hits.push(VectorHit {
                path,
                chunk_index: chunk_index as u32,
                score,
                snippet,
            });
        }

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit.max(1));
        Ok(hits)
    }

    pub fn remove_path(&self, path: &str) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM embeddings WHERE path = ?1", params![path])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Cosine similarity (both vectors assumed non-zero; caller normalizes denominators).
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut dot = 0.0f32;
    for i in 0..n {
        dot += a[i] * b[i];
    }
    dot
}

fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

fn f32s_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

fn blob_to_f32s(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
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

    static N: AtomicU32 = AtomicU32::new(0);

    fn temp_store() -> VectorStore {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("tpt-vec-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        VectorStore::open(&dir).unwrap()
    }

    #[test]
    fn put_and_search_orders_by_score() {
        let store = temp_store();
        store
            .put_path(
                "/docs/invoice.txt",
                "test-model",
                &[vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]],
                &["chunk zero".into(), "chunk one".into()],
            )
            .unwrap();
        store
            .put_path(
                "/docs/other.txt",
                "test-model",
                &[vec![0.0, 0.0, 1.0]],
                &["other".into()],
            )
            .unwrap();

        assert!(store
            .has_embeddings("/docs/invoice.txt", "test-model")
            .unwrap());
        assert_eq!(store.count(Some("test-model")).unwrap(), 3);

        let hits = store
            .search(&[1.0, 0.0, 0.0], "test-model", 5, None)
            .unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].path, "/docs/invoice.txt");
        assert_eq!(hits[0].chunk_index, 0);
        assert!(hits[0].score > 0.99);
        // Scores descending.
        assert!(hits[0].score >= hits[1].score && hits[1].score >= hits[2].score);

        let _ = std::fs::remove_dir_all(store.path().parent().unwrap());
    }

    #[test]
    fn path_prefix_filters() {
        let store = temp_store();
        store
            .put_path("/a/x.txt", "m", &[vec![1.0, 0.0]], &["a".into()])
            .unwrap();
        store
            .put_path("/b/y.txt", "m", &[vec![1.0, 0.0]], &["b".into()])
            .unwrap();
        let hits = store.search(&[1.0, 0.0], "m", 10, Some("/a/")).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "/a/x.txt");
        let _ = std::fs::remove_dir_all(store.path().parent().unwrap());
    }

    #[test]
    fn replace_path_removes_old_chunks() {
        let store = temp_store();
        store
            .put_path(
                "/p",
                "m",
                &[vec![1.0, 0.0], vec![0.0, 1.0]],
                &["a".into(), "b".into()],
            )
            .unwrap();
        store
            .put_path("/p", "m", &[vec![1.0, 1.0]], &["only".into()])
            .unwrap();
        assert_eq!(store.count(Some("m")).unwrap(), 1);
        let _ = std::fs::remove_dir_all(store.path().parent().unwrap());
    }

    #[test]
    fn blob_roundtrip() {
        let v = vec![1.5f32, -2.25, 0.0, 3.0e10];
        let blob = f32s_to_blob(&v);
        let back = blob_to_f32s(&blob);
        assert_eq!(v, back);
    }

    #[test]
    fn empty_query_returns_empty() {
        let store = temp_store();
        let hits = store.search(&[], "m", 5, None).unwrap();
        assert!(hits.is_empty());
        let _ = std::fs::remove_dir_all(store.path().parent().unwrap());
    }
}
