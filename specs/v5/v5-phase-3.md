# V5 Phase 3: Testing + Documentation

## Overview

End-to-end integration tests for the two-tier fill algorithm, update the
Envelope Budget Summary report, and update all documentation.

## Context Files

| File | Why |
|------|-----|
| `CLAUDE.md` | Needs V5 updates |
| `specs/v5/v5-SPEC.md` | Full feature spec |
| `specs/v5/v5-progress.md` | Task tracking |
| `specs/guide/user-guide.md` | Needs envelope updates |
| `src/reports/envelope_budget.rs` | Envelope Budget Summary report |
| `src/services/journal.rs` | Fill algorithm |
| `src/db/envelope_repo.rs` | Repo methods |

## Tasks

### Task 1: Integration Tests

**File:** `src/services/journal.rs` or appropriate test module

Write integration tests covering the full two-tier fill lifecycle:

1. **Basic scenario:** Set up 3 accounts with primary allocations only (no caps,
   no secondary). Post a cash receipt JE. Verify fills match existing behavior
   exactly. This confirms backward compatibility.

2. **Cap scenario:** Account A at primary 10% with $500 cap. Currently at $450.
   Post $1,000 receipt. Verify: A gets $50 (fills to cap), overflow = $50.
   No secondary → overflow is unearmarked.

3. **Full two-tier scenario:** Account A primary 10% cap $500 (at cap). Account B
   primary 30% no cap. Account C secondary 60%. Account D secondary 40%.
   Post $2,000 receipt. Verify:
   - A: primary $200 blocked (at cap), overflow += $200
   - B: primary $600 fills normally
   - C: secondary 60% of $200 = $120
   - D: secondary 40% of $200 = $80
   - Unearmarked: $2,000 - $600 - $120 - $80 = $1,200 (remaining 60% of receipt
     that wasn't allocated to primary, plus the capped portion was redistributed)

4. **Dual allocation:** Account A has primary 10% (cap $100, at $100) AND
   secondary 5%. Overflow pool from A = $100 (assuming $1,000 receipt).
   A receives secondary: 5% of overflow. Verify A goes above cap.

5. **Reversal:** Post a receipt that triggers two-tier fills. Reverse the JE.
   Verify all fills (primary and secondary) are reversed. Envelope balances
   return to pre-receipt state.

6. **Resume after spend:** Account A at cap ($500). Transfer $100 out of A
   (spend). Post new receipt. Verify: A's primary fill resumes (room under
   cap = $100), fills up to $100 of its primary allocation.

**Commit:** `V5 Phase 3, Task 1: integration tests for two-tier envelope fill`

---

### Task 2: Update Envelope Budget Summary Report

**File:** `src/reports/envelope_budget.rs`

Add the new fields to the report output:

Each account row should show:
- Account number and name
- Primary % (or `—`)
- Secondary % (or `—`)
- Cap (or `—`)
- GL Balance
- Earmarked
- Available (clamped to $0)

Add totals for Primary % and Secondary % at the bottom.

Match the existing box-drawing report style.

**Tests:** Report generates correctly with two-tier allocation data.

**Commit:** `V5 Phase 3, Task 2: update Envelope Budget Summary report with new fields`

---

### Task 3: Update Documentation

**Files:** `CLAUDE.md`, `specs/guide/user-guide.md`, `specs/v5/v5-progress.md`

**CLAUDE.md:**

Add V5 specs table, key decisions:
- Two-tier allocations: primary with cap → overflow → secondary
- Cap only gates primary, not secondary
- Available clamps to $0
- Sequential editing prompts
- Schema: ALTER TABLE migration for new columns

Add V5 gotchas:
- Cap of $0 means primary never fills (always at cap)
- Dual-allocation accounts can go above cap via secondary fills
- Overflow only comes from capped primary allocations, not from unallocated %
- Reversal undoes specific fill amounts, doesn't recalculate tiers

**User guide:**

Update the Envelopes tab section:
- New column descriptions (Avail, Cap, Primary %, Secondary %)
- How to set primary allocation, cap, and secondary allocation
- Explain the two-tier fill behavior with a simple example
- Explain what happens when a cap is reached
- Explain that Available clamps to $0

**v5-progress.md:** Mark all complete.

**Commit:** `V5 Phase 3, Task 3: update CLAUDE.md, user guide, and progress tracking`

---

## Phase Completion Checklist

- [ ] Backward compatibility test passes (primary-only = same as before)
- [ ] Cap scenario fills to cap and calculates overflow correctly
- [ ] Full two-tier scenario distributes overflow to secondary accounts
- [ ] Dual-allocation scenario allows secondary to exceed cap
- [ ] Reversal undoes all primary and secondary fills
- [ ] Resume-after-spend fills to cap correctly
- [ ] Envelope Budget Summary report shows new fields
- [ ] CLAUDE.md updated with V5 decisions and gotchas
- [ ] User guide has complete two-tier envelope documentation
- [ ] v5-progress.md shows all tasks complete
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass

## Post-Review

After Opus review, cut release: `v0.6.0` (minor — new feature).

Test by:
1. Set up envelopes with primary allocations, caps, and secondary allocations
2. Post a cash receipt JE → verify fills respect caps and overflow goes to secondary
3. Spend from a capped envelope → verify primary fills resume on next receipt
4. Generate Envelope Budget Summary report → verify new columns
5. Verify Available never shows negative anywhere
