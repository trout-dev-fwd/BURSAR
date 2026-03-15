# Phase 3: General Ledger, AR/AP, Fiscal Periods

**Goal**: View per-account transaction history, manage receivables/payables with partial payments,
close/reopen fiscal periods with year-end closing entries.

**Depends on**: Phase 2b (journal entries, posting, reversing, audit log).

**Estimated tasks**: 12

---

## Tasks

### Task 1: Implement General Ledger tab
**Context**: `src/tabs/general_ledger.rs` (stub), `src/db/journal_repo.rs`,
`src/db/account_repo.rs`, `src/tabs/chart_of_accounts.rs` (for reference pattern).
**Action**: Replace stub with real implementation:
- Account selector at top (list or picker from CoA)
- Displays all posted entry lines affecting selected account, chronological order
- Columns: Date, JE Number, Memo, Debit, Credit, Running Balance
- Reconcile state indicator (✓ / ✓✓) inline per line
- Date range filter
- `Enter` on a row → `TabAction::NavigateTo(TabId::JournalEntries, RecordId::JournalEntry(je_id))`
- Implement `navigate_to(RecordId::Account(id))` — switch to that account's ledger
Add to `JournalRepo`:
- `list_lines_for_account(account_id: AccountId, date_range: Option<DateRange>) -> Result<Vec<LedgerRow>>`
**Verify**: Post several JEs affecting the same account. GL shows them chronologically.
Running balance is correct. Filter by date range works. Navigate to source JE works.
**Do NOT**: Implement CIP-specific display or depreciation-specific views (Phase 4).

---

### Task 2: Wire CoA → GL navigation
**Context**: `src/tabs/chart_of_accounts.rs`, `src/tabs/general_ledger.rs`.
**Action**: Update the CoA tab's `Enter` action (from Phase 2b Task 8) to return
`TabAction::NavigateTo(TabId::GeneralLedger, RecordId::Account(selected_id))`.
**Verify**: Select an account on CoA → press Enter → GL tab opens showing that account's history.
**Do NOT**: Add any other cross-tab navigation paths yet.

---

### Task 3: Create ArRepo **[TEST-FIRST]**
**Context**: `src/db/mod.rs`, `specs/data-model.md` (ar_items, ar_payments tables),
`specs/type-system.md` (AR/AP Item Lifecycle).
**Action**: Create `src/db/ar_repo.rs`. Implement `ArRepo<'conn>`:
- `create_item(new: &NewArItem) -> Result<ArItemId>`
- `record_payment(item_id: ArItemId, je_id: JournalEntryId, amount: Money, date: NaiveDate) -> Result<()>`
  — inserts payment, recomputes and writes status (Open/Partial/Paid)
- `list(filter: &ArFilter) -> Result<Vec<ArItem>>` — filter by status
- `get_with_payments(id: ArItemId) -> Result<(ArItem, Vec<ArPayment>)>`
- `get_total_paid(id: ArItemId) -> Result<Money>`
Wire into `EntityDb`.
**Verify**: Create item → partial payment → status=Partial. Remaining payment → status=Paid.
Overpayment attempt → error. Verify Paid is terminal (no further payments accepted).
**Do NOT**: Implement auto-creation of payment JEs (Phase 6 polish). Do NOT implement AP (Task 4).

---

### Task 4: Create ApRepo **[TEST-FIRST]**
**Context**: `src/db/ar_repo.rs` (as reference pattern), `specs/data-model.md` (ap_items, ap_payments).
**Action**: Create `src/db/ap_repo.rs`. Mirrors AR repo with `vendor_name` instead of `customer_name`.
Wire into `EntityDb`.
**Verify**: Same test pattern as AR repo.
**Do NOT**: Duplicate code unnecessarily — if AR and AP share logic, consider shared helper functions.

---

### Task 5: Implement Accounts Receivable tab
**Context**: `src/tabs/accounts_receivable.rs` (stub), `src/db/ar_repo.rs`,
`src/widgets/confirmation.rs`, `src/db/audit_repo.rs`.
**Action**: Replace stub:
- List view: all AR items. Columns: Customer, Description, Amount, Paid, Remaining, Due Date,
  Status, Days Outstanding (computed from due_date vs today)
- Sort by due date (default)
- Filter by status (Open/Partial/Paid/All)
- Visual indicator: overdue items (due_date < today AND status != Paid) highlighted in red/yellow
- Actions:
  - `n` — new AR item: form for customer name, description, amount, due date.
    Select originating JE from a list of posted entries (or enter JE number).
  - `p` — record payment on selected item: amount input, payment date.
    User must specify or create the payment JE (manual linkage for now).
  - `Enter` — view payment history for selected item
- All mutations write to audit log
**Verify**: Create AR item, record partial payment → status changes. View payment history.
Overdue items show visual indicator. Filter works. Audit log entries exist.
**Do NOT**: Auto-create payment JEs (Phase 6 polish). Do NOT implement aging report (Phase 5).

---

### Task 6: Implement Accounts Payable tab
**Context**: `src/tabs/accounts_payable.rs` (stub), `src/db/ap_repo.rs`.
**Action**: Same pattern as AR tab with `vendor_name`. Mirror the implementation.
**Verify**: Same test pattern as AR tab.
**Do NOT**: Duplicate the entire AR tab — share widget logic where possible.

---

### Task 7: AR/AP → JE cross-tab navigation
**Context**: `src/tabs/accounts_receivable.rs`, `src/tabs/accounts_payable.rs`,
`src/tabs/journal_entries.rs`.
**Action**: On AR/AP tabs, add hotkey (e.g., `j`) on selected item to navigate to its
originating journal entry: `TabAction::NavigateTo(TabId::JournalEntries, RecordId::JournalEntry(je_id))`.
**Verify**: Select AR item → press `j` → JE tab opens with the originating entry highlighted.
**Do NOT**: Add reverse navigation (JE → AR/AP) at this time.

