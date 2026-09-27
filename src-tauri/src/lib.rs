// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! tpt-finder library: indexing engine + IPC command layer.

pub mod actions;
pub mod content;
pub mod index;
pub mod ipc;
pub mod logging;
pub mod perf;
pub mod popup;
pub mod semantic;
pub mod settings;

use std::sync::{Arc, Mutex};

use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::index::SearchEngine;
use crate::ipc::{attach_status_forwarder, AppState};
use crate::settings::SettingsStore;

pub use index::{IndexStatus, SearchResult};
pub use ipc::{
    app_version, clear_error_log, content_status, copy_file, copy_path, delete_path,
    error_log_info, get_settings, hide_popup, index_status, load_index, ollama_health, open_path,
    parse_nl_query, ping, rebuild_content, rebuild_index, rebuild_semantic, reveal_path,
    save_index, search, search_content, search_hybrid, search_semantic, semantic_status,
    set_settings, show_popup, toggle_popup, vector_count, AppState as _AppState,
};

pub use index::SearchEngine as _SearchEngineReexport;

fn register_hotkey(app: &tauri::AppHandle, hotkey: &str) -> Result<(), String> {
    let shortcut: Shortcut = hotkey
        .parse()
        .map_err(|e| format!("invalid hotkey '{hotkey}': {e}"))?;
    let gs = app.global_shortcut();
    // Best-effort unregister of default then register desired.
    let _ = gs.unregister_all();
    gs.register(shortcut)
        .map_err(|e| format!("register hotkey failed: {e}"))
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        let _ = popup::toggle_popup(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            if let Some(popup) = app.get_webview_window("popup") {
                let _ = popup.hide();
            }

            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("tpt-finder"));
            std::fs::create_dir_all(&data_dir).ok();

            let settings_store = Arc::new(SettingsStore::new(&data_dir));
            let settings = settings_store.load().normalized();
            let hotkey = settings.hotkey.clone();
            let show_main = settings.show_main_on_start;

            register_hotkey(app.handle(), &hotkey)?;

            let error_log = Arc::new(crate::logging::ErrorLog::open(&data_dir));
            error_log.set_enabled(settings.error_log_enabled);

            let engine = Arc::new(SearchEngine::new(data_dir.clone()));
            let restored = engine.try_load_persisted();
            attach_status_forwarder(app.handle().clone(), &engine);

            let content_store = match crate::content::ContentStore::open(&data_dir) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("content store open failed (continuing without FTS): {e}");
                    // Fall back to an in-memory store under temp so commands don't panic.
                    crate::content::ContentStore::open_file(
                        std::env::temp_dir().join("tpt-finder-content-fallback"),
                    )
                    .expect("fallback content store")
                }
            };
            let content = crate::content::ExtractionWorker::spawn(
                content_store,
                crate::content::SkipRules::from_settings(&settings),
            );
            content.set_error_log(Arc::clone(&error_log));
            content.set_throttle_ms(settings.extraction_throttle_ms.min(5_000));

            let embed_cfg = crate::semantic::EmbedConfig {
                model: settings.embedding_model.clone(),
                chunk_chars: settings.chunk_chars.clamp(64, 8192),
                max_chunks_per_file: settings.max_chunks_per_file.clamp(1, 256),
                throttle_ms: settings.embed_throttle_ms.min(5_000),
            };
            let vector_store = match crate::semantic::VectorStore::open(&data_dir) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("vector store open failed (semantic disabled): {e}");
                    crate::semantic::VectorStore::open_file(
                        std::env::temp_dir().join("tpt-finder-vectors-fallback"),
                    )
                    .expect("fallback vector store")
                }
            };
            let embed = crate::semantic::EmbeddingWorker::spawn(
                vector_store,
                embed_cfg,
                settings.semantic_enabled,
            );
            embed.set_error_log(Arc::clone(&error_log));

            // Incremental pipeline: extracted text flows straight into the
            // embedding queue (jobs are dropped while semantic is disabled).
            content.set_embed_worker(Arc::clone(&embed));

            // Live FS changes → content extraction (+ stale row cleanup).
            engine.on_change({
                let content = Arc::clone(&content);
                let embed = Arc::clone(&embed);
                use crate::index::backend::FsChange;
                move |change| match change {
                    FsChange::Created(e) | FsChange::Updated(e) if !e.is_dir => {
                        content.enqueue(crate::content::ExtractionJob {
                            path: std::path::PathBuf::from(&e.path),
                            size: e.size,
                            file_modified_ms: e.modified_ms,
                        });
                    }
                    FsChange::Removed { path, .. } => {
                        if let Ok(store) = content.store().lock() {
                            let _ = store.remove(path);
                        }
                        if let Ok(store) = embed.store().lock() {
                            let _ = store.remove_path(path);
                        }
                    }
                    FsChange::Renamed { from, to } => {
                        if let Ok(store) = content.store().lock() {
                            let _ = store.remove(from);
                        }
                        if let Ok(store) = embed.store().lock() {
                            let _ = store.remove_path(from);
                        }
                        crate::content::worker::try_enqueue_path(
                            &content,
                            std::path::PathBuf::from(to),
                        );
                    }
                    _ => {}
                }
            });

            app.manage(AppState {
                engine: Arc::clone(&engine),
                settings: Arc::new(Mutex::new(settings.clone())),
                settings_store,
                content,
                embed,
                error_log,
            });

            if !restored {
                let e = Arc::clone(&engine);
                let _ = e.rebuild_async();
            }

            if !show_main {
                if let Some(main) = app.get_webview_window("main") {
                    let _ = main.hide();
                }
            }

            let saver = Arc::clone(&engine);
            std::thread::Builder::new()
                .name("index-autosave".into())
                .spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    if let Err(e) = saver.save() {
                        eprintln!("autosave failed: {e}");
                    }
                })
                .ok();

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            app_version,
            search,
            index_status,
            rebuild_index,
            save_index,
            load_index,
            get_settings,
            set_settings,
            show_popup,
            hide_popup,
            toggle_popup,
            open_path,
            reveal_path,
            delete_path,
            copy_path,
            copy_file,
            search_content,
            rebuild_content,
            content_status,
            ollama_health,
            semantic_status,
            parse_nl_query,
            search_semantic,
            rebuild_semantic,
            vector_count,
            search_hybrid,
            error_log_info,
            clear_error_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tpt-finder");
}
