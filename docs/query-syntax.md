# Query syntax reference

tpt-finder searches match **as you type** against file and folder names. Whitespace-separated terms are combined with AND.

| Example | Matches |
| --- | --- |
| `report` | names containing `report` (best match ranked first) |
| `invoice 2026` | names containing **both** `invoice` and `2026` |
| `report.pdf` | exact full name ranks highest |

## Filters

Filters can be combined freely with text terms:

| Filter | Example | Meaning |
| --- | --- | --- |
| `ext:` | `ext:pdf invoice` | extension equals `pdf` (dot optional, case-insensitive) |
| `path:` | `path:documents` | full path contains `documents` (case-insensitive) |
| `size:` | `size:>10mb` | size bounds — see operators below |
| `dated:` | `dated:2026-03` | modified date prefix match: `YYYY`, `YYYY-MM`, or `YYYY-MM-DD` |
| `fuzzy:` / `~` | `fuzzy:invc` or `~invc` | subsequence (fuzzy) match for all free-text terms |
| `dirs` / `is:dir` | `report is:dir` | only folders |
| `files` / `is:file` | `report files` | only files |

### Size operators

| Syntax | Meaning |
| --- | --- |
| `size:500` | exactly 500 bytes |
| `size:>10mb` | more than 10 MB |
| `size:>=1gb` | at least 1 GB |
| `size:<100kb` | less than 100 KB |
| `size:<=5mb` | at most 5 MB |

Units: plain bytes, `k`/`kb`, `m`/`mb`, `g`/`gb`, `t`/`tb` (binary multiples).

### Examples

```
ext:pdf invoice dated:2026-03
size:>100mb path:downloads files
~hndbk is:file
ext:rs path:tpt-finder
```

## Match ranking

Results are scored by match quality:

1. **Exact name** (1000)
2. **Name prefix** (800)
3. **Name substring** (600)
4. **Path substring** (400)
5. **Fuzzy subsequence** (200)

## Natural language queries

With the semantic layer enabled (see [ollama-setup.md](ollama-setup.md)), press **NL** in the main window and type plain language:

> the PDF invoice from John last March

A local Ollama chat model parses this into structured filters (`ext:pdf dated:2026-03 …`) plus semantic terms. Natural-language parsing runs **only** on your machine.

## Hybrid ranking

When the semantic layer is enabled, a query merges three signals:

1. filename relevance (the index search described above),
2. full-text relevance (extracted file contents, SQLite FTS5),
3. vector similarity (embedded content via Ollama).

When Ollama is unavailable, tpt-finder silently degrades to filename (+ full-text when available) search — nothing else changes.
