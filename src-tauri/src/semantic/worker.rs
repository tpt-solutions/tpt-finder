// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Background embedding worker: pulls extracted content, chunks, embeds via
//! Ollama, and stores vectors. Rate-limited and skippable when disabled.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::chunk::chunk_text;
use super::ollama::OllamaClient;
use super::vector::VectorStore;
use crate::logging::ErrorLog;

#[derive(Clone, Debug)]
pub struct EmbedJob {
    pub path: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct EmbedStats {
    pub queued: u64,
    pub processed: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub skipped: u64,
    pub running: bool,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct EmbedConfig {
    pub model: String,
    pub chunk_chars: usize,
    pub max_chunks_per_file: usize,
    pub throttle_ms: u64,
}

impl Default for EmbedConfig {
    fn default() -> Self {
        Self {
            model: "nomic-embed-text".into(),
            chunk_chars: 512,
            max_chunks_per_file: 32,
            throttle_ms: 50,
        }
    }
}

struct Shared {
    stats: Mutex<EmbedStats>,
    stop: AtomicBool,
    enabled: AtomicBool,
    config: Mutex<EmbedConfig>,
    base_url: Mutex<String>,
}

pub struct EmbeddingWorker {
    shared: Arc<Shared>,
    tx: Mutex<Option<Sender<EmbedJob>>>,
    store: Arc<Mutex<VectorStore>>,
    error_log: Mutex<Option<Arc<ErrorLog>>>,
}

impl EmbeddingWorker {
    pub fn spawn(store: VectorStore, config: EmbedConfig, enabled: bool) -> Arc<Self> {
        let store = Arc::new(Mutex::new(store));
        let (tx, rx) = channel::<EmbedJob>();
        let shared = Arc::new(Shared {
            stats: Mutex::new(EmbedStats::default()),
            stop: AtomicBool::new(false),
            enabled: AtomicBool::new(enabled),
            config: Mutex::new(config),
            base_url: Mutex::new(super::ollama::DEFAULT_BASE_URL.to_string()),
        });

        let worker = Arc::new(Self {
            shared: Arc::clone(&shared),
            tx: Mutex::new(Some(tx)),
            store: Arc::clone(&store),
            error_log: Mutex::new(None),
        });

        let w_shared = Arc::clone(&shared);
        let w_store = Arc::clone(&store);
        let w_worker = Arc::clone(&worker);
        std::thread::Builder::new()
            .name("embed-worker".into())
            .spawn(move || worker_loop(rx, w_shared, w_store, w_worker))
            .ok();

        worker
    }

    /// Attach the opt-in error log (Phase 8). Embed failures land in the
    /// local error.log when the user has enabled it.
    pub fn set_error_log(&self, log: Arc<ErrorLog>) {
        *self.error_log.lock().unwrap() = Some(log);
    }

    fn log_failure(&self, msg: &str) {
        if let Ok(guard) = self.error_log.lock() {
            if let Some(log) = guard.as_ref() {
                log.log("embed", msg);
            }
        }
    }

    pub fn enqueue(&self, job: EmbedJob) {
        if self.shared.stop.load(Ordering::Relaxed) {
            return;
        }
        {
            let mut s = self.shared.stats.lock().unwrap();
            s.queued += 1;
            s.running = true;
        }
        let tx = self.tx.lock().unwrap();
        if let Some(tx) = tx.as_ref() {
            let _ = tx.send(job);
        }
    }

    pub fn stats(&self) -> EmbedStats {
        self.shared.stats.lock().unwrap().clone()
    }

    pub fn set_enabled(&self, on: bool) {
        self.shared.enabled.store(on, Ordering::Relaxed);
    }

    pub fn enabled(&self) -> bool {
        self.shared.enabled.load(Ordering::Relaxed)
    }

    pub fn config(&self) -> EmbedConfig {
        self.shared.config.lock().unwrap().clone()
    }

    pub fn set_config(&self, cfg: EmbedConfig) {
        *self.shared.config.lock().unwrap() = cfg;
    }

    /// Loopback-only Ollama base URL (validated by [`OllamaClient::new`]).
    pub fn set_base_url(&self, url: &str) {
        if crate::semantic::ollama::validate_base_url(url).is_ok() {
            *self.shared.base_url.lock().unwrap() = url.trim_end_matches('/').to_string();
        }
    }

    pub fn base_url(&self) -> String {
        self.shared.base_url.lock().unwrap().clone()
    }

    pub fn store(&self) -> Arc<Mutex<VectorStore>> {
        Arc::clone(&self.store)
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        *self.tx.lock().unwrap() = None;
    }
}

impl Drop for EmbeddingWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn worker_loop(
    rx: Receiver<EmbedJob>,
    shared: Arc<Shared>,
    store: Arc<Mutex<VectorStore>>,
    worker: Arc<EmbeddingWorker>,
) {
    while !shared.stop.load(Ordering::Relaxed) {
        let job = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(j) => j,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let mut s = shared.stats.lock().unwrap();
                if s.queued == 0 {
                    s.running = false;
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };

        {
            let mut s = shared.stats.lock().unwrap();
            s.queued = s.queued.saturating_sub(1);
            s.processed += 1;
        }

        if !shared.enabled.load(Ordering::Relaxed) {
            let mut s = shared.stats.lock().unwrap();
            s.skipped += 1;
            continue;
        }

        let cfg = shared.config.lock().unwrap().clone();
        let url = shared.base_url.lock().unwrap().clone();
        // Privacy: `OllamaClient::new` rejects non-loopback hosts.
        let client = match OllamaClient::new(&url) {
            Ok(c) => c,
            Err(e) => {
                worker.log_failure(&format!("{}: {}", job.path, e));
                let mut s = shared.stats.lock().unwrap();
                s.failed += 1;
                s.last_error = Some(e);
                continue;
            }
        };

        let mut chunks = chunk_text(&job.text, cfg.chunk_chars);
        if chunks.len() > cfg.max_chunks_per_file {
            chunks.truncate(cfg.max_chunks_per_file);
        }
        if chunks.is_empty() {
            let mut s = shared.stats.lock().unwrap();
            s.skipped += 1;
            continue;
        }

        match client.embed(&cfg.model, &chunks) {
            Ok(vectors) => {
                let st = store.lock().unwrap();
                if let Err(e) = st.put_path(&job.path, &cfg.model, &vectors, &chunks) {
                    drop(st);
                    worker.log_failure(&format!("{}: {}", job.path, e));
                    let mut s = shared.stats.lock().unwrap();
                    s.failed += 1;
                    s.last_error = Some(e);
                } else {
                    drop(st);
                    let mut s = shared.stats.lock().unwrap();
                    s.succeeded += 1;
                    s.last_error = None;
                }
            }
            Err(e) => {
                worker.log_failure(&format!("{}: {}", job.path, e));
                let mut s = shared.stats.lock().unwrap();
                s.failed += 1;
                s.last_error = Some(e);
            }
        }

        let throttle = cfg.throttle_ms;
        if throttle > 0 {
            std::thread::sleep(Duration::from_millis(throttle));
        }
    }
    let mut s = shared.stats.lock().unwrap();
    s.running = false;
}

/// Helper used by `rebuild_semantic` IPC: read file text from content store
/// (caller supplies text) and enqueue.
pub fn enqueue_text(worker: &EmbeddingWorker, path: String, text: String) {
    if text.trim().is_empty() {
        return;
    }
    worker.enqueue(EmbedJob { path, text });
}

/// Convenience for tests / callers with a path (reads from disk).
pub fn try_enqueue_file(worker: &EmbeddingWorker, path: PathBuf) {
    let Ok(meta) = std::fs::metadata(&path) else {
        return;
    };
    if !meta.is_file() {
        return;
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    worker.enqueue(EmbedJob {
        path: path.to_string_lossy().into_owned(),
        text,
    });
}
