// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Baseline search + scale benchmarks against synthetic volumes (Phase 7).
//!
//! Run (debug is too slow for 10M; use release):
//! ```sh
//! cargo run --release --bin index-bench -- 1000000
//! cargo run --release --bin index-bench -- 10000000
//! ```
//! Phases: bulk-load throughput, query latencies, cold-start persistence
//! round-trip, incremental-update latency, memory footprint, and (small)
//! content-extraction pipeline throughput. `--no-extract` skips the last.

use std::io::Write as _;
use std::time::Instant;

use tpt_finder_lib::index::model::{FileEntry, VolumeInfo};
use tpt_finder_lib::index::persistence::Persistence;
use tpt_finder_lib::index::query::ParsedQuery;
use tpt_finder_lib::index::store::IndexStore;
use tpt_finder_lib::perf;

fn synth_entry(i: usize) -> FileEntry {
    let folder = i % 1000;
    let is_dir = i % 50 == 0;
    let path = if is_dir {
        format!("/data/vol{folder}/dir_{folder}/folder_{i}")
    } else {
        format!(
            "/data/vol{}/dir_{folder}/file_{i}_{}.{}",
            folder,
            "alpha beta gamma delta epsilon"
                .split(' ')
                .nth(i % 5)
                .unwrap_or("x"),
            ["txt", "pdf", "png", "rs", "md"][i % 5]
        )
    };
    FileEntry::from_meta(
        0,
        path,
        (i as u64).wrapping_mul(137) % 50_000_000,
        1_600_000_000_000 + i as i64,
        1_700_000_000_000 + (i as i64 % 100_000),
        0,
        if is_dir { 0x10 } else { 0x20 },
        0,
        is_dir,
        0,
    )
}

fn time_query(store: &mut IndexStore, label: &str, q: &str, iters: usize) {
    let parsed = ParsedQuery::parse(q);
    // Warmup
    let _ = store.search(&parsed, 200);
    let start = Instant::now();
    let mut total = 0usize;
    for _ in 0..iters {
        let r = store.search(&parsed, 200);
        total += r.total_matched;
    }
    let avg_us = start.elapsed().as_micros() / iters.max(1) as u128;
    println!("  {label:28} avg {avg_us:>8} µs  (matches≈{total})");
}

fn bench_cold_start(store: &IndexStore, dir: &std::path::Path) {
    println!("\ncold start (persistence round-trip):");
    let vols = vec![VolumeInfo {
        id: 1,
        root: "/data".into(),
        fs: "synthetic".into(),
        total_bytes: 0,
        free_bytes: 0,
        live_updates: false,
    }];
    let persistence = Persistence::new(dir);
    let start = Instant::now();
    let save_result = persistence.save(store, &vols, "walk", &[]);
    let save_dt = start.elapsed();
    match save_result {
        Ok(()) => {
            let bytes = std::fs::metadata(dir.join("index.bin"))
                .or_else(|_| std::fs::metadata(dir.join("index.snapshot")))
                .map(|m| m.len())
                .unwrap_or(0);
            println!(
                "  save: {:>10.1} ms   file {:.1} MB",
                save_dt.as_secs_f64() * 1e3,
                bytes as f64 / 1e6
            );

            let start = Instant::now();
            let loaded = persistence.load().ok().flatten();
            let load_dt = start.elapsed();
            match loaded {
                Some(snap) => {
                    let start = Instant::now();
                    let restored = Persistence::restore_store(&snap);
                    let restore_dt = start.elapsed();
                    println!(
                        "  load: {:>10.1} ms   restore {} entries in {:.1} ms",
                        load_dt.as_secs_f64() * 1e3,
                        restored.len(),
                        restore_dt.as_secs_f64() * 1e3
                    );
                }
                None => println!("  load: no snapshot found"),
            }
        }
        Err(e) => println!("  save failed: {e}"),
    }
}

fn bench_incremental(entries: &[FileEntry]) {
    println!("\nincremental update latency:");
    let mut store = IndexStore::new();
    // Start from a smaller base so the workload is dominated by updates.
    let base: Vec<FileEntry> = entries
        .iter()
        .take(100_000.min(entries.len()))
        .cloned()
        .collect();
    store.bulk_load(base);

    let batch: Vec<FileEntry> = (0..10_000)
        .map(|i| {
            let mut e = entries[i % entries.len()].clone();
            e.path = format!("/live/change_{}.txt", i);
            e.name = format!("change_{i}.txt");
            e.name_lower = e.name.to_ascii_lowercase();
            e.is_dir = false;
            e
        })
        .collect();

    let start = Instant::now();
    for e in &batch {
        store.upsert(e.clone());
    }
    let dt = start.elapsed();
    println!(
        "  upsert avg {:>6.0} ns/entry  (10k batch in {:.1} ms)",
        dt.as_nanos() / batch.len() as u128,
        dt.as_secs_f64() * 1e3
    );

    let start = Instant::now();
    for e in &batch {
        store.remove_path(&e.path);
    }
    let dt = start.elapsed();
    println!(
        "  remove avg {:>6.0} ns/entry  (10k batch in {:.1} ms)",
        dt.as_nanos() / batch.len() as u128,
        dt.as_secs_f64() * 1e3
    );
}

