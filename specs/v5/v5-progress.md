# V5 Progress Tracker

## Current State
- **Active Phase**: Phase 3 — complete
- **Last Completed Task**: Phase 3, Task 3
- **Next Task**: None (V5 complete — awaiting release v0.6.0)
- **Blockers**: None

## Completed Phases
- [x] V5 Phase 1: Schema + Migration + Repo Changes
- [x] V5 Phase 2: Fill Algorithm + Allocation Config UI
- [x] V5 Phase 3: Testing + Documentation_

## Current Phase Progress

### Phase 1: Schema + Migration + Repo Changes
- [x] Task 1: add secondary_percentage and cap_amount columns with migration
- [x] Task 2: update EnvelopeRepo with secondary percentage and cap
- [x] Task 3: update all EnvelopeAllocation consumers for new fields

### Phase 2: Fill Algorithm + Allocation Config UI
- [x] Task 1: two-tier envelope fill algorithm with caps and overflow
- [x] Task 2: update Allocation Config view with new columns and totals
- [x] Task 3: sequential editing for primary, cap, and secondary allocation

### Phase 3: Testing + Documentation
- [x] Task 1: integration tests for two-tier envelope fill
- [x] Task 2: update Envelope Budget Summary report with new fields
- [x] Task 3: update CLAUDE.md, user guide, and progress tracking

## Decisions & Discoveries

- **[Pre-implementation]**: Primary allocations ≤ 100%, secondary allocations ≤ 100%.
  Both enforced on save. An account can have both.

- **[Pre-implementation]**: Cap only gates primary fills. Secondary fills ignore caps.
  An account at cap still receives secondary fills (goes over cap).

- **[Pre-implementation]**: Overflow pool = sum of blocked primary fills from capped
  accounts. Distributed by secondary percentages. Remainder is unearmarked.

- **[Pre-implementation]**: Available clamps to $0. No negative display. Overspending
  means the user manually transfers from another envelope.

- **[Pre-implementation]**: Schema uses ALTER TABLE ADD COLUMN migration. Existing
  allocations get secondary_percentage=0, cap_amount=NULL (no behavior change).

- **[Pre-implementation]**: Column name `percentage` stays as-is in the DB. UI labels
  it "Primary %" for clarity.

- **[Pre-implementation]**: Editing uses sequential prompts (Primary % → Cap → Secondary %)
  rather than a multi-field form. Simpler implementation.

- **[Pre-implementation]**: No weekly/monthly/annual cap periods. Cap is a total amount.
  Spending reduces Available; when Available is under cap, primary fills resume.

- **[Phase 1, Task 2]**: `set_allocation` signature change is a breaking change for all
  callers. Tasks 2 and 3 were implemented together in one session to keep the working
  directory in a compilable state throughout; committed separately for history clarity.
  The Task 2 commit alone would not compile standalone — acceptable tradeoff vs. one
  large commit.

- **[Phase 1, Task 2]**: `row_to_allocation` updated to read columns at indices 3
  (secondary_percentage) and 4 (cap_amount). Query selects 7 columns total; created_at
  is now at index 5, updated_at at index 6.

- **[Phase 1, Task 3]**: All consumers pass `Percentage(0), None` for the new fields,
  preserving existing behavior. Phase 2 will add the UI to set real values.

- **[Phase 2, Task 1]**: Split `services/journal.rs` into `journal/mod.rs` + `journal/tests.rs`
  before adding new tests (file was at 1375 lines; new tests would exceed 1500-line limit).
  This is a pure reorganization — no public API changes.

- **[Phase 2, Task 1]**: The two-tier algorithm calls `env.get_balance(account_id)` inside
  the transaction for cap checking. Since each account has at most one allocation, and the
  fill is written AFTER the check for that account, the cap check always sees the pre-posting
  balance. No ordering issues.

- **[Phase 2, Task 2]**: `AllocState` is a simple local struct (not the full
  `EnvelopeAllocation` repo type) to keep the tab's state lean. It carries only the three
  fields needed for display and editing.

- **[Phase 2, Task 3]**: Editing is triggered by `Enter` key (not `d`). The spec says "d key
  (distribute/edit allocation)" but the existing code uses `Enter` for edit and `d` for
  remove. Followed the existing implementation pattern.

- **[Phase 2, Task 3]**: Cap = $0 is treated as "no cap" (same as empty). A cap of $0 would
  permanently block all primary fills, which is unusual. The spec says "Empty/0 clears the cap."

- **[Phase 3, Task 1]**: Integration tests placed in `tests_v5.rs` (sibling module) rather than
  `tests.rs` to avoid pushing `tests.rs` past the 1,500-line limit (it was at 1,426 lines).
  Added `#[cfg(test)] mod tests_v5;` to `journal/mod.rs`. Two new tests added: full four-account
  two-tier scenario and resume-after-spend. Scenarios 1, 2, 4, 5 from the spec were already
  covered by existing tests from Phase 2, Task 1.

- **[Phase 3, Task 2]**: Envelope Budget Summary report updated with new columns (Primary %,
  Secondary %, Cap). Old "Allocation %" column replaced. Zero % values display as em dash (`—`).
  Available column now clamps to $0. Totals row shows both primary and secondary percentage sums.

- **[Phase 3, Task 3]**: User guide Envelopes section rewritten to document two-tier fill
  behavior with an example, new Allocation Config column table, and sequential editing flow.
  Envelope Budget Summary report description in Reports section updated to mention new columns.
  CLAUDE.md updated with V5 specs table, key decisions, and gotchas section.

## Known Issues
- None currently.
