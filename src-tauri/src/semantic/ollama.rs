// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Minimal blocking Ollama HTTP client (embeddings + chat + health).
//!
//! Only localhost/loopback base URLs are accepted — see [`validate_base_url`].

use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct OllamaClient {
    base_url: String,
}

/// Health/status snapshot for the UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OllamaHealth {
    pub reachable: bool,
    pub models: Vec<String>,
    pub embedding_model_available: bool,
    pub chat_model_available: bool,
    pub error: Option<String>,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
    format: &'static str,
    options: ChatOptions,
}

#[derive(Serialize)]
struct ChatOptions {
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    name: String,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
}

/// Reject anything that isn't loopback — privacy hard requirement.
pub fn validate_base_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim().trim_end_matches('/');
    let rest = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .ok_or_else(|| "Ollama URL must start with http:// or https://".to_string())?;
    let hostport = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = hostport.split('@').next_back().unwrap_or(""); // drop userinfo
    let host = if let Some(bracketed) = host.strip_prefix('[') {
        // Bracketed IPv6 literal: "[::1]:11434" → "::1"
        bracketed.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    let host_l = host.to_ascii_lowercase();
    let ok = matches!(
        host_l.as_str(),
        "localhost" | "127.0.0.1" | "::1" | "[::1]" | "0.0.0.0"
    );
    if ok {
        Ok(())
    } else {
        Err(format!(
            "Ollama must stay on-device; host '{host_l}' is not localhost/loopback"
        ))
    }
}

impl OllamaClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self, String> {
        let base_url = base_url.into();
        validate_base_url(&base_url)?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn agent(&self) -> ureq::Agent {
        ureq::AgentBuilder::new().timeout(TIMEOUT).build()
    }

    /// Health probe: GET /api/tags (model list).
    pub fn health(&self, embedding_model: &str, chat_model: &str) -> OllamaHealth {
        match self.agent().get(&self.url("/api/tags")).call() {
            Ok(resp) => match resp.into_json::<TagsResponse>() {
                Ok(tags) => {
                    let models: Vec<String> = tags.models.into_iter().map(|m| m.name).collect();
                    OllamaHealth {
                        reachable: true,
                        embedding_model_available: model_present(&models, embedding_model),
                        chat_model_available: model_present(&models, chat_model),
                        models,
                        error: None,
                    }
                }
                Err(e) => OllamaHealth {
                    reachable: false,
                    models: vec![],
                    embedding_model_available: false,
                    chat_model_available: false,
                    error: Some(format!("parse /api/tags: {e}")),
                },
            },
            Err(e) => OllamaHealth {
                reachable: false,
                models: Vec::new(),
                embedding_model_available: false,
                chat_model_available: false,
                error: Some(e.to_string()),
            },
        }
    }

    /// Embed a batch of texts. Returns one vector per input.
    pub fn embed(&self, model: &str, inputs: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let body = EmbedRequest {
            model,
            input: inputs.to_vec(),
        };
        let resp = self
            .agent()
            .post(&self.url("/api/embed"))
            .send_json(body)
            .map_err(|e| ollama_err("embed", e))?;
        let parsed: EmbedResponse = resp
            .into_json()
            .map_err(|e| format!("embed response parse: {e}"))?;
        if parsed.embeddings.len() != inputs.len() {
            return Err(format!(
                "embed count mismatch: got {} want {}",
                parsed.embeddings.len(),
                inputs.len()
            ));
        }
        Ok(parsed.embeddings)
    }

    /// One-shot chat completion with a JSON-format response hint.
    pub fn chat_json(&self, model: &str, system: &str, user: &str) -> Result<String, String> {
        let body = ChatRequest {
            model,
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: system,
                },
                ChatMessage {
                    role: "user",
                    content: user,
                },
            ],
            stream: false,
            format: "json",
            options: ChatOptions { temperature: 0.0 },
        };
        let resp = self
            .agent()
            .post(&self.url("/api/chat"))
            .send_json(body)
            .map_err(|e| ollama_err("chat", e))?;
        let parsed: ChatResponse = resp
            .into_json()
            .map_err(|e| format!("chat response parse: {e}"))?;
        Ok(parsed.message.content)
    }
}

/// Format a ureq error, surfacing Ollama's JSON `{"error": ...}` body when present.
fn ollama_err(op: &str, e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            match serde_json::from_str::<ErrorResponse>(&body) {
                Ok(err) => format!("{op} failed ({code}): {}", err.error),
                Err(_) => format!("{op} failed ({code}): {body}"),
            }
        }
        e => format!("{op} request failed: {e}"),
    }
}

fn model_present(models: &[String], want: &str) -> bool {
    if want.is_empty() {
        return false;
    }
    models
        .iter()
        .any(|m| m == want || m.starts_with(&format!("{want}:")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_remote_hosts() {
        assert!(validate_base_url("http://example.com:11434").is_err());
        assert!(validate_base_url("https://api.openai.com/v1").is_err());
        assert!(validate_base_url("ollama.local").is_err());
    }

    #[test]
    fn accepts_loopback() {
        assert!(validate_base_url("http://127.0.0.1:11434").is_ok());
        assert!(validate_base_url("http://localhost:11434/").is_ok());
        assert!(validate_base_url("http://[::1]:11434").is_ok());
        assert!(validate_base_url("http://0.0.0.0:11434").is_ok());
        assert!(validate_base_url("http://[::1]").is_ok());
    }

    #[test]
    fn rejects_non_loopback_ipv6() {
        assert!(validate_base_url("http://[fe80::1]:11434").is_err());
    }

    #[test]
    fn rejects_userinfo_tricks() {
        assert!(validate_base_url("http://evil@127.0.0.1:11434").is_ok()); // userinfo dropped, host is loopback
        assert!(validate_base_url("http://127.0.0.1@evil.com:11434").is_err());
    }

    #[test]
    fn client_rejects_remote() {
        assert!(OllamaClient::new("http://evil.example.com").is_err());
        assert!(OllamaClient::new("http://127.0.0.1:11434").is_ok());
    }

    #[test]
    fn unreachable_health_degrades_gracefully() {
        // Nothing should be listening on this port in CI.
        let c = OllamaClient::new("http://127.0.0.1:59999").unwrap();
        let h = c.health("nomic-embed-text", "llama3.2");
        assert!(!h.reachable);
        assert!(h.error.is_some());
        assert!(h.models.is_empty());
    }

    #[test]
    fn model_present_matches_tag() {
        let models = vec![
            "nomic-embed-text:latest".to_string(),
            "llama3.2:3b".to_string(),
        ];
        assert!(model_present(&models, "nomic-embed-text"));
        assert!(model_present(&models, "llama3.2"));
        assert!(!model_present(&models, "mistral"));
    }
}