/// Small-scale content extraction throughput (no Ollama needed).
fn bench_extraction() {
    println!("\nextraction pipeline throughput:");
    let dir = std::env::temp_dir().join(format!("tpt-bench-extract-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let n_files = 300usize;
    let body = "lorem ipsum dolor sit amet ".repeat(400); // ~10 KB
    for i in 0..n_files {
        let mut f = std::fs::File::create(dir.join(format!("doc_{i}.txt"))).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    let store = match tpt_finder_lib::content::ContentStore::open(&dir) {
        Ok(s) => s,
        Err(e) => {
            println!("  content store unavailable: {e}");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
    };
    let rules = tpt_finder_lib::content::SkipRules {
        exclude_path_contains: vec![],
        ..tpt_finder_lib::content::SkipRules::default()
    };
    let worker = tpt_finder_lib::content::ExtractionWorker::spawn(store, rules);

    let start = Instant::now();
    for i in 0..n_files {
        worker.enqueue(tpt_finder_lib::content::ExtractionJob {
            path: dir.join(format!("doc_{i}.txt")),
            size: body.len() as u64,
            file_modified_ms: 0,
        });
    }
    let deadline = start + std::time::Duration::from_secs(120);
    loop {
        let s = worker.stats();
        if s.succeeded + s.failed >= n_files as u64 || Instant::now() > deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let dt = start.elapsed();
    let s = worker.stats();
    println!(
        "  {} files (~{} KB each): {:?}  → {:.0} files/s (ok={} failed={})",
        n_files,
        body.len() / 1024,
        dt,
        n_files as f64 / dt.as_secs_f64(),
        s.succeeded,
        s.failed
    );

    worker.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let skip_extract = args.iter().any(|a| a == "--no-extract");

    println!("== tpt-finder index bench ==");
    println!(
        "process RSS at start: {:.1} MB",
        perf::rss_bytes() as f64 / 1e6
    );
    println!("generating {n} synthetic entries…");
    let gen_start = Instant::now();
    let entries: Vec<FileEntry> = (0..n).map(synth_entry).collect();
    println!("  generated in {:?}", gen_start.elapsed());
    let rss_raw = perf::rss_bytes();

    // Sample for the incremental bench up front so the generator can drop.
    let incremental_sample: Vec<FileEntry> = entries.iter().take(10_000).cloned().collect();

    let mut store = IndexStore::new();
    let insert_start = Instant::now();
    store.bulk_load(entries.clone());
    let insert_dt = insert_start.elapsed();
    println!(
        "bulk_load {n} entries in {:?}  ({:.1}M entries/s)",
        insert_dt,
        n as f64 / insert_dt.as_secs_f64() / 1e6
    );
    println!("  files={} dirs={}", store.file_count(), store.dir_count());
    println!(
        "  RSS with live index + generator vec: {:.1} MB (peak {:.1} MB)",
        perf::rss_bytes() as f64 / 1e6,
        perf::peak_rss_bytes() as f64 / 1e6
    );
    drop(entries);
    println!(
        "  RSS with live index only: {:.1} MB  ≈ {:.0} bytes/entry",
        perf::rss_bytes() as f64 / 1e6,
        perf::rss_bytes().saturating_sub(rss_raw) as f64 / n.max(1) as f64
    );

    let iters = if n >= 5_000_000 { 5 } else { 20 };
    println!("\nqueries ({iters} iterations each):");
    time_query(&mut store, "prefix name", "file_1", iters);
    time_query(&mut store, "exact-ish name", "file_12345", iters);
    time_query(&mut store, "substring", "gamma", iters);
    time_query(&mut store, "multi-term AND", "file alpha", iters);
    time_query(&mut store, "ext: filter", "ext:pdf", iters);
    time_query(&mut store, "path: filter", "path:vol42", iters);
    time_query(&mut store, "prefix count helper", "file_99", iters);

    // Prefix helper timing (sorted names binary search only).
    let start = Instant::now();
    for _ in 0..iters {
        let _ = store.count_prefix("file_1");
    }
    println!(
        "  {count_prefix_only:28} avg {:>8} µs",
        start.elapsed().as_micros() / iters.max(1) as u128,
        count_prefix_only = "count_prefix only"
    );

    // Phase 7: cold start / persistence.
    let bench_dir = std::env::temp_dir().join(format!("tpt-bench-persist-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&bench_dir);
    bench_cold_start(&store, &bench_dir);
    let _ = std::fs::remove_dir_all(&bench_dir);

    // Phase 7: incremental updates.
    bench_incremental(&incremental_sample);

    // Phase 7: extraction pipeline (skippable for quick runs).
    if !skip_extract {
        bench_extraction();
    }

    println!("\ndone.");
}
