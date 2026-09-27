// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! IPC command layer between the Rust indexing engine and the TS frontend.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::index::{IndexStatus, SearchEngine, SearchResult, SearchResultItem};
use crate::settings::{Settings, SettingsStore};

/// Payload for `index://status` events.
#[derive(Clone, Serialize)]
pub struct StatusEvent {
    pub status: IndexStatus,
}

/// Payload for `index://progress` events during a rebuild.
#[derive(Clone, Serialize)]
pub struct ProgressEvent {
    pub phase: String,
    pub file_count: u64,
    pub dir_count: u64,
}

pub struct AppState {
    pub engine: Arc<SearchEngine>,
    pub settings: Arc<Mutex<Settings>>,
    pub settings_store: Arc<SettingsStore>,
    pub content: Arc<crate::content::ExtractionWorker>,
    pub embed: Arc<crate::semantic::EmbeddingWorker>,
    pub error_log: Arc<crate::logging::ErrorLog>,
}

#[tauri::command]
pub fn ping() -> Result<String, String> {
    Ok(format!("tpt-finder backend v{}", env!("CARGO_PKG_VERSION")))
}

#[tauri::command]
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Instant search against the in-memory index.
#[tauri::command]
pub fn search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<SearchResult, String> {
    let fallback = state.settings.lock().map(|s| s.search_limit).unwrap_or(200);
    let limit = limit.unwrap_or(fallback).clamp(1, 2000);
    Ok(state.engine.search(&query, limit))
}

/// Current index/build status (includes content extraction counters when available).
#[tauri::command]
pub fn index_status(state: State<'_, AppState>, app: AppHandle) -> Result<IndexStatus, String> {
    let mut st = state.engine.status();
    let stats = state.content.stats();
    st.content_queued = stats.queued;
    st.content_extracted = stats.succeeded;
    st.content_failed = stats.failed;
    let _ = app; // reserved for future event fan-out
    Ok(st)
}

/// Trigger a full background rescan + index rebuild.
#[tauri::command]
pub fn rebuild_index(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    state.engine.rebuild_async()?;
    let status = state.engine.status();
    let _ = app.emit("index://status", StatusEvent { status });
    Ok(())
}

/// Persist the index to disk now.
#[tauri::command]
pub fn save_index(state: State<'_, AppState>) -> Result<(), String> {
    state.engine.save()
}

/// Load a previously persisted index (cold start helper / manual refresh).
#[tauri::command]
pub fn load_index(state: State<'_, AppState>, app: AppHandle) -> Result<IndexStatus, String> {
    let _restored = state.engine.try_load_persisted();
    let status = state.engine.status();
    let _ = app.emit(
        "index://status",
        StatusEvent {
            status: status.clone(),
        },
    );
    Ok(status)
}

// ── Settings ──────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    state
        .settings
        .lock()
        .map(|s| s.clone())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_settings(
    state: State<'_, AppState>,
    app: AppHandle,
    settings: Settings,
) -> Result<Settings, String> {
    if settings.hotkey.trim().is_empty() {
        return Err("hotkey must not be empty".into());
    }
    // Validate hotkey syntax before committing.
    use tauri_plugin_global_shortcut::Shortcut;
    let shortcut: Shortcut = settings
        .hotkey
        .parse()
        .map_err(|e| format!("invalid hotkey '{}': {e}", settings.hotkey))?;

    // Reject non-loopback Ollama URLs (privacy: inference stays on-device).
    crate::semantic::OllamaClient::new(&settings.ollama_url)
        .map_err(|e| format!("invalid Ollama URL '{}': {e}", settings.ollama_url))?;

    let settings = settings.normalized();
    state.settings_store.save(&settings)?;
    {
        let mut s = state.settings.lock().map_err(|e| e.to_string())?;
        *s = settings.clone();
    }

    // Re-register global hotkey: drop any prior registration, then bind the new one.
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    if let Err(e) = gs.register(shortcut) {
        return Err(format!("failed to register hotkey: {e}"));
    }

    // Sync semantic worker with the new settings.
    state.embed.set_enabled(settings.semantic_enabled);
    state.embed.set_base_url(&settings.ollama_url);
    state.embed.set_config(crate::semantic::EmbedConfig {
        model: settings.embedding_model.clone(),
        chunk_chars: settings.chunk_chars.clamp(64, 8192),
        max_chunks_per_file: settings.max_chunks_per_file.clamp(1, 256),
        throttle_ms: settings.embed_throttle_ms.min(5_000),
    });

    // Sync extraction pipeline: skip rules + throttle + error log (Phase 8).
    state
        .content
        .set_rules(crate::content::SkipRules::from_settings(&settings));
    state
        .content
        .set_throttle_ms(settings.extraction_throttle_ms.min(5_000));
    state.error_log.set_enabled(settings.error_log_enabled);
    if !settings.error_log_enabled {
        state.error_log.clear();
    }

    // Frontend applies theme / first-run state off this event.
    let _ = app.emit("settings://changed", settings.clone());
    Ok(settings)
}

