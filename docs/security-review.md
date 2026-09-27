# Security review (v1, Phase 9)

Scope: path handling, IPC boundary, indexing scope, network egress, local data.
Method: manual review of every IPC command surface (`src-tauri/src/ipc.rs`, `actions.rs`, `popup.rs`) plus targeted tests. Findings marked **fixed** were addressed in this release; **accepted** items are deliberate trade-offs.

## Path traversal / dangerous paths

| Surface | Risk | Status |
| --- | --- | --- |
| `open_path` / `reveal_path` / `delete_path` | Relative paths, NUL bytes, or filesystem roots (`C:\`, `/`) reaching file actions | **Fixed** — `actions::validate_action_path` rejects empty/relative/NUL paths and any path whose `Path::parent()` is `None` (drive roots, UNC roots, `/`). Unit-tested (`validate_rejects_bad_paths`). |
| `delete_path` | Deleting a live directory tree | Accepted — action goes to the **recycle bin** (`trash` crate), is confirmed by a `window.confirm` dialog in the UI, and root paths are rejected above. |

The UI only ever sends paths that came from the index (real filesystem entries), and every action re-validates existence before acting.

## IPC boundary

- The WebView can invoke only the commands registered in `tauri::generate_handler!` (allow-list by construction); no arbitrary shell/command execution is exposed.
- CSP is set in `tauri.conf.json`: `default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:` — no remote script origins. The frontend ships no third-party runtime code.
- `copy_file` (Windows) shells out to PowerShell with the path interpolated into a single-quoted string; single quotes are escaped by doubling (`''`), and inside single-quoted PowerShell strings `$`, backticks, and newlines are literal, so no injection vector was found. The helper runs with `-NoProfile -NonInteractive` and `CREATE_NO_WINDOW`.
- Clipboard access is limited to the exact string the user copied (path) or the file the user picked.

## Indexing scope

- The engine walks **fixed volumes only** as enumerated by the platform backend; removable/network drives are skipped at the volume-enumeration layer.
- Live watches are read-only consumers of USN Journal / inotify events; nothing in the engine writes to indexed locations.
- Content **extraction** is bounded by skip rules: per-file size cap (default 16 MB), extension deny-list, path-fragment deny-list (`node_modules`, `.git`, …), directory-name deny-list, and a stored-text cap per file (512 KB). All lists are user-editable in Settings and applied live.

## Network egress / privacy

- The only outbound HTTP client is `semantic::OllamaClient` (ureq). `OllamaClient::new` **rejects non-loopback base URLs** (tested against remote hosts, non-loopback IPv6, and userinfo tricks like `http://user@evil.com`).
- `set_settings` re-validates the Ollama URL server-side before persisting, so a bad URL can never be stored.
- No telemetry, update pings, or analytics of any kind.

## Local data

- Index, content DB, vectors, and settings live in the OS app-data dir with user-level permissions.
- **Error log is opt-in** (`settings.json: error_log_enabled`, default off): when enabled it records one-line, timestamped action/extraction/embedding failures to `error.log` in the app-data dir; turning it off deletes the file. It never contains file contents, only paths and error strings, and never leaves the device.
- Index persistence writes are atomic (`*.tmp` + rename) to avoid corruption on crash.

## WebView content injection

- All DOM insertion of dynamic values uses `textContent` / `createElement`; the two `innerHTML` template literals in the codebase are static strings defined at build time. Result paths are also set as `title` attributes via property assignment (not HTML parsing).

## Known limitations (accepted for v1)

- No executable code-signing yet (installer SmartScreen warning is expected; see [install.md](install.md)).
- `settings.json` is not encrypted — it contains no secrets (no API keys exist in tpt-finder's threat model since all AI is local).
- Windows PowerShell clipboard helper depends on `powershell.exe` being present (fallback: copies the path as text).
