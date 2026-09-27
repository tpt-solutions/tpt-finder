// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! User settings persisted as JSON in the app data directory.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Global hotkey to summon/dismiss the popup (Tauri shortcut syntax).
    pub hotkey: String,
    /// Show main window on app launch (otherwise only popup via hotkey).
    pub show_main_on_start: bool,
    /// Default max results returned to the UI.
    pub search_limit: usize,
    /// Open popup centered on the active monitor.
    pub popup_center: bool,
    // ── Semantic layer (Phase 4) ────────────────────────────────────
    /// Master switch for Ollama-backed semantic search / NL parsing.
    pub semantic_enabled: bool,
    /// Local Ollama base URL (must be loopback — enforced by the client).
    pub ollama_url: String,
    /// Embedding model name (e.g. "nomic-embed-text").
    pub embedding_model: String,
    /// Chat model used for natural-language query parsing (e.g. "llama3.2").
    pub chat_model: String,
    /// Max characters per embedding chunk.
    pub chunk_chars: usize,
    /// Max chunks embedded per file (bounds memory on huge docs).
    pub max_chunks_per_file: usize,
    /// Milliseconds between embedding API calls (0 = no throttle).
    pub embed_throttle_ms: u64,
    // ── Polish (Phase 8) ────────────────────────────────────────────
    /// UI theme: "system" | "light" | "dark".
    pub theme: String,
    /// Extensions the content extractor must skip (lowercase, no dot).
    /// Seeded from the built-in defaults; the UI edits the full list.
    pub exclude_extensions: Vec<String>,
    /// Directory names the extractor never walks into.
    pub exclude_dirs: Vec<String>,
    /// Files larger than this (bytes) are never extracted.
    pub max_extract_bytes: u64,
    /// Milliseconds pause between background extractions (0 = no throttle).
    pub extraction_throttle_ms: u64,
    /// Opt-in local error log (app data dir); never leaves the machine.
    pub error_log_enabled: bool,
}

impl Settings {
    /// Normalize user-editable free-form fields into their canonical form.
    pub fn normalized(mut self) -> Self {
        if !matches!(self.theme.as_str(), "light" | "dark") {
            self.theme = "system".into();
        }
        self.exclude_extensions = dedup_lc(&self.exclude_extensions);
        self.exclude_dirs = dedup_lc(&self.exclude_dirs);
        self
    }
}

fn dedup_lc(items: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(items.len());
    for i in items {
        let v = i.trim().trim_start_matches('.').to_ascii_lowercase();
        if !v.is_empty() && !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

impl Default for Settings {
    fn default() -> Self {
        let rules = crate::content::SkipRules::default();
        Self {
            hotkey: "CommandOrControl+Space".into(),
            show_main_on_start: true,
            search_limit: 200,
            popup_center: true,
            semantic_enabled: false,
            ollama_url: "http://127.0.0.1:11434".into(),
            embedding_model: "nomic-embed-text".into(),
            chat_model: "llama3.2".into(),
            chunk_chars: 512,
            max_chunks_per_file: 32,
            embed_throttle_ms: 50,
            theme: "system".into(),
            exclude_extensions: rules.exclude_extensions,
            exclude_dirs: rules.exclude_dir_names,
            max_extract_bytes: rules.max_file_bytes,
            extraction_throttle_ms: 0,
            error_log_enabled: false,
        }
    }
}

pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let mut path = dir.into();
        path.push("settings.json");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Settings {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self, settings: &Settings) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_defaults() {
        let dir = std::env::temp_dir().join(format!("tpt-settings-{}", std::process::id()));
        let store = SettingsStore::new(&dir);
        let s = store.load();
        assert_eq!(s.hotkey, "CommandOrControl+Space");
        assert!(s.show_main_on_start);
        assert!(!s.semantic_enabled);
        assert_eq!(s.embedding_model, "nomic-embed-text");
        let mut s2 = s.clone();
        s2.hotkey = "Alt+Space".into();
        s2.search_limit = 50;
        s2.semantic_enabled = true;
        s2.embedding_model = "mxbai-embed-large".into();
        store.save(&s2).unwrap();
        let loaded = store.load();
        assert_eq!(loaded.hotkey, "Alt+Space");
        assert_eq!(loaded.search_limit, 50);
        assert!(loaded.semantic_enabled);
        assert_eq!(loaded.embedding_model, "mxbai-embed-large");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalized_cleanups() {
        let n = Settings {
            theme: "banana".into(),
            exclude_extensions: vec![" .PDF ".into(), "pdf".into(), "".into(), "  ".into()],
            exclude_dirs: vec!["Node_Modules".into(), "node_modules".into()],
            ..Settings::default()
        }
        .normalized();
        assert_eq!(n.theme, "system");
        assert_eq!(n.exclude_extensions, vec!["pdf"]);
        assert_eq!(n.exclude_dirs, vec!["node_modules"]);
        assert_eq!(Settings::default().theme, "system");
        assert!(!Settings::default().error_log_enabled);
    }
}
