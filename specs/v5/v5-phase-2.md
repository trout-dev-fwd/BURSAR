# V5 Phase 2: Fill Algorithm + Allocation Config UI

## Overview

Implement the two-tier fill algorithm (primary with caps → overflow → secondary)
and update the Allocation Config view with the new columns and editing flow.

## Context Files

| File | Why |
|------|-----|
| `CLAUDE.md` | Coding style, verification |
| `specs/v5/v5-SPEC.md` | Fill Algorithm, Allocation Config View Changes sections |
| `specs/v5/v5-progress.md` | Decisions log |
| `src/services/journal.rs` | Existing envelope fill logic |
| `src/db/envelope_repo.rs` | Updated repo from Phase 1 |
| `src/tabs/envelopes.rs` | Envelopes tab |

## Tasks

### Task 1: Two-Tier Fill Algorithm

**File:** `src/services/journal.rs`

Find the existing envelope fill logic that runs on JE posting (look for the
cash receipt detection and `record_fill` calls). Replace the fill step with
the two-tier algorithm:

**Step 1 — Primary fills:**
```rust
let mut overflow = Money(0);

for each allocation with percentage > 0:
    let primary_amount = cash_receipt.apply_percentage(allocation.percentage);

    let actual_fill = if let Some(cap) = allocation.cap_amount {
        let current = envelope_repo.get_balance(account_id);
        let room = (cap - current).max(Money(0));
        let fill = primary_amount.min(room);
        overflow = overflow + (primary_amount - fill);
        fill
    } else {
        primary_amount
    };

    if actual_fill > Money(0) {
        record_fill(account_id, actual_fill, source_je_id);
    }
```

**Step 2 — Secondary fills:**
```rust
if overflow > Money(0) {
    for each allocation with secondary_percentage > 0:
        let secondary_amount = overflow.apply_percentage(allocation.secondary_percentage);
        if secondary_amount > Money(0) {
            record_fill(account_id, secondary_amount, source_je_id);
        }
}
```

Secondary fills are NOT gated by cap. They always go through.

**Important:** The existing reversal logic should not need changes — each fill
is an individual ledger entry with a specific amount and source JE. Reversals
undo those specific entries regardless of how they were calculated.

**Tests:** [TEST-FIRST]
- Basic primary fill (no cap, no secondary) — existing behavior preserved
- Primary fill with cap: fills to cap, overflow calculated correctly
- Secondary fill: overflow distributed by secondary percentages
- Dual allocation: account with both primary 10% (capped) and secondary 5%
  receives secondary fill from overflow even though primary is blocked
- Cap already reached: primary fill = $0, full amount goes to overflow
- Partially capped: room under cap is less than primary amount
- No secondary allocations: overflow stays unearmarked (no fills)
- Secondary < 100%: remainder is unearmarked
- Zero cash receipt: no fills at all
- Reversal of a JE with two-tier fills: all fills reversed correctly

**Commit:** `V5 Phase 2, Task 1: two-tier envelope fill algorithm with caps and overflow`

---

### Task 2: Allocation Config View — New Columns

**File:** `src/tabs/envelopes.rs`

Update the Allocation Config view to show the new columns:

```
─ Allocation Config ──────────────────────────────────────────────────
#     Account Name           Avail      Cap        Primary %   Secondary %
```

**Column widths:**
- `#`: fixed (6)
- `Account Name`: flexible fill
- `Avail`: fixed (12) — earmarked minus GL balance, clamped to $0
- `Cap`: fixed (12) — cap amount or `—`
- `Primary %`: fixed (12) — existing percentage or `—`
- `Secondary %`: fixed (12) — secondary percentage or `—`

**Avail calculation:** Same as CoA Avail column — earmarked minus GL balance for
the current fiscal year, clamped to `Money(0)`.

**Totals row:** At the bottom, show total Primary % and total Secondary %:
```
      TOTAL                                              80.00%      75.00%
```

Style the totals row distinctly (bold or different color). If either total
exceeds 100%, show it in red as a warning.

**Commit:** `V5 Phase 2, Task 2: update Allocation Config view with new columns and totals`

---

### Task 3: Allocation Editing Flow

**File:** `src/tabs/envelopes.rs`

Update the allocation editing key handler (currently `d` key) to prompt for all
three fields sequentially:

1. "Primary allocation % (current: 10.00%):" → TextInputModal
   - Parse as Percentage
   - Validate: new total primary ≤ 100%
   - If invalid, show error and re-prompt

2. "Cap amount (current: $5,000.00, blank for no cap):" → TextInputModal
   - Parse as Money, or empty string → None (remove cap)
   - Cap must be ≥ $0

3. "Secondary allocation % (current: 0.00%):" → TextInputModal
   - Parse as Percentage
   - Validate: new total secondary ≤ 100%
   - If invalid, show error and re-prompt

On completion, call `set_allocation(account_id, primary, secondary, cap)`.
Refresh the view.

If the user presses Esc at any prompt, cancel the entire edit (don't save
partial changes).

**Tests:**
- Edit all three fields → saved correctly
- Esc mid-edit → no changes saved
- Primary exceeding 100% → error shown
- Secondary exceeding 100% → error shown
- Cap set to empty → NULL in DB (no cap)
- Cap set to 0 → treated as "cap of $0" (always at cap, primary never fills)

**Commit:** `V5 Phase 2, Task 3: sequential editing for primary, cap, and secondary allocation`

---

## Phase Completion Checklist

- [ ] Primary fills respect cap amounts
- [ ] Overflow from capped accounts calculated correctly
- [ ] Secondary fills distribute overflow by secondary percentages
- [ ] Secondary fills NOT gated by cap
- [ ] Dual-allocation accounts work correctly
- [ ] Reversal undoes all fills correctly
- [ ] Allocation Config shows Avail, Cap, Primary %, Secondary %
- [ ] Avail clamps to $0 (never negative)
- [ ] Totals row shows sums, red if over 100%
- [ ] Editing prompts for all three fields sequentially
- [ ] Esc cancels entire edit
- [ ] Validation enforces ≤ 100% for both tiers
- [ ] Existing envelopes with no secondary/cap work identically to before
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass
