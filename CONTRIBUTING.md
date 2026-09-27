# Contributing to tpt-finder

Thanks for your interest in contributing!

## Licensing (SPDX headers)

tpt-finder is dual-licensed under **MIT OR Apache-2.0** (TPT Solutions copyright).

Every new source file should carry an SPDX header as its first comment line(s):

### Rust (`.rs`)

```rust
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions
```

### TypeScript / JavaScript (`.ts`, `.js`)

```ts
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions
```

### HTML (`.html`)

```html
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
<!-- Copyright 2026 TPT Solutions -->
```

### CSS (`.css`)

```css
/* SPDX-License-Identifier: MIT OR Apache-2.0 */
/* Copyright 2026 TPT Solutions */
```

Do **not** add per-file copyright assignment language beyond the standard
header above; the repository-level `LICENSE-MIT`, `LICENSE-APACHE`, and
`NOTICE` files govern the overall licensing.

## Development

See the [README](README.md) for build instructions.

## Pull requests

- Keep changes focused and scoped.
- Run `cargo fmt`, `cargo clippy -- -D warnings`, `npm run lint`, and
  `npm run typecheck` before submitting.
- Ensure CI passes on Windows and Linux runners.

## Bug reports & triage (post-release)

1. **Filing**: open a GitHub issue using the bug template — include platform (Windows/Linux), tpt-finder version, steps to reproduce, and expected vs actual behavior. Attach `error.log` from the app data dir **only if you explicitly enabled it** (it is opt-in and local-only by design).
2. **Triage SLA**: a maintainer labels new issues within a week: `bug` / `regression` / `perf` / `security`, plus priority (`P0` data loss or crash-on-start, `P1` core search broken, `P2` workaround exists, `P3` polish). `security` issues go to private security advisory instead of public issues.
3. **Reproduce first**: confirm the bug on a current build before assigning a milestone; can't-reproduce issues get the `need-info` label and auto-close after 30 days of inactivity.
4. **Fixes**: reference the issue in the PR (`Fixes #N`), add a regression test when practical, and record user-visible changes under the **Unreleased** section of [CHANGELOG.md](CHANGELOG.md) following [Keep a Changelog](https://keepachangelog.com/) + SemVer.
