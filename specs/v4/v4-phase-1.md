# V4 Phase 1: Tab Restructuring + Schema

## Overview

Move Audit Log to position 0, create the Tax tab shell at position 9, add the
tax_tags table with reason column, implement form configuration, add `m` key for
memo editing on JE tab, and hide per-line memos from JE UI.

## Context Files

| File | Why |
|------|-----|
| `CLAUDE.md` | Coding style, verification |
| `specs/v4/v4-SPEC.md` | Tab Restructuring, Schema Changes, Form Classifications |
| `specs/v4/v4-progress.md` | Decisions log |
| `src/tabs/mod.rs` | Tab trait, TabId enum |
| `src/app/mod.rs` | EntityContext::new(), tab vector |
| `src/app/key_dispatch.rs` | Tab index routing, help overlay |
| `src/tabs/journal_entries.rs` | JE tab — add `m` key, hide line_memo |
| `src/widgets/je_form.rs` | JE form — hide line_memo field |
| `src/config.rs` | Entity TOML config |

## Tasks

### Task 1: Move Audit Log to Position 0

**Files:** `src/tabs/mod.rs`, `src/app/mod.rs`, `src/app/key_dispatch.rs`

Change `TabId::AuditLog` index from 8 to 0. Reorder tab vector in
`EntityContext::new()`. Update `0` key mapping. Update help overlay to show
`0` for Audit Log, `1-9` for the rest.

**Commit:** `V4 Phase 1, Task 1: move Audit Log tab to position 0`

---

### Task 2: Create tax_tags Table and TaxTag Types

**Files:** `src/db/schema.rs`, `src/db/tax_tag_repo.rs` (new), `src/db/mod.rs`, `src/types/enums.rs`

Add `tax_tags` table to `initialize_schema()` with `reason TEXT` column.

Add enums:
- `TaxReviewStatus`: `Unreviewed`, `AiPending`, `AiSuggested`, `Confirmed`, `NonDeductible`
- `TaxFormTag`: all 13 variants + `NonDeductible`. All need `FromStr`/`Display`/`Copy`.

`TaxFormTag` needs helper methods:
- `fn all() -> Vec<TaxFormTag>` — returns all variants
- `fn display_name(&self) -> &str` — human-readable name (e.g., "Schedule C")
- `fn description(&self) -> &str` — short description for picker

Create `TaxTagRepo`:
```rust
pub fn get_for_je(&self, je_id: JournalEntryId) -> Result<Option<TaxTag>>
pub fn set_manual(&self, je_id: JournalEntryId, form_tag: TaxFormTag, reason: Option<&str>) -> Result<()>
pub fn set_ai_pending(&self, je_id: JournalEntryId) -> Result<()>
pub fn set_ai_suggested(&self, je_id: JournalEntryId, form: TaxFormTag, reason: &str) -> Result<()>
pub fn accept_suggestion(&self, je_id: JournalEntryId) -> Result<()>
pub fn set_non_deductible(&self, je_id: JournalEntryId, reason: Option<&str>) -> Result<()>
pub fn get_pending(&self) -> Result<Vec<TaxTag>>
pub fn list_for_date_range(&self, start: NaiveDate, end: NaiveDate) -> Result<Vec<TaxTagWithJe>>
```

`set_manual` and `set_non_deductible` use UPSERT (INSERT ON CONFLICT UPDATE) so
they work on any existing status.

Add accessor: `pub fn tax_tags(&self) -> TaxTagRepo`

**Tests:** CRUD, status transitions, re-flagging from any status, enum round-trips.

**Commit:** `V4 Phase 1, Task 2: tax_tags table, enums, and TaxTagRepo`

---

### Task 3: Create Tax Tab Shell

**Files:** `src/tabs/tax.rs` (new), `src/tabs/mod.rs`, `src/app/mod.rs`

Create stub tab implementing `Tab` trait:
- `title()` → `"Tax"`
- `render()` → placeholder text
- `TabId::Tax` at index 9

Wire into `EntityContext::new()`.

**Commit:** `V4 Phase 1, Task 3: create Tax tab shell at position 9`

---

### Task 4: Form Configuration + Entity TOML

**Files:** `src/config.rs`, `src/tabs/tax.rs`

Add `[tax]` section to entity TOML: `enabled_forms: Option<Vec<String>>`.
When absent, all forms enabled.

Add `c` key handler in Tax tab — modal with `[✓]`/`[ ]` toggles for each form.
Space toggles, Enter saves to entity TOML via `toml_edit`, Esc cancels.

**Commit:** `V4 Phase 1, Task 4: form configuration screen and entity TOML`

---

### Task 5: Memo Editing + Hide Per-Line Memo

**Files:** `src/tabs/journal_entries.rs`, `src/widgets/je_form.rs`, `src/tabs/tax.rs`

Add `m` key to JE tab: opens `TextInputModal` pre-filled with current JE memo.
On submit, updates `journal_entries.memo` via JournalRepo.

Hide `line_memo`/`Note` column from JE form and JE detail view. Column stays in
schema, just not displayed or editable.

The Tax tab will also use `m` (wired in Phase 3).

**Commit:** `V4 Phase 1, Task 5: memo editing on JE tab and hide per-line memo`

---

## Phase Completion Checklist

- [ ] Audit Log at position 0 via `0` key
- [ ] Tax tab at position 9 via `9` key
- [ ] `tax_tags` table with `reason` column in fresh DBs
- [ ] `TaxReviewStatus` and `TaxFormTag` enums with `FromStr`/`Display`
- [ ] `TaxTagRepo` methods all work, re-flagging from any status works
- [ ] Form config toggles on/off, saves to entity TOML
- [ ] All forms enabled by default when config absent
- [ ] `m` key edits memo on JE tab
- [ ] Per-line memo hidden from JE UI
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass
