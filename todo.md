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

- [ ] Init git repo, `.gitignore` (Rust + Node/Tauri targets)
- [ ] Add `LICENSE-MIT` and `LICENSE-APACHE` (dual license, TPT Solutions copyright)
- [ ] Add SPDX dual-license header/notice convention for source files
- [ ] `README.md` with project description (from spec.txt), build instructions, license badges
- [ ] Scaffold Tauri app (`src-tauri/` Rust backend, plain TS/HTML `src/` frontend, Vite)
- [ ] Configure `tauri.conf.json` (app id, window defaults for popup overlay + main window, bundle identifiers for tpt-finder)
- [ ] Set up TypeScript config (tsconfig, no framework, module bundler)
- [ ] Set up linting/formatting (rustfmt + clippy, eslint/prettier or biome for TS)
- [ ] Set up CI pipeline (build + lint on push, Windows + Linux runners)
- [ ] Decide repo hosting under `tpt-solutions` GitHub org, link to `tpt-archon-relational`

## Phase 1 — Core Indexing Engine (Windows MVP)

- [ ] Design core index data model (path, name, size, dates, attributes, volume) shared across backends
- [ ] Design filesystem abstraction trait (so Windows/Linux/Archon backends share an interface)
- [ ] USN Journal reader: initial full volume enumeration (MFT), then live journal tail for incremental updates
- [ ] In-memory index structure for instant prefix/substring/fuzzy matching (Everything-style performance bar)
- [ ] Multi-volume support (enumerate all NTFS volumes)
- [ ] Index persistence to disk for fast cold-start (avoid full rescan every launch)
- [ ] IPC command/event layer between Rust engine and TS frontend
- [ ] Benchmark against large volumes (10M+ files) baseline

## Phase 2 — Search UI (Popup + Main Window)

- [ ] Global hotkey registration (configurable, e.g. Ctrl+Space) to summon popup overlay
- [ ] Popup overlay UI: single input, instant results list, keyboard-only navigation (arrows/enter/esc), dismiss-on-blur
- [ ] Persistent main window UI: search bar + sortable results table (name, path, size, modified, type)
- [ ] Shared search/result-list component reused between popup and main window
- [ ] Result actions: open, open containing folder, copy path, copy file, delete (to recycle bin)
- [ ] Basic result preview (file info, type icon/thumbnail)
- [ ] Instant as-you-type search wired to Rust engine via IPC
- [ ] Query syntax: basic filters (`ext:`, `size:`, `dated:`, `path:`) similar to Everything
- [ ] Settings: hotkey customization, popup vs main-window behavior

## Phase 3 — Content Extraction Pipeline

- [ ] Design extraction trait (file type → plain text + metadata)
- [ ] Plain text/code file extraction (direct read)
- [ ] PDF text extraction (e.g. `pdf-extract`/poppler bindings)
- [ ] Office document extraction (docx/xlsx/pptx text + author/created-by metadata)
- [ ] Background extraction worker queue (incremental, watches new/changed files, rate-limited)
- [ ] Extracted text + metadata storage (SQLite + FTS5) alongside the file index
- [ ] Skip rules for binaries/large files, size and type exclusion config
- [ ] Extraction failure handling, retry/skip list

## Phase 4 — Semantic Layer (Ollama Integration)

- [ ] Detect local Ollama install (`localhost:11434`), model availability check
- [ ] Embedding pipeline: chunk extracted text, call Ollama embedding endpoint, store vectors
- [ ] Vector storage/index (`sqlite-vec` or embedded HNSW crate) alongside SQLite metadata store
- [ ] Natural-language query parsing via local Ollama chat model → structured filters (type/date/author) + semantic terms
- [ ] Hybrid ranking: structured filters + vector similarity + filename/full-text relevance
- [ ] Background embedding worker (incremental, throttled to avoid saturating CPU/GPU)
- [ ] Settings: enable/disable semantic layer, model selection, resource limits
- [ ] Graceful degradation when Ollama isn't installed/running (feature disabled, standard search still works)
- [ ] Privacy check: confirm no network calls beyond local Ollama; all data stays on-device

## Phase 5 — Linux Support

- [ ] Implement filesystem abstraction trait for Linux
- [ ] Initial full-volume scan via `readdir` (no USN Journal equivalent on Linux)
- [ ] Live updates via `inotify` (recursive watch, handle watch-limit exhaustion on large trees)
- [ ] Index persistence reload (shared format with Windows where possible)
- [ ] Verify Phase 2 UI (hotkey + popup + main window) on X11 and Wayland
- [ ] Verify Phase 3/4 extraction + semantic layer on Linux (Ollama detection too)
- [ ] Linux packaging (AppImage/.deb/.rpm via Tauri bundler)
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

- [ ] Stress test against volumes with 10M+ files
- [ ] Memory profiling (Rust engine + WebView frontend + vector index)
- [ ] Cold start time optimization (index load from disk, journal catch-up)
- [ ] Incremental update latency measurement (USN Journal/inotify → UI refresh)
- [ ] Extraction/embedding pipeline throughput tuning (avoid disk/CPU thrash during initial bulk indexing)

## Phase 8 — Polish & Settings

- [ ] Light/dark theme support
- [ ] Result list customization (columns, sort, icon/thumbnail size)
- [ ] Exclusion rules UI (folders/extensions to skip indexing)
- [ ] Accessibility pass (keyboard-only navigation, screen reader labels)
- [ ] App icon, branding assets (TPT Solutions)
- [ ] Crash reporting/local error logging (opt-in only, consistent with local-first privacy stance)
- [ ] First-run onboarding (initial index build progress, Ollama setup guidance)

## Phase 9 — Testing & QA

- [ ] Unit tests: index engine (Windows USN + Linux scan/inotify)
- [ ] Unit tests: content extraction pipeline (per file type)
- [ ] Unit tests: semantic layer (mocked Ollama responses, hybrid ranking)
- [ ] Integration tests: end-to-end query → results across engine + UI
- [ ] Manual QA pass on Windows
- [ ] Manual QA pass on Linux (X11 + Wayland)
- [ ] Manual QA pass on Archon (once unblocked)
- [ ] Security review (path traversal, IPC boundary, indexing scope limits, Ollama request handling)

## Phase 10 — Release v1

- [ ] Finalize versioning scheme (SemVer) and changelog process
- [ ] Windows installer/bundle (MSI/NSIS via Tauri bundler), code signing decision
- [ ] Linux bundles published (AppImage/.deb/.rpm)
- [ ] Publish LICENSE files + notices in release artifacts
- [ ] User-facing docs (install guide, query syntax reference, Ollama setup guide, keybindings)
- [ ] Tag v1.0.0 release, publish to GitHub (tpt-solutions org)
- [ ] Post-release triage process for bug reports

---

## Backlog / Future Phases (post-v1)

- [ ] OCR for scanned/image-based PDFs
- [ ] Email (.eml/.msg) content + metadata extraction as a first-class source type
- [ ] Cloud storage backend adapters (OneDrive/Google Drive/S3)
- [ ] Network/remote (SFTP/SMB) volume indexing
- [ ] macOS support
- [ ] General app-launcher/plugin system (calculator, web search, shell commands) — deferred, out of scope for v1
