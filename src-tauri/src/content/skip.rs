// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Skip rules: which files the content extractor should not touch.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SkipRules {
    /// Files larger than this (bytes) are never extracted.
    pub max_file_bytes: u64,
    /// File extensions excluded (lowercase, no dot).
    pub exclude_extensions: Vec<String>,
    /// Path substrings that cause a skip (e.g. "node_modules").
    pub exclude_path_contains: Vec<String>,
    /// Directory basename patterns always skipped during walk enqueue.
    pub exclude_dir_names: Vec<String>,
    /// Maximum characters of extracted text stored per file.
    pub max_text_chars: usize,
}

impl Default for SkipRules {
    fn default() -> Self {
        Self {
            max_file_bytes: 16 * 1024 * 1024,
            exclude_extensions: vec![
                "exe".into(),
                "dll".into(),
                "so".into(),
                "dylib".into(),
                "bin".into(),
                "o".into(),
                "a".into(),
                "obj".into(),
                "lib".into(),
                "pdb".into(),
                "wasm".into(),
                "png".into(),
                "jpg".into(),
                "jpeg".into(),
                "gif".into(),
                "bmp".into(),
                "ico".into(),
                "svg".into(),
                "mp4".into(),
                "mkv".into(),
                "mov".into(),
                "avi".into(),
                "mp3".into(),
                "wav".into(),
                "flac".into(),
                "ogg".into(),
                "zip".into(),
                "tar".into(),
                "gz".into(),
                "7z".into(),
                "rar".into(),
                "iso".into(),
                "db".into(),
                "sqlite".into(),
                "sqlite3".into(),
                "woff".into(),
                "woff2".into(),
                "ttf".into(),
                "otf".into(),
            ],
            exclude_path_contains: vec![
                "node_modules".into(),
                "/.git/".into(),
                "\\.git\\".into(),
                "/target/debug/".into(),
                "/target/release/".into(),
                "/.cache/".into(),
                "\\AppData\\Local\\Temp".into(),
            ],
            exclude_dir_names: vec![
                "node_modules".into(),
                ".git".into(),
                "target".into(),
                ".cache".into(),
                "__pycache__".into(),
                ".venv".into(),
                "venv".into(),
            ],
            max_text_chars: 512 * 1024,
        }
    }
}

impl SkipRules {
    /// Build rules from user settings (Phase 8 exclusion-rules UI).
    /// Empty lists in settings fall back to the built-in defaults so an
    /// old/hand-edited settings file cannot silently index everything.
    pub fn from_settings(s: &crate::settings::Settings) -> Self {
        let d = Self::default();
        Self {
            max_file_bytes: if s.max_extract_bytes == 0 {
                d.max_file_bytes
            } else {
                s.max_extract_bytes
            },
            exclude_extensions: if s.exclude_extensions.is_empty() {
                d.exclude_extensions
            } else {
                s.exclude_extensions.clone()
            },
            exclude_path_contains: d.exclude_path_contains,
            exclude_dir_names: if s.exclude_dirs.is_empty() {
                d.exclude_dir_names
            } else {
                s.exclude_dirs.clone()
            },
            max_text_chars: d.max_text_chars,
        }
    }

    /// Why this path should be skipped, or `None` to proceed.
    pub fn skip_reason(&self, path: &Path, size: u64, is_dir: bool) -> Option<&'static str> {
        if is_dir {
            return None;
        }
        if size > self.max_file_bytes {
            return Some("too large");
        }
        let path_str = path.to_string_lossy();
        let lower = path_str.to_ascii_lowercase();
        for needle in &self.exclude_path_contains {
            if lower.contains(&needle.to_ascii_lowercase()) {
                return Some("excluded path");
            }
        }
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext = ext.to_ascii_lowercase();
            if self.exclude_extensions.contains(&ext) {
                return Some("excluded extension");
            }
        }
        // Also check filename (e.g. Dockerfile has no useful ext).
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let n = name.to_ascii_lowercase();
            if self
                .exclude_extensions
                .iter()
                .any(|e| n == *e || n.ends_with(&format!(".{e}")))
            {
                return Some("excluded extension");
            }
        }
        None
    }

    pub fn should_skip(&self, path: &Path, size: u64, is_dir: bool) -> bool {
        self.skip_reason(path, size, is_dir).is_some()
    }

    /// Directory basenames that should not be walked into.
    pub fn skip_dir_name(&self, name: &str) -> bool {
        let n = name.to_ascii_lowercase();
        self.exclude_dir_names
            .iter()
            .any(|d| d.to_ascii_lowercase() == n)
    }

    /// Clamp extracted text to the configured max.
    pub fn clamp_text(&self, text: String) -> String {
        if text.chars().count() <= self.max_text_chars {
            text
        } else {
            text.chars().take(self.max_text_chars).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_large_files() {
        let r = SkipRules::default();
        assert!(r.should_skip(Path::new("/a/b.txt"), r.max_file_bytes + 1, false));
        assert!(!r.should_skip(Path::new("/a/b.txt"), 100, false));
    }

    #[test]
    fn skips_excluded_extension() {
        let r = SkipRules::default();
        assert!(r.should_skip(Path::new("/x/y.png"), 10, false));
        assert!(r.should_skip(Path::new("/x/y.dll"), 10, false));
        assert!(!r.should_skip(Path::new("/x/y.rs"), 10, false));
    }

    #[test]
    fn skips_excluded_path() {
        let r = SkipRules::default();
        #[cfg(windows)]
        let p = r"C:\proj\node_modules\foo\index.js";
        #[cfg(not(windows))]
        let p = "/proj/node_modules/foo/index.js";
        assert!(r.should_skip(Path::new(p), 10, false));
    }

    #[test]
    fn skip_dir_names() {
        let r = SkipRules::default();
        assert!(r.skip_dir_name("node_modules"));
        assert!(r.skip_dir_name(".git"));
        assert!(!r.skip_dir_name("src"));
    }

    #[test]
    fn clamp_text() {
        let r = SkipRules {
            max_text_chars: 5,
            ..Default::default()
        };
        assert_eq!(r.clamp_text("hello world".into()), "hello");
        assert_eq!(r.clamp_text("hi".into()), "hi");
    }
}
