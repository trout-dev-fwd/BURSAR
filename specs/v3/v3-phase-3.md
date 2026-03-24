# V3 Phase 3: Review Screen UI — Transfer Matches Section

## Overview

Add a distinct "Transfer Matches" section to the import review screen. Users can
confirm (skip import, store import_ref) or reject (send to Pass 2) each match.

## Context Files

| File | Why |
|------|-----|
| `CLAUDE.md` | Coding style, verification commands |
| `specs/v3/v3-SPEC.md` | Review Screen Changes section |
| `specs/v3/v3-progress.md` | Decisions log |
| `src/app/import_handler.rs` | Review screen rendering and key handling |

## Tasks

### Task 1: Add Transfer Matches to ImportFlowState

**File:** `src/app/import_handler.rs`

Add to `ImportFlowState` (or the review-phase struct):

```rust
transfer_matches: Vec<TransferMatchRow>,
transfer_selected: usize,
```

Define `TransferMatchRow`:
```rust
struct TransferMatchRow {
    // Current transaction details
    date: NaiveDate,
    amount: Money,
    description: String,
    import_ref: String,
    // Matched draft details
    matched_je_id: JournalEntryId,
    matched_je_number: String,
    matched_date: NaiveDate,
    matched_amount: Money,
    matched_bank: String,
    // User decision
    confirmed: bool,  // true = skip import, false = reject → send to Pass 2
}
```

Initialize `confirmed: true` for all matches (default is to skip).

During the transition from Pass 1 to the review screen, populate `transfer_matches`
from the transactions that were marked as `TransferMatch` in Phase 2.

**Commit:** `V3 Phase 3, Task 1: add transfer match state to import flow`

---

### Task 2: Render Transfer Matches Section

**File:** `src/app/import_handler.rs`

In the review screen render function, add a "Transfer Matches" section at the top,
above the normal transaction list. Only render if `transfer_matches` is non-empty.

Layout:
```
─── Transfer Matches (3) ───────────────────────────────
  ✓  Jan 14  +$500.00   "ACH Deposit Chase"  →  JE #47 (Chase, -$500, Jan 14)
  ✓  Jan 18  +$2000.00  "Payment Thank You"   →  JE #62 (Chase, -$2000, Jan 17)
  ✗  Jan 22  +$150.00   "Transfer"            →  JE #71 (Ally, -$150, Jan 22)
```

- `✓` (green) = confirmed (will skip import)
- `✗` (red) = rejected (will send to Pass 2)
- Selected row highlighted with standard highlight style
- Separator line between transfer matches section and normal transactions

**Commit:** `V3 Phase 3, Task 2: render transfer matches section in review screen`

---

### Task 3: Handle Key Events for Transfer Matches

**File:** `src/app/import_handler.rs`

When the review screen is showing and the cursor is in the transfer matches section:

| Key | Action |
|-----|--------|
| `↑/↓` | Navigate within transfer matches |
| `Enter` / `Space` | Toggle confirmed ↔ rejected |
| `↓` past last match | Move cursor to normal transactions section |

When cursor is in the normal transactions section:
| Key | Action |
|-----|--------|
| `↑` past first row | Move cursor to transfer matches section (if non-empty) |

The existing review screen keys (account editing, reject, etc.) only apply to normal
transactions, not transfer matches. Transfer matches only support confirm/reject toggle.

**Tests:**
- Toggle confirmed → rejected and back
- Navigation between transfer section and normal section
- Empty transfer section → normal navigation unchanged

**Commit:** `V3 Phase 3, Task 3: handle key events for transfer matches`

---

## Phase Completion Checklist

- [ ] Transfer matches section renders at top of review screen
- [ ] `✓` / `✗` indicators with correct colors
- [ ] Toggle confirm/reject works via Enter/Space
- [ ] Navigation flows between transfer section and normal section
- [ ] Empty transfer matches → review screen unchanged
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass
