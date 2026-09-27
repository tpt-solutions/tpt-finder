// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Query string parsing (Everything-style filters) and match scoring.

use std::time::{SystemTime, UNIX_EPOCH};

/// Parsed search query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedQuery {
    /// Free-text terms; all must match (AND).
    pub terms: Vec<String>,
    /// `ext:pdf` — extension without dot, lowercase.
    pub ext: Option<String>,
    /// `path:Documents` — substring of the full path (case-insensitive).
    pub path: Option<String>,
    /// `size:>10mb` lower bound in bytes (inclusive if `size_min_set`).
    pub size_min: Option<u64>,
    pub size_max: Option<u64>,
    /// `dated:2026` / `dated:2026-03` / `dated:2026-03-15` against modified time.
    pub dated: Option<String>,
    /// Force fuzzy (subsequence) matching for free text: `fuzzy:term` or leading `~`.
    pub fuzzy: bool,
    /// Only directories.
    pub dirs_only: bool,
    /// Only files.
    pub files_only: bool,
}

impl ParsedQuery {
    pub fn parse(input: &str) -> Self {
        let mut q = ParsedQuery::default();
        for raw in input.split_whitespace() {
            let token = raw;
            let lower = token.to_ascii_lowercase();

            if let Some(rest) = lower.strip_prefix("ext:") {
                let rest = rest.trim_start_matches('.');
                if !rest.is_empty() {
                    q.ext = Some(rest.to_string());
                }
                continue;
            }
            if let Some(rest) = lower.strip_prefix("path:") {
                if !rest.is_empty() {
                    q.path = Some(rest.to_string());
                }
                continue;
            }
            if let Some(rest) = lower.strip_prefix("size:") {
                parse_size_filter(rest, &mut q);
                continue;
            }
            if let Some(rest) = lower.strip_prefix("dated:") {
                if !rest.is_empty() {
                    q.dated = Some(rest.to_string());
                }
                continue;
            }
            if let Some(rest) = lower.strip_prefix("fuzzy:") {
                if !rest.is_empty() {
                    q.terms.push(rest.to_string());
                    q.fuzzy = true;
                }
                continue;
            }
            if lower == "dirs" || lower == "is:dir" {
                q.dirs_only = true;
                continue;
            }
            if lower == "files" || lower == "is:file" {
                q.files_only = true;
                continue;
            }
            if let Some(rest) = token.strip_prefix('~') {
                if !rest.is_empty() {
                    q.terms.push(rest.to_ascii_lowercase());
                    q.fuzzy = true;
                }
                continue;
            }
            if !token.is_empty() {
                q.terms.push(lower);
            }
        }
        q
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.ext.is_none()
            && self.path.is_none()
            && self.size_min.is_none()
            && self.size_max.is_none()
            && self.dated.is_none()
            && !self.dirs_only
            && !self.files_only
    }

    /// Structural filters only (no free-text terms).
    pub fn has_structural_filters(&self) -> bool {
        self.ext.is_some()
            || self.path.is_some()
            || self.size_min.is_some()
            || self.size_max.is_some()
            || self.dated.is_some()
            || self.dirs_only
            || self.files_only
    }
}

fn parse_size_filter(rest: &str, q: &mut ParsedQuery) {
    let (op, num) = if let Some(r) = rest.strip_prefix(">=") {
        (">=", r)
    } else if let Some(r) = rest.strip_prefix("<=") {
        ("<=", r)
    } else if let Some(r) = rest.strip_prefix('>') {
        (">", r)
    } else if let Some(r) = rest.strip_prefix('<') {
        ("<", r)
    } else if let Some(r) = rest.strip_prefix('=') {
        ("=", r)
    } else {
        ("=", rest)
    };
    let num = num.trim();
    if num.is_empty() {
        return;
    }
    let (digits, unit) = split_size_unit(num);
    let Ok(base) = digits.parse::<u64>() else {
        return;
    };
    let bytes = match unit.as_str() {
        "k" | "kb" => base.saturating_mul(1024),
        "m" | "mb" => base.saturating_mul(1024 * 1024),
        "g" | "gb" => base.saturating_mul(1024 * 1024 * 1024),
        "t" | "tb" => base.saturating_mul(1024u64.pow(4)),
        _ => base,
    };
    match op {
        ">" => q.size_min = Some(bytes.saturating_add(1)),
        ">=" => q.size_min = Some(bytes),
        "<" => q.size_max = Some(bytes.saturating_sub(1)),
        "<=" | "=" => q.size_max = Some(bytes),
        _ => {}
    }
    if op == "=" {
        q.size_min = Some(bytes);
    }
}

fn split_size_unit(s: &str) -> (String, String) {
    let s = s.to_ascii_lowercase();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
        i += 1;
    }
    // Integer sizes only for now; drop fractional part.
    let (digits, unit) = s.split_at(i);
    let digits = digits.split('.').next().unwrap_or(digits).to_string();
    (digits, unit.trim().to_string())
}

/// Why an entry matched (for scoring).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchKind {
    ExactName,
    Prefix,
    Substring,
    PathSubstring,
    Fuzzy,
}

