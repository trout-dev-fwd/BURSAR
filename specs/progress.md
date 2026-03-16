# Progress Tracker

## Current State
- **Active Phase**: Phase 4
- **Last Completed Task**: Phase 4, Task 5 (envelope transfers)
- **Next Task**: Phase 4, Task 6 (envelope indicators on CoA tab)
- **Blockers**: None

## Completed Phases
- [x] Phase 1: Foundation (completed 2026-03-15)
- [x] Phase 2a: Chart of Accounts (completed 2026-03-15, review fixes applied 2026-03-15)
- [x] Phase 2b: Journal Entries (completed 2026-03-15, review fixes applied 2026-03-15)
- [x] Phase 3: GL, AR/AP, Fiscal Periods (completed 2026-03-15, review fixes applied 2026-03-16)

## Current Phase Progress

### Phase 4: Envelopes, Fixed Assets, Depreciation
- [x] Task 1: Create EnvelopeRepo [TEST-FIRST]
- [x] Task 2: Wire envelope fill into JE post orchestration [TEST-FIRST]
- [x] Task 3: Wire envelope reversal into JE reverse orchestration
- [x] Task 4: Implement Envelopes tab — allocation config + balances
- [x] Task 5: Implement envelope transfers
- [ ] Task 6: Add envelope indicators to Chart of Accounts tab
- [ ] Task 7: Create AssetRepo [TEST-FIRST]
- [ ] Task 8: Implement Fixed Assets tab
- [ ] Task 9: Place in Service action on CoA tab
- [ ] Task 10: Depreciation rounding verification [TEST-FIRST]

### Phase 3: General Ledger, AR/AP, Fiscal Periods
- [x] Task 1: Implement General Ledger tab
- [x] Task 2: Wire CoA → GL navigation
- [x] Task 3: Create ArRepo [TEST-FIRST]
- [x] Task 4: Create ApRepo [TEST-FIRST]
- [x] Task 5: Implement Accounts Receivable tab
- [x] Task 6: Implement Accounts Payable tab
- [x] Task 7: AR/AP → JE cross-tab navigation
- [x] Task 8: Implement fiscal period close/reopen [TEST-FIRST]
- [x] Task 9: Implement year-end close [TEST-FIRST]
- [x] Task 10: Fiscal period management UI modal (global hotkey `f`)
- [x] Task 11: Enforce period lock on all mutations (added create_draft check)
- [x] Task 12: JE detail → GL navigation via `g` key

### Phase 2b: Journal Entries
- [x] Task 1: Create JournalRepo [TEST-FIRST]
- [x] Task 2: Post/reverse orchestration (services/journal.rs)
- [x] Task 3: Create JeForm widget
- [x] Task 4: JE tab — list view
- [x] Task 5: JE tab — actions (new, post, reverse)
- [x] Task 6: Reconciliation state changes
- [x] Task 7: Account balances reflect posted entries
- [x] Task 8: Cross-tab navigation (CoA → JE)

### Phase 2a: Chart of Accounts
- [x] Task 1: Create AccountRepo [TEST-FIRST]
- [x] Task 2: Create AuditRepo [TEST-FIRST]
- [x] Task 3: CoA tab — list view
- [x] Task 4: CoA tab — CRUD actions
- [x] Task 5: Account picker widget
- [x] Task 6: Confirmation widget

### Phase 1: Foundation (complete)
- [x] Task 1: Initialize Cargo project with dependencies
- [x] Task 2: Create Money(i64) newtype
- [x] Task 3: Create Percentage(i64) newtype
- [x] Task 4: Create ID newtypes via macro
- [x] Task 5: Create all enums with FromStr/Display
- [x] Task 6: Create types/mod.rs re-exports
- [x] Task 7: Create workspace config (config.rs)
- [x] Task 8: Create database schema (db/schema.rs)
- [x] Task 9: Create default account seeding
- [x] Task 10: Create EntityDb struct
- [x] Task 11: Create FiscalRepo with year/period creation
- [x] Task 12: Wire fiscal year into EntityDb::create
- [x] Task 13: Create Tab trait and enums
- [x] Task 14: Create stub tab implementations
- [x] Task 15: Create StatusBar widget
- [x] Task 16: Create App struct and event loop
- [x] Task 17: Create main.rs entry point
- [x] Task 18: Entity creation flow in TUI
- [x] Task 19: Entity open/picker flow
- [x] Task 20: Set up pre-commit hook