// ── Popup window control ──────────────────────────────────────────────

/// Show and focus the popup overlay (used by hotkey / tests).
#[tauri::command]
pub fn show_popup(app: AppHandle) -> Result<(), String> {
    crate::popup::show_popup(&app)
}

/// Hide the popup overlay.
#[tauri::command]
pub fn hide_popup(app: AppHandle) -> Result<(), String> {
    crate::popup::hide_popup(&app)
}

/// Toggle popup visibility; returns whether it is now visible.
#[tauri::command]
pub fn toggle_popup(app: AppHandle) -> Result<bool, String> {
    crate::popup::toggle_popup(&app)
}

// ── Result actions ────────────────────────────────────────────────────

/// Record an action failure in the opt-in local error log (no-op when off).
fn log_action_err(state: &AppState, action: &str, path: &str, err: &str) {
    state
        .error_log
        .log("action", &format!("{action} {path}: {err}"));
}

#[tauri::command]
pub fn open_path(state: State<'_, AppState>, app: AppHandle, path: String) -> Result<(), String> {
    crate::actions::open_path(&app, &path).inspect_err(|e| log_action_err(&state, "open", &path, e))
}

#[tauri::command]
pub fn reveal_path(state: State<'_, AppState>, app: AppHandle, path: String) -> Result<(), String> {
    crate::actions::reveal_path(&app, &path)
        .inspect_err(|e| log_action_err(&state, "reveal", &path, e))
}

#[tauri::command]
pub fn delete_path(state: State<'_, AppState>, path: String) -> Result<(), String> {
    crate::actions::delete_to_trash(&path)
        .inspect_err(|e| log_action_err(&state, "delete", &path, e))
}

/// Copy a file path to the system clipboard (via plugin).
#[tauri::command]
pub fn copy_path(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .write_text(path)
        .map_err(|e| format!("clipboard write failed: {e}"))
}

/// Copy the file itself to the system clipboard as a file drop (Windows CF_HDROP;
/// other platforms fall back to writing the path).
#[tauri::command]
pub fn copy_file(app: AppHandle, path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        crate::actions::copy_file_windows(&app, &path)
    }
    #[cfg(not(target_os = "windows"))]
    {
        copy_path(app, path)
    }
}

/// Wire engine status callbacks to Tauri events. Call once from setup.
pub fn attach_status_forwarder(app: AppHandle, engine: &SearchEngine) {
    engine.on_status(move |status| {
        let _ = app.emit("index://status", StatusEvent { status });
    });
}

// ── Local error log (opt-in, Phase 8) ─────────────────────────────────

#[derive(Clone, Serialize)]
pub struct ErrorLogInfo {
    pub enabled: bool,
    pub path: String,
    pub size_bytes: u64,
}

/// Current opt-in local error log state (path lives in the app data dir).
#[tauri::command]
pub fn error_log_info(state: State<'_, AppState>) -> ErrorLogInfo {
    ErrorLogInfo {
        enabled: state.error_log.is_enabled(),
        path: state.error_log.path().to_string_lossy().into_owned(),
        size_bytes: state.error_log.size_bytes(),
    }
}

/// Disable + delete the local error log.
#[tauri::command]
pub fn clear_error_log(state: State<'_, AppState>) -> Result<(), String> {
    state.error_log.clear();
    Ok(())
}

/// Convenience: does the given window label exist?
pub fn window_exists(app: &AppHandle, label: &str) -> bool {
    app.get_webview_window(label).is_some()
}

// ── Content extraction (Phase 3) ──────────────────────────────────────

/// Result of a full-text content search (SQLite FTS5).
#[derive(Clone, Serialize)]
pub struct ContentHit {
    pub path: String,
    pub snippet: String,
}

