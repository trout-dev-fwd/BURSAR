# V4 Phase 2: Tax Reference Library — Ingestion

## Overview

Add the `tax_reference` table, fetch IRS publications as HTML, parse into section
chunks, and store in SQLite. Triggered by `u` hotkey in the Tax tab.

## Context Files

| File | Why |
|------|-----|
| `CLAUDE.md` | Coding style, verification |
| `specs/v4/v4-SPEC.md` | IRS Tax Reference Library section, Publication List |
| `specs/v4/v4-progress.md` | Decisions log |
| `src/db/schema.rs` | Schema additions |
| `src/db/mod.rs` | EntityDb accessors |
| `src/tabs/tax.rs` | Tax tab hotkey wiring |

## Tasks

### Task 1: Schema + TaxRefRepo

**Files:** `Cargo.toml`, `src/db/schema.rs`, `src/db/tax_ref_repo.rs` (new), `src/db/mod.rs`

Add `scraper = "0.22"` to Cargo.toml.

Add `tax_reference` table to `initialize_schema()` with `publication`, `section`,
`topic_tags`, `content`, `tax_year`, `ingested_at` columns and indexes.

Create `TaxRefRepo`: `clear()`, `insert()`, `search_by_tag()`, `count()`.
Add accessor: `pub fn tax_refs(&self) -> TaxRefRepo`

**Tests:** Insert/retrieve, search by tag, clear, count.

**Commit:** `V4 Phase 2, Task 1: tax_reference schema, scraper dependency, TaxRefRepo`

---

### Task 2: HTML Fetcher and Parser

**File:** `src/tax_ingestion.rs` (new)

Publication list constant (20 entries) with number, name, topic_tags.

`fetch_and_parse(pub_def) -> Result<Vec<ParsedChunk>, String>`:
GET HTML, parse with `scraper`, split at `<h2>` headings, strip tags. Split
sections >16,000 chars at `<h3>` boundaries. Fallback to `<h3>`, then single chunk.

**Tests:** Parse sample HTML, long section splitting, no headings, tag stripping.

**Commit:** `V4 Phase 2, Task 2: HTML fetcher and parser for IRS publications`

---

### Task 3: Wire Ingestion to Tax Tab Hotkey

**Files:** `src/tabs/tax.rs`, `src/app/mod.rs`

Handle `u` key: show status, force render, begin transaction, clear + fetch/parse/insert
each publication with progress, commit. Failed publications skipped with warning.

**Commit:** `V4 Phase 2, Task 3: wire tax reference ingestion to 'u' hotkey`

---

## Phase Completion Checklist

- [ ] `tax_reference` table in fresh DBs, `scraper` in Cargo.toml
- [ ] HTML parser chunks by `<h2>` correctly, splits long sections at `<h3>`
- [ ] `u` hotkey triggers ingestion with progress display
- [ ] Failed publications skipped, transaction protects existing data
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass
