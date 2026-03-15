# Phase 2a: Chart of Accounts

**Goal**: Full account CRUD, hierarchical display, audit logging. The first real domain feature.

**Depends on**: Phase 1 (types, schema, TUI shell, entity DB).

**Estimated tasks**: 6

---

## Tasks

### Task 1: Create AccountRepo **[TEST-FIRST]**
**Context**: `src/db/mod.rs`, `src/types/`, `specs/data-model.md` (accounts table).
**Action**: Create `src/db/account_repo.rs`. Implement `AccountRepo<'conn>`:
- `list_all() -> Result<Vec<Account>>`
- `list_active() -> Result<Vec<Account>>` (is_active = 1)
- `get_by_id(AccountId) -> Result<Account>`
- `create(new: &NewAccount) -> Result<AccountId>`
- `update(id: AccountId, changes: &AccountUpdate) -> Result<()>`
- `deactivate(id: AccountId) -> Result<()>`
- `activate(id: AccountId) -> Result<()>`
- `get_children(parent_id: AccountId) -> Result<Vec<Account>>`
- `search(query: &str) -> Result<Vec<Account>>` (LIKE on name and number)
- `get_balance(id: AccountId) -> Result<Money>` (sum of posted JE lines — returns Money::zero()
  for now since no JE lines exist yet; the query should be correct for when they do)

Define `Account`, `NewAccount`, `AccountUpdate` structs in this file or a shared models file.
Wire into `EntityDb` as `fn accounts(&self) -> AccountRepo<'_>`.
**Verify**: Tests against in-memory DB with seeded accounts:
- `list_active()` returns seeded accounts
- `create()` adds a new account, `get_by_id()` retrieves it
- `update()` changes name/number
- `deactivate()` sets is_active=0, `list_active()` excludes it
- `get_children()` returns correct sub-accounts
- `search("cash")` returns Cash & Bank Accounts
- Reject: create account with duplicate number → error
- Reject: create account referencing non-existent parent → error
**Do NOT**: Implement fixed asset details, envelope indicators, or balance calculations
involving journal entries. The `get_balance` query will be correct SQL but return 0 until JEs exist.

---

### Task 2: Create AuditRepo **[TEST-FIRST]**
**Context**: `src/db/mod.rs`, `specs/data-model.md` (audit_log table), `specs/type-system.md` (AuditAction).
**Action**: Create `src/db/audit_repo.rs`. Implement `AuditRepo<'conn>`:
- `append(action: AuditAction, entity_name: &str, record_type: &str, record_id: i64, description: &str) -> Result<AuditLogId>`
- `list(filter: &AuditFilter) -> Result<Vec<AuditEntry>>` (filterable by date range, action type)

Define `AuditEntry` and `AuditFilter` structs.
Wire into `EntityDb`.
**Verify**: Append 3 entries with different action types. `list` with no filter returns all 3.
Filter by action type returns only matching. Filter by date range works.
Verify: no `update` or `delete` methods exist (append-only by design).
**Do NOT**: Wire audit logging into other operations yet (that's Tasks 3-4).

---

### Task 3: Implement Chart of Accounts tab — list view
**Context**: `src/tabs/chart_of_accounts.rs` (stub from Phase 1), `src/db/account_repo.rs`,
`src/widgets/status_bar.rs`.
**Action**: Replace the stub with a real implementation:
- Display: hierarchical account list with indentation for sub-accounts
- Columns: Number, Name, Type, Balance, Active/Inactive indicator, Placeholder indicator
- Navigation: `↑↓` to scroll through the list, `Enter` to expand/collapse sub-account groups
- Search: `/` activates substring filter on name and number
- Implement `refresh()` to re-query accounts from the DB
**Verify**: Launch app → CoA tab shows seeded accounts in hierarchy. Scroll works.
Expand/collapse works. Search filters correctly. Placeholder accounts show indicator.
**Do NOT**: Implement add/edit/deactivate actions (Task 4), envelope indicators (Phase 4),
or place-in-service action (Phase 4). Display only.

---

### Task 4: Implement Chart of Accounts tab — CRUD actions
**Context**: `src/tabs/chart_of_accounts.rs`, `src/db/account_repo.rs`, `src/db/audit_repo.rs`.
**Action**: Add actions to the CoA tab:
- `a` — add new account: modal/inline form for number, name, type, parent (account picker),
  contra flag, placeholder flag. Calls `account_repo.create()`. Writes audit log entry
  (AuditAction::AccountCreated with description including account name and number).
- `e` — edit selected account: modal for name, number changes. Type cannot change after creation.
  Calls `account_repo.update()`. Writes audit log (AccountModified).
- `d` — deactivate/activate toggle: confirmation prompt. Calls `deactivate()/activate()`.
  Writes audit log (AccountDeactivated).
- After each mutation: return `TabAction::RefreshData`
**Verify**: Add a new sub-account → appears in hierarchy at correct position. Edit its name →
reflected immediately. Deactivate → shows inactive indicator. Check audit log table has entries
for all 3 operations with correct descriptions.
**Do NOT**: Implement account deletion (accounts are deactivated, never deleted).
Do NOT implement fixed asset details or place-in-service.

---

### Task 5: Create account picker widget
**Context**: `specs/architecture.md` (AccountPicker section).
**Action**: Create `src/widgets/account_picker.rs`. A reusable popup widget:
- Text input field that filters account list in real-time (substring on name and number)
- Shows matching accounts in a dropdown-style list below the input
- Excludes placeholder accounts (`is_placeholder = 1`) and inactive accounts from results
- `Enter` selects the highlighted account, `Esc` cancels
- Returns `Option<AccountId>` (None if cancelled)
**Verify**: Open picker → type "cas" → shows Cash-related accounts. Arrow keys navigate list.
Enter confirms selection. Esc returns None. Placeholder accounts do not appear.
**Do NOT**: Implement fuzzy matching. Substring/prefix only.

---

### Task 6: Create confirmation widget
**Context**: `specs/architecture.md` (Confirmation section).
**Action**: Create `src/widgets/confirmation.rs`. A reusable modal:
- Displays a message string and two buttons: Yes / No
- `y` or `Enter` (on Yes) confirms, `n` or `Esc` cancels
- Returns `bool`
**Verify**: Display confirmation, press `y` → returns true. Press `n` → returns false.
Press `Esc` → returns false.
**Do NOT**: Add any domain-specific logic. This is a pure UI widget.

---

## Phase 2a Complete When

- [ ] All Phase 1 checks still pass
- [ ] Chart of Accounts displays seeded accounts in hierarchy with correct indentation
- [ ] Scrolling and expand/collapse work
- [ ] Search filters accounts by name and number
- [ ] Can create new accounts (including sub-accounts with parent)
- [ ] Can edit account name/number
- [ ] Can deactivate/activate accounts
- [ ] Account picker widget filters and selects correctly
- [ ] Confirmation widget works
- [ ] Audit log records all account mutations with descriptive messages
- [ ] `cargo clippy -D warnings` and `cargo test` pass
- [ ] `progress.md` updated

## Phase 2a Does NOT Cover

- Journal entries (Phase 2b)
- Account balances from transactions (will show $0.00 until JEs are posted)
- Envelope indicators on accounts (Phase 4)
- Fixed asset details or place-in-service (Phase 4)
- General Ledger view (Phase 3)

**After completing Phase 2a**: Developer reviews all code and signs off before Phase 2b begins.