/// Search extracted file contents (FTS5). Prefix each term with `content:` in
/// the main search box, or call this directly from the UI.
#[tauri::command]
pub fn search_content(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<ContentHit>, String> {
    let limit = limit.unwrap_or(50).clamp(1, 500);
    let store = state.content.store();
    let store = store.lock().map_err(|e| e.to_string())?;
    let hits = store
        .search_fts(&query, limit)?
        .into_iter()
        .map(|(path, snippet)| ContentHit { path, snippet })
        .collect();
    Ok(hits)
}

/// Kick a background extraction pass over every indexed file (idempotent;
/// skips files that already have content unless `force`).
#[tauri::command]
pub fn rebuild_content(state: State<'_, AppState>, force: bool) -> Result<(), String> {
    let engine = Arc::clone(&state.engine);
    let content = Arc::clone(&state.content);
    std::thread::Builder::new()
        .name("content-enqueue".into())
        .spawn(move || {
            let entries = engine.snapshot_entries();
            for e in entries {
                if e.is_dir {
                    continue;
                }
                if !force {
                    let has = content
                        .store()
                        .lock()
                        .ok()
                        .and_then(|s| s.has_content(&e.path).ok())
                        .unwrap_or(false);
                    if has {
                        continue;
                    }
                }
                let size = e.size;
                let modified = e.modified_ms;
                content.enqueue(crate::content::ExtractionJob {
                    path: std::path::PathBuf::from(&e.path),
                    size,
                    file_modified_ms: modified,
                });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Extraction pipeline stats (queued/processed/failed).
#[tauri::command]
pub fn content_status(
    state: State<'_, AppState>,
) -> Result<crate::content::worker::ExtractionStats, String> {
    Ok(state.content.stats())
}

// ── Semantic layer (Phase 4) ──────────────────────────────────────────

/// Probe local Ollama: reachable? models present?
#[tauri::command]
pub fn ollama_health(state: State<'_, AppState>) -> Result<crate::semantic::OllamaHealth, String> {
    let (url, emb, chat) = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        (
            s.ollama_url.clone(),
            s.embedding_model.clone(),
            s.chat_model.clone(),
        )
    };
    // Degrade gracefully: invalid URL (e.g. remote host) → unreachable health.
    match crate::semantic::OllamaClient::new(&url) {
        Ok(c) => Ok(c.health(&emb, &chat)),
        Err(e) => Ok(crate::semantic::OllamaHealth {
            reachable: false,
            models: Vec::new(),
            embedding_model_available: false,
            chat_model_available: false,
            error: Some(e),
        }),
    }
}

/// Embedding worker stats.
#[tauri::command]
pub fn semantic_status(state: State<'_, AppState>) -> Result<crate::semantic::EmbedStats, String> {
    Ok(state.embed.stats())
}

/// Natural-language query parse via local Ollama chat model.
/// Returns the structured Everything-style query (and whether semantic
/// ranking should also run). Falls back to the raw input if Ollama is down.
#[tauri::command]
pub fn parse_nl_query(
    state: State<'_, AppState>,
    query: String,
) -> Result<crate::semantic::ParsedNL, String> {
    let (enabled, url, model) = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        (
            s.semantic_enabled,
            s.ollama_url.clone(),
            s.chat_model.clone(),
        )
    };
    if !enabled {
        return Ok(crate::semantic::ParsedNL {
            terms: query,
            semantic: false,
            ..Default::default()
        });
    }
    let client = crate::semantic::OllamaClient::new(&url)?;
    Ok(crate::semantic::parse_query(&client, &model, &query))
}

/// Vector similarity search over embedded content (top-k paths + snippets).
#[tauri::command]
pub fn search_semantic(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<crate::semantic::VectorHit>, String> {
    let (enabled, url, model) = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        (
            s.semantic_enabled,
            s.ollama_url.clone(),
            s.embedding_model.clone(),
        )
    };
    if !enabled {
        return Ok(Vec::new());
    }
    let client = crate::semantic::OllamaClient::new(&url)?;
    let emb = client.embed(&model, &[query])?;
    let v = emb.into_iter().next().unwrap_or_default();
    if v.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.unwrap_or(20).clamp(1, 200);
    let store = state.embed.store();
    let store = store.lock().map_err(|e| e.to_string())?;
    store.search(&v, &model, limit, None)
}

/// Kick a background embedding pass over extracted content (skips already-embedded
/// paths unless `force`).
#[tauri::command]
pub fn rebuild_semantic(state: State<'_, AppState>, force: bool) -> Result<(), String> {
    if !state.embed.enabled() {
        return Err("semantic layer is disabled (enable it in Settings)".into());
    }
    let content = Arc::clone(&state.content);
    let embed = Arc::clone(&state.embed);
    let model = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        s.embedding_model.clone()
    };
    std::thread::Builder::new()
        .name("semantic-enqueue".into())
        .spawn(move || {
            // Every extracted text row whose path lacks embeddings → embed queue.
            for (path, text) in collect_paths_for_embedding(&content, &embed, &model, force) {
                embed.enqueue(crate::semantic::EmbedJob { path, text });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// `(path, text)` rows from the content store that still need embeddings.
fn collect_paths_for_embedding(
    content: &crate::content::ExtractionWorker,
    embed: &crate::semantic::EmbeddingWorker,
    model: &str,
    force: bool,
) -> Vec<(String, String)> {
    let store_arc = content.store();
    let Ok(store) = store_arc.lock() else {
        return Vec::new();
    };
    store
        .text_rows_for_embedding(force, model, |path| {
            embed
                .store()
                .lock()
                .map(|v| v.has_embeddings(path, model).unwrap_or(false))
                .unwrap_or(false)
        })
        .unwrap_or_default()
}

/// Semantic/vector counts for the status bar.
#[tauri::command]
pub fn vector_count(state: State<'_, AppState>) -> Result<u64, String> {
    let model = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        s.embedding_model.clone()
    };
    let store = state.embed.store();
    let store = store.lock().map_err(|e| e.to_string())?;
    store.count(Some(&model))
}

/// Hybrid search: filename index + FTS content + vector similarity merged into
/// one ranked list. Degrades to plain filename search when the semantic layer
/// is disabled or Ollama is unreachable.
#[tauri::command]
pub fn search_hybrid(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<SearchResult, String> {
    use crate::index::query::ParsedQuery;
    use crate::semantic::hybrid;
    use std::time::Instant;

    let started = Instant::now();
    let (enabled, url, model) = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        (
            s.semantic_enabled,
            s.ollama_url.clone(),
            s.embedding_model.clone(),
        )
    };
    let fallback = state.settings.lock().map(|s| s.search_limit).unwrap_or(200);
    let limit = limit.unwrap_or(fallback).clamp(1, 2000);

    let base = state.engine.search(&query, limit);
    if !enabled || query.trim().is_empty() {
        return Ok(base);
    }

    // Free-text terms only — strip Everything operators before FTS/embedding.
    let terms = ParsedQuery::parse(&query).terms.join(" ");

    // ── Signal 1: filename (existing index search) ─────────────────────
    let base_paths: Vec<String> = base.items.iter().map(|i| i.path.clone()).collect();
    let raw: Vec<u32> = base.items.iter().map(|i| i.score).collect();
    let norm = hybrid::normalize(&raw);
    let filename: Vec<(String, f32)> = base_paths.into_iter().zip(norm).collect();

    // ── Signal 2: full-text (FTS5) — failure degrades to filename-only ─
    let content_paths: Vec<String> = if terms.is_empty() {
        Vec::new()
    } else {
        state
            .content
            .store()
            .lock()
            .ok()
            .and_then(|s| s.search_fts(&terms, limit).ok())
            .map(|hits| hits.into_iter().map(|(p, _)| p).collect())
            .unwrap_or_default()
    };

    // ── Signal 3: vector similarity — Ollama down? skip the signal ─────
    let vector: Vec<(String, f32)> = if terms.is_empty() {
        Vec::new()
    } else {
        crate::semantic::OllamaClient::new(&url)
            .and_then(|c| c.embed(&model, std::slice::from_ref(&terms)))
            .ok()
            .and_then(|embs| embs.into_iter().next())
            .and_then(|v| {
                if v.is_empty() {
                    return None;
                }
                let store = state.embed.store();
                let store = store.lock().ok()?;
                store.search(&v, &model, limit, None).ok()
            })
            .map(|hits| hits.into_iter().map(|h| (h.path, h.score)).collect())
            .unwrap_or_default()
    };

    let merged = hybrid::merge_scores(&filename, &content_paths, &vector);

    // Fill in metadata: reuse base items when present, else index lookup.
    let mut base_items: std::collections::HashMap<String, SearchResultItem> = base
        .items
        .into_iter()
        .map(|i| (i.path.clone(), i))
        .collect();
    let mut items: Vec<SearchResultItem> = Vec::with_capacity(limit);
    for (path, score) in merged {
        if items.len() >= limit {
            break;
        }
        let mut item = match base_items.remove(&path) {
            Some(i) => i,
            None => match state.engine.lookup_item(&path) {
                Some(i) => i,
                None => continue, // stale content/vector row
            },
        };
        item.score = (score * 1000.0).round() as u32;
        items.push(item);
    }

    Ok(SearchResult {
        total_matched: items.len(),
        took_ms: started.elapsed().as_millis() as u64,
        query,
        items,
    })
}
