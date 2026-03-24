# V3 Progress Tracker

## Current State
- **Active Phase**: Phase 1 — Schema Migration (complete)
- **Last Completed Task**: Phase 1, Task 3
- **Next Task**: Phase 2, Task 1
- **Blockers**: None

## Completed Phases
_(none fully completed yet — Phase 1 tasks done, awaiting developer sign-off)_

## Phase 1 Progress
- [x] Task 1: Create junction table and migration
- [x] Task 2: Create ImportRefRepo
- [x] Task 3: Migrate all import_ref usage to junction table

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

## Known Issues
- None currently.
