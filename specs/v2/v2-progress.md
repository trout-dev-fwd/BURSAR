# V2 Progress

## Current State

- **Active Phase:** Phase 2 — AI Client & Chat Panel
- **Last Completed Task:** Phase 2, Task 11 — Help Overlay Update
- **Next Task:** Phase 2, Task 12 — Loading State Status Bar Messages
- **Blockers:** None
- **Prerequisites:** V1 complete (372 tests passing), draft editing feature merged

---

## Phase 1 — Foundation (12/12) ✅

- [x] Task 1: New Enums and Types [TEST-FIRST]
- [x] Task 2: New Struct Types [TEST-FIRST]
- [x] Task 3: Configuration Extensions [TEST-FIRST]
- [x] Task 4: Context File Management [TEST-FIRST]
- [x] Task 5: Schema — import_mappings Table [TEST-FIRST]
- [x] Task 6: ImportMappingRepo [TEST-FIRST]
- [x] Task 7: Schema — import_ref Column Migration
- [x] Task 8: Journal Repo — Import Queries
- [x] Task 9: Envelopes Tab — Replace Tab with V
- [x] Task 10: JE Form — Arrow Key Navigation
- [x] Task 11: Audit Repo — AI Entry Convenience Methods
- [x] Task 12: CSV Parser Stub [TEST-FIRST]

**Phase 1 Review:** ✅ Complete — 487 tests passing (115 new from Phase 1)

---

## Phase 2 — AI Client & Chat Panel (11/12)

- [x] Task 1: Add ureq Dependency
- [x] Task 2: AI Client — Core Request/Response [TEST-FIRST]
- [x] Task 3: AI Client — Tool Use Loop [TEST-FIRST]
- [x] Task 4: Tool Definitions [TEST-FIRST]
- [x] Task 5: Tool Fulfillment Handlers
- [x] Task 6: Chat Panel Widget — Structure and Rendering
- [x] Task 7: Chat Panel Widget — Key Handling
- [x] Task 8: App Integration — Focus Model and Layout
- [x] Task 9: App Integration — AI Request Orchestration
- [x] Task 10: Slash Command Execution
- [x] Task 11: Help Overlay Update
- [ ] Task 12: Loading State Status Bar Messages

**Phase 2 Review:** ⬜ Pending

---

## Phase 3 — CSV Import Pipeline (0/13)

- [ ] Task 1: Import Flow State and TabAction Extension
- [ ] Task 2: Import Wizard — File Path Input Modal
- [ ] Task 3: Import Wizard — Bank Selection Modal
- [ ] Task 4: Import Wizard — New Bank Setup (Name + Detection)
- [ ] Task 5: Import Wizard — Confirmation + Account Picker
- [ ] Task 6: Duplicate Detection Step
- [ ] Task 7: Pass 1 — Local Matching
- [ ] Task 8: Pass 2 — AI Matching
- [ ] Task 9: Pass 3 — Clarification Dialog
- [ ] Task 10: Review Screen
- [ ] Task 11: Draft Creation
- [ ] Task 12: Batch Re-Match (Shift+U) and /match Completion
- [ ] Task 13: Help Overlay + Final Polish

**Phase 3 Review:** ⬜ Pending

---

## Decisions & Discoveries

_Record architectural decisions, trade-offs, and unexpected findings during implementation._

| Date | Phase.Task | Decision / Discovery |
|------|-----------|---------------------|
| 2026-03-17 | 1.7 | Added `import_ref TEXT` to schema `CREATE TABLE` (not just migration) to fix in-memory test DBs |
| 2026-03-17 | 1.8 | `account_id NOT NULL` on `journal_entry_lines` means `get_incomplete_imports` checks total line count (< 2), not null account_id |
| 2026-03-17 | 1.10 | Down/Up navigate between rows (same column type); Left/Right navigate between columns within a row |
| 2026-03-17 | 1.12 | parse_money_str uses integer arithmetic only (no f64 intermediate) per spec |
| 2026-03-17 | Review | **Phase 3 draft creation strategy: single-line drafts (Option A).** Unmatched imports create a draft JE with only the bank account line (one line). The contra line is added after matching resolves (via update_draft or manual edit). Rationale: drafts don't require balanced debits/credits (enforced at post time only); `get_incomplete_imports` correctly identifies these as entries with < 2 lines; no schema changes needed (`account_id` stays NOT NULL); no sentinel accounts polluting the CoA. |

---

## Known Issues

_Track bugs, edge cases, or technical debt discovered during implementation._

| Issue | Severity | Phase | Status |
|-------|----------|-------|--------|
| | | | |

---

## Test Count

| Phase | New Tests | Running Total |
|-------|-----------|---------------|
| V1 | 372 | 372 |
| V2 Phase 1 | 115 | 487 |
| V2 Phase 2 (Tasks 1–11) | 97 | 584 |
| V2 Phase 3 | — | — |