## Decisions & Discoveries

- **[Phase 4, Task 5]**: Transfer modal uses a `TransferStep` enum state machine (SelectSource →
  SelectDest → EnterAmount → Confirm) stored in a `TransferModal` struct on `EnvelopesTab`.
  Account list for source/dest comes from `self.accounts` filtered by `self.allocations`. Dest
  list excludes the selected source. Balance validation: `amount > src_envelope_balance → error`.
  On confirm: calls `EnvelopeRepo::record_transfer()`, updates local `envelope_balances` cache
  immediately (no full reload needed), writes `EnvelopeTransfer` audit entry. `t` key only active
  in Balances view. `parse_money` from `widgets/je_form.rs` reused for amount parsing.

- **[Phase 4, Task 4]**: `EnvelopesTab` has two sub-views toggled by Tab: Allocation Config
  (all non-placeholder accounts with editable %) and Envelope Balances (allocated accounts only,
  shows GL Balance / Earmarked / Available). Balance data pre-loaded in `refresh()` since
  `render()` lacks `&EntityDb`. Allocation rows highlighted in Cyan when allocated.

- **[Phase 4, Task 3]**: `reverse_journal_entry()` pre-fetches fills with `get_fills_for_je()`
  before opening the transaction, then calls `record_reversal()` inside the transaction for each.

- **[Phase 4, Task 2]**: Cash account detected by: `account_type == Asset && !is_placeholder &&
  name.to_lowercase().contains(cash|bank|checking|savings)`. Owner's Draw suppression check:
  `account_type == Equity && is_contra`. No schema change needed.

- **[Phase 4, Task 1]**: `EnvelopeRepo` uses `ON CONFLICT(account_id) DO UPDATE` for
  `set_allocation` (upsert). `record_transfer` creates paired rows with shared `transfer_group_id`
  (UUID). `get_balance` uses `COALESCE(SUM(amount), 0)`.

- **[Phase 3, Task 12]**: `g` key in JE detail view returns
  `TabAction::NavigateTo(TabId::GeneralLedger, RecordId::Account(line.account_id))`. The full
  navigation loop (CoA → GL → JE → GL) now works. `TabId` was added to the imports in
  `journal_entries.rs`.

- **[Phase 3, Task 11]**: `create_draft` now rejects closed fiscal periods at creation time
  (direct SQL check, not via FiscalRepo to avoid circular imports). Rationale: drafts in closed
  periods can never be posted, so refusing early avoids orphaned un-postable entries. One test
  added: `create_draft_rejects_closed_period`.

- **[Phase 3, Task 10]**: Fiscal period modal (`FiscalModal`) is a `src/widgets/fiscal_modal.rs`
  overlay, NOT a separate tab. Opened via global hotkey `f`. State machine: Browsing →
  ConfirmClose / ConfirmReopen / YearEndReview. Year-end review shows human-readable preview of
  closing entries, then Enter creates drafts + calls `execute_year_end_close` in one shot.
  `FiscalYear` struct + `list_fiscal_years()` added to `FiscalRepo`.

- **[Phase 3, Task 9]**: Year-end close implemented as two service functions in
  `src/services/fiscal.rs`: `generate_closing_entries` (returns `Vec<NewJournalEntry>` for review)
  and `execute_year_end_close` (creates drafts + posts + marks FY closed). Uses a single combined
  closing JE rather than 3 Income Summary JEs. Account 1100 "Checking Account" used in tests
  (not 1100 the placeholder — actually 1110 is Checking Account; 1100 is "Cash & Bank Accounts",
  a placeholder parent).

- **[Phase 3, Tasks 5-7]**: AR and AP tabs implement `set_entity_name()`, status filter cycling
  (`s` key), overdue highlighting (red for due_date < today && status != Paid), payment history
  view (Enter), and `o` key for originating JE navigation. AR account = 1200, AP account = 2100
  (both looked up by number + !is_placeholder).

- **[Phase 3, Task 3-4]**: `ArRepo` and `ApRepo` mirror `JournalRepo` dynamic SQL filter pattern.
  `record_payment` recomputes status after each payment: Open → Partial → Paid (terminal).
  Overpayment guard checks total_paid + new_amount ≤ total_amount.

