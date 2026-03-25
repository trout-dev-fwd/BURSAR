# V5 Feature Specification — Enhanced Envelope Allocations

## Overview

Extend the envelope budgeting system with two-tier allocations and caps. Primary
allocations fill first on every cash receipt. When a primary allocation hits its cap,
the blocked amount flows into an overflow pool. Secondary allocations distribute the
overflow pool. Any remainder stays unearmarked.

**Who**: A user who wants to save toward specific goals (emergency fund, taxes, vacation)
with automatic filling and spending caps, while directing surplus into secondary accounts.

**Why**: The current system allocates a fixed percentage indefinitely. There's no way to
say "save 10% until I have $5,000, then redirect that 10% elsewhere." The two-tier
system solves this without manual intervention.

---

## Success Criteria

- [ ] `envelope_allocations` table has `secondary_percentage` and `cap_amount` columns
- [ ] Allocation Config view shows: Account | Avail | Cap | Primary % | Secondary %
- [ ] Users can set primary %, secondary %, and cap per account
- [ ] Primary allocations total ≤ 100% (enforced)
- [ ] Secondary allocations total ≤ 100% (enforced)
- [ ] An account can have both primary and secondary allocations
- [ ] Cap only gates primary fills — secondary fills ignore the cap
- [ ] When a primary fill is blocked by cap, the blocked amount goes to overflow pool
- [ ] Overflow pool is distributed by secondary percentages
- [ ] Overflow remainder (secondary < 100%) stays unearmarked
- [ ] Available amount never displays negative (clamps to $0)
- [ ] Fill behavior triggers on JE posting (existing behavior preserved)
- [ ] Existing envelopes with only primary allocations work exactly as before
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass

---

## Schema Changes

### Modified Table: `envelope_allocations`

Add two columns:

```sql
ALTER TABLE envelope_allocations ADD COLUMN secondary_percentage INTEGER NOT NULL DEFAULT 0;
ALTER TABLE envelope_allocations ADD COLUMN cap_amount INTEGER;  -- NULL = no cap
```

- `percentage` (existing) → renamed conceptually to "primary percentage" in the UI,
  but the column name stays `percentage` for backward compatibility
- `secondary_percentage` — stored as `Percentage(i64)` same scale as primary (10^6).
  Default 0 (no secondary allocation)
- `cap_amount` — stored as `Money(i64)` same scale (10^8). NULL means no cap.

**Migration:** On `EntityDb::open()`, check if the columns exist. If not, add them
via `ALTER TABLE ADD COLUMN`. Existing allocations get `secondary_percentage = 0`
and `cap_amount = NULL` (no behavior change for existing data).

---

## Fill Algorithm

The existing fill algorithm runs in `services/journal.rs` when a JE is posted and
a cash receipt is detected. The new algorithm replaces the fill step:

### Step 1: Primary Fills

For each account with `percentage > 0`:

```
primary_amount = cash_receipt × (percentage / 100)

if cap_amount is not NULL:
    current_earmarked = get_envelope_balance(account_id)
    room_under_cap = max(0, cap_amount - current_earmarked)
    actual_fill = min(primary_amount, room_under_cap)
    overflow += (primary_amount - actual_fill)
else:
    actual_fill = primary_amount

record_fill(account_id, actual_fill, source_je_id)
```

### Step 2: Secondary Fills

If `overflow > 0` and any account has `secondary_percentage > 0`:

```
for each account with secondary_percentage > 0:
    secondary_amount = overflow × (secondary_percentage / 100)
    record_fill(account_id, secondary_amount, source_je_id)
```

Secondary fills are NOT gated by cap. An account at its cap still receives
secondary fills (it goes over the cap — the cap only controls primary).

### Step 3: Remainder

If secondary percentages total less than 100%, the leftover overflow is simply
not earmarked. No special handling needed.

### Dual-Allocation Example

Account: Primary 10%, Secondary 5%, Cap $100. Currently at $100.
Cash receipt: $1,000.

1. Primary: 10% = $100. Cap hit (room = $0). Actual fill = $0. Overflow += $100.
2. (Other primary fills happen normally for uncapped accounts)
3. Secondary: 5% of overflow pool. If overflow = $100 → $5 fills this account.
4. Account goes from $100 → $105. Cap doesn't gate secondary.

---

## Reversal Algorithm

When a JE with envelope fills is reversed, the existing reversal logic undoes the
fills. No changes needed to the reversal algorithm — it already reverses specific
fill amounts per account. The primary/secondary distinction doesn't matter for
reversal since each fill is recorded as an individual ledger entry.

---

## Allocation Config View Changes

Current columns: `# | Account Name | Allocation %`

New columns: `# | Account Name | Avail | Cap | Primary % | Secondary %`

```
─ Allocation Config ──────────────────────────────────────────────────
#     Account Name           Avail      Cap        Primary %   Secondary %
1110  Checking Account       —          —          —           —
1510  Land                   0.00       5,000.00   10.00%      —
5100  Rent                   2,400.00   —          15.00%      —
5200  Utilities              1,800.00   2,000.00   10.00%      —
5300  Insurance              340.00     —          5.00%       40.00%
5800  Interest Expense       0.00       —          —           60.00%
```

**Avail column:** Earmarked minus GL balance, clamped to $0. Same calculation as
the Balances view and CoA Avail column. Shown here for convenience so the user
can see the current state while configuring.

**Cap column:** Shows the cap amount if set, `—` if no cap.

### Editing

The existing `d` key (distribute/edit allocation) needs to be extended to support
the new fields. When pressing the edit key on an account:

**Option A (simplest):** Three sequential prompts:
1. "Primary allocation % (current: 10.00%):" → enter new value or Enter to keep
2. "Cap amount (current: $5,000, empty for no cap):" → enter new value or Enter to keep
3. "Secondary allocation % (current: 0.00%):" → enter new value or Enter to keep

**Option B:** A small form with all three fields. More polished but more code.

Recommend Option A for V5 — sequential prompts are simpler and match the existing
single-prompt pattern for allocation editing.

### Validation

On save:
- Total primary % across all accounts must be ≤ 100%. If exceeded, show error.
- Total secondary % across all accounts must be ≤ 100%. If exceeded, show error.
- Cap amount must be ≥ $0 if set. Empty/0 clears the cap.
- These validations match the existing behavior for primary (just extended to
  secondary).

### Totals Row

Add a totals row at the bottom of the Allocation Config view:

```
      TOTAL                                     80.00%      75.00%
```

This helps the user see at a glance how much primary and secondary capacity is used.

---

## Balances View Changes

The existing Balances view shows: `# | Account Name | GL Balance | Earmarked | Available`

No changes needed to the Balances view itself — it already shows the right data.
The fill algorithm changes produce different earmarked amounts, which flow through
to the existing display.

**One change:** Available must clamp to $0 (currently can show negative). This is
Fix 1 from the bug fixes, but ensure it's also correct here after the algorithm
changes.

---

## Envelope Budget Summary Report

The existing Envelope Budget Summary report should reflect the new columns.
Add Primary %, Secondary %, and Cap to the report output. No new report needed.

---

## Out of Scope (V5)

- Weekly/monthly/annual cap periods (cap is a total amount, not time-based)
- Caps on secondary allocations
- Automatic cap adjustment based on spending patterns
- Overflow cascading (secondary overflow going to a tertiary tier)
- Notification when a cap is reached

---

## Implementation Order

3 phases:

1. **Schema + migration + repo changes** — new columns, migration, updated repo methods, validation
2. **Fill algorithm + Allocation Config UI** — two-tier fill logic, updated config view with new columns, editing flow, totals row
3. **Testing + documentation** — integration tests for fill/cap/overflow scenarios, report updates, CLAUDE.md, user guide
