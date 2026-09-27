// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Per-type extractors: plain text/code, PDF, and Office Open XML documents.

use std::path::Path;

/// Result of a successful extraction.
#[derive(Clone, Debug, Default)]
pub struct Extracted {
    pub text: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub created_ms: Option<i64>,
    pub language: Option<String>,
    /// Extractor name (e.g. "text", "pdf", "docx") for diagnostics.
    pub source: &'static str,
}

#[derive(Debug)]
pub enum ExtractionError {
    Io(std::io::Error),
    Unsupported(String),
    Parse(String),
    TooLarge { size: u64, max: u64 },
    Skipped(&'static str),
}

impl std::fmt::Display for ExtractionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtractionError::Io(e) => write!(f, "io: {e}"),
            ExtractionError::Unsupported(s) => write!(f, "unsupported: {s}"),
            ExtractionError::Parse(s) => write!(f, "parse: {s}"),
            ExtractionError::TooLarge { size, max } => {
                write!(f, "too large: {size} > {max} bytes")
            }
            ExtractionError::Skipped(reason) => write!(f, "skipped: {reason}"),
        }
    }
}

impl std::error::Error for ExtractionError {}

impl From<std::io::Error> for ExtractionError {
    fn from(e: std::io::Error) -> Self {
        ExtractionError::Io(e)
    }
}

/// Hard cap on bytes read into memory for text extraction.
const MAX_TEXT_BYTES: u64 = 16 * 1024 * 1024;
/// Hard cap for PDF/Office sources.
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

/// Extract plain text + metadata from a file based on its type.
pub fn extract_file(path: &Path) -> Result<Extracted, ExtractionError> {
    let meta = std::fs::metadata(path)?;
    if meta.is_dir() {
        return Err(ExtractionError::Skipped("directory"));
    }
    let size = meta.len();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "pdf" => {
            if size > MAX_ARCHIVE_BYTES {
                return Err(ExtractionError::TooLarge {
                    size,
                    max: MAX_ARCHIVE_BYTES,
                });
            }
            extract_pdf(path)
        }
        "docx" => extract_docx(path, size),
        "xlsx" | "xlsm" => extract_xlsx(path, size),
        "pptx" => extract_pptx(path, size),
        "" | "txt" | "md" | "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "c" | "h"
        | "cpp" | "hpp" | "cs" | "java" | "kt" | "swift" | "rb" | "php" | "sh" | "ps1" | "bat"
        | "cmd" | "json" | "yaml" | "yml" | "toml" | "xml" | "html" | "css" | "sql" | "csv"
        | "log" | "ini" | "cfg" | "conf" | "env" | "gitignore" | "dockerfile" | "makefile" => {
            if size > MAX_TEXT_BYTES {
                return Err(ExtractionError::TooLarge {
                    size,
                    max: MAX_TEXT_BYTES,
                });
            }
            extract_text(path)
        }
        _ => {
            // Peek: if the first 8 KiB looks like UTF-8 text, treat as text.
            if size <= MAX_TEXT_BYTES && looks_like_text(path)? {
                extract_text(path)
            } else {
                Err(ExtractionError::Unsupported(format!("extension .{ext}")))
            }
        }
    }
}

fn extract_text(path: &Path) -> Result<Extracted, ExtractionError> {
    let bytes = std::fs::read(path)?;
    let text = decode_bytes(&bytes);
    Ok(Extracted {
        text,
        title: None,
        author: None,
        created_ms: file_created_ms(path),
        language: None,
        source: "text",
    })
}

fn extract_pdf(path: &Path) -> Result<Extracted, ExtractionError> {
    let text =
        pdf_extract::extract_text(path).map_err(|e| ExtractionError::Parse(e.to_string()))?;
    Ok(Extracted {
        text,
        title: None,
        author: None,
        created_ms: file_created_ms(path),
        language: None,
        source: "pdf",
    })
}

// ── Office Open XML (zip + XML) ───────────────────────────────────────

