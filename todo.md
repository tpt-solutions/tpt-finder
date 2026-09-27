# tpt-finder — Project Checklist

TPT Solutions — dual-licensed under MIT OR Apache-2.0.
Modern file search & launcher (Everything / PowerToys Run alternative). Rust engine (USN Journal on Windows, inotify on Linux) + Tauri UI, with an optional local semantic search layer powered by Ollama. Windows, Linux, and Archon (via tpt-archon-relational).

Decisions locked in for this roadmap:
- Local AI layer integrates with **Ollama** (not tpt-eve/tpt-anima) — all inference stays on localhost.
- Content extraction/indexing (PDF, Office docs, etc.) is a **core v1 phase**, not backlog — needed for semantic queries like "the PDF invoice from John last March".
- UI ships **both** a global-hotkey popup overlay and a persistent main window.
- Scope is **file search + launch only** — no general app-launcher/plugin system (calculator, web search, etc.).

---

## Phase 0 — Project Setup & Licensing

- [x] Init git repo, `.gitignore` (Rust + Node/Tauri targets)
- [x] Add `LICENSE-MIT` and `LICENSE-APACHE` (dual license, TPT Solutions copyright)
- [x] Add SPDX dual-license header/notice convention for source files
- [x] `README.md` with project description (from spec.txt), build instructions, license badges
- [x] Scaffold Tauri app (`src-tauri/` Rust backend, plain TS/HTML `src/` frontend, Vite)
- [x] Configure `tauri.conf.json` (app id, window defaults for popup overlay + main window, bundle identifiers for tpt-finder)
- [x] Set up TypeScript config (tsconfig, no framework, module bundler)
- [x] Set up linting/formatting (rustfmt + clippy, eslint/prettier or biome for TS)
- [x] Set up CI pipeline (build + lint on push, Windows + Linux runners)
- [x] Decide repo hosting under `tpt-solutions` GitHub org, link to `tpt-archon-relational`

## Phase 1 — Core Indexing Engine (Windows MVP)

- [x] Design core index data model (path, name, size, dates, attributes, volume) shared across backends
- [x] Design filesystem abstraction trait (so Windows/Linux/Archon backends share an interface)
- [x] USN Journal reader: initial full volume enumeration (MFT), then live journal tail for incremental updates
- [x] In-memory index structure for instant prefix/substring/fuzzy matching (Everything-style performance bar)
- [x] Multi-volume support (enumerate all NTFS volumes)
- [x] Index persistence to disk for fast cold-start (avoid full rescan every launch)
- [x] IPC command/event layer between Rust engine and TS frontend
- [x] Benchmark against large volumes (10M+ files) baseline

## Phase 2 — Search UI (Popup + Main Window)

- [x] Global hotkey registration (configurable, e.g. Ctrl+Space) to summon popup overlay
- [x] Popup overlay UI: single input, instant results list, keyboard-only navigation (arrows/enter/esc), dismiss-on-blur
- [x] Persistent main window UI: search bar + sortable results table (name, path, size, modified, type)
- [x] Shared search/result-list component reused between popup and main window
- [x] Result actions: open, open containing folder, copy path, copy file, delete (to recycle bin)
- [x] Basic result preview (file info, type icon/thumbnail)
- [x] Instant as-you-type search wired to Rust engine via IPC
- [x] Query syntax: basic filters (`ext:`, `size:`, `dated:`, `path:`) similar to Everything
- [x] Settings: hotkey customization, popup vs main-window behavior

## Phase 3 — Content Extraction Pipeline

- [x] Design extraction trait (file type → plain text + metadata)
- [x] Plain text/code file extraction (direct read)
- [x] PDF text extraction (e.g. `pdf-extract`/poppler bindings)
- [x] Office document extraction (docx/xlsx/pptx text + author/created-by metadata)
- [x] Background extraction worker queue (incremental, watches new/changed files, rate-limited)
- [x] Extracted text + metadata storage (SQLite + FTS5) alongside the file index
- [x] Skip rules for binaries/large files, size and type exclusion config
- [x] Extraction failure handling, retry/skip list

## Phase 4 — Semantic Layer (Ollama Integration)

- [x] Detect local Ollama install (`localhost:11434`), model availability check
- [x] Embedding pipeline: chunk extracted text, call Ollama embedding endpoint, store vectors
- [x] Vector storage/index (`sqlite-vec` or embedded HNSW crate) alongside SQLite metadata store
- [x] Natural-language query parsing via local Ollama chat model → structured filters (type/date/author) + semantic terms
- [x] Hybrid ranking: structured filters + vector similarity + filename/full-text relevance
- [x] Background embedding worker (incremental, throttled to avoid saturating CPU/GPU)
- [x] Settings: enable/disable semantic layer, model selection, resource limits
- [x] Graceful degradation when Ollama isn't installed/running (feature disabled, standard search still works)
- [x] Privacy check: confirm no network calls beyond local Ollama; all data stays on-device

## Phase 5 — Linux Support

