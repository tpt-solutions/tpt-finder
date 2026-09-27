// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Core indexing engine: data model, in-memory store, filesystem backends,
//! persistence, and the search orchestrator.

pub mod backend;
pub mod engine;
pub mod model;
pub mod persistence;
pub mod query;
pub mod store;

pub use engine::{IndexStatus, SearchEngine};
pub use model::{FileEntry, SearchResult, SearchResultItem, VolumeInfo};
pub use query::ParsedQuery;
pub use store::IndexStore;
