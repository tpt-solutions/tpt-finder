// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Content extraction pipeline: file type → plain text + metadata.
//!
//! Phase 3 of the roadmap. Extracted text is stored in SQLite FTS5 alongside
//! the path index so queries can match file contents as well as names.

pub mod extract;
pub mod skip;
pub mod store;
pub mod worker;

pub use extract::{extract_file, Extracted, ExtractionError};
pub use skip::SkipRules;
pub use store::ContentStore;
pub use worker::{ExtractionJob, ExtractionWorker};