- **[Phase 3, Task 1]**: GL tab uses `list_lines_for_account(account_id, date_range)` which
  computes running balance in SQL using `SUM() OVER (ORDER BY entry_date, je_id)`. Debit-normal
  accounts (Asset, Expense): balance = SUM(debit - credit). Credit-normal accounts (Liability,
  Equity, Revenue): balance = SUM(credit - debit). The sign flip is applied at display time in
  the tab based on `AccountType::normal_balance()`.

- **[Phase 2b, Task 8]**: CoA tab `Enter` now differentiates: group accounts (has_children) toggle
  expand/collapse as before; leaf accounts return `TabAction::ShowMessage("General Ledger not yet
  available")`. This wires the hotkey path for Phase 3 without duplicating the expand key. Hint bar
  updated to reflect both behaviors. JE tab `navigate_to(RecordId::JournalEntry(id))` was already
  implemented in Task 4 and verified correct.

- **[Phase 2b, Task 3]**: `JeForm` is self-contained — embeds `AccountPicker` directly and returns
  `JeFormAction::Submitted(JeFormOutput)` / `Cancelled` / `Pending`. `JeFormOutput` does not include
  `fiscal_period_id`; the caller (JE tab or inter-entity modal) resolves that from `entry_date`.
  `parse_money(s)` is public for use by callers that need to display Money from user strings.
  `let-chains` (`if let A && let B`) required to satisfy `clippy::collapsible_if`.

- **[Phase 2b, Task 2]**: `post_journal_entry` validates Draft status, ≥2 lines, balanced
  debits==credits, all accounts active+non-placeholder, fiscal period open. Contains
  `// TODO(Phase 4): Check for cash receipt and trigger envelope fills` at the envelope
  fill insertion point. `reverse_journal_entry` creates a mirror draft (flipped debit/credit),
  promotes it to Posted, then marks the original `is_reversed=true` — all in one transaction.
  `JournalError` variants holding IDs use `i64` (not `JournalEntryId`) because `JournalEntryId`
  does not implement `Display` (required by `thiserror`).

- **[Phase 2b, Task 1]**: `JournalRepo::list()` uses dynamic SQL building with
  `params_from_iter` — not the sentinel pattern from `AuditRepo`. `NewJournalEntry` includes
  `reversal_of_je_id: Option<JournalEntryId>` (NULL for normal entries) so Task 2 can set the
  link at creation time. `entity_db_from_conn()` test helper added to `db/mod.rs` (cfg(test))
  to wrap an in-memory connection for service-layer tests.

- **[Phase 2a, Task 4 + review fix]**: CRUD modals implemented as a `CoaModal` enum on the tab struct.
  After review, the Add form's parent field was wired to use the AccountPicker widget (opens as a
  sub-overlay popup), and the deactivate/activate confirmation was wired to use the Confirmation
  widget. This establishes the integration pattern for Phase 2b's JE form. Entity name is set on
  the tab via `set_entity_name()` called from `EntityContext::new`.

- **[Phase 2a, Task 3]**: `EntityContext::new` now calls `tab.refresh(&db)` on all tabs after
  construction so data shows immediately on first render. `Table::highlight_style` is deprecated
  in ratatui 0.29 — use `row_highlight_style` instead. `TableState` must be cloned for immutable
  `render()` since `render_stateful_widget` requires `&mut TableState`.

- **[Phase 2a, Task 2 + review fix]**: `AuditRepo::list` uses empty-string sentinels
  (`?1 = '' OR ...`) — acceptable for the small audit_log table but NOT to be propagated to
  high-volume repos like JournalRepo (use dynamic SQL building instead). After review,
  `append()` was changed to accept `Option<&str>` / `Option<i64>` for `record_type` /
  `record_id`, matching the nullable schema columns. Entity-level events (e.g., YearEndClose)
  can now pass None.

- **[Phase 2a, Task 1]**: `row_to_account` is a free function (not a method) to satisfy rusqlite's
  `FnMut(&Row) -> Result<T>` callback signature — closures borrowing `self` cause lifetime conflicts
  with `query_map`. Parent-existence check is done app-side with a COUNT query before INSERT
  (belt-and-suspenders, as SQLite FK constraints also enforce this with PRAGMA foreign_keys=ON).
  `get_balance` returns raw `SUM(debit_amount - credit_amount)` across posted JE lines; direction
  interpretation (normal vs. contra) is deferred to Phase 3 display logic.


