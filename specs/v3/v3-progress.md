# V3 Progress Tracker

## Current State
- **Active Phase**: Complete — released as v0.4.0
- **Last Completed Task**: Phase 4, Task 3
- **Release**: v0.4.0 (2026-03-24)
- **Blockers**: None

## Completed Phases
_(All V3 phases complete — released as v0.4.0)_

## Phase 1 Progress
- [x] Task 1: Create junction table and migration
- [x] Task 2: Create ImportRefRepo
- [x] Task 3: Migrate all import_ref usage to junction table

## Phase 2 Progress
- [x] Task 1: Add transfer match query to JournalRepo
- [x] Task 2: Integrate transfer detection into Pass 1

## Phase 3 Progress
- [x] Task 1: Add transfer match state to import flow
- [x] Task 2: Render transfer matches section in review screen
- [x] Task 3: Handle key events for transfer matches

## Phase 4 Progress
- [x] Task 1: Process confirmed matches during draft creation
- [x] Task 2: End-to-end integration test for transfer detection
- [x] Task 3: Update documentation for transfer detection

## Decisions & Discoveries

- **[Pre-implementation]**: Transfer detection matches on negated amount within ±$3
  and date within 3 calendar days. No account type filtering — catches bank-to-bank,
  bank-to-credit-card, and any other inter-account transfers.

- **[Pre-implementation]**: Single match → flagged in review screen. Multiple matches →
  sent to Pass 2 for AI resolution. Avoids false positives from ambiguous local matching.

- **[Pre-implementation]**: Confirmed matches create no new draft. Only a second
  import_ref is stored in the junction table. Existing draft is untouched — user fixes
  categorization during normal draft review if needed.

- **[Pre-implementation]**: `import_ref` column replaced by `journal_entry_import_refs`
  junction table. Supports multiple import_refs per JE. Schema migration runs on
  EntityDb::open() (same pattern as existing column migrations).

- **[Pre-implementation]**: Detection runs in Pass 1, before AI calls. Saves API costs
  by removing transfer matches from the unmatched pool before Pass 2.

- **[Pre-implementation]**: User can delete existing DB and start fresh — no need to
  preserve backward compatibility with old data, only schema migration for the structure.

- **[Phase 1, Task 1]**: Combined schema changes with SQL updates in journal_repo.rs
  and inter_entity/recovery.rs in Task 1 to keep tests passing. Tasks 1 and 3 are
  interdependent — removing the column without updating the SQL would break tests.
  This is the practical sequencing when schema changes and query updates are tightly coupled.

- **[Phase 1, Task 1]**: Migration uses `PRAGMA foreign_keys=OFF` + table rebuild pattern
  (CREATE new → INSERT SELECT → DROP old → RENAME). Standard SQLite approach for dropping
  columns. All FK values (ids) are preserved so referencing tables remain valid.

- **[Phase 1, Task 1]**: `JournalEntry.import_ref: Option<String>` field retained in the
  Rust struct, now populated via correlated subquery `(SELECT import_ref FROM
  journal_entry_import_refs WHERE journal_entry_id = journal_entries.id LIMIT 1)`.
  Returns only the first import_ref for the JE — sufficient for all current callers
  (re-match, /match command, etc.) which only need one ref to reconstruct the transaction.

- **[Phase 1, Task 2]**: `ImportRefRepo` uses both `conn` borrows and is constructed
  inside `JournalRepo::create_draft_with_import_ref` from the same `&Connection`.
  No separate import_refs accessor needed from the JE repo — both repos borrow the same conn.

- **[Phase 1, Task 3]**: After Task 1 changes, remaining callers in key_dispatch.rs,
  ai_handler.rs, and tabs/journal_entries.rs all access `JournalEntry.import_ref` (the Rust
  field), which is now populated from the junction table subquery. No code changes needed
  in those files. grep confirms zero direct column reads on journal_entries.import_ref.

- **[Phase 2, Task 1]**: `TransferMatch` struct added to `journal_repo.rs`. The `find_transfer_matches`
  query uses `(debit_amount - credit_amount) BETWEEN lower AND upper` to match the signed line amount
  against the negated input amount ±$3 tolerance. Results are deduplicated by `je_id` in Rust since
  multiple lines of the same JE may satisfy the filter.

- **[Phase 2, Task 1]**: Test helper `make_transfer_draft` uses `get_next_je_number()` to avoid
  UNIQUE constraint failures when called multiple times in the same test.

- **[Phase 2, Task 2]**: `MatchSource::TransferMatch` added as a unit variant (keeps `Copy` derive).
  Transfer match details stored in `ImportMatch::transfer_match: Option<TransferMatch>` field.
  This keeps the sections array in `build_review_rows` intact — TransferMatch items are simply
  invisible in all four existing sections until Phase 3 adds the dedicated section.

- **[Phase 2, Task 2]**: Three guards added in `import_handler.rs`: (1) `has_unmatched` excludes
  TransferMatch so they don't trigger Pass 2; (2) `unmatched_indices` in `run_pass2_step` also
  excludes them; (3) the Creating loop skips TransferMatch items (no new draft created — wiring
  is Phase 4).

- **[Phase 3, Task 1]**: `TransferMatchRow` struct placed in `csv_import.rs` alongside `ImportFlowState`
  so the state type stays in one file. No separate `transfer_selected` field added — unified
  `selected_index` drives navigation for both transfer and normal rows via the integrated
  `build_review_rows` list. This simplifies navigation without needing a section-tracking bool.

- **[Phase 3, Task 2]**: Transfer matches rendered as `TransferHeader` + `TransferItem` rows at the
  top of `build_review_rows`, before `ApproveAction`. Header is magenta/bold; items show ✓ (green)
  or ✗ (red) indicator. Detail pane shows the action that will be taken (skip or send to Pass 2).

- **[Phase 3, Task 3]**: Enter/Space toggled for `TransferItem` in the unified `match row` arm.
  `ApproveAction` restricted to Enter only (Space excluded) to avoid accidental approval while
  toggling transfer matches. Navigation between sections works automatically since transfer items
  occupy the first N indices in the row list. 7 new tests added (711 total).

- **[Phase 4, Task 1]**: `transfer_matches_snapshot` added to the extract tuple in `run_draft_creation_step`
  alongside `matches_snapshot`. Confirmed matches call `import_refs().insert()` inside a
  `'transfers:` loop (separate from `'batch:`). Rejected matches call `create_draft_with_import_ref`
  with just the bank line — V3 simplification documented in spec and code comment.

- **[Phase 4, Task 1]**: Status message updated to include "N transfer match(es) linked." suffix
  when confirmed_transfer_count > 0.

- **[Phase 4, Task 2]**: `NormalizedTransaction` is defined in `src/ai/mod.rs` (not `csv_import.rs`);
  use `crate::ai::NormalizedTransaction` in tests. `JournalRepo` uses `get_with_lines(id)` not `get(id)`.

- **[Phase 4, Task 2]**: `make_transfer_draft_je` helper added to `integration_tests.rs` for
  end-to-end tests; constructs a single-line draft JE matching the transfer detection pattern
  (bank line only, with import_ref).

## Known Issues
- None currently.
