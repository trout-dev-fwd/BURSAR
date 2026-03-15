# Data Model — SQLite Schema Specification

## Overview

Each legal entity has its own `.sqlite` file. All tables below exist per-entity database.
The workspace config (`workspace.toml`) lives outside any SQLite file and is not covered here.

## Design Decisions

### Money Representation
- All monetary amounts stored as `INTEGER` (`i64` in Rust).
- **1 dollar = 100,000,000 internal units** (10^8 / 8 implied decimal places).
- `i64` max = 9,223,372,036,854,775,807 → ~$92.2 billion at this precision.
- Rounding to 2 decimal places happens **only at the display boundary**.
- Internal calculations retain full 8-decimal precision to prevent compounding tax rounding errors.
- **WHY**: Fixed two-decimal-place storage causes tax calculation rounding issues in real business use.
  Eight decimal places eliminate compounding errors across multi-step calculations.

### Percentage Representation
- Envelope allocation percentages stored as `INTEGER` (`i64` in Rust).
- **1% = 1,000,000 internal units** (10^6 / 6 implied decimal places).
- Precision to 0.000001%.
- Display formatting shows 2 decimal places (e.g., "15.50%").

### Enum Storage
- All enums stored as `TEXT` in SQLite, not `INTEGER`.
- **WHY**: Human-readable when inspecting the database directly via SQLite MCP server or CLI tools.

### Account Numbers
- Stored as `TEXT`, not `INTEGER`.
- **WHY**: Supports numbering schemes like "1010.01" for sub-accounts, leading zeros like "0100",
  and non-numeric prefixes without surprises. Sorting works with consistent digit-width conventions.

### Account Hierarchy
- Adjacency list with `parent_id` foreign key.
- SQLite recursive CTEs available for subtotals across account branches.
- **WHY**: Simple, sufficient for the expected scale (dozens of accounts per entity).

### Timestamps & Dates
- All timestamps stored as `TEXT` in ISO 8601 format.
- Dates: `YYYY-MM-DD`
- Timestamps: `YYYY-MM-DDTHH:MM:SS`

### Explicit Storage Over Derived Values
- Fiscal periods, AR/AP statuses, and reconciliation states are stored explicitly in tables
  rather than derived from business logic at query time.
- **WHY**: Schema migrations remain data-safe even if business logic is refactored later.

---

## Tables

### `accounts`

Core chart of accounts. Each row is one account within a single entity.

```sql
CREATE TABLE accounts (
    id              INTEGER PRIMARY KEY,
    number          TEXT    NOT NULL UNIQUE,
    name            TEXT    NOT NULL,
    account_type    TEXT    NOT NULL,            -- Asset, Liability, Equity, Revenue, Expense
    parent_id       INTEGER REFERENCES accounts(id),
    is_active       INTEGER NOT NULL DEFAULT 1,
    is_contra       INTEGER NOT NULL DEFAULT 0,  -- contra-asset, contra-equity
    is_placeholder  INTEGER NOT NULL DEFAULT 0,  -- if 1, cannot post transactions to this account
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL
);
```

**Notes:**
- `is_placeholder`: Prevents posting journal entry lines directly to category-level parent accounts
  (e.g., "Assets", "Liabilities"). These accounts exist only to organize the hierarchy.
