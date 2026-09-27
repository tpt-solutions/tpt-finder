// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Text chunking for embeddings (paragraph-aware with hard size caps).

/// Split text into embedding-sized chunks.
///
/// Strategy: accumulate paragraphs until `max_chars`, hard-split any single
/// paragraph longer than `max_chars` on sentence/space boundaries.
pub fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(64);
    let mut chunks = Vec::new();
    let mut buf = String::new();

    for para in text.split('\n') {
        let para = para.trim();
        if para.is_empty() {
            // Paragraph boundary: flush if we have content.
            if !buf.trim().is_empty() {
                chunks.push(std::mem::take(&mut buf));
            }
            continue;
        }

        if para.len() > max_chars {
            // Flush current, then hard-split the long paragraph.
            if !buf.trim().is_empty() {
                chunks.push(std::mem::take(&mut buf));
            }
            for piece in hard_split(para, max_chars) {
                chunks.push(piece);
            }
            continue;
        }

        if buf.len() + para.len() + 1 > max_chars {
            if !buf.trim().is_empty() {
                chunks.push(std::mem::take(&mut buf));
            }
            buf.push_str(para);
        } else {
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(para);
        }
    }

    if !buf.trim().is_empty() {
        chunks.push(buf);
    }
    chunks
}

fn hard_split(s: &str, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    // Prefer sentence boundaries, fall back to spaces, fall back to bytes.
    for sentence in s.split_inclusive(['.', '!', '?']) {
        if sentence.len() > max_chars {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            for word in sentence.split_inclusive(' ') {
                if cur.len() + word.len() > max_chars && !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                // Extreme fallback: slice on char boundary.
                if word.len() > max_chars {
                    let mut start = 0;
                    while start < word.len() {
                        let mut end = (start + max_chars).min(word.len());
                        while end > start && !word.is_char_boundary(end) {
                            end -= 1;
                        }
                        out.push(word[start..end].to_string());
                        start = end;
                    }
                } else {
                    cur.push_str(word);
                }
            }
            continue;
        }
        if cur.len() + sentence.len() > max_chars && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push_str(sentence);
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text() {
        assert!(chunk_text("", 256).is_empty());
        assert!(chunk_text("   \n\n  ", 256).is_empty());
    }

    #[test]
    fn short_text_single_chunk() {
        let chunks = chunk_text("Hello world.\nSecond paragraph.", 256);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("Hello world."));
        assert!(chunks[0].contains("Second paragraph."));
    }

    #[test]
    fn respects_max_chars() {
        let text = "word ".repeat(500); // 2500 chars, one "paragraph" (no newlines until end)
        let chunks = chunk_text(&text, 256);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.len() <= 256 + 64, "chunk too long: {}", c.len());
        }
        // Content preserved (approximately — splitting may trim boundary spaces).
        let joined: String = chunks.join(" ");
        assert!(joined.contains("word word word"));
    }

    #[test]
    fn splits_long_sentences() {
        let text = "A".repeat(1000);
        let chunks = chunk_text(&text, 100);
        assert!(chunks.len() >= 10);
        for c in &chunks {
            assert!(c.len() <= 100);
        }
    }

    #[test]
    fn multiple_paragraphs_split_when_over_budget() {
        let mut parts = Vec::new();
        for i in 0..20 {
            parts.push(format!("Paragraph number {i} with some filler text."));
        }
        let text = parts.join("\n\n");
        let chunks = chunk_text(&text, 80);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.len() <= 80 + 100, "len {}", c.len());
        }
    }
}
