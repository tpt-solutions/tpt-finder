// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Natural-language query parsing via a local Ollama chat model.
//!
//! The model returns JSON: `{"terms": "...", "ext": ["pdf"], "path": ["docs"],
//! "dated": "2026-03", "semantic": true}` which is merged into the existing
//! Everything-style query string.

use serde::{Deserialize, Serialize};

use super::ollama::OllamaClient;

const SYSTEM: &str = r#"You convert a user's natural-language file search into JSON.
Return ONLY a JSON object with these optional keys:
  "terms": string — keywords for filename matching (space-separated, no operators)
  "ext": array of lowercase file extensions without dots
  "path": array of path substrings
  "dated": string — YYYY or YYYY-MM if a date was mentioned
  "semantic": boolean — true if the query needs meaning beyond keywords (synonyms, "the invoice from John", etc.)
Rules: keep terms concise; omit keys that do not apply; do not invent dates or extensions."#;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ParsedNL {
    #[serde(default)]
    pub terms: String,
    #[serde(default)]
    pub ext: Vec<String>,
    #[serde(default)]
    pub path: Vec<String>,
    #[serde(default)]
    pub dated: Option<String>,
    #[serde(default)]
    pub semantic: bool,
}

impl ParsedNL {
    /// Build an Everything-style query string from the parse result.
    pub fn to_query(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.terms.trim().is_empty() {
            parts.push(self.terms.trim().to_string());
        }
        for e in &self.ext {
            let e = e.trim_start_matches('.').to_ascii_lowercase();
            if !e.is_empty() {
                parts.push(format!("ext:{e}"));
            }
        }
        for p in &self.path {
            let p = p.trim();
            if !p.is_empty() {
                parts.push(format!("path:{}", p.replace(' ', "_")));
            }
        }
        if let Some(d) = &self.dated {
            let d = d.trim();
            if !d.is_empty() {
                parts.push(format!("dated:{d}"));
            }
        }
        parts.join(" ")
    }
}

/// Parse NL → structured via Ollama. On any failure, returns a conservative
/// fallback (`semantic=false`, terms = raw input) so search still works.
pub fn parse_query(client: &OllamaClient, model: &str, user: &str) -> ParsedNL {
    let raw = user.trim();
    if raw.is_empty() {
        return ParsedNL::default();
    }
    match client.chat_json(model, SYSTEM, raw) {
        Ok(content) => match serde_json::from_str::<ParsedNL>(&content) {
            Ok(mut p) => {
                if p.terms.trim().is_empty() && !p.ext.is_empty() {
                    // Pure filter query — keep as-is via to_query.
                }
                // If the model emptied terms but gave no filters, fall back.
                if p.terms.trim().is_empty()
                    && p.ext.is_empty()
                    && p.path.is_empty()
                    && p.dated.is_none()
                {
                    p.terms = raw.to_string();
                }
                p
            }
            Err(_) => ParsedNL {
                terms: raw.to_string(),
                semantic: false,
                ..ParsedNL::default()
            },
        },
        Err(_) => ParsedNL {
            terms: raw.to_string(),
            semantic: false,
            ..ParsedNL::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_query_builds_operators() {
        let p = ParsedNL {
            terms: "quarterly report".into(),
            ext: vec!["pdf".into(), ".docx".into()],
            path: vec!["My Documents".into()],
            dated: Some("2026-03".into()),
            semantic: true,
        };
        let q = p.to_query();
        assert!(q.contains("quarterly report"));
        assert!(q.contains("ext:pdf"));
        assert!(q.contains("ext:docx"));
        assert!(q.contains("path:My_Documents"));
        assert!(q.contains("dated:2026-03"));
    }

    #[test]
    fn to_query_empty() {
        assert_eq!(ParsedNL::default().to_query(), "");
    }

    #[test]
    fn deserialize_partial_json() {
        let j = r#"{"terms":"invoice", "ext":["pdf"], "semantic":true}"#;
        let p: ParsedNL = serde_json::from_str(j).unwrap();
        assert_eq!(p.terms, "invoice");
        assert_eq!(p.ext, vec!["pdf"]);
        assert!(p.semantic);
        assert!(p.dated.is_none());
    }

    #[test]
    fn fallback_parses_without_server() {
        // Unreachable port → graceful degradation to raw terms.
        let c = OllamaClient::new("http://127.0.0.1:59998").unwrap();
        let p = parse_query(&c, "llama3.2", "find my vacation photos");
        assert_eq!(p.terms, "find my vacation photos");
        assert!(!p.semantic);
    }
}
