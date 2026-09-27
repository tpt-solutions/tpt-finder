# tpt-finder

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![CI](https://github.com/tpt-solutions/tpt-finder/actions/workflows/ci.yml/badge.svg)](https://github.com/tpt-solutions/tpt-finder/actions/workflows/ci.yml)

**Instant file search with optional semantic understanding.**

A modern [Everything](https://www.voidtools.com/) / [PowerToys Run](https://learn.microsoft.com/en-us/windows/powertoys/run)-style file search and launcher for **Windows**, **Linux**, and **Archon**.

- **Rust engine** — USN Journal on Windows, `inotify` on Linux, in-memory index for instant prefix/substring/fuzzy matching.
- **Tauri UI** — keyboard-first global-hotkey popup overlay + persistent main window.
- **Optional local semantic layer** — [Ollama](https://ollama.com)-powered natural-language queries like *"the PDF invoice from John last March"*, with all inference on `localhost` and no cloud calls.
- **Content extraction** — plain text, code, PDF, and Office documents indexed for full-text and semantic search.

Part of the [TPT Solutions](https://github.com/tpt-solutions) ecosystem. Archon support integrates with [tpt-archon-relational](https://github.com/tpt-solutions/tpt-archon) for SQL-style filesystem queries (`SELECT path FROM files WHERE content SIMILAR TO 'invoice'`) with zero-copy memory access.

## Scope

File search + launch only — no general app-launcher/plugin system (calculator, web search, etc.). See [`todo.md`](todo.md) for the full roadmap.

## Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) 20+
- Platform tooling for [Tauri 2](https://v2.tauri.app/start/prerequisites/):
  - **Windows**: WebView2 (usually preinstalled), MSVC toolchain
  - **Linux**: `webkit2gtk-4.1`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev` (Debian/Ubuntu package names)
- Optional for semantic search: [Ollama](https://ollama.com) running locally on `localhost:11434`

## Build

```bash
# Install JS dependencies
npm install

# Development (hot-reload frontend + Rust backend)
npm run tauri dev

# Production build
npm run tauri build
```

### Frontend only

```bash
npm run dev        # Vite dev server
npm run build      # Type-check + production bundle
```

### Rust engine only

```bash
cd src-tauri
cargo build
cargo test
```

## Lint & format

```bash
# Rust
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings

# TypeScript / frontend
npm run lint
npm run format:check
npm run typecheck
```

## Project layout

```
src/                 # Plain TypeScript + HTML frontend (Vite), no framework
src-tauri/           # Rust backend (indexing engine, IPC commands, Tauri host)
  src/
    main.rs          # Tauri entrypoint
    lib.rs           # Command registration / app setup
    index/           # Core indexing engine (Phase 1+)
todo.md              # Phased roadmap / checklist
spec.txt             # Original concept brief
```

## Query syntax

Everything-style filters, wired to the Rust engine over IPC:

| Filter       | Example                      |
| ------------ | ---------------------------- |
| `ext:`       | `ext:pdf report`             |
| `size:`      | `size:>10MB`                 |
| `dated:`     | `dated:2026-03`              |
| `path:`      | `path:Documents\invoices`    |
| `fuzzy:`/`~` | `~invc`                      |
| `is:dir`     | `downloads is:dir`           |

See the full [query syntax reference](docs/query-syntax.md). Natural-language queries are parsed via local Ollama into structured filters + semantic terms when the semantic layer is enabled.

## Documentation

| Guide | Contents |
| --- | --- |
| [Install & build](docs/install.md) | prerequisites, build outputs, data locations, first run |
| [Query syntax](docs/query-syntax.md) | filters, operators, ranking, natural language |
| [Ollama setup](docs/ollama-setup.md) | enabling local semantic search |
| [Keybindings](docs/keybindings.md) | global hotkey, popup and main window keys |
| [Security review](docs/security-review.md) | threat model notes and hardening applied for v1 |

## Privacy

- Index and extracted content stay on-device.
- Semantic search uses only local Ollama (`localhost:11434`); non-loopback URLs are rejected by the client and by settings validation.
- No telemetry, no cloud inference; the error log is opt-in and local-only.

## Platform status

| Platform | Status |
| -------- | ------ |
| Windows  | Primary target — USN Journal engine, installer builds |
| Linux    | Supported — walk + inotify backend, AppImage/.deb/.rpm (X11/Wayland QA pending) |
| Archon   | Research-gated, blocked pending [`tpt-archon-relational`](https://github.com/tpt-solutions/tpt-archon) public API |

## Contributing

SPDX dual-license header conventions and development workflow are documented in [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under either of:

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option. See [NOTICE](NOTICE) for attribution requirements under the Apache license.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional terms or conditions.