pub fn score_kind(kind: MatchKind) -> u32 {
    match kind {
        MatchKind::ExactName => 1000,
        MatchKind::Prefix => 800,
        MatchKind::Substring => 600,
        MatchKind::PathSubstring => 400,
        MatchKind::Fuzzy => 200,
    }
}

/// ASCII case-insensitive `haystack` starts with `prefix`.
pub fn starts_with_ci(haystack: &str, prefix: &str) -> bool {
    if prefix.len() > haystack.len() {
        return false;
    }
    haystack.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

pub fn contains_ci(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    // Fast path: ASCII lowercase haystack already stored for names; for paths use windows/eq.
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if haystack.is_ascii() && needle.is_ascii() {
        h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
    } else {
        haystack
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
    }
}

/// Subsequence fuzzy match (all chars of `needle` appear in order).
pub fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut it = haystack.as_bytes().iter();
    for &c in needle.as_bytes() {
        let c = c.to_ascii_lowercase();
        loop {
            match it.next() {
                Some(&h) if h.to_ascii_lowercase() == c => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

/// Best match kind for a single term against an entry-like record.
pub fn match_term(
    name_lower: &str,
    path_lower: &str,
    term: &str,
    fuzzy: bool,
) -> Option<MatchKind> {
    if term.is_empty() {
        return Some(MatchKind::Substring);
    }
    if name_lower == term {
        return Some(MatchKind::ExactName);
    }
    if starts_with_ci(name_lower, term) {
        return Some(MatchKind::Prefix);
    }
    if contains_ci(name_lower, term) {
        return Some(MatchKind::Substring);
    }
    if contains_ci(path_lower, term) {
        return Some(MatchKind::PathSubstring);
    }
    if fuzzy && fuzzy_match(name_lower, term) {
        return Some(MatchKind::Fuzzy);
    }
    None
}

/// Check `dated:YYYY[-MM[-DD]]` against modified_ms.
pub fn matches_dated(dated: &str, modified_ms: i64) -> bool {
    if modified_ms <= 0 {
        return false;
    }
    let secs = modified_ms.div_euclid(1000);
    let Some(dt) = unix_secs_to_ymd(secs) else {
        return false;
    };
    // Compare as zero-padded prefixes of "YYYY-MM-DD".
    let full = format!("{:04}-{:02}-{:02}", dt.0, dt.1, dt.2);
    full.starts_with(dated)
}

/// Civil date from Unix seconds (Howard Hinnant's algorithm, no deps).
fn unix_secs_to_ymd(secs: i64) -> Option<(i64, u32, u32)> {
    let days = secs.div_euclid(86_400);
    civil_from_days(days)
}

fn civil_from_days(z: i64) -> Option<(i64, u32, u32)> {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    Some((if m <= 2 { y + 1 } else { y }, m, d))
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_terms_and_ext() {
        let q = ParsedQuery::parse("ext:pdf invoice report");
        assert_eq!(q.ext.as_deref(), Some("pdf"));
        assert_eq!(q.terms, vec!["invoice", "report"]);
    }

    #[test]
    fn parse_size_ops() {
        let q = ParsedQuery::parse("size:>10mb");
        assert_eq!(q.size_min, Some(10 * 1024 * 1024 + 1));
        let q = ParsedQuery::parse("size:<=2kb");
        assert_eq!(q.size_max, Some(2048));
        let q = ParsedQuery::parse("size:500");
        assert_eq!(q.size_min, Some(500));
        assert_eq!(q.size_max, Some(500));
    }

    #[test]
    fn parse_path_and_dated() {
        let q = ParsedQuery::parse(r"path:documents dated:2026-03");
        assert_eq!(q.path.as_deref(), Some("documents"));
        assert_eq!(q.dated.as_deref(), Some("2026-03"));
    }

    #[test]
    fn match_kinds() {
        assert_eq!(
            match_term("report.pdf", "c:/docs/report.pdf", "report", false),
            Some(MatchKind::Prefix)
        );
        assert_eq!(
            match_term("report.pdf", "c:/docs/report.pdf", "report.pdf", false),
            Some(MatchKind::ExactName)
        );
        assert_eq!(
            match_term("annual_report.pdf", "c:/x", "report", false),
            Some(MatchKind::Substring)
        );
        assert_eq!(
            match_term("a.txt", "c:/documents/a.txt", "documents", false),
            Some(MatchKind::PathSubstring)
        );
        assert_eq!(match_term("abc.txt", "x", "axc", false), None);
        assert_eq!(
            match_term("abc.txt", "x", "atx", true),
            Some(MatchKind::Fuzzy)
        );
    }

    #[test]
    fn dated_prefix() {
        // 2026-03-15 12:00:00 UTC
        let ms = 1_774_000_000_000i64;
        // Ensure it parses; exact day depends on timestamp — test with known value:
        // 2026-01-01T00:00:00Z = 1767225600
        let jan1 = 1_767_225_600_000i64;
        assert!(matches_dated("2026", jan1));
        assert!(matches_dated("2026-01", jan1));
        assert!(matches_dated("2026-01-01", jan1));
        assert!(!matches_dated("2025-12", jan1));
        let _ = ms;
    }
}