- `is_contra`: Flags contra accounts (Accumulated Depreciation, Owner's Draw) whose normal balance
  is opposite to their parent account type.
- `account_type` constrained to: `Asset`, `Liability`, `Equity`, `Revenue`, `Expense`.

---

### `fixed_asset_details`

Companion table for accounts that represent fixed assets. Only accounts of type Asset that are
fixed/depreciable (or explicitly non-depreciable like land) get a row here.

```sql
CREATE TABLE fixed_asset_details (
    id                    INTEGER PRIMARY KEY,
    account_id            INTEGER NOT NULL UNIQUE REFERENCES accounts(id),
    cost_basis            INTEGER NOT NULL,        -- Money: 8 decimal places
    in_service_date       TEXT,                     -- ISO 8601 date; NULL if not yet in service
    useful_life_months    INTEGER,                  -- NULL for land (non-depreciable)
    is_depreciable        INTEGER NOT NULL DEFAULT 1,
    source_cip_account_id INTEGER REFERENCES accounts(id),
    created_at            TEXT    NOT NULL,
    updated_at            TEXT    NOT NULL
);
```

**Notes:**
- `source_cip_account_id`: Traceability from fixed asset back to the CIP account it originated from.
  Populated by the "Place in Service" action.
- `in_service_date` and `useful_life_months` populated when the CIP → Fixed Asset transfer occurs.
- Land accounts: `is_depreciable = 0`, `useful_life_months = NULL`.
- Depreciation formula: `cost_basis / useful_life_months` per month (straight-line only).

---

### `journal_entries`

Header table for journal entries. Holds metadata; line items are in `journal_entry_lines`.

```sql
CREATE TABLE journal_entries (
    id                  INTEGER PRIMARY KEY,
    je_number           TEXT    NOT NULL UNIQUE,   -- "JE-0001", sequential, immutable
    entry_date          TEXT    NOT NULL,           -- user-specified accounting date (ISO 8601)
    memo                TEXT,
    status              TEXT    NOT NULL DEFAULT 'Draft',  -- Draft, Posted
    is_reversed         INTEGER NOT NULL DEFAULT 0,
    reversed_by_je_id   INTEGER REFERENCES journal_entries(id),
    reversal_of_je_id   INTEGER REFERENCES journal_entries(id),
    inter_entity_uuid   TEXT,                       -- NULL for single-entity entries; shared UUID for inter-entity pairs
    source_entity_name  TEXT,                       -- display name of the other entity (inter-entity only)
    fiscal_period_id    INTEGER NOT NULL REFERENCES fiscal_periods(id),
    created_at          TEXT    NOT NULL,           -- data-entry timestamp (when user typed it)
    updated_at          TEXT    NOT NULL
);
```

**Notes:**
- `je_number`: Generated app-side via `SELECT MAX(je_number) + 1` pattern, formatted as "JE-NNNN".
  Not generated by SQLite trigger.
- `entry_date`: The **accounting date** — when the transaction economically occurred. This is distinct
  from `created_at`, which records when the entry was created in the system.
- `is_reversed` + `reversed_by_je_id` + `reversal_of_je_id`: Bidirectional linkage between original
  and reversal entries. Both the original and its reversal are `Posted`.
- `inter_entity_uuid`: Non-NULL only for inter-entity transactions. Both entities' entries share the
  same UUID for cross-database linkage.
- `fiscal_period_id`: Direct FK makes "is this period closed?" a single join, no date-range math.

---

### `journal_entry_lines`

Individual debit/credit rows within a journal entry.

```sql
CREATE TABLE journal_entry_lines (
    id                INTEGER PRIMARY KEY,
    journal_entry_id  INTEGER NOT NULL REFERENCES journal_entries(id),
    account_id        INTEGER NOT NULL REFERENCES accounts(id),
    debit_amount      INTEGER NOT NULL DEFAULT 0,   -- Money: 8 decimal places (one of debit/credit is 0)
    credit_amount     INTEGER NOT NULL DEFAULT 0,   -- Money: 8 decimal places
    line_memo         TEXT,
    reconcile_state   TEXT    NOT NULL DEFAULT 'Uncleared',  -- Uncleared, Cleared, Reconciled
    sort_order        INTEGER NOT NULL DEFAULT 0,
    created_at        TEXT    NOT NULL
);
```

**Notes:**
- Two-column debit/credit design: for any given line, one column is non-zero and the other is zero.
  `SUM(debit_amount) - SUM(credit_amount)` across all lines must equal 0 for a valid entry.
- `reconcile_state`: Three-state enum. `Uncleared → Cleared` (user marks via hotkey),
  `Cleared → Reconciled` (formal reconciliation). `Reconciled` is terminal (cannot revert).
  `Cleared` can revert to `Uncleared`.
- `sort_order`: Preserves user's intended row ordering in the entry form.

---

### `ar_items`

Tracks individual accounts receivable items.

```sql
CREATE TABLE ar_items (
    id                  INTEGER PRIMARY KEY,
    account_id          INTEGER NOT NULL REFERENCES accounts(id),
    customer_name       TEXT    NOT NULL,
    description         TEXT,
    amount              INTEGER NOT NULL,             -- Money: original total
    due_date            TEXT    NOT NULL,              -- ISO 8601 date
    status              TEXT    NOT NULL DEFAULT 'Open',  -- Open, Partial, Paid
    originating_je_id   INTEGER NOT NULL REFERENCES journal_entries(id),
    created_at          TEXT    NOT NULL,
    updated_at          TEXT    NOT NULL
);
```

### `ar_payments`

Junction table supporting multiple partial payments per AR item.

```sql
CREATE TABLE ar_payments (
    id              INTEGER PRIMARY KEY,
    ar_item_id      INTEGER NOT NULL REFERENCES ar_items(id),
    je_id           INTEGER NOT NULL REFERENCES journal_entries(id),
    amount          INTEGER NOT NULL,               -- Money: this payment's amount
    payment_date    TEXT    NOT NULL,                -- ISO 8601 date
    created_at      TEXT    NOT NULL
);
```

**Notes:**
- Current amount paid: `SELECT SUM(amount) FROM ar_payments WHERE ar_item_id = ?`
- When sum equals `ar_items.amount` → status = `Paid`. Between 0 and total → `Partial`.
- `Paid` is **terminal**. Corrections (overpayments, returned checks) create new AR items
  with their own journal entries. No backward state transitions.
- `account_id` on `ar_items` ties each receivable to a specific AR account in the chart of accounts.

---

### `ap_items`

Tracks individual accounts payable items. Mirrors AR structure.

```sql
CREATE TABLE ap_items (
    id                  INTEGER PRIMARY KEY,
    account_id          INTEGER NOT NULL REFERENCES accounts(id),
    vendor_name         TEXT    NOT NULL,
    description         TEXT,
    amount              INTEGER NOT NULL,             -- Money: original total
    due_date            TEXT    NOT NULL,              -- ISO 8601 date
    status              TEXT    NOT NULL DEFAULT 'Open',  -- Open, Partial, Paid
    originating_je_id   INTEGER NOT NULL REFERENCES journal_entries(id),
    created_at          TEXT    NOT NULL,
    updated_at          TEXT    NOT NULL
);
```

### `ap_payments`

```sql
CREATE TABLE ap_payments (
    id              INTEGER PRIMARY KEY,
    ap_item_id      INTEGER NOT NULL REFERENCES ap_items(id),
    je_id           INTEGER NOT NULL REFERENCES journal_entries(id),
    amount          INTEGER NOT NULL,               -- Money: this payment's amount
    payment_date    TEXT    NOT NULL,                -- ISO 8601 date
    created_at      TEXT    NOT NULL
);
```

**Notes:**
- Identical behavior to AR payments. `Paid` is terminal. Same derivation logic for status.

---

### `envelope_allocations`

Configuration table: what percentage of incoming cash gets earmarked for each account.

```sql
CREATE TABLE envelope_allocations (
    id              INTEGER PRIMARY KEY,
    account_id      INTEGER NOT NULL UNIQUE REFERENCES accounts(id),
    percentage      INTEGER NOT NULL,               -- Percentage: 6 decimal places (15.5% = 15500000)
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL
);
```

**Notes:**
- Percentages do not need to sum to 100%. Unallocated revenue stays unearmarked.
- Allocations are uniform across all revenue sources (one global table per entity).
- Changing a percentage is **forward-only** — does not retroactively recalculate past fills.

---

### `envelope_ledger`

Auditable transaction log for envelope balances. Current balance is `SUM(amount) WHERE account_id = ?`.

```sql
CREATE TABLE envelope_ledger (
    id                  INTEGER PRIMARY KEY,
    account_id          INTEGER NOT NULL REFERENCES accounts(id),
    entry_type          TEXT    NOT NULL,            -- Fill, Transfer, Reversal
    amount              INTEGER NOT NULL,            -- Money: signed (positive = add, negative = remove)
    source_je_id        INTEGER REFERENCES journal_entries(id),  -- NULL for transfers
    related_account_id  INTEGER REFERENCES accounts(id),          -- other side of a transfer
    transfer_group_id   TEXT,                         -- UUID: pairs the two rows of a transfer
    memo                TEXT,
    created_at          TEXT    NOT NULL
);
```

**Notes:**
- **Fill**: Created automatically when a cash receipt JE is posted. `source_je_id` references the JE.
- **Transfer**: Moving earmarked dollars between accounts. Creates two rows (one negative, one positive)
  linked by `transfer_group_id` (UUID). No journal entry created — purely budgetary.
- **Reversal**: Created when a JE that triggered a fill is reversed. Negative amount undoes the fill.
- Current balance: `SELECT SUM(amount) FROM envelope_ledger WHERE account_id = ?`

---

### `fiscal_years`

```sql
CREATE TABLE fiscal_years (
    id              INTEGER PRIMARY KEY,
    start_date      TEXT    NOT NULL,               -- ISO 8601 date (first day of fiscal year)
    end_date        TEXT    NOT NULL,
    is_closed       INTEGER NOT NULL DEFAULT 0,
    closed_at       TEXT,                            -- timestamp when year-end close ran
    created_at      TEXT    NOT NULL
);
```

### `fiscal_periods`

```sql
CREATE TABLE fiscal_periods (
    id              INTEGER PRIMARY KEY,
    fiscal_year_id  INTEGER NOT NULL REFERENCES fiscal_years(id),
    period_number   INTEGER NOT NULL,               -- 1–12
    start_date      TEXT    NOT NULL,
    end_date        TEXT    NOT NULL,
    is_closed       INTEGER NOT NULL DEFAULT 0,
    closed_at       TEXT,                            -- timestamp when period was closed
    reopened_at     TEXT,                            -- last reopen timestamp; NULL if never reopened
    created_at      TEXT    NOT NULL
);
```

**Notes:**
- Explicit table, not derived from fiscal year start date + date math.
- `reopened_at` supports audit trail cross-referencing when periods are reopened.
- Journal entries reference `fiscal_period_id` directly for efficient period-lock checks.

---

### `recurring_entry_templates`

Stores the schedule for recurring journal entries. References an existing JE as the template.

```sql
CREATE TABLE recurring_entry_templates (
    id                    INTEGER PRIMARY KEY,
    source_je_id          INTEGER NOT NULL REFERENCES journal_entries(id),
    frequency             TEXT    NOT NULL,          -- Monthly, Quarterly, Annually
    next_due_date         TEXT    NOT NULL,          -- ISO 8601 date
    is_active             INTEGER NOT NULL DEFAULT 1,
    last_generated_date   TEXT,
    created_at            TEXT    NOT NULL,
    updated_at            TEXT    NOT NULL
);
```

**Notes:**
- Does **not** copy entry data. References the original JE; generation copies line items from it.
- On generation: creates a new Draft JE dated at `next_due_date`, advances `next_due_date` by frequency.
- `is_active` allows pausing without deleting the template.
- Upcoming entries queryable via: `SELECT * FROM recurring_entry_templates WHERE is_active = 1 ORDER BY next_due_date`.

---

### `audit_log`

Immutable, write-once event log. No `updated_at` column.

```sql
CREATE TABLE audit_log (
    id              INTEGER PRIMARY KEY,
    action_type     TEXT    NOT NULL,               -- see AuditAction enum in type-system.md
    entity_name     TEXT    NOT NULL,
    record_type     TEXT,                            -- JournalEntry, Account, FiscalPeriod, etc.
    record_id       INTEGER,
    description     TEXT    NOT NULL,                -- human-readable summary including accounts/amounts
    created_at      TEXT    NOT NULL
);
```

**Notes:**
- Lives in the **same SQLite file** as entity data for transactional consistency.
  `BEGIN; INSERT journal_entry; INSERT audit_log; COMMIT;` is atomic.
- `description` includes a human-readable summary (e.g., "Posted JE-0042: Debit Cash $500.00,
  Credit Revenue $500.00") so the log is scannable without joins.
- `record_type` + `record_id` provide join path to the specific row that was mutated,
  for when full detail is needed beyond the summary.
- Rows are **never modified or deleted**.

---

## Integrity Invariants

These should be enforced by the application and validated by tests:

1. **Trial Balance Proof**: For all posted entries,
   `SUM(debit_amount) - SUM(credit_amount) = 0` across all `journal_entry_lines`.

2. **Entry Balance**: For any single journal entry,
   `SUM(debit_amount) = SUM(credit_amount)` across its lines. Enforced before posting.

3. **No Placeholder Posts**: Journal entry lines must not reference accounts where `is_placeholder = 1`.

4. **Period Lock**: No inserts/updates to journal entries in a fiscal period where `is_closed = 1`.

5. **Reconciled Lock**: Lines with `reconcile_state = 'Reconciled'` cannot be modified.

6. **AR/AP Status Consistency**: `ar_items.status` must match the computed state from
   `SUM(ar_payments.amount)` vs `ar_items.amount`.

7. **Envelope Transfer Balance**: For any `transfer_group_id`,
   `SUM(amount) = 0` across the paired rows.
