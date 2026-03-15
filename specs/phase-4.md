# Phase 4: Envelopes, Fixed Assets, Depreciation

**Goal**: Envelope budgeting with automatic cash-receipt fills and manual transfers.
Fixed asset register with straight-line depreciation generation.

**Depends on**: Phase 3 (journal entries, GL, fiscal periods, audit log).

**Estimated tasks**: 10

---

## Tasks

### Task 1: Create EnvelopeRepo **[TEST-FIRST]**
**Context**: `src/db/mod.rs`, `specs/data-model.md` (envelope_allocations, envelope_ledger tables),
`specs/type-system.md` (Envelope algorithms).
**Action**: Create `src/db/envelope_repo.rs`. Implement `EnvelopeRepo<'conn>`:
- `set_allocation(account_id: AccountId, pct: Percentage) -> Result<()>`
- `remove_allocation(account_id: AccountId) -> Result<()>`
- `get_all_allocations() -> Result<Vec<EnvelopeAllocation>>`
- `record_fill(account_id: AccountId, amount: Money, je_id: JournalEntryId) -> Result<()>`
- `record_transfer(source: AccountId, dest: AccountId, amount: Money) -> Result<Uuid>`
  — creates paired ledger rows with shared `transfer_group_id`
- `record_reversal(account_id: AccountId, amount: Money, je_id: JournalEntryId) -> Result<()>`
- `get_balance(account_id: AccountId) -> Result<Money>` — SUM(amount) from ledger
- `get_ledger(account_id: AccountId) -> Result<Vec<EnvelopeLedgerEntry>>`
Wire into `EntityDb`.
**Verify**: Set allocation → record fill → check balance. Transfer between accounts → both
balances update. SUM of transfer pair rows with same transfer_group_id = 0. Record reversal → balance decreases.
**Do NOT**: Wire into JE posting (Task 2). This is data access only.

---

