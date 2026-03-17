# Type System & State Machines

## Overview

This document defines the Rust types that sit between the SQLite schema and the application logic.
These types enforce correctness at compile time: newtypes prevent value mixups, enums make invalid
states unrepresentable, and state machine transition functions return `Result` to make failures explicit.

All enums that map to SQLite TEXT columns implement `FromStr` and `Display` for serialization.

---

## Newtypes

Thin wrappers around primitives that prevent mixing up semantically different values.
The agent should generate ID newtypes with a macro to avoid boilerplate.

### `Money(i64)`

All monetary amounts throughout the application.

- **Scale**: 1 dollar = 100,000,000 internal units (10^8).
- **Max value**: ~$92.2 billion (i64::MAX / 10^8).
- **Implements**: `Add`, `Sub`, `Mul<i64>`, `Neg`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`,
  `Clone`, `Copy`, `Debug`, `Display`.
- **Constructor**: `Money::from_dollars(f64) -> Money` — for parsing user input.
  Converts to internal representation with rounding at the 8th decimal place.
- **Display**: `Display` impl formats to 2 decimal places with thousands separators for user-facing
  output (e.g., `Money(123456789012)` displays as `"1,234.57"`).
- **Methods**:
  - `fn cents_rounded(&self) -> i64` — Returns the value rounded to 2 decimal places (cents),
    for display and report formatting.
  - `fn apply_percentage(&self, pct: Percentage) -> Money` — Multiplies by percentage, retains
    full internal precision. Used for envelope fill calculations.
  - `fn is_zero(&self) -> bool`
  - `fn abs(&self) -> Money`
- **Key invariant**: Raw `i64` values never appear in function signatures outside this type.
  All database reads deserialize to `Money`; all writes serialize from `Money`.

### `Percentage(i64)`

Envelope allocation percentages.

- **Scale**: 1% = 1,000,000 internal units (10^6). Precision to 0.000001%.
- **Constructor**: `Percentage::from_display(f64) -> Percentage` — parses "15.5" into 15,500,000.
- **Display**: Formats to 2 decimal places (e.g., "15.50%").
- **Methods**:
  - `fn as_multiplier(&self) -> f64` — For use in calculations where f64 intermediate is acceptable
    (the result is immediately captured back into `Money`).

### ID Newtypes

One per foreign key type. Prevents passing an `AccountId` where a `JournalEntryId` was expected.

```rust
// Generated via macro. Each wraps i64, derives standard traits,
// and implements Into<i64> / From<i64> for database interop.

newtype_id!(AccountId);
newtype_id!(JournalEntryId);
newtype_id!(JournalEntryLineId);
newtype_id!(FiscalYearId);
newtype_id!(FiscalPeriodId);
newtype_id!(ArItemId);
newtype_id!(ApItemId);
newtype_id!(EnvelopeAllocationId);
newtype_id!(EnvelopeLedgerId);
newtype_id!(FixedAssetDetailId);
newtype_id!(RecurringTemplateId);
newtype_id!(AuditLogId);
```

---

## Enums

### `AccountType`

The five fundamental account types. Stored in `accounts.account_type`.

```rust
enum AccountType {
    Asset,
    Liability,
    Equity,
    Revenue,
    Expense,
}

impl AccountType {
    /// Returns the normal balance direction for this account type.
    /// Used in report generation to determine positive/negative display.
    fn normal_balance(&self) -> BalanceDirection {
        match self {
            Asset | Expense => BalanceDirection::Debit,
            Liability | Equity | Revenue => BalanceDirection::Credit,
        }
    }
}
```

### `BalanceDirection`

Used with `AccountType` for report calculations. Not stored in the database.

```rust
enum BalanceDirection {
    Debit,
    Credit,
}
```

### `ReconcileState`

Three-state reconciliation lifecycle. Stored in `journal_entry_lines.reconcile_state`.

```rust
enum ReconcileState {
    Uncleared,
    Cleared,
    Reconciled,
}
```

**Valid transitions** (see State Machines section below):
- `Uncleared → Cleared` (user marks via hotkey `c`)
- `Cleared → Uncleared` (user un-marks)
- `Cleared → Reconciled` (formal reconciliation finalized)
- `Reconciled` is **terminal** — no transitions out.

### `JournalEntryStatus`

```rust
enum JournalEntryStatus {
    Draft,
    Posted,
}
```

**Note**: Reversal is tracked via `is_reversed` flag and FK references on `journal_entries`,
not as a third status variant. A reversed entry is still `Posted`.

### `ArApStatus`

Shared by both AR and AP items. Stored in `ar_items.status` / `ap_items.status`.

```rust
enum ArApStatus {
    Open,
    Partial,
    Paid,
}
```

**Derived from payment data but stored explicitly** for query performance.
App recomputes and writes status on every payment insertion.
`Paid` is terminal — corrections create new items.

### `EntryFrequency`

Stored in `recurring_entry_templates.frequency`.

```rust
enum EntryFrequency {
    Monthly,
    Quarterly,
    Annually,
}

impl EntryFrequency {
    /// Advances a date by this frequency interval.
    fn advance_date(&self, date: NaiveDate) -> NaiveDate;
}
```

### `EnvelopeEntryType`

Stored in `envelope_ledger.entry_type`.

```rust
enum EnvelopeEntryType {
    Fill,
    Transfer,
    Reversal,
}
```

### `AuditAction`

Stored in `audit_log.action_type`. Exhaustive enum ensures the compiler catches missing cases
in any match expression.

```rust
enum AuditAction {
    JournalEntryCreated,
    JournalEntryPosted,
    JournalEntryReversed,
    AccountCreated,
    AccountModified,
    AccountDeactivated,
    PeriodClosed,
    PeriodReopened,
    YearEndClose,
    EnvelopeAllocationChanged,
    EnvelopeTransfer,
    PlaceInService,
    InterEntityEntryPosted,
}
```

---

## State Machines

Each lifecycle below is implemented as a transition function that takes the current state and
an action, returning `Result<NewState, TransitionError>`. Invalid transitions return descriptive
errors, never panic.

### Journal Entry Lifecycle

```
Draft ──[post]──► Posted
                    │
                    ├──[reverse]──► Posted (original: is_reversed = true)
                    │                 └──► New JE created with mirror amounts (reversal_of_je_id set)
                    │
                    └──[period closes]──► Posted + Locked (immutable via period lock)
```

**Transition: `post`**
- Preconditions (all must pass):
  - Status is `Draft`
  - `SUM(debit_amount) = SUM(credit_amount)` across all lines
  - All referenced accounts are active (`is_active = 1`)
  - No referenced accounts are placeholders (`is_placeholder = 0`)
  - Fiscal period (`fiscal_period_id`) is open (`is_closed = 0`)
  - At least two lines exist
- Effects:
  - Set `status = 'Posted'`
  - Set `updated_at` to current timestamp
  - Write audit log entry with description summarizing accounts and amounts
  - If the JE debits a Cash/Bank account (cash receipt): trigger envelope fill calculation

**Transition: `reverse`**
- Preconditions:
  - Status is `Posted`
  - `is_reversed = false`
  - The fiscal period of the *reversal date* (specified by user) is open
- Effects:
  - Set `is_reversed = true` on the original entry
  - Create a new `Posted` journal entry with:
    - All debit/credit amounts swapped (debits become credits, credits become debits)
    - `reversal_of_je_id` pointing to original
    - `memo` prefixed with "Reversal of JE-XXXX: "
  - Set `reversed_by_je_id` on original, pointing to the new entry
  - If original triggered envelope fills: create `Reversal` entries in `envelope_ledger`
  - Write audit log entry

### AR/AP Item Lifecycle

```
Open ──[partial payment]──► Partial ──[final payment]──► Paid
  │                                                        ▲
  └────────────[full payment]──────────────────────────────┘