---

### Task 8: Implement fiscal period close/reopen **[TEST-FIRST]**
**Context**: `src/db/fiscal_repo.rs`, `src/db/audit_repo.rs`,
`specs/type-system.md` (Fiscal Period Lifecycle).
**Action**: Add to `FiscalRepo`:
- `close_period(id: FiscalPeriodId) -> Result<()>` — validates no draft JEs in period, sets is_closed=1
- `reopen_period(id: FiscalPeriodId) -> Result<()>` — sets is_closed=0, updates reopened_at
- `get_open_periods() -> Result<Vec<FiscalPeriod>>`
Both operations write audit log entries.
**Verify**: Close period → JEs in that period cannot be posted/reversed (test via `post_journal_entry`).
Reopen → mutations allowed again. Attempt to close period with draft JEs → error.
**Do NOT**: Implement year-end close yet (Task 9).

---

### Task 9: Implement year-end close **[TEST-FIRST]**
**Context**: `src/db/fiscal_repo.rs`, `src/db/journal_repo.rs`, `src/db/account_repo.rs`,
`specs/type-system.md` (Year-End Close algorithm in feature spec Section 10.3).
**Action**: Add to `FiscalRepo` (or a service function):
`generate_closing_entries(fiscal_year_id: FiscalYearId) -> Result<Vec<NewJournalEntry>>`:
- Calculate closing entries: zero out Revenue → Income Summary, zero out Expense → Income Summary,
  net Income Summary → Retained Earnings
- Return as draft JEs for user review
`execute_year_end_close(fiscal_year_id: FiscalYearId, closing_je_ids: Vec<JournalEntryId>) -> Result<()>`:
- Post the closing entries, mark fiscal year as closed
**Verify**: With test data (posted revenue and expense entries):
- Generated closing entries zero out all Revenue and Expense accounts
- Retained Earnings receives the net income
- After close, Revenue/Expense balances are zero
- Fiscal year is marked closed
**Do NOT**: Generate closing entries automatically — always present to user for review.

---

### Task 10: Fiscal period management UI
**Context**: `src/app.rs`, `src/db/fiscal_repo.rs`, `src/widgets/confirmation.rs`.
**Action**: Add a fiscal period management modal accessible via a global hotkey (e.g., `F`
or from a menu). Shows all periods with open/closed status. Actions:
- `c` — close selected period (with confirmation)
- `o` — reopen selected period (with confirmation: "Reopening allows modifications to entries in this period.")
- `y` — year-end close (shows generated closing entries for review, confirmation to post)
**Verify**: Close a period → lock indicator appears. Reopen → lock removed.
Year-end close → closing entries generated, reviewed, posted.
**Do NOT**: Make this a full tab — it's a modal/overlay accessible from any context.

---

### Task 11: Enforce period lock on all mutations
**Context**: `src/db/journal_repo.rs`, orchestration functions, `src/db/fiscal_repo.rs`.
**Action**: Verify that ALL journal entry mutations check period lock:
- `post_journal_entry` — already checks fiscal_period_id
- `reverse_journal_entry` — checks the reversal date's period
- `update_reconcile_state` — checks the entry's period
- `create_draft` — should allow drafts in closed periods? **No** — drafts in closed periods
  cannot be posted anyway, so reject at creation.
Add a helper: `fiscal_repo.is_period_open(id: FiscalPeriodId) -> Result<bool>`.
**Verify**: Attempt each mutation type against an entry in a closed period → all rejected with
clear error message.
**Do NOT**: Add period lock checks to non-JE operations (account CRUD is not period-dependent).

---

### Task 12: GL → JE navigation and JE detail → GL navigation
**Context**: `src/tabs/general_ledger.rs`, `src/tabs/journal_entries.rs`.
**Action**: Verify GL → JE works (from Task 1). Add JE detail → GL:
when viewing a JE's lines, a hotkey (e.g., `g`) on a selected line navigates to that
account's General Ledger: `TabAction::NavigateTo(TabId::GeneralLedger, RecordId::Account(account_id))`.
**Verify**: Full navigation loop: CoA → GL → JE → GL (different account) works.
**Do NOT**: Add navigation to AR/AP from JE detail (not enough context to know which AR item).

---

## Phase 3 Complete When

- [ ] All Phase 2b checks still pass
- [ ] General Ledger shows per-account history with running balance and reconcile indicators
- [ ] GL date range filtering works
- [ ] Cross-tab navigation: CoA → GL → JE → GL all work
- [ ] AR items: creation, partial payments, status transitions (Open → Partial → Paid)
- [ ] AP items mirror AR functionality
- [ ] Overdue items visually highlighted
- [ ] AR/AP → JE navigation works
- [ ] Fiscal periods can be closed and reopened
- [ ] Year-end close generates correct closing entries
- [ ] Period lock prevents all mutations in closed periods
- [ ] `cargo clippy -D warnings` and `cargo test` pass
- [ ] `progress.md` updated

## Phase 3 Does NOT Cover

- Envelope budgeting (Phase 4)
- Fixed assets and depreciation (Phase 4)
- Reports (Phase 5)
- Recurring entries (Phase 5)
- Inter-entity transactions (Phase 6)
- Auto-creation of payment JEs for AR/AP (Phase 6 polish)
- Formal bank reconciliation workflow (future scope)

**After completing Phase 3**: Developer reviews all code and signs off before Phase 4 begins.
