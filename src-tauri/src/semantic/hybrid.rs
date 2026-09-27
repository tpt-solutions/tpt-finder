// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Hybrid ranking: merge filename (in-memory index), full-text (FTS5), and
//! vector similarity (Ollama embeddings) signals into one ranked path list.

use std::collections::HashMap;

/// Weight of the filename/index signal (score already normalized to 0..1).
pub const W_FILENAME: f32 = 1.0;
/// Weight of a full-text (FTS5) hit (binary signal, no score from SQLite).
pub const W_CONTENT: f32 = 0.6;
/// Weight of the vector similarity signal (cosine already in 0..1).
pub const W_VECTOR: f32 = 0.8;

/// Merge per-signal scores for the same set of paths.
///
/// * `filename` — `(path, score)` where score is normalized to 0..1
/// * `content`  — paths that matched the FTS query (equal weight)
/// * `vector`   — `(path, cosine)` where cosine is 0..1
///
/// Returns `(path, total_score)` sorted descending, ties broken by path so the
/// ordering is deterministic.
pub fn merge_scores(
    filename: &[(String, f32)],
    content: &[String],
    vector: &[(String, f32)],
) -> Vec<(String, f32)> {
    let mut acc: HashMap<String, f32> = HashMap::new();
    for (path, score) in filename {
        *acc.entry(path.clone()).or_default() += W_FILENAME * score.clamp(0.0, 1.0);
    }
    for path in content {
        *acc.entry(path.clone()).or_default() += W_CONTENT;
    }
    for (path, score) in vector {
        *acc.entry(path.clone()).or_default() += W_VECTOR * score.clamp(0.0, 1.0);
    }
    let mut merged: Vec<(String, f32)> = acc.into_iter().collect();
    merged.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    merged
}

/// Normalize raw index scores (u32, `score_kind` scale) to 0..1 against the max.
pub fn normalize(raw: &[u32]) -> Vec<f32> {
    let max = raw.iter().copied().max().unwrap_or(1).max(1);
    raw.iter().map(|s| *s as f32 / max as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(s: &str) -> String {
        s.to_string()
    }

    #[test]
    fn multi_signal_outranks_single_signal() {
        let filename = vec![(f("a.pdf"), 0.8)];
        let content = vec![f("a.pdf")];
        let vector = vec![(f("a.pdf"), 0.9)];
        let merged = merge_scores(&filename, &content, &vector);
        assert_eq!(merged.len(), 1);
        let expected = W_FILENAME * 0.8 + W_CONTENT + W_VECTOR * 0.9;
        assert!((merged[0].1 - expected).abs() < 1e-6);
    }

    #[test]
    fn filename_beats_content_beats_vector_for_equal_single_hits() {
        // Filename at full strength (1.0) > content (0.6) > vector at 0.5 cos.
        let merged = merge_scores(
            &[(f("name.pdf"), 1.0)],
            &[f("content.pdf")],
            &[(f("vector.pdf"), 0.5)],
        );
        let order: Vec<&str> = merged.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(order, vec!["name.pdf", "content.pdf", "vector.pdf"]);
    }

    #[test]
    fn ties_break_deterministically_by_path() {
        let merged = merge_scores(&[(f("b.txt"), 0.6)], &[f("a.txt")], &[]);
        assert_eq!(merged[0].0, "a.txt");
        assert_eq!(merged[1].0, "b.txt");
        assert!((merged[0].1 - merged[1].1).abs() < 1e-6);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn normalize_scales_by_max() {
        let v = normalize(&[1000, 500, 0]);
        assert!((v[0] - 1.0).abs() < 1e-6);
        assert!((v[1] - 0.5).abs() < 1e-6);
        assert_eq!(v[2], 0.0);
        assert_eq!(normalize(&[]), Vec::<f32>::new());
        assert_eq!(normalize(&[0]), vec![0.0]);
    }

    #[test]
    fn out_of_range_scores_clamp() {
        let merged = merge_scores(&[(f("x"), 1.5)], &[], &[(f("y"), -0.2)]);
        let map: HashMap<&str, f32> = merged.iter().map(|(p, s)| (p.as_str(), *s)).collect();
        assert!((map["x"] - 1.0).abs() < 1e-6);
        assert_eq!(map["y"], 0.0);
    }
}