- [x] Implement filesystem abstraction trait for Linux
- [x] Initial full-volume scan via `readdir` (no USN Journal equivalent on Linux)
- [x] Live updates via `inotify` (recursive watch, handle watch-limit exhaustion on large trees)
- [x] Index persistence reload (shared format with Windows where possible)
- [ ] Verify Phase 2 UI (hotkey + popup + main window) on X11 and Wayland
- [x] Verify Phase 3/4 extraction + semantic layer on Linux (Ollama detection too)
- [x] Linux packaging (AppImage/.deb/.rpm via Tauri bundler)
- [ ] Manual QA pass on at least one major distro (X11 + Wayland)

## Phase 6 — Archon Platform Support (research-gated)

- [ ] Track `tpt-archon-relational` API design (github.com/tpt-solutions/tpt-archon)
- [ ] Define/confirm client API contract for SQL-style filesystem queries (`SELECT path FROM files WHERE content SIMILAR TO ...`)
- [ ] Implement Archon backend adapter behind the filesystem abstraction trait (Phase 1)
- [ ] Map semantic "SIMILAR TO" queries to Archon's native content-similarity operator (avoid duplicate local embedding work when Archon backend is active)
- [ ] Zero-copy query execution path validation
- [ ] Auto-detect/select Archon backend vs standard indexing per volume/mount
- [ ] Performance validation on Archon (SQL query latency vs local index)
- [ ] Manual QA pass on Archon environment
- [ ] Status: blocked pending public `tpt-archon-relational` API

## Phase 7 — Performance & Scale Hardening

- [x] Stress test against volumes with 10M+ files (synthetic 10M-entry index run; see docs/benchmarks.md)
- [x] Memory profiling (Rust engine + WebView frontend + vector index) — perf module + per-entry footprint measured
- [x] Cold start time optimization (index load from disk, journal catch-up) — 1M ≈ 0.9 s, 10M ≈ 22 s; save() re-entrancy deadlock fixed
- [x] Incremental update latency measurement (USN Journal/inotify → UI refresh) — sub-16 µs/entry at 10M
- [x] Extraction/embedding pipeline throughput tuning (avoid disk/CPU thrash during initial bulk indexing) — SQLite WAL: 58 → 522 files/s

## Phase 8 — Polish & Settings

- [x] Light/dark theme support (system/light/dark, live-applied to both windows)
- [x] Result list customization (columns, sort, icon/thumbnail size) — column toggles + sort + row density, persisted
- [x] Exclusion rules UI (folders/extensions to skip indexing) — settings-driven skip rules, hot-swapped
- [x] Accessibility pass (keyboard-only navigation, screen reader labels) — listbox ARIA pattern, labels, focus rings, reduced motion
- [x] App icon, branding assets (TPT Solutions) — full icon set (32→512 + multi-res .ico)
- [x] Crash reporting/local error logging (opt-in only, consistent with local-first privacy stance) — local error.log, off by default, deleted when disabled
- [x] First-run onboarding (initial index build progress, Ollama setup guidance)

## Phase 9 — Testing & QA

- [x] Unit tests: index engine (Windows USN + Linux scan/inotify) — 80 lib tests incl. engine/persistence/query/backend
- [x] Unit tests: content extraction pipeline (per file type) — text/code/docx/xlsx/pptx + skip rules + worker
- [x] Unit tests: semantic layer (mocked Ollama responses, hybrid ranking) — loopback validation, unreachable degradation, merge scores
- [x] Integration tests: end-to-end query → results across engine + UI — tests/integration.rs (walk → index → query → persist → extract → FTS)
- [x] Manual QA pass on Windows (release exe smoke test: theme, onboarding, view options, search, settings; found + fixed save() deadlock, empty-snapshot rebuild suppression, startup state race, slow walk)
- [ ] Manual QA pass on Linux (X11 + Wayland)
- [ ] Manual QA pass on Archon (once unblocked)
- [x] Security review (path traversal, IPC boundary, indexing scope limits, Ollama request handling) — docs/security-review.md; path validation + capability tightening applied

## Phase 10 — Release v1

- [x] Finalize versioning scheme (SemVer) and changelog process — CHANGELOG.md + SemVer policy; version 1.0.0
- [x] Windows installer/bundle (MSI/NSIS via Tauri bundler), code signing decision — both built; unsigned for v1 (documented in docs/install.md)
- [ ] Linux bundles published (AppImage/.deb/.rpm) — bundler targets configured; publish happens on release tag
- [x] Publish LICENSE files + notices in release artifacts — bundle.license → NOTICE; LICENSE-MIT/LICENSE-APACHE in repo
- [x] User-facing docs (install guide, query syntax reference, Ollama setup guide, keybindings) — docs/ directory
- [ ] Tag v1.0.0 release, publish to GitHub (tpt-solutions org) — ready; tag/publish is the release action
- [x] Post-release triage process for bug reports — CONTRIBUTING.md "Bug reports & triage"

---

## Backlog / Future Phases (post-v1)

- [ ] OCR for scanned/image-based PDFs
- [ ] Email (.eml/.msg) content + metadata extraction as a first-class source type
- [ ] Cloud storage backend adapters (OneDrive/Google Drive/S3)
- [ ] Network/remote (SFTP/SMB) volume indexing
- [ ] macOS support
- [ ] General app-launcher/plugin system (calculator, web search, shell commands) — deferred, out of scope for v1