fn extract_docx(path: &Path, size: u64) -> Result<Extracted, ExtractionError> {
    check_archive_size(size)?;
    let mut zip = open_zip(path)?;
    let xml = read_zip_entry(&mut zip, "word/document.xml")?;
    let text = xml_text_content(&xml);
    let (title, author) = zip_core_props(&mut zip, "docProps/core.xml");
    Ok(Extracted {
        text,
        title,
        author,
        created_ms: file_created_ms(path),
        language: None,
        source: "docx",
    })
}

fn extract_xlsx(path: &Path, size: u64) -> Result<Extracted, ExtractionError> {
    check_archive_size(size)?;
    let mut zip = open_zip(path)?;
    // Shared strings first (cell values often reference them).
    let mut parts: Vec<String> = Vec::new();
    if let Ok(ss) = read_zip_entry(&mut zip, "xl/sharedStrings.xml") {
        parts.push(xml_text_content(&ss));
    }
    // Sheet cell inline strings / values.
    for name in zip_entry_names(&mut zip) {
        if name.starts_with("xl/worksheets/") && name.ends_with(".xml") {
            if let Ok(sheet) = read_zip_entry(&mut zip, &name) {
                parts.push(xml_text_content(&sheet));
            }
        }
    }
    let (title, author) = zip_core_props(&mut zip, "docProps/core.xml");
    Ok(Extracted {
        text: parts.join("\n"),
        title,
        author,
        created_ms: file_created_ms(path),
        language: None,
        source: "xlsx",
    })
}

fn extract_pptx(path: &Path, size: u64) -> Result<Extracted, ExtractionError> {
    check_archive_size(size)?;
    let mut zip = open_zip(path)?;
    let mut parts: Vec<String> = Vec::new();
    for name in zip_entry_names(&mut zip) {
        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
            if let Ok(slide) = read_zip_entry(&mut zip, &name) {
                parts.push(xml_text_content(&slide));
            }
        }
        if name.starts_with("ppt/notesSlides/") && name.ends_with(".xml") {
            if let Ok(notes) = read_zip_entry(&mut zip, &name) {
                parts.push(xml_text_content(&notes));
            }
        }
    }
    let (title, author) = zip_core_props(&mut zip, "docProps/core.xml");
    Ok(Extracted {
        text: parts.join("\n"),
        title,
        author,
        created_ms: file_created_ms(path),
        language: None,
        source: "pptx",
    })
}

