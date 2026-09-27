# Changelog

All notable changes to tpt-finder are documented here. Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning is [SemVer](https://semver.org/):

- **MAJOR** — breaking changes (index format resets, IPC removals, config incompatibilities)
- **MINOR** — new features, backward-compatible
- **PATCH** — bug fixes and performance work without API changes

## [1.0.0] – 2026-09-25

First stable release.

### Added
- Light / dark / system theme support (Settings → Theme), applied live to popup and main window.
- Result view options: toggle Path / Size / Modified columns, pick default sort + direction, compact row density (persisted locally).
- Indexing exclusion rules UI (extensions, folders, max file size, extraction throttle) applied live to the extraction pipeline.
- Opt-in local error log (`error.log` in the app data dir; off by default, deleted when disabled, never uploaded).
- First-run onboarding panel with live index-build progress and Ollama setup guidance.
- Accessibility pass: listbox `aria-activedescendant` selection model, labeled controls, visible focus rings, `prefers-reduced-motion` support.
- Cross-platform memory probe (`perf::rss_bytes` / `peak_rss_bytes`) and extended `index-bench` covering persistence round-trip (cold start), incremental update latency, RSS-per-entry footprint, and extraction pipeline throughput.

### Changed
- Extraction skip rules now derive from settings (`SkipRules::from_settings`) and hot-swap when settings change.
- Per-column metadata spans in result rows (enables column toggles).

### Security
- All file actions validate paths before acting: absolute, NUL-free, and never a filesystem root (see [docs/security-review.md](docs/security-review.md)).
- Ollama URL is validated (loopback-only) in `set_settings` before it can be persisted.

## [0.1.0] – 2026-09-25

Initial development releases of the phased roadmap (Phases 0–5):

- Core indexing engine: USN Journal (Windows) + portable walk backend, in-memory index, multi-volume, persistence, live incremental updates.
- Search UI: global-hotkey popup overlay + main window, shared result list, result actions (open / reveal / copy / recycle), preview pane.
- Query syntax: `ext:`, `size:`, `dated:`, `path:`, `fuzzy:`/`~`, `is:dir`/`is:file`.
- Content extraction pipeline: text/code/PDF/Office → SQLite FTS5 with background worker, skip rules, failure tracking.
- Semantic layer (opt-in): Ollama detection, chunked embeddings, vector store, NL query parsing, hybrid ranking, graceful degradation.
- Linux support: walk + inotify backend, packaging targets.
- CI (Windows + Linux), lint/format tooling (clippy, rustfmt, Biome), dual MIT/Apache-2.0 licensing.
