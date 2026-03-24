# V3 Phase 2: Transfer Detection Logic

## Overview

Implement the matching function that detects cross-bank transfers by comparing new
import transactions against existing draft JEs. Integrate into Pass 1 of the import
pipeline so matches are caught before AI categorization.

## Context Files

| File | Why |
|------|-----|
| `CLAUDE.md` | Coding style, verification commands |
| `specs/v3/v3-SPEC.md` | Match Rule section |
| `specs/v3/v3-progress.md` | Decisions log |
| `src/db/import_ref_repo.rs` | Junction table access (from Phase 1) |
| `src/db/journal_repo.rs` | Draft JE queries |
| `src/ai/csv_import.rs` | Import transaction types, Pass 1 logic |
| `src/app/import_handler.rs` | Import pipeline orchestration |
| `src/types/money.rs` | Money type for amount comparison |

## Tasks

### Task 1: Add Transfer Match Query to JournalRepo

**File:** `src/db/journal_repo.rs`

Add a method to find potential transfer matches:

```rust
/// Find draft JEs that might be the other side of a transfer.
/// Matches: has import_ref, amount (negated) within ±tolerance, date within ±days.
pub fn find_transfer_matches(
    &self,
    amount: Money,          // the new transaction's amount (will be negated internally)
    date: NaiveDate,        // the new transaction's date
    tolerance: Money,       // ±$3 = Money(300_000_000)
    day_range: i64,         // 3
) -> Result<Vec<TransferMatch>>
```

Define `TransferMatch`:
```rust
pub struct TransferMatch {
    pub je_id: JournalEntryId,
    pub je_number: String,
    pub entry_date: NaiveDate,
    pub amount: Money,           // the matched line's amount
    pub memo: String,
    pub bank_name: String,       // extracted from the existing import_ref
}
```

The query joins `journal_entries` → `journal_entry_lines` → `journal_entry_import_refs`:
- `journal_entries.status = 'Draft'`
- `journal_entry_import_refs.import_ref IS NOT NULL` (has at least one import_ref)
- Line amount within range of negated input amount
- `entry_date BETWEEN date - day_range AND date + day_range`

**Tests:**
- Matching draft within range → found
- Draft outside date range → not found
- Draft outside amount tolerance → not found
- Posted entry (not draft) → not found
- Draft with no import_ref → not found
- Multiple matches returned when they exist

**Commit:** `V3 Phase 2, Task 1: add transfer match query to JournalRepo`

---

### Task 2: Integrate Transfer Detection into Pass 1

**Files:** `src/ai/csv_import.rs`, `src/app/import_handler.rs`

Add transfer detection to Pass 1, after learned mapping checks:

For each unmatched transaction after learned mapping lookup:
1. Call `journal_repo.find_transfer_matches(amount, date, $3_tolerance, 3_days)`
2. If exactly one match → mark transaction as `MatchSource::TransferMatch` with the
   matched JE's details. Remove from unmatched pool.
3. If multiple matches → leave unmatched (will go to Pass 2 for AI)
4. If zero matches → leave unmatched (normal flow)

Add a new variant to `MatchSource` (or the appropriate enum):
```rust
TransferMatch {
    matched_je_id: JournalEntryId,
    matched_je_number: String,
    matched_date: NaiveDate,
    matched_amount: Money,
    matched_memo: String,
    matched_bank: String,
}
```

Update `MatchConfidence` if needed — transfer matches are high confidence.

**Tests:**
- Transaction with one matching draft → marked as TransferMatch
- Transaction with multiple matching drafts → stays unmatched
- Transaction with no matches → stays unmatched (normal flow)
- Already-matched-by-learned-mapping transaction → skips transfer detection

**Commit:** `V3 Phase 2, Task 2: integrate transfer detection into Pass 1`

---

## Phase Completion Checklist

- [ ] `find_transfer_matches` query returns correct results for all edge cases
- [ ] Pass 1 runs transfer detection after learned mappings
- [ ] Single matches flagged as `TransferMatch`, removed from unmatched pool
- [ ] Multiple matches left for Pass 2
- [ ] No changes to Pass 2 or Pass 3 behavior
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass
