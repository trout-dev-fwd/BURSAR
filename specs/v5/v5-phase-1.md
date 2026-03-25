# V5 Phase 1: Schema + Migration + Repo Changes

## Overview

Add `secondary_percentage` and `cap_amount` columns to `envelope_allocations`,
migrate existing databases, and update the repo methods with validation.

## Context Files

| File | Why |
|------|-----|
| `CLAUDE.md` | Coding style, verification |
| `specs/v5/v5-SPEC.md` | Schema Changes section |
| `specs/v5/v5-progress.md` | Decisions log |
| `src/db/schema.rs` | CREATE TABLE statements |
| `src/db/mod.rs` | EntityDb::open(), existing migrations |
| `src/db/envelope_repo.rs` | Envelope repo methods |
| `src/types/money.rs` | Money type |
| `src/types/percentage.rs` | Percentage type |

## Tasks

### Task 1: Add Columns and Migration

**Files:** `src/db/schema.rs`, `src/db/mod.rs`

Update `initialize_schema()` — add the two new columns to the CREATE TABLE
for `envelope_allocations`:

```sql
secondary_percentage INTEGER NOT NULL DEFAULT 0,
cap_amount INTEGER
```

In `EntityDb::open()`, add migration logic (same pattern as existing column
migrations):
1. Check if `secondary_percentage` column exists via `PRAGMA table_info(envelope_allocations)`
2. If not: `ALTER TABLE envelope_allocations ADD COLUMN secondary_percentage INTEGER NOT NULL DEFAULT 0`
3. Check if `cap_amount` column exists
4. If not: `ALTER TABLE envelope_allocations ADD COLUMN cap_amount INTEGER`

**Tests:**
- Fresh DB has both columns
- Migration from old schema adds columns without data loss
- Existing allocations get secondary_percentage=0, cap_amount=NULL

**Commit:** `V5 Phase 1, Task 1: add secondary_percentage and cap_amount columns with migration`

---

### Task 2: Update EnvelopeRepo Methods

**File:** `src/db/envelope_repo.rs`

Update the `EnvelopeAllocation` struct to include new fields:

```rust
pub struct EnvelopeAllocation {
    pub id: EnvelopeAllocationId,
    pub account_id: AccountId,
    pub percentage: Percentage,            // primary
    pub secondary_percentage: Percentage,  // new
    pub cap_amount: Option<Money>,         // new
}
```

Update `set_allocation` to accept the new fields:

```rust
pub fn set_allocation(
    &self,
    account_id: AccountId,
    percentage: Percentage,
    secondary_percentage: Percentage,
    cap_amount: Option<Money>,
) -> Result<()>
```

Use UPSERT (ON CONFLICT) to update all fields atomically.

Add validation methods:

```rust
pub fn total_primary_percentage(&self) -> Result<Percentage>
pub fn total_secondary_percentage(&self) -> Result<Percentage>
```

These query `SUM(percentage)` and `SUM(secondary_percentage)` respectively.
Used by the UI to validate ≤ 100% before saving.

Update all existing queries that read `EnvelopeAllocation` to include the new
columns.

**Tests:**
- Set allocation with primary, secondary, and cap
- UPSERT updates all fields
- Total primary and secondary percentages correct
- Cap of None stored as NULL
- Existing tests still pass (backward compatible)

**Commit:** `V5 Phase 1, Task 2: update EnvelopeRepo with secondary percentage and cap`

---

### Task 3: Update EnvelopeAllocation Consumers

**Files:** Various — grep for `EnvelopeAllocation` and `set_allocation` usage

Find all code that reads or writes `EnvelopeAllocation` and update for the new
fields:

1. **Envelopes tab** (`src/tabs/envelopes.rs`) — the allocation editing flow needs
   to pass the new fields. For now, keep the existing single-prompt pattern but
   pass `Percentage(0)` for secondary and `None` for cap. Phase 2 will update
   the UI.

2. **Envelope fill** (`src/services/journal.rs`) — currently reads `percentage`
   to calculate fills. For now, keep existing behavior — Phase 2 updates the
   algorithm. Just ensure the code compiles with the new struct fields.

3. **Reports** (`src/reports/envelope_budget.rs`) — may read allocations. Update
   to include new fields in the struct but don't change the report output yet
   (Phase 3).

4. **AI tools** (`src/ai/tools.rs`) — `get_envelope_balances` tool may return
   allocation data. Update the struct access.

The goal of this task is: everything compiles and all tests pass with the new
struct shape, without changing any behavior yet.

**Tests:** All existing tests pass with no behavior change.

**Commit:** `V5 Phase 1, Task 3: update all EnvelopeAllocation consumers for new fields`

---

## Phase Completion Checklist

- [ ] `envelope_allocations` table has `secondary_percentage` and `cap_amount` columns
- [ ] Migration adds columns to existing DBs without data loss
- [ ] `EnvelopeAllocation` struct has both new fields
- [ ] `set_allocation` accepts and stores all fields
- [ ] Total primary/secondary percentage queries work
- [ ] All existing code compiles with new struct shape
- [ ] All existing tests pass with no behavior change
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass
