# V5 Progress Tracker

## Current State
- **Active Phase**: Phase 1 — complete
- **Last Completed Task**: Phase 1, Task 3
- **Next Task**: Phase 2, Task 1
- **Blockers**: None

## Completed Phases
_(none fully released yet)_

## Current Phase Progress

### Phase 1: Schema + Migration + Repo Changes
- [x] Task 1: add secondary_percentage and cap_amount columns with migration
- [x] Task 2: update EnvelopeRepo with secondary percentage and cap
- [x] Task 3: update all EnvelopeAllocation consumers for new fields

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

## Known Issues
- None currently.