fn check_archive_size(size: u64) -> Result<(), ExtractionError> {
    if size > MAX_ARCHIVE_BYTES {
        return Err(ExtractionError::TooLarge {
            size,
            max: MAX_ARCHIVE_BYTES,
        });
    }
    Ok(())
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, ExtractionError> {
    let f = std::fs::File::open(path)?;
    zip::ZipArchive::new(f).map_err(|e| ExtractionError::Parse(e.to_string()))
}

fn zip_entry_names(zip: &mut zip::ZipArchive<std::fs::File>) -> Vec<String> {
    (0..zip.len())
        .filter_map(|i| zip.name_for_index(i).map(|s| s.to_string()))
        .collect()
}

fn read_zip_entry(
    zip: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
) -> Result<String, ExtractionError> {
    let mut file = zip
        .by_name(name)
        .map_err(|e| ExtractionError::Parse(format!("{name}: {e}")))?;
    let mut buf = Vec::new();
    use std::io::Read;
    file.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Extract all character data from an XML document (skips tags).
fn xml_text_content(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    use quick_xml::events::Event;
    loop {
        match reader.read_event() {
            Ok(Event::Text(t)) => {
                // BytesText supports unescape() → Cow<str> in quick-xml 0.36.
                match t.unescape() {
                    Ok(decoded) => {
                        out.push_str(&decoded);
                        out.push(' ');
                    }
                    Err(_) => {
                        let raw = String::from_utf8_lossy(t.as_ref());
                        out.push_str(&raw);
                        out.push(' ');
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    // Collapse whitespace runs.
    let collapsed: Vec<&str> = out.split_whitespace().collect();
    collapsed.join(" ")
}

/// Best-effort core.xml → (title, author).
fn zip_core_props(
    zip: &mut zip::ZipArchive<std::fs::File>,
    entry: &str,
) -> (Option<String>, Option<String>) {
    let Ok(xml) = read_zip_entry(zip, entry) else {
        return (None, None);
    };
    let title = xml_tag_value(&xml, "dc:title");
    let author =
        xml_tag_value(&xml, "dc:creator").or_else(|| xml_tag_value(&xml, "cp:lastModifiedBy"));
    (title, author)
}

fn xml_tag_value(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let v = xml[start..end].trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

fn decode_bytes(bytes: &[u8]) -> String {
    // Strip UTF-8 BOM.
    let bytes = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };
    // UTF-16 BOM heuristics.
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (s, _, _) = encoding_rs::UTF_16LE.decode(bytes);
        return s.into_owned();
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (s, _, _) = encoding_rs::UTF_16BE.decode(bytes);
        return s.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            // Windows-1252 fallback (common on legacy docs/logs).
            let (s, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
            s.into_owned()
        }
    }
}

fn looks_like_text(path: &Path) -> Result<bool, ExtractionError> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 8192];
    let n = f.read(&mut buf)?;
    let slice = &buf[..n];
    if slice.is_empty() {
        return Ok(true);
    }
    // Null byte ⇒ binary.
    if slice.contains(&0) {
        return Ok(false);
    }
    // High ratio of non-printable non-whitespace bytes ⇒ binary.
    let non_print = slice
        .iter()
        .filter(|b| {
            let b = **b;
            b < 0x09 || (b > 0x0D && b < 0x20) || b == 0x7F
        })
        .count();
    Ok(non_print * 10 < slice.len())
}

fn file_created_ms(path: &Path) -> Option<i64> {
    let meta = std::fs::metadata(path).ok()?;
    let created = meta.created().ok()?;
    let dur = created.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(name: &str, data: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tpt-extract-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(data).unwrap();
        p
    }

    #[test]
    fn extract_plain_text() {
        let p = write_temp("hello.txt", b"Hello, extraction pipeline!\nSecond line.");
        let out = extract_file(&p).unwrap();
        assert!(out.text.contains("Hello, extraction pipeline!"));
        assert_eq!(out.source, "text");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn extract_code_file() {
        let p = write_temp("main.rs", b"fn main() { println!(\"hi\"); }");
        let out = extract_file(&p).unwrap();
        assert!(out.text.contains("fn main"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn binary_ext_unsupported() {
        let p = write_temp("blob.dll", &[0x00, 0x01, 0x02, 0x03, 0x00, 0xFF]);
        let err = extract_file(&p).unwrap_err();
        assert!(matches!(err, ExtractionError::Unsupported(_)));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn directory_skipped() {
        let err = extract_file(Path::new("/")).unwrap_err();
        assert!(matches!(err, ExtractionError::Skipped(_)));
    }

    #[test]
    fn docx_roundtrip_minimal() {
        // Build a minimal docx (zip with word/document.xml).
        let dir = std::env::temp_dir().join(format!("tpt-docx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("sample.docx");
        {
            let f = std::fs::File::create(&p).unwrap();
            let mut zw = zip::ZipWriter::new(f);
            let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            zw.start_file("word/document.xml", opts).unwrap();
            let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body><w:p><w:r><w:t>Quarterly invoice for Acme</w:t></w:r></w:p></w:body>
</w:document>"#;
            std::io::Write::write_all(&mut zw, xml.as_bytes()).unwrap();
            zw.start_file("docProps/core.xml", opts).unwrap();
            let core = r#"<?xml version="1.0"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Invoice</dc:title><dc:creator>Jane Doe</dc:creator></cp:coreProperties>"#;
            std::io::Write::write_all(&mut zw, core.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        let out = extract_file(&p).unwrap();
        assert!(out.text.contains("Quarterly invoice for Acme"));
        assert_eq!(out.title.as_deref(), Some("Invoice"));
        assert_eq!(out.author.as_deref(), Some("Jane Doe"));
        assert_eq!(out.source, "docx");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn xml_text_content_strips_tags() {
        let xml = "<a>Hi <b>there</b> friend</a>";
        let t = xml_text_content(xml);
        assert_eq!(t, "Hi there friend");
    }
}