```

**Transition: `record_payment`**
- Input: payment amount, journal entry ID, payment date
- Logic:
  - Insert row into `ar_payments` (or `ap_payments`)
  - Compute `total_paid = SUM(amount) FROM ar_payments WHERE ar_item_id = ?`
  - If `total_paid = ar_items.amount` → set status to `Paid`
  - If `0 < total_paid < ar_items.amount` → set status to `Partial`
  - If `total_paid > ar_items.amount` → reject with error (overpayment)
- `Paid` is **terminal**. No backward transitions.
  Overpayments, returned checks → create new AR/AP items with their own journal entries.

### Fiscal Period Lifecycle

```
Open ──[close]──► Closed ──[reopen (with confirmation)]──► Open
```

**Transition: `close`**
- Preconditions:
  - Period is currently `Open` (`is_closed = 0`)
  - All journal entries in this period are `Posted` (no lingering Drafts)
- Effects:
  - Set `is_closed = 1`, `closed_at = current_timestamp`
  - All journal entries in this period become immutable (enforced by period-lock check on all
    mutation operations)
  - Write audit log entry

**Transition: `reopen`**
- Preconditions:
  - Period is currently `Closed` (`is_closed = 1`)
  - User has confirmed via confirmation prompt
- Effects:
  - Set `is_closed = 0`, `reopened_at = current_timestamp`
  - Journal entries in this period become mutable again
  - Write audit log entry

### Inter-Entity Transaction Protocol

```
1. User fills form          ──► Validated (each entity's lines balance independently)
2. Write Draft to Entity A  ──► Draft A exists in Entity A's database
3. Write Draft to Entity B  ──► Draft B exists in Entity B's database
4. Post Entity A            ──► Posted A
5. Post Entity B            ──► Posted B ──► Done (both sides consistent)
```

**Validation** (before step 2):
- Each entity's line items independently satisfy `SUM(debit) = SUM(credit)`
- All referenced accounts in each entity are active and not placeholders
- Both entities' target fiscal periods are open

**Failure Recovery** (runs on startup, prompts immediately):
- Query each loaded entity's database:
  ```sql
  SELECT id, je_number, inter_entity_uuid, status, source_entity_name
  FROM journal_entries
  WHERE inter_entity_uuid IS NOT NULL AND status = 'Draft'
  ```
- For each orphaned Draft, check the corresponding entry in the other entity's database
  (matched by `inter_entity_uuid`):
  - **One Posted, one Draft** → offer to: complete (post the Draft) or roll back
    (delete the Draft, reverse the Posted one)
  - **Both Draft** → offer to: post both or delete both
  - Present options to user via prompt; require explicit choice before proceeding

### Reconciliation State Transitions

```
Uncleared ──[user marks 'c']──► Cleared ──[reconciliation finalized]──► Reconciled
               ▲                     │
               └──[user un-marks]────┘
```

**Transition: `mark_cleared`**
- Precondition: current state is `Uncleared`
- Effect: set `reconcile_state = 'Cleared'`

**Transition: `unmark_cleared`**
- Precondition: current state is `Cleared`
- Effect: set `reconcile_state = 'Uncleared'`

**Transition: `finalize_reconciliation`**
- Precondition: current state is `Cleared`
- Effect: set `reconcile_state = 'Reconciled'`

**Rejected transition: any mutation of `Reconciled`**
- Return error: "Cannot modify reconciled entries. Reconciled state is permanent."

---

## Algorithms

### Depreciation Generation

Runs on startup check or manual trigger. Not a state machine, but a defined procedure.

1. Query all fixed assets: `SELECT * FROM fixed_asset_details WHERE is_depreciable = 1 AND in_service_date IS NOT NULL`
2. For each asset:
   a. Find the last month for which depreciation was generated (query existing journal entries
      that credit this asset's accumulated depreciation account).
   b. Calculate monthly amount: `cost_basis / useful_life_months`
   c. For each month between last-generated and current period's end:
      - Generate a Draft journal entry:
        - Debit: Depreciation Expense account
        - Credit: Accumulated Depreciation account (the contra-asset linked to this fixed asset)
        - Amount: monthly depreciation amount
        - Memo: "Monthly depreciation: [Asset Name] — Month X of Y"
   d. **Rounding**: If `cost_basis % useful_life_months != 0`, the final month's entry absorbs the
      remainder so total depreciation exactly equals cost basis.
3. Present all generated Draft entries to user for review and batch posting.

### Envelope Fill on Cash Receipt

Triggered when a journal entry is posted that debits a Cash/Bank account.

1. Determine the cash receipt amount (sum of debit amounts on Cash/Bank account lines).
2. Check: if the credit side includes Owner's Capital → **do** trigger fill.
3. Check: if the debit side includes Owner's Draw → **do not** trigger fill. Exit.
4. For each row in `envelope_allocations`:
   a. Calculate fill amount: `cash_receipt_amount.apply_percentage(allocation.percentage)`
   b. Insert into `envelope_ledger`:
      - `account_id`: the allocated account
      - `entry_type`: `Fill`
      - `amount`: the calculated fill amount (positive)
      - `source_je_id`: the posted journal entry's ID
5. No journal entries are created by this process — envelope fills are purely budgetary.

### Envelope Transfer

User moves earmarked dollars between accounts without changing percentage allocations.

1. Input: source account, destination account, transfer amount.
2. Validate: source account's current envelope balance >= transfer amount.
   (`SELECT SUM(amount) FROM envelope_ledger WHERE account_id = source_id`)
3. Generate a UUID for `transfer_group_id`.
4. In a single database transaction, insert two rows into `envelope_ledger`:
   - Row 1: `account_id = source`, `amount = -transfer_amount`, `entry_type = 'Transfer'`,
     `related_account_id = destination`, `transfer_group_id = uuid`
   - Row 2: `account_id = destination`, `amount = +transfer_amount`, `entry_type = 'Transfer'`,
     `related_account_id = source`, `transfer_group_id = uuid`
5. No journal entry created. No GL impact. Percentage allocations unchanged.
