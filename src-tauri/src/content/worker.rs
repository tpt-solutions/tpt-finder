// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Background extraction worker: rate-limited queue over new/changed files.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use super::extract::{extract_file, ExtractionError};
use super::skip::SkipRules;
use super::store::ContentStore;
use crate::logging::ErrorLog;
use crate::semantic::{EmbedJob, EmbeddingWorker};

/// Optional downstream consumer: successfully extracted text is handed to the
/// embedding queue (incremental semantic indexing) when present + enabled.
type EmbedHook = Arc<EmbeddingWorker>;

#[derive(Clone, Debug)]
pub struct ExtractionJob {
    pub path: PathBuf,
    pub size: u64,
    pub file_modified_ms: i64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct ExtractionStats {
    pub queued: u64,
    pub processed: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub skipped: u64,
    pub running: bool,
}

struct Shared {
    stats: Mutex<ExtractionStats>,
    stop: AtomicBool,
    /// Milliseconds to pause between successful extractions (0 = no throttle).
    throttle_ms: AtomicU64,
}

/// Owns the store + worker thread. Cheap to share via `Arc`.
pub struct ExtractionWorker {
    shared: Arc<Shared>,
    tx: Mutex<Option<Sender<ExtractionJob>>>,
    store: Arc<Mutex<ContentStore>>,
    embed: Arc<Mutex<Option<EmbedHook>>>,
    rules: Arc<RwLock<SkipRules>>,
    error_log: Mutex<Option<Arc<ErrorLog>>>,
}

impl ExtractionWorker {
    pub fn spawn(store: ContentStore, rules: SkipRules) -> Arc<Self> {
        let store = Arc::new(Mutex::new(store));
        let rules = Arc::new(RwLock::new(rules));
        let (tx, rx) = channel::<ExtractionJob>();
        let shared = Arc::new(Shared {
            stats: Mutex::new(ExtractionStats::default()),
            stop: AtomicBool::new(false),
            throttle_ms: AtomicU64::new(0),
        });
        let embed = Arc::new(Mutex::new(None));

        let worker = Arc::new(Self {
            shared: Arc::clone(&shared),
            tx: Mutex::new(Some(tx)),
            store: Arc::clone(&store),
            embed: Arc::clone(&embed),
            rules: Arc::clone(&rules),
            error_log: Mutex::new(None),
        });

        let w_shared = Arc::clone(&shared);
        let w_store = Arc::clone(&store);
        let w_rules = Arc::clone(&rules);
        let w_worker = Arc::clone(&worker);
        std::thread::Builder::new()
            .name("content-extract".into())
            .spawn(move || worker_loop(rx, w_shared, w_store, w_rules, w_worker))
            .ok();

        worker
    }

    /// Wire the embedding worker so extracted text flows straight into the
    /// semantic index (incremental; jobs are dropped when embedding is off).
    pub fn set_embed_worker(&self, embed: Arc<EmbeddingWorker>) {
        *self.embed.lock().unwrap() = Some(embed);
    }

    /// Attach the opt-in error log (Phase 8). Failures are recorded locally.
    pub fn set_error_log(&self, log: Arc<ErrorLog>) {
        *self.error_log.lock().unwrap() = Some(log);
    }

    /// Hot-swap skip rules (settings change); applies from the next job on.
    pub fn set_rules(&self, rules: SkipRules) {
        *self.rules.write().unwrap() = rules;
    }

    pub fn enqueue(&self, job: ExtractionJob) {
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
            // Drop if the channel is full (backpressure: ignore overflow jobs).
            let _ = tx.send(job);
        }
    }

    pub fn stats(&self) -> ExtractionStats {
        self.shared.stats.lock().unwrap().clone()
    }

