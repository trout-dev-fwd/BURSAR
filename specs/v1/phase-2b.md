# Phase 2b: Journal Entries

**Goal**: Create, post, and reverse journal entries with full validation. The JE form widget
is the most complex UI component and is reused in inter-entity mode (Phase 6).

**Depends on**: Phase 2a (account repo, CoA tab, audit repo, account picker, confirmation widget).

**Estimated tasks**: 8

---

## Tasks

### Task 1: Create JournalRepo **[TEST-FIRST]**
**Context**: `src/db/mod.rs`, `src/db/account_repo.rs`, `src/db/fiscal_repo.rs`,
`specs/data-model.md` (journal_entries, journal_entry_lines tables),
`specs/type-system.md` (Journal Entry Lifecycle).
**Action**: Create `src/db/journal_repo.rs`. Implement `JournalRepo<'conn>`:
- `create_draft(entry: &NewJournalEntry) -> Result<JournalEntryId>` — creates JE + lines, status=Draft
- `get_next_je_number() -> Result<String>` — "JE-NNNN" via MAX+1
- `get_with_lines(id: JournalEntryId) -> Result<(JournalEntry, Vec<JournalEntryLine>)>`
- `list(filter: &JournalFilter) -> Result<Vec<JournalEntry>>` — filter by status, date range
- `update_status(id: JournalEntryId, status: JournalEntryStatus) -> Result<()>`
- `mark_reversed(id: JournalEntryId, reversed_by: JournalEntryId) -> Result<()>`

Define `JournalEntry`, `JournalEntryLine`, `NewJournalEntry`, `NewJournalEntryLine`, `JournalFilter`.
Wire into `EntityDb`.
**Verify**: Tests against in-memory DB:
- Create draft → retrieve with lines → data matches
- JE numbers are sequential: first is "JE-0001", second is "JE-0002"
- List with status filter works
- List with date range filter works
**Do NOT**: Implement the post/reverse business logic here — that's the orchestration in Task 2.
This repo is the data access layer only.

---

### Task 2: Implement post and reverse orchestration **[TEST-FIRST]**
**Context**: `src/db/journal_repo.rs`, `src/db/account_repo.rs`, `src/db/fiscal_repo.rs`,
`src/db/audit_repo.rs`, `specs/type-system.md` (Journal Entry Lifecycle transitions).
**Action**: Create orchestration functions (free functions or in a `src/services/journal.rs`):

`post_journal_entry(db: &EntityDb, je_id: JournalEntryId) -> Result<()>`:
- Validate: status is Draft
- Validate: SUM(debit) == SUM(credit) across all lines
- Validate: all referenced accounts are active and non-placeholder
- Validate: fiscal period (from `fiscal_period_id`) is open
- Validate: at least 2 lines exist
- In a SQLite transaction: update status to Posted, write audit log
- Structure the function to allow envelope fill insertion later (Phase 4) — leave a
  clearly marked `// TODO(Phase 4): Check for cash receipt and trigger envelope fills` comment

`reverse_journal_entry(db: &EntityDb, je_id: JournalEntryId, reversal_date: NaiveDate) -> Result<JournalEntryId>`:
- Validate: status is Posted, not already reversed
- Validate: fiscal period of reversal_date is open
- Create new JE with swapped debit/credit amounts, memo prefixed "Reversal of JE-XXXX:"
- Set `reversal_of_je_id` on new entry, `reversed_by_je_id` + `is_reversed` on original
- Write audit log
- Return the new reversal JE's ID