- **[Phase 1, Task 2]**: Introduced `src/lib.rs` to avoid dead_code warnings in the binary crate.
  In a pure binary, all types are considered dead until reachable from `main()`. With a lib.rs,
  `pub` items are not flagged. This is standard Rust practice for binaries with substantial library code.
  All future modules are declared in lib.rs.

- **[Phase 1, Task 9]**: "feature spec Section 2.2 (Standard Built-in Account Categories)" referenced
  in phase-1.md was not found in any spec file. The account hierarchy was designed as a judgment call
  for a small real estate LLC. Hierarchy: Assets (1000–1521), Liabilities (2000–2400),
  Equity (3000–3300), Revenue (4000–4200), Expenses (5000–5800). Contra accounts: Accumulated
  Depreciation - Buildings (1521), Owner's Draw (3200). **Developer should validate this hierarchy
  and request additions/removals before Phase 2a.**

- **[Phase 1, Task 13]**: Tab trait defined with `navigate_to` as a default no-op method matching
  the architecture spec exactly.

- **[Phase 1, Task 16]**: App uses `&&let` pattern (Rust 2024 let-chains) for collapsing nested
  `if poll() { if let Event::Key ... }` — required by clippy::collapsible_if.

- **[Phase 1, Task 20]**: Pre-commit hook sources `~/.cargo/env` before running cargo commands,
  which is necessary because git hooks run in a minimal shell environment without the user's PATH.

## Phase 2a Review Fixes (2026-03-15)

Applied 9 fixes from the end-of-phase developer review:

1. **CoA tab wired to use AccountPicker + Confirmation widgets** (was inline reimplementations)
2. **AccountRepo::update() made atomic** — single COALESCE-based UPDATE instead of two statements
3. **AuditRepo::append() signature fixed** — record_type/record_id now Option to match nullable schema
4. **AuditRepo::list() sentinel pattern documented** — doc comment warns against propagation
5. **N+1 balance queries replaced** — new get_all_balances() bulk query, CoA refresh uses 1 query
6. **Duplicated now_str() consolidated** — shared helper in db/mod.rs
7. **Duplicated centered_rect() consolidated** — shared helper in widgets/mod.rs
8. **Edit form Enter behavior fixed** — now advances through fields before submit (consistent with Add)
9. **AccountReactivated audit action added** — reactivations no longer logged as AccountDeactivated

## Phase 2b Review Fixes (2026-03-15)

Applied 3 fixes from the end-of-phase developer review:

1. **`update_reconcile_state()` guard rails** (must-fix) — repo method now queries current
   reconcile state and period `is_closed` before updating. Rejects Reconciled lines
   ("permanent state") and lines in closed fiscal periods. Two tests added for both rejection
   paths. Defense-in-depth: the tab layer already checked these, but the repo now enforces
   the invariants independently for future callers.
2. **Defensive tests for post/reverse edge cases** — added `post_already_posted_entry_returns_not_draft_error`
   and `reverse_draft_entry_returns_not_posted_error` to `services/journal.rs` tests.
3. **`get_next_je_number()` cleanup** — replaced chained `.unwrap_or("0").parse().unwrap_or(0)`
   with `.and_then(|suffix| suffix.parse().ok()).unwrap_or(0)` for clarity.

## Phase 3 Review Fixes (2026-03-16)

Applied 3 fixes from post-phase developer review:

1. **AccountPicker placeholder bug** — CoA tab's parent picker now uses `AccountPicker::with_placeholders()`
   so placeholder accounts appear as valid parent choices. Added `include_placeholders: bool` config to
   `AccountPicker` (defaults to false, preserving JE form behavior).
2. **ArApStatus parameterization** — AR/AP INSERT queries now use parameterized enum values instead of
   hardcoded status strings.
3. **Account deletion for unused accounts** — `AccountRepo::delete()` permanently removes accounts after
   six guard checks (journal entries, AR/AP items, child accounts, envelope allocations, fixed assets).
   CoA tab `x` key opens confirmation dialog, writes `AccountDeleted` audit entry. `AuditAction::AccountDeleted`
   variant added to the enum.

## Known Issues
- None currently.
