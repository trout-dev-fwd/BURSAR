# V5 Progress Tracker

## Current State
- **Active Phase**: Pre-implementation — Feature spec complete, build specs ready
- **Last Completed Task**: None
- **Next Task**: Phase 1, Task 1
- **Blockers**: None

## Completed Phases
_(none yet)_

## Current Phase Progress
_(see phase files for task lists)_

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

## Known Issues
- None currently.
