// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! End-to-end integration: filesystem walk → in-memory index → query →
//! persistence round-trip → content extraction → FTS search.
//!
//! Deliberately engine-only (no Tauri runtime): this exercises the exact
//! pipeline the IPC layer fronts, including graceful degrade paths.

use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use tpt_finder_lib::content::{ContentStore, ExtractionJob, ExtractionWorker, SkipRules};
use tpt_finder_lib::index::backend::{FilesystemBackend, ScanProgress, WalkBackend};
use tpt_finder_lib::index::model::{FileEntry, VolumeInfo};
use tpt_finder_lib::index::persistence::Persistence;
use tpt_finder_lib::index::query::ParsedQuery;
use tpt_finder_lib::index::store::IndexStore;

fn make_tree() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tpt-integration-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::create_dir_all(dir.join("media")).unwrap();
    std::fs::write(
        dir.join("readme.txt"),
        b"tpt-finder integration test payload",
    )
    .unwrap();
    std::fs::write(
        dir.join("docs").join("quarterly-report.txt"),
        b"revenue increased by eleven percent quarter over quarter",
    )
    .unwrap();
    std::fs::write(
        dir.join("docs").join("notes.md"),
        b"# Notes\nsemantic search notes",
    )
    .unwrap();
    std::fs::write(dir.join("media").join("logo.png"), [0u8; 64]).unwrap();
    dir
}

fn scan_to_store(dir: &std::path::Path) -> IndexStore {
    let backend = WalkBackend {
        use_platform_watch: false,
        ..WalkBackend::default()
    };
    let vol = VolumeInfo {
        id: 1,
        root: dir.to_string_lossy().into_owned(),
        fs: "test".into(),
        total_bytes: 0,
        free_bytes: 0,
        live_updates: false,
    };
    let (tx, rx) = channel();
    backend.scan(&vol, tx).expect("scan");
    let mut entries: Vec<FileEntry> = Vec::new();
    while let Ok(ev) = rx.recv_timeout(Duration::from_secs(5)) {
        if let ScanProgress::Entry(e) = ev {
            entries.push(e);
        }
    }
    let mut store = IndexStore::new();
    store.bulk_load(entries);
    store
}

#[test]
fn walk_index_search_persist_extract_fts() {
    let tree = make_tree();

    // ── walk → index → query ───────────────────────────────────────────
    let mut store = scan_to_store(&tree);
    assert!(store.file_count() >= 3, "expected ≥3 files");

    let r = store.search(&ParsedQuery::parse("quarterly"), 10);
    assert_eq!(r.total_matched, 1, "prefix search should find report");
    assert!(r.items[0].path.contains("quarterly-report.txt"));

    let r = store.search(&ParsedQuery::parse("ext:md"), 10);
    assert_eq!(r.total_matched, 1, "ext: filter should find notes.md");

    // ── persistence round-trip (cold start) ───────────────────────────
    let persist_dir = tree.join(".persist");
    let p = Persistence::new(&persist_dir);
    p.save(&store, &[], "walk", &[]).expect("save snapshot");
    let snap = p.load().expect("load snapshot").expect("snapshot exists");
    let mut restored = Persistence::restore_store(&snap);
    assert_eq!(restored.len(), store.len(), "round-trip preserves entries");
    let r = restored.search(&ParsedQuery::parse("readme"), 10);
    assert_eq!(r.total_matched, 1);

    // ── extraction → FTS ──────────────────────────────────────────────
    let content_store = ContentStore::open(tree.join(".content")).expect("content store");
    // Temp dirs hit the default \AppData\Local\Temp path exclusion — clear it.
    let rules = SkipRules {
        exclude_path_contains: vec![],
        ..SkipRules::default()
    };
    let worker = ExtractionWorker::spawn(content_store, rules);
    for rel in ["readme.txt", "docs/quarterly-report.txt", "docs/notes.md"] {
        let path = tree.join(rel);
        let meta = std::fs::metadata(&path).unwrap();
        worker.enqueue(ExtractionJob {
            path,
            size: meta.len(),
            file_modified_ms: 0,
        });
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = worker.stats();
        if s.succeeded + s.failed >= 3 || Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let stats = worker.stats();
    assert!(
        stats.succeeded >= 3,
        "all text files should extract, stats: {stats:?}"
    );

    let hits = worker
        .store()
        .lock()
        .unwrap()
        .search_fts("revenue", 10)
        .expect("fts search");
    assert!(
        hits.iter().any(|(p, _)| p.contains("quarterly-report")),
        "FTS should surface the report by content, got: {hits:?}"
    );

    worker.stop();
    let _ = std::fs::remove_dir_all(&tree);
}
