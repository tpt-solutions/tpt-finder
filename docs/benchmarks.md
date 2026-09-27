# Benchmarks (Phase 7)

Methodology: `src-tauri/src/bin/index_bench.rs` generates synthetic entries (1000 folders, mixed extensions, realistic name shapes), bulk-loads the in-memory index, and measures each phase. Release build, Windows 11 x64, 16 GB RAM, NVMe SSD (2026-09-25):

```sh
cargo build --release --bin index_bench
./target/release/index_bench 1000000      # or 10000000; --no-extract skips extraction
```

## Results

| Metric | 1M entries | 10M entries |
| --- | --- | --- |
| Bulk load throughput | 1.4M entries/s (0.74 s) | 0.6M entries/s (18.0 s) |
| Live index footprint | ~566 MB (**~303 bytes/entry**) | ~5.2 GB (**~258 bytes/entry**) |
| Save (persistence) | 486 ms → 159 MB file | 18.0 s → 1.62 GB file |
| Cold-start load + restore | 260 ms + 671 ms | 5.1 s + 16.8 s |
| Incremental upsert | ~0.6 µs/entry | ~15 µs/entry (log-depth path edits) |
| Incremental remove | ~0.3 µs/entry | ~1.1 µs/entry |
| Extraction pipeline (10 KB text files, FTS5 + SQLite WAL) | **522 files/s** | 524 files/s |

### Query latency (avg over iterations, limit 200)

| Query | @ 1M | @ 10M |
| --- | --- | --- |
| `ext:pdf` | 24 ms | 279 ms |
| `path:vol42` | 119 ms | 2.5 s |
| prefix name (broad) | 236 ms | 4.8 s |
| exact-ish name | 157 ms | 2.9 s |
| substring (broad) | 286 ms | 5.9 s |
| multi-term AND | 733 ms | 13.5 s |
| `count_prefix` binary-search helper | **< 1 µs** | **3 µs** |

## Findings & tuning applied

1. **Extraction throughput — fixed (9×)**: the content store originally ran SQLite with default journal settings; every extracted file paid a rollback-journal commit (~17 ms). Switching to `journal_mode=WAL` + `synchronous=NORMAL` (crash-safe for rebuildable index data) took 300 × 10 KB files from **58 files/s to 522 files/s** (flat from 1M to 10M indexed entries). The vector store gets the same treatment.
2. **Walk enumeration speed — fixed**: `WalkBackend::scan_dir` opened an extra file handle per entry solely to fetch the Windows FRN (`platform_file_id`), which the walk path never consumes (the USN watch maintains its own FRN map). Dropping that per-entry handle open turns a full-volume walk from hours into minutes; real-machine cold start now walks all five local volumes in a few minutes and persists the result.
3. **Memory**: the live index costs ~260–300 bytes/entry — a 10M-file volume indexes in ~5 GB RAM with a 1.6 GB persisted snapshot. Cold start at 10M (load + map rebuild) is ~22 s, still far cheaper than a full rescan; USN journal checkpoints skip catch-up on Windows.
4. **Incremental updates stay sub-16 µs/entry even at 10M entries** — live UI refresh latency is dominated by IPC + render, not the index mutation.
5. **Known optimization opportunity (top post-v1 candidate)**: text queries scan + score all matches (O(index size)), so at 10M entries broad queries take seconds. The `count_prefix` binary-search helper answers in 3 µs at 10M — routing prefix/exact queries through the sorted-name index with early termination at the limit is the clear next engine optimization.

## Reproducing

The bench prints phase-by-phase numbers and is deterministic apart from IO speed. `--no-extract` skips the extraction-throughput phase when you only want index numbers. Embedding throughput depends on your GPU/model and is throttled by design (`embed_throttle_ms`) — measure it via the semantic status counters in the app while Ollama is running.