    pub fn set_throttle_ms(&self, ms: u64) {
        self.shared.throttle_ms.store(ms, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        // Drop sender so the loop exits after draining.
        *self.tx.lock().unwrap() = None;
    }

    pub fn store(&self) -> Arc<Mutex<ContentStore>> {
        Arc::clone(&self.store)
    }
}

impl Drop for ExtractionWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn worker_loop(
    rx: Receiver<ExtractionJob>,
    shared: Arc<Shared>,
    store: Arc<Mutex<ContentStore>>,
    rules: Arc<RwLock<SkipRules>>,
    worker: Arc<ExtractionWorker>,
) {
    while !shared.stop.load(Ordering::Relaxed) {
        let job = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(j) => j,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Idle: clear running flag so UI can show "caught up".
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

        let snapshot = rules.read().map(|r| r.clone()).unwrap_or_default();
        process_one(&job, &store, &snapshot, &shared, &worker);

        let throttle = shared.throttle_ms.load(Ordering::Relaxed);
        if throttle > 0 {
            std::thread::sleep(Duration::from_millis(throttle));
        }
    }

    let mut s = shared.stats.lock().unwrap();
    s.running = false;
}

fn log_failure(worker: &ExtractionWorker, path: &str, msg: &str) {
    if let Ok(guard) = worker.error_log.lock() {
        if let Some(log) = guard.as_ref() {
            log.log("extract", &format!("{path}: {msg}"));
        }
    }
}

fn process_one(
    job: &ExtractionJob,
    store: &Arc<Mutex<ContentStore>>,
    rules: &SkipRules,
    shared: &Arc<Shared>,
    worker: &Arc<ExtractionWorker>,
) {
    let reason = rules.skip_reason(&job.path, job.size, false);
    if let Some(reason) = reason {
        let mut s = shared.stats.lock().unwrap();
        s.skipped += 1;
        // Don't record "too large"/"excluded" as failures — expected skips.
        let _ = reason;
        return;
    }

    match extract_file(&job.path) {
        Ok(extracted) => {
            let text = rules.clamp_text(extracted.text.clone());
            let path_str = job.path.to_string_lossy().into_owned();
            // Hold the store lock only for the upsert (never across a re-lock:
            // std::sync::Mutex is not reentrant and a double lock deadlocks).
            let upsert = {
                let st = store.lock().unwrap();
                st.upsert_ok(
                    &path_str,
                    &extracted,
                    job.size,
                    job.file_modified_ms,
                    text.clone(),
                )
            };
            match upsert {
                Ok(()) => {
                    {
                        let mut s = shared.stats.lock().unwrap();
                        s.succeeded += 1;
                    }
                    // Incremental semantic indexing: extracted text → embed queue.
                    let hook = worker.embed.lock().unwrap();
                    if let Some(h) = hook.as_ref() {
                        if h.enabled() && !text.trim().is_empty() {
                            h.enqueue(EmbedJob {
                                path: path_str,
                                text,
                            });
                        }
                    }
                }
                Err(e) => {
                    {
                        let mut s = shared.stats.lock().unwrap();
                        s.failed += 1;
                    }
                    log_failure(worker, &path_str, &e);
                    let st = store.lock().unwrap();
                    let _ = st.record_failure(&path_str, &e);
                }
            }
        }
        Err(ExtractionError::Unsupported(_)) | Err(ExtractionError::Skipped(_)) => {
            let mut s = shared.stats.lock().unwrap();
            s.skipped += 1;
        }
        Err(e) => {
            let msg = e.to_string();
            let path_str = job.path.to_string_lossy().into_owned();
            log_failure(worker, &path_str, &msg);
            {
                let st = store.lock().unwrap();
                let _ = st.record_failure(&path_str, &msg);
            }
            let mut s = shared.stats.lock().unwrap();
            s.failed += 1;
        }
    }
}

/// Convenience: enqueue a path if it passes skip rules (size read from disk).
pub fn try_enqueue_path(worker: &ExtractionWorker, path: PathBuf) {
    let meta = match std::fs::metadata(&path) {
        Ok(m) if m.is_file() => m,
        _ => return,
    };
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    worker.enqueue(ExtractionJob {
        path,
        size: meta.len(),
        file_modified_ms: modified_ms,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Instant;

    #[test]
    fn worker_extracts_text_file() {
        let dir = std::env::temp_dir().join(format!("tpt-worker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("note.txt");
        {
            let mut f = std::fs::File::create(&file).unwrap();
            write!(f, "alpha bravo charlie extraction test").unwrap();
        }

        let store = ContentStore::open(&dir).unwrap();
        // Don't use SkipRules::default() — temp paths hit the AppData\Local\Temp exclude.
        let rules = SkipRules {
            exclude_path_contains: vec![],
            ..SkipRules::default()
        };
        let worker = ExtractionWorker::spawn(store, rules);
        try_enqueue_path(&worker, file.clone());

        // Wait for the worker to drain.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let s = worker.stats();
            if s.succeeded >= 1 || Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let s = worker.stats();
        assert!(s.succeeded >= 1, "stats: {s:?}");
        assert!(worker
            .store()
            .lock()
            .unwrap()
            .has_content(&file.to_string_lossy())
            .unwrap());

        worker.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_excluded_early() {
        let dir = std::env::temp_dir().join(format!("tpt-worker-skip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("big.png");
        std::fs::write(&file, vec![0u8; 16]).unwrap();

        let store = ContentStore::open(&dir).unwrap();
        let worker = ExtractionWorker::spawn(store, SkipRules::default());
        try_enqueue_path(&worker, file);
        std::thread::sleep(Duration::from_millis(400));
        let s = worker.stats();
        assert!(s.skipped >= 1, "stats: {s:?}");
        worker.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_success_feeds_embed_hook() {
        use crate::semantic::{EmbedConfig, EmbeddingWorker, VectorStore};

        let dir = std::env::temp_dir().join(format!("tpt-worker-hook-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("hook.txt");
        std::fs::write(&file, "semantic pipeline hook test text").unwrap();

        let store = ContentStore::open(&dir).unwrap();
        let rules = SkipRules {
            exclude_path_contains: vec![],
            ..SkipRules::default()
        };
        let worker = ExtractionWorker::spawn(store, rules);

        // Embed worker enabled, but pointed at an unreachable loopback port so
        // the request fails fast and deterministically (no Ollama needed).
        let vstore = VectorStore::open_file(dir.join("vectors.db")).unwrap();
        let embed = EmbeddingWorker::spawn(vstore, EmbedConfig::default(), true);
        embed.set_base_url("http://127.0.0.1:59997");
        worker.set_embed_worker(Arc::clone(&embed));

        try_enqueue_path(&worker, file);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (c, e) = (worker.stats(), embed.stats());
            let embed_done = e.failed >= 1 || e.succeeded >= 1;
            if (c.succeeded >= 1 && embed_done) || Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let c = worker.stats();
        let e = embed.stats();
        assert!(c.succeeded >= 1, "content: {c:?}");
        assert!(
            e.processed >= 1 && e.failed >= 1,
            "embed hook should have drained the job (fast-fail): {e:?}"
        );

        embed.stop();
        worker.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_embed_hook_drops_jobs_early() {
        use crate::semantic::{EmbedConfig, EmbeddingWorker, VectorStore};

        let dir = std::env::temp_dir().join(format!("tpt-worker-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("off.txt");
        std::fs::write(&file, "semantic disabled no embed").unwrap();

        let store = ContentStore::open(&dir).unwrap();
        let rules = SkipRules {
            exclude_path_contains: vec![],
            ..SkipRules::default()
        };
        let worker = ExtractionWorker::spawn(store, rules);

        let vstore = VectorStore::open_file(dir.join("vectors.db")).unwrap();
        let embed = EmbeddingWorker::spawn(vstore, EmbedConfig::default(), false);
        worker.set_embed_worker(Arc::clone(&embed));

        try_enqueue_path(&worker, file);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if worker.stats().succeeded >= 1 || Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(worker.stats().succeeded >= 1);
        // Disabled → no job ever reaches the embed queue.
        assert_eq!(embed.stats().processed, 0);

        embed.stop();
        worker.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