### Task 2: Wire envelope fill into JE post orchestration **[TEST-FIRST]**
**Context**: `src/db/envelope_repo.rs`, post orchestration function from Phase 2b,
`specs/type-system.md` (Envelope Fill on Cash Receipt algorithm).
**Action**: Update `post_journal_entry()`. After posting, check if any line debits a Cash/Bank
account (identify by account type = Asset and account name/number matching a Cash pattern,
OR add a `is_cash_account` flag to accounts — discuss with developer). If yes:
- Sum all Cash/Bank debit amounts = total cash received
- Check credit side: if Owner's Draw is credited → do NOT fill. Skip.
- Owner's Capital contribution (credit to Owner's Capital) → DO fill.
- For each configured allocation: `fill_amount = cash_received.apply_percentage(pct)`
- Insert Fill entries into envelope_ledger
**Verify**: Post a cash receipt JE → envelope balances increase by correct amounts.
Post an Owner's Draw JE → no envelope changes.
Post a capital contribution (debit Cash, credit Owner's Capital) → fills occur.
Post a non-cash JE (debit Expense, credit AP) → no fills.
**Do NOT**: Implement envelope reversal wiring yet (Task 3).

---

### Task 3: Wire envelope reversal into JE reverse orchestration
**Context**: `src/db/envelope_repo.rs`, reverse orchestration function from Phase 2b.
**Action**: Update `reverse_journal_entry()`. If the original JE triggered envelope fills
(check envelope_ledger for Fill entries with this je_id), create Reversal entries that undo them.
**Verify**: Post cash receipt → fills created. Reverse → reversal entries created.
Net envelope balance for each account = 0.
**Do NOT**: Handle edge cases of partial reversals — a reversal always undoes the full entry.

---

### Task 4: Implement Envelopes tab — allocation config
**Context**: `src/tabs/envelopes.rs` (stub), `src/db/envelope_repo.rs`, `src/db/account_repo.rs`.
**Action**: Replace stub with real implementation. Two sub-views:
1. **Allocation config**: List of all accounts with a percentage column.
   Edit percentage inline (select account, type percentage, press Enter to save).
   Save calls `set_allocation()`. Remove allocation by setting to 0 or pressing `d`.
   All changes write to audit log.
2. **Envelope balances**: Accounts with allocations. Columns: Account Name, Allocation %,
   Total Account Balance, Earmarked Amount, Available (Balance - Earmarked).
**Verify**: Set allocations → post cash receipt from JE tab → return to Envelopes tab →
balances reflect fills. Change an allocation → future fills use new percentage.
**Do NOT**: Implement transfer UI yet (Task 5).

---

### Task 5: Implement envelope transfers
**Context**: `src/tabs/envelopes.rs`, `src/db/envelope_repo.rs`, `src/widgets/confirmation.rs`.
**Action**: Add transfer action to Envelopes tab:
- `t` — transfer: select source account (from allocated accounts), destination account, amount.
- Validate: source envelope balance >= transfer amount.
- Confirmation prompt.
- Calls `envelope_repo.record_transfer()`.
- No journal entry created. No GL impact.
**Verify**: Transfer $500 from account A to account B → A's earmark decreases, B's increases.
Total earmarked across all accounts unchanged. View ledger → transfer entries visible.
Attempt transfer exceeding balance → error.
**Do NOT**: Create any journal entries for transfers. They are purely budgetary.

---

### Task 6: Add envelope indicators to Chart of Accounts tab
**Context**: `src/tabs/chart_of_accounts.rs`, `src/db/envelope_repo.rs`.
**Action**: Update CoA tab: for any account with an envelope allocation, display the earmarked
amount inline next to the account balance. Visual distinction: brackets or secondary color
separating total balance from earmarked portion. E.g., `$5,000.00 [$1,500.00 earmarked]`.
**Verify**: Accounts with allocations show earmarked amounts. Accounts without allocations
show only the balance. Earmarked amounts update after cash receipts and transfers.
**Do NOT**: Change any other tab's display.

---

### Task 7: Create AssetRepo **[TEST-FIRST]**
**Context**: `src/db/mod.rs`, `specs/data-model.md` (fixed_asset_details table),
`specs/type-system.md` (Depreciation Generation algorithm).
**Action**: Create `src/db/asset_repo.rs`. Implement `AssetRepo<'conn>`:
- `create_fixed_asset(account_id: AccountId, details: &NewFixedAssetDetails) -> Result<FixedAssetDetailId>`
- `place_in_service(cip_account_id: AccountId, target_asset_account_id: AccountId,
  in_service_date: NaiveDate, useful_life_months: u32) -> Result<JournalEntryId>`
  — generates transfer JE (Debit Fixed Asset, Credit CIP), populates fixed_asset_details
- `list_assets() -> Result<Vec<FixedAssetWithDetails>>` — includes computed book value
- `generate_pending_depreciation(as_of_period: FiscalPeriodId) -> Result<Vec<NewJournalEntry>>`
  — generates draft JEs for un-generated months
Wire into `EntityDb`.
**Verify**: Create CIP account, place in service → transfer JE generated with correct amounts.
Generate depreciation → correct monthly amounts. Land account → no depreciation generated.
**Do NOT**: Handle depreciation rounding yet (Task 9).

---

### Task 8: Implement Fixed Assets tab
**Context**: `src/tabs/fixed_assets.rs` (stub), `src/db/asset_repo.rs`.
**Action**: Replace stub:
- Asset register: list all fixed assets with cost basis, in-service date, useful life,
  monthly depreciation amount, accumulated depreciation, current book value.
- Land accounts show "Non-Depreciable" flag, no depreciation fields.
- Depreciation schedule view: select asset → see month-by-month entries.
- `g` — generate depreciation: generates pending entries for all assets through current period.
  Presents as draft JEs for review (user must post them from JE tab).
**Verify**: View asset register with correct computed values. Generate depreciation →
draft entries appear in JE tab. Post them → accumulated depreciation updates on asset register.
**Do NOT**: Implement MACRS or other depreciation methods (out of scope).

---

### Task 9: Place in Service action on CoA tab
**Context**: `src/tabs/chart_of_accounts.rs`, `src/db/asset_repo.rs`.
**Action**: When a CIP (Construction in Progress) account is selected, add action:
- `s` — place in service: form for target Fixed Asset account (picker), in-service date,
  useful life (months). Calls `asset_repo.place_in_service()`.
**Verify**: Select CIP account → press `s` → fill form → transfer JE created.
Fixed asset appears in asset register. CIP account balance decreases.
**Do NOT**: Allow place-in-service on non-CIP accounts.

---

### Task 10: Depreciation rounding verification **[TEST-FIRST]**
**Context**: `src/db/asset_repo.rs`.
**Action**: Create specific test: asset where `cost_basis % useful_life_months != 0`.
Generate ALL depreciation entries through end of useful life.
Verify total depreciation exactly equals cost basis (final month absorbs remainder).
**Verify**: Explicit test case. E.g., cost $10,000, 36 months → $277.78/mo × 35 + $277.70 final.
Total must equal exactly $10,000.
**Do NOT**: Change the depreciation method. Straight-line only, rounding on final month.

---

## Phase 4 Complete When

- [ ] All Phase 3 checks still pass
- [ ] Envelope allocations configurable per account
- [ ] Cash receipt JEs auto-fill envelopes at configured percentages
- [ ] Owner's Draw does NOT trigger fills; Owner's Capital DOES
- [ ] Reversing a cash receipt reverses the fills
- [ ] Envelope transfers work (budgetary only, no GL impact)
- [ ] Earmarked amounts display on Chart of Accounts
- [ ] CIP → Fixed Asset "place in service" generates correct transfer JE
- [ ] Depreciation generates correct monthly amounts (straight-line)
- [ ] Final-month rounding absorbs remainder to match cost basis exactly
- [ ] Land accounts flagged non-depreciable, no depreciation generated
- [ ] `cargo clippy -D warnings` and `cargo test` pass
- [ ] `progress.md` updated

## Phase 4 Does NOT Cover

- Reports (Phase 5)
- Recurring entries (Phase 5)
- Inter-entity transactions (Phase 6)
- Startup checks (Phase 5)
- Help overlay (Phase 5)

**After completing Phase 4**: Developer reviews all code and signs off before Phase 5 begins.
