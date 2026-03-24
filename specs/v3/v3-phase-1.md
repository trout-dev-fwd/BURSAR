# V3 Phase 1: Schema Migration — Junction Table for Import Refs

## Overview

Replace the `import_ref` column on `journal_entries` with a `journal_entry_import_refs`
junction table. Migrate existing data. Update all queries that read/write import_ref.

## Context Files

| File | Why |
|------|-----|
| `CLAUDE.md` | Coding style, verification commands |
| `specs/v3/v3-SPEC.md` | Schema Changes section |
| `specs/v3/v3-progress.md` | Decisions log |
| `src/db/schema.rs` | Current CREATE TABLE statements |
| `src/db/mod.rs` | EntityDb::open(), existing migrations |
| `src/db/journal_repo.rs` | import_ref queries (duplicate detection, draft creation) |
| `src/db/import_mapping_repo.rs` | May reference import_ref |
| `src/ai/csv_import.rs` | import_ref construction |
| `src/app/import_handler.rs` | import_ref usage during import flow |

## Tasks

### Task 1: Create Junction Table and Migration

**File:** `src/db/schema.rs`, `src/db/mod.rs`

Add to `initialize_schema()`:
```sql
CREATE TABLE IF NOT EXISTS journal_entry_import_refs (
    id INTEGER PRIMARY KEY,
    journal_entry_id INTEGER NOT NULL REFERENCES journal_entries(id),
    import_ref TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(import_ref)
);
```

In `EntityDb::open()`, add migration logic (same pattern as existing column migrations):
1. Check if `journal_entry_import_refs` table exists (`SELECT name FROM sqlite_master`)
2. If not: create the table, copy non-NULL `import_ref` values from `journal_entries`
3. Rebuild `journal_entries` without the `import_ref` column (SQLite requires table
   rebuild for column drops — use CREATE new → INSERT SELECT → DROP old → ALTER RENAME)

Update `initialize_schema()` to remove `import_ref` from the `journal_entries` CREATE TABLE.

**Tests:**
- Fresh DB has junction table, no `import_ref` column on `journal_entries`
- Migration from old schema: create a DB with old schema, insert JEs with import_ref,
  open with new code, verify data migrated to junction table

**Commit:** `V3 Phase 1, Task 1: create import_refs junction table and migration`

---

### Task 2: Create ImportRefRepo

**File:** `src/db/import_ref_repo.rs` (new)

Create a repo for the junction table with methods:

```rust
pub fn insert(&self, je_id: JournalEntryId, import_ref: &str) -> Result<()>
pub fn exists(&self, import_ref: &str) -> Result<bool>
pub fn get_for_je(&self, je_id: JournalEntryId) -> Result<Vec<String>>
pub fn get_je_id(&self, import_ref: &str) -> Result<Option<JournalEntryId>>
```

Add accessor to `EntityDb`: `pub fn import_refs(&self) -> ImportRefRepo`

Export from `src/db/mod.rs`.

**Tests:**
- Insert and retrieve import_ref for a JE
- `exists()` returns true/false correctly
- Duplicate import_ref insert returns error (UNIQUE constraint)
- `get_for_je()` returns multiple refs for same JE

**Commit:** `V3 Phase 1, Task 2: create ImportRefRepo for junction table`

---

### Task 3: Update Existing Code to Use Junction Table

**Files:** `src/db/journal_repo.rs`, `src/app/import_handler.rs`, `src/ai/csv_import.rs`

Find all references to the old `import_ref` column and update:

1. **Duplicate detection** (import_handler.rs) — currently queries `WHERE import_ref = ?`
   on `journal_entries`. Change to query `journal_entry_import_refs` via `ImportRefRepo::exists()`
2. **Draft creation** (journal_repo.rs or import_handler.rs) — currently sets `import_ref`
   on INSERT. Change to: create JE without import_ref, then call `ImportRefRepo::insert()`
3. **Re-match** (`selected_draft_import_ref` on tabs) — if this reads the old column,
   update to query the junction table
4. **AI tools** — check if `get_journal_entry` tool exposes import_ref; if so, update

Run `grep -rn "import_ref" src/` to find all references. Every usage must be migrated.

**Tests:**
- Import a CSV → drafts created → junction table has import_refs → re-import same CSV
  → duplicates detected correctly
- Existing import pipeline tests still pass

**Commit:** `V3 Phase 1, Task 3: migrate all import_ref usage to junction table`

---

## Phase Completion Checklist

- [ ] `journal_entry_import_refs` table exists in fresh DBs
- [ ] Old `import_ref` column removed from `journal_entries`
- [ ] Migration handles old → new schema correctly
- [ ] All import_ref reads/writes use the junction table
- [ ] Duplicate detection works via junction table
- [ ] Draft creation writes to junction table
- [ ] `grep -rn "import_ref" src/` shows no references to the old column (only junction table)
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass
