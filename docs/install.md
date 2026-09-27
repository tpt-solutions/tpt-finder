# Installing & building tpt-finder

## From source

Prerequisites:

- [Rust](https://rustup.rs/) stable (MSVC toolchain on Windows)
- [Node.js](https://nodejs.org/) 20+
- [Tauri 2 platform prerequisites](https://v2.tauri.app/start/prerequisites/):
  - **Windows**: WebView2 Runtime (preinstalled on Windows 10/11)
  - **Linux**: `webkit2gtk-4.1`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev` (Debian/Ubuntu names)

```bash
npm install
npm run tauri build        # installers land in src-tauri/target/release/bundle/
```

Build outputs:

| Platform | Artifacts |
| --- | --- |
| Windows | NSIS `.exe` installer + portable binary (Tauri bundler) |
| Linux | `.deb`, `.rpm`, AppImage |
| macOS | not yet supported (backlog) |

### Development

```bash
npm run tauri dev          # hot-reload Tauri app
npm run dev                # frontend only (Vite)
cd src-tauri && cargo test # engine test suite
```

## First run

1. tpt-finder builds its index by walking every fixed drive once (subsequent launches restore the persisted index in milliseconds and catch up via USN Journal / inotify).
2. Press the global hotkey (**Ctrl+Space** by default) anywhere to open the quick-search popup; **Enter** opens the selection, **Esc** dismisses.
3. The main window offers a sortable results table, preview pane, settings, and view options.

## Index & data locations

| Data | Location (Windows) | Location (Linux) |
| --- | --- | --- |
| Index snapshot | `%APPDATA%\com.tptsolutions.tptfinder\index.bin` | `~/.local/share/com.tptsolutions.tptfinder/index.bin` |
| Extracted text (SQLite FTS5) | same directory, `content.db` | same directory |
| Embedding vectors | same directory, vector store DB | same directory |
| Settings | same directory, `settings.json` | same directory |
| Error log (opt-in) | same directory, `error.log` | same directory |

Deleting `index.bin` forces a full re-index on next start.

## Windows code signing

Release installers are currently **unsigned**. Windows SmartScreen may warn on first run; choose *More info → Run anyway*, or build from source. Signed builds are a post-v1 consideration (requires an org code-signing certificate).