**Verify**: Tests:
- Post valid entry → status changes to Posted, audit log entry exists
- Post unbalanced entry → error
- Post to placeholder account → error
- Post to inactive account → error
- Post to closed period → error (need to close a period in test setup)
- Post draft with 1 line → error (need at least 2)
- Reverse posted entry → mirror entry created, original marked reversed
- Reverse already-reversed entry → error
- Reverse entry in closed period (reversal date's period) → error
**Do NOT**: Implement envelope fills (Phase 4). Do NOT implement inter-entity posting (Phase 6).

---

### Task 3: Create journal entry form widget
**Context**: `src/widgets/account_picker.rs`, `specs/architecture.md` (JeForm section).
**Action**: Create `src/widgets/je_form.rs`. A reusable multi-field form widget:
- Date field (validated text input: YYYY-MM-DD format)
- Memo field (free text)
- Line items: dynamic rows, each with:
  - Account (via AccountPicker widget integration)
  - Debit amount (validated numeric input, displays as Money)
  - Credit amount (validated numeric input, displays as Money)
  - Line memo (optional free text)
- Add row / remove row controls
- Running totals displayed at bottom: Total Debits, Total Credits, Difference
- Difference highlighted in red when non-zero, green when balanced
- `Tab` moves between fields, `Enter` on last field of last row adds a new row
- Submit returns `NewJournalEntry` struct; cancel returns None
**Verify**: Manual testing:
- Fill out a 3-line entry, account picker works in each line
- Running totals update as amounts are entered
- Can add and remove rows
- Tab navigation moves through all fields
- Submit with balanced entry → returns data
- Cannot submit with zero-amount lines (validation)
**Do NOT**: Implement recurring entry flagging (Phase 5). Do NOT add inter-entity line sections (Phase 6).

---

### Task 4: Implement Journal Entries tab — list view
**Context**: `src/tabs/journal_entries.rs` (stub), `src/db/journal_repo.rs`.
**Action**: Replace stub with real implementation:
- List view: all journal entries, columns for JE Number, Date, Memo, Status, Reversed indicator
- Navigation: `↑↓` to scroll
- `Enter` on a row → expand to show detail (line items with accounts, debits, credits)
- Filter: by status (Draft/Posted/All), date range
- Implement `refresh()` to re-query from DB
- Implement `navigate_to(RecordId::JournalEntry(id))` — scroll to and highlight the specified entry
**Verify**: With test data: list shows entries, scrolling works, detail view shows correct lines,
filter by status works. NavigateTo scrolls to the correct entry.
**Do NOT**: Implement new/post/reverse actions yet (Task 5). Display and navigation only.

---

### Task 5: Implement Journal Entries tab — actions
**Context**: `src/tabs/journal_entries.rs`, `src/widgets/je_form.rs`,
`src/widgets/confirmation.rs`, `src/db/journal_repo.rs`, orchestration functions from Task 2.
**Action**: Add action hotkeys:
- `n` — new entry: opens JE form widget. On submit, calls `journal_repo.create_draft()`.
  Returns `TabAction::RefreshData`.
- `p` — post selected draft: confirmation prompt ("Post JE-XXXX?"). On confirm, calls
  `post_journal_entry()`. Shows success/error in status bar. Returns `TabAction::RefreshData`.
- `r` — reverse selected posted entry: prompts for reversal date (date input). Confirmation
  ("Reverse JE-XXXX?"). Calls `reverse_journal_entry()`. Returns `TabAction::RefreshData`.
**Verify**: Full workflow: create draft → see it in list → post it → status changes to Posted →
reverse it → reversal entry appears. JE numbers sequential throughout. Error cases show
status bar messages (unbalanced, inactive account, etc.). Audit log has entries for all actions.
**Do NOT**: Implement recurring entry creation (Phase 5). Do NOT implement the `i` hotkey
for inter-entity mode (Phase 6).

---

### Task 6: Implement reconciliation state changes
**Context**: `src/tabs/journal_entries.rs` (detail view), `src/db/journal_repo.rs`,
`specs/type-system.md` (ReconcileState transitions).
**Action**: In the JE detail view (when viewing lines of a posted entry):
- `c` — toggle reconcile state: Uncleared → Cleared, Cleared → Uncleared
- Visual indicators: blank for Uncleared, `✓` for Cleared, `✓✓` for Reconciled
- Reject state changes on lines in closed fiscal periods (show error in status bar)
- Reject any change to Reconciled lines ("Cannot modify reconciled entries")

Add to `JournalRepo`:
- `update_reconcile_state(line_id: JournalEntryLineId, new_state: ReconcileState) -> Result<()>`
**Verify**: Mark a line as Cleared → `✓` appears. Press `c` again → reverts to Uncleared.
Attempt on Reconciled line → error. Attempt on line in closed period → error.
**Do NOT**: Implement the Reconciled state transition (that's formal reconciliation, future scope).
Users can only toggle between Uncleared and Cleared in V1.

---

### Task 7: Account balances now reflect posted entries
**Context**: `src/db/account_repo.rs`, `src/tabs/chart_of_accounts.rs`.
**Action**: Verify that `account_repo.get_balance()` correctly sums posted JE lines.
Update the CoA tab's `refresh()` to display these balances.
If the balance query written in Phase 2a is correct, this may just require verifying it works
with real posted entries. Fix if needed.
**Verify**: Create and post a JE that debits Cash $1,000, credits Revenue $1,000.
CoA tab shows Cash balance of $1,000.00 and Revenue balance of $1,000.00.
**Do NOT**: Add envelope earmarked amounts to the display (Phase 4).

---

### Task 8: Cross-tab navigation — CoA to Journal Entries
**Context**: `src/tabs/chart_of_accounts.rs`, `src/tabs/journal_entries.rs`.
**Action**: On the CoA tab, add a hotkey (e.g., `Enter` on a selected account) that navigates
to the General Ledger for that account. Since GL is not implemented yet, for now wire this to
return `TabAction::ShowMessage("General Ledger not yet available")`.
Also verify that the Journal Entries tab's `navigate_to()` works from any future caller.
**Verify**: CoA tab Enter on account → shows "not yet available" message (will be wired in Phase 3).
**Do NOT**: Implement the General Ledger tab (Phase 3).

---

## Phase 2b Complete When

- [ ] All Phase 2a checks still pass
- [ ] Can create draft journal entries with multiple lines via the form widget
- [ ] Account picker works within the JE form
- [ ] Running debit/credit totals and difference indicator work
- [ ] Posting validates: balanced debits/credits, active accounts, non-placeholder, open period, ≥2 lines
- [ ] Reversing creates a mirror entry with correct linkage and memo
- [ ] JE numbers are sequential and immutable
- [ ] Cleared/Uncleared toggle works on posted entry lines
- [ ] Account balances on CoA tab reflect posted entries
- [ ] Audit log records all JE mutations
- [ ] `cargo clippy -D warnings` and `cargo test` pass
- [ ] `progress.md` updated

## Phase 2b Does NOT Cover

- General Ledger tab (Phase 3)
- AR/AP features (Phase 3)
- Envelope fill on cash receipt (Phase 4)
- Fixed assets, depreciation (Phase 4)
- Recurring entries (Phase 5)
- Inter-entity transactions (Phase 6)
- Period closing/reopening (Phase 3)

**After completing Phase 2b**: Developer reviews all code and signs off before Phase 3 begins.
