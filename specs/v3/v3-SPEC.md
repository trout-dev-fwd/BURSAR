# V3 Feature Specification — Cross-Bank Transfer Detection

## Overview

When importing CSV bank statements from multiple accounts, inter-account transfers
appear as separate transactions on each side (a withdrawal from Bank A and a deposit
to Bank B). Without detection, both sides get imported as separate draft JEs, double-
counting the transfer. V3 adds automatic detection and merge-or-skip for these matches.

**Who**: A user importing CSVs from multiple bank accounts in the same session or across
sessions (e.g., importing January statements from Chase, Ally, Marcus, Amex at once).

**Why**: Prevents double-counting of transfers, credit card payments, and other inter-
account movements without manual reconciliation.

---

## Success Criteria

- [ ] Schema migrated: `journal_entry_import_refs` junction table replaces `import_ref` column
- [ ] Existing `import_ref` data migrated to the new table on DB open
- [ ] Duplicate detection uses the junction table (existing behavior preserved)
- [ ] During Pass 1, new transactions are checked against existing drafts for transfer matches
- [ ] Match rule: negated amount within $3 AND date within 3 calendar days
- [ ] Single match → flagged as transfer match in review screen
- [ ] Multiple matches → sent to Pass 2 for AI categorization
- [ ] No match → normal flow (learned mappings, then Pass 2)
- [ ] Review screen shows transfer matches in a distinct section
- [ ] User can confirm (skip import, store import_ref on existing JE) or reject (send to Pass 2)
- [ ] Confirmed matches create no new draft — only add a second import_ref to the existing JE
- [ ] Rejected matches proceed through normal import flow
- [ ] Future imports detect both import_refs as duplicates (no re-import)
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass

---

## Match Rule

For each new CSV transaction being imported:

1. Negate the transaction's amount
2. Query existing draft JEs that have at least one `import_ref` in the junction table
3. Filter to JEs where:
   - A line's amount matches the negated value within ±$3 (±300,000,000 internal units)
   - The JE's `entry_date` is within 3 calendar days of the transaction's date
4. If exactly one match → flag as transfer match
5. If multiple matches → skip detection, send to Pass 2 for AI to resolve
6. If zero matches → normal Pass 1 flow (learned mappings)

---

## Schema Changes

### New Table

```sql
CREATE TABLE journal_entry_import_refs (
    id INTEGER PRIMARY KEY,
    journal_entry_id INTEGER NOT NULL REFERENCES journal_entries(id),
    import_ref TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(import_ref)
);
```

The `UNIQUE(import_ref)` constraint ensures no import_ref can be stored twice,
preventing re-import.

### Removed Column

The `import_ref` column on `journal_entries` is dropped (SQLite requires table
rebuild for column removal — handle via migration).

### Migration

On `EntityDb::open()`, detect whether the old schema (column exists) or new schema
(junction table exists) is in use. If old schema:
1. Create the junction table
2. Copy all non-NULL `import_ref` values into the junction table
3. Rebuild `journal_entries` without the `import_ref` column

---

## Pipeline Changes

### Current Pipeline
```
Duplicate check → Pass 1 (learned mappings) → Pass 2 (AI) → Pass 3 (clarification) → Review → Draft creation
```

### New Pipeline
```
Duplicate check → Pass 1 (learned mappings + transfer detection) → Pass 2 (AI) → Pass 3 (clarification) → Review → Draft creation
```

Transfer detection runs as part of Pass 1. Transactions matched as transfers are
removed from the unmatched pool (they don't go to Pass 2). They appear in a separate
"Transfer Matches" section in the review screen.

---

## Review Screen Changes

A new section appears at the top of the review screen when transfer matches exist:

```
─── Transfer Matches (3) ───────────────────────────────────
  ✓  Jan 14  +$500.00   "ACH Deposit Chase"  →  JE #47 (Chase, -$500, Jan 14)
  ✓  Jan 18  +$2000.00  "Payment Thank You"   →  JE #62 (Chase, -$2000, Jan 17)
  ✓  Jan 22  +$150.00   "Transfer"            →  JE #71 (Ally, -$150, Jan 22)
```

Each row shows: the current transaction's details on the left, the matched draft's
details on the right. The `✓` indicates it will be skipped (confirmed). User can
toggle to `✗` to reject (send to Pass 2 instead).

Navigation: arrow keys to select, Enter or Space to toggle confirm/reject.

---

## Confirmed Match Behavior

When a transfer match is confirmed in the review screen:

1. No new draft JE is created for this transaction
2. A new row is inserted into `journal_entry_import_refs` linking the existing
   draft's JE ID to the current transaction's `import_ref` string
3. The existing draft JE is otherwise untouched (no account changes, no amount changes)

This ensures:
- The existing draft retains whatever categorization it has (correct or not)
- Future CSV imports from either bank will detect both import_refs as duplicates
- The user fixes any incorrect offsetting accounts during normal draft review

---

## Out of Scope (V3)

- Automatic correction of the existing draft's offsetting account
- AI-assisted matching for multiple-match scenarios within the review screen
- Fee-adjusted matching (tolerance handles small fees; large fee discrepancies need manual handling)
- Transfer detection between posted entries (only checks drafts)
- Retroactive detection on already-imported data

---

## Implementation Order

4 phases, designed for minimal context per session:

1. **Schema migration** — junction table, data migration, update duplicate detection
2. **Transfer detection logic** — matching function, integration into Pass 1
3. **Review screen UI** — transfer matches section, confirm/reject interaction
4. **Wiring and integration** — connect confirmed matches to junction table writes, end-to-end testing
