// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Semantic layer (Phase 4): local Ollama embeddings + natural-language queries.
//!
//! Privacy invariant: the client only ever talks to a user-configured base URL
//! that must resolve to localhost/loopback. No other network calls are made.

pub mod chunk;
pub mod hybrid;
pub mod ollama;
pub mod query;
pub mod vector;
pub mod worker;

pub use chunk::chunk_text;
pub use ollama::{OllamaClient, OllamaHealth};
pub use query::{parse_query, ParsedNL};
pub use vector::{VectorHit, VectorStore};
pub use worker::{EmbedConfig, EmbedJob, EmbedStats, EmbeddingWorker};
