# V1 Handoff — Double-Entry Bookkeeping TUI

This document provides everything needed to understand and extend this project. V1 is complete — all 85 tasks across 7 phases are done, 372 tests pass, and the application is ready for daily use.

---

## What This Is

A **terminal-based double-entry bookkeeping application** written in Rust using Ratatui + SQLite. Designed for a single business owner managing 1–2 LLCs (e.g., a land-holding entity and a rental entity) who needs proper accrual-basis accounting with intercompany journal entry support and envelope budgeting layered on top.

**Key characteristics:**
- Fully keyboard-driven TUI (no mouse, no web UI)
- Synchronous — no async, no tokio, no network calls
- Single-user, no authentication
- Each legal entity has its own `.sqlite` file
- Accrual-basis accounting with envelope budgeting via cash receipts
- Inter-entity journal entries between two entities with crash recovery

---

## Tech Stack

| Component | Choice | Version |
|-----------|--------|---------|
| Language | Rust (2024 edition, stable toolchain) | |
| TUI framework | `ratatui` + `crossterm` | 0.29 / 0.28 |
| Database | SQLite via `rusqlite` (bundled) | 0.32 |
| Config | `serde` + `toml` | 1.x / 0.8 |
| Dates | `chrono` (NaiveDate, NaiveDateTime) | 0.4 |
| UUIDs | `uuid` (v4) | 1.x |
| Error handling | `thiserror` (domain) + `anyhow` (CLI boundary) | 2.x / 1.x |
| Logging | `tracing` + `tracing-subscriber` | 0.1 / 0.3 |
| Async runtime | **None** | — |

---

## Codebase Overview

**30,269 lines of Rust** across 54 source files. **372 passing tests.**

### Module Structure

```
src/
├── main.rs                 (46 lines)    Entry point: load config, open entity, run event loop
├── app.rs                  (1,215)       App struct, event loop, global hotkey dispatch
├── lib.rs                  (13)          Crate root, module declarations
├── config.rs               (196)         workspace.toml parsing, tilde expansion
├── startup.rs              (601)         Startup checks: recurring entries, inter-entity recovery, depreciation
├── types/
│   ├── mod.rs              (16)          Re-exports
│   ├── money.rs            (179)         Money(i64) newtype — 10^8 scale
│   ├── percentage.rs       (94)          Percentage(i64) newtype — 10^6 scale
│   ├── ids.rs              (72)          ID newtypes via newtype_id! macro
│   └── enums.rs            (465)         AccountType, JournalEntryStatus, ReconcileState, etc.
├── db/
│   ├── mod.rs              (219)         EntityDb struct (owns Connection, provides repo accessors)
│   ├── schema.rs           (509)         CREATE TABLE statements, schema init, seed data, migrations
│   ├── account_repo.rs     (1,096)       CRUD for accounts + balances + search
│   ├── journal_repo.rs     (1,404)       JE + lines: create, list, search, reconcile state
│   ├── ar_repo.rs          (689)         AR items + payments
│   ├── ap_repo.rs          (535)         AP items + payments
│   ├── envelope_repo.rs    (662)         Allocations, fill, transfer, balance queries
│   ├── fiscal_repo.rs      (654)         Fiscal years + periods: create, close, reopen
│   ├── asset_repo.rs       (1,225)       Fixed asset register + depreciation generation
│   ├── recurring_repo.rs   (568)         Recurring templates: list upcoming, generate entries
│   └── audit_repo.rs       (335)         Audit log: append-only writes, filtered reads
├── services/
│   ├── mod.rs              (2)
│   ├── journal.rs          (1,370)       Post/reverse orchestration with envelope fills
│   └── fiscal.rs           (664)         Year-end close: generate closing entries, execute
├── tabs/
│   ├── mod.rs              (126)         Tab trait, TabAction enum, TabId enum, RecordId enum
│   ├── chart_of_accounts.rs (1,794)      Account list, balances, envelope indicators, CRUD, place-in-service
│   ├── general_ledger.rs   (673)         Per-account transaction history, date filtering
│   ├── journal_entries.rs  (1,516)       Entry list, new entry form, recurring template creation
│   ├── accounts_receivable.rs (1,296)    Open items, payments, history, JE navigation
│   ├── accounts_payable.rs (1,205)       Open items, payments, history, JE navigation
│   ├── envelopes.rs        (1,077)       Allocation config, balances (FY-filtered), transfers
│   ├── fixed_assets.rs     (425)         Asset register, depreciation schedule
│   ├── reports.rs          (700)         Report selection, parameter input, file generation
│   └── audit_log.rs        (545)         Immutable event list, date/action filtering
├── inter_entity/
│   ├── mod.rs              (427)         InterEntityMode: owns secondary DB, manages modal lifecycle
│   ├── form.rs             (878)         Split-pane form with two JeForm instances
│   ├── write_protocol.rs   (534)         Draft→Post two-phase commit across two DBs
│   └── recovery.rs         (452)         Startup orphan detection and resolution prompts
├── reports/
│   ├── mod.rs              (538)         Report trait, shared formatting (box-drawing, tables, headers)
│   ├── trial_balance.rs    (286)
│   ├── balance_sheet.rs    (272)
│   ├── income_statement.rs (260)
│   ├── cash_flow.rs        (295)
│   ├── account_detail.rs   (288)
│   ├── ar_aging.rs         (311)
│   ├── ap_aging.rs         (249)
│   └── fixed_asset_schedule.rs (227)
├── widgets/
│   ├── mod.rs              (35)          Shared helpers: centered_rect(), now_str()
│   ├── account_picker.rs   (460)         Substring-match account selector (reused across tabs)
│   ├── confirmation.rs     (213)         Yes/No modal
│   ├── je_form.rs          (1,042)       Journal entry form widget (reused in JE tab + inter-entity)
│   ├── fiscal_modal.rs     (603)         Fiscal period management overlay (close/reopen/year-end)
│   └── status_bar.rs       (235)         Entity name, fiscal period, unsaved indicator, messages
└── integration_tests.rs    (478)         Full lifecycle end-to-end test
```

---

## Architecture

### Event Loop

Synchronous `crossterm` polling with a **500ms tick rate**. No tokio. The loop:
1. Renders via `terminal.draw()`
2. Polls for keyboard input (500ms timeout)
3. Dispatches to modal → inter-entity → global hotkeys → active tab
4. Ticks status bar (message timeouts, unsaved indicator)

### App Structure

```
App
├── entity: EntityContext
│   ├── db: EntityDb           (holds rusqlite::Connection)
│   ├── name: String
│   └── tabs: Vec<Box<dyn Tab>>  (9 tabs)
├── config: WorkspaceConfig
├── active_tab: usize
├── mode: AppMode              (Normal | InterEntity | Modal)
├── status_bar: StatusBar
└── should_quit: bool
```

### Tab Trait

Every tab implements:
- `handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction` — process input, return action
- `render(&self, frame: &mut Frame, area: Rect)` — draw UI
- `refresh(&mut self, db: &EntityDb)` — re-query display data after mutations
- `navigate_to(&mut self, record_id: RecordId, db: &EntityDb)` — cross-tab navigation (default no-op)
- `title(&self) -> &str` — tab bar label
- `has_unsaved_changes(&self) -> bool` — for `[*]` indicator (default false)

Tabs never mutate App state directly. They return `TabAction` variants:
- `None` — no-op
- `SwitchTab(TabId)` — switch tabs
- `NavigateTo(TabId, RecordId)` — switch + focus record
- `ShowMessage(String)` — status bar message (green, 3s)
- `RefreshData` — App calls `refresh()` on all tabs
- `StartInterEntityMode` — open inter-entity modal
- `Quit`

### Repository Pattern

`EntityDb` owns the `rusqlite::Connection` and exposes repo accessors:
```rust
db.accounts()  → AccountRepo<'_>
db.journals()  → JournalRepo<'_>
db.ar()        → ArRepo<'_>
db.ap()        → ApRepo<'_>
db.envelopes() → EnvelopeRepo<'_>
db.fiscal()    → FiscalRepo<'_>
db.assets()    → AssetRepo<'_>
db.recurring() → RecurringRepo<'_>
db.audit()     → AuditRepo<'_>
```

Each repo borrows `&Connection`. Cross-repo operations (posting JEs with envelope fills, year-end close) use `db.conn().transaction()` directly, wrapping multiple repo calls in one SQLite transaction. These live in `src/services/`.

### Entity Management

Single active entity at a time. The second entity is **only** opened inside the inter-entity modal. When the modal closes, the second `EntityDb` is dropped and the connection released.

---

## Data Model

Each entity is its own `.sqlite` file. **14 tables:**

| Table | Purpose |
|-------|---------|
| `accounts` | Chart of accounts with hierarchy (parent_id), type, placeholder/contra flags |
| `fixed_asset_details` | Companion to asset accounts: cost basis, in-service date, useful life, depreciation config |
| `journal_entries` | JE headers: number, date, memo, status (Draft/Posted), reversal links, inter-entity UUID |
| `journal_entry_lines` | JE line items: account, debit/credit amounts, reconcile state, sort order |
| `ar_items` | Accounts receivable: customer, amount, due date, status (Open/Partial/Paid) |
| `ar_payments` | AR partial payments (junction table) |
| `ap_items` | Accounts payable: vendor, amount, due date, status |
| `ap_payments` | AP partial payments (junction table) |
| `envelope_allocations` | Per-account fill percentages |
| `envelope_ledger` | Auditable fill/transfer/reversal log; balance = SUM(amount) |
| `fiscal_years` | Year boundaries, closed flag |
| `fiscal_periods` | Monthly periods with close/reopen tracking |
| `recurring_entry_templates` | Schedule for auto-generating JE drafts |
| `audit_log` | Append-only event log (never UPDATE/DELETE) |

### Key Data Conventions

- **Money**: `INTEGER` — 1 dollar = 100,000,000 units (10^8). Display rounds to 2 decimal places. Never use `f64` for money.
- **Percentages**: `INTEGER` — 1% = 1,000,000 units (10^6).
- **Enums**: stored as `TEXT` (human-readable). All have `FromStr`/`Display` impls in Rust.
- **Account numbers**: `TEXT` (supports "1010.01", leading zeros, etc.)
- **Timestamps**: `TEXT` in ISO 8601 format
- **SQLite pragmas**: `PRAGMA journal_mode=WAL` and `PRAGMA foreign_keys=ON` set on every connection open.

### Schema Migrations

The schema is defined in `src/db/schema.rs` in `initialize_schema()`. When columns were added after initial table creation (e.g., `accum_depreciation_account_id` on `fixed_asset_details`), `EntityDb::open()` runs `PRAGMA table_info()` and adds missing columns via `ALTER TABLE ADD COLUMN`. This handles databases created before the column existed.

---

## Feature Summary

### 9 Tabs

| # | Tab | Key Features |
|---|-----|--------------|
| 1 | Chart of Accounts | Hierarchical list, balances, envelope "Avail" column, add/edit/deactivate/delete/reactivate accounts, place-in-service for CIP accounts |
| 2 | General Ledger | Per-account transaction list with running balance, date filtering, debit-normal vs credit-normal display |
| 3 | Journal Entries | List with status filter, new entry form (JeForm widget), post (`p`), reverse (`r`), reconciliation state (`c`), recurring template creation |
| 4 | Accounts Receivable | Open/Partial/Paid items, status filter cycling, payment recording, overdue highlighting, JE navigation (`o`) |
| 5 | Accounts Payable | Mirrors AR tab structure |
| 6 | Envelopes | Two sub-views: Allocation Config (edit %) and Balances (FY-filtered, Available = Earmarked − GL Balance). Transfers between envelopes. |
| 7 | Fixed Assets | Asset register with depreciation schedule |
| 8 | Reports | 8 report types, parameter input, generates `.txt` files with box-drawing borders |
| 9 | Audit Log | Immutable event list with date/action filtering |

### 8 Reports (text files with box-drawing borders)

1. Trial Balance
2. Balance Sheet
3. Income Statement
4. Cash Flow Statement
5. Account Detail
6. AR Aging
7. AP Aging
8. Fixed Asset Schedule

All use shared formatting from `reports/mod.rs`. Reports include entity name header, "Accrual Basis" label, generation timestamp, and "— End of Report —" marker.

### Global Hotkeys

| Key | Action |
|-----|--------|
| `1`–`9` | Switch to tab by number |
| `n` | New journal entry |
| `i` | Inter-entity journal entry |
| `/` | Search in current tab |
| `?` | Help overlay |
| `f` | Fiscal period management modal |
| `q` | Quit |
| `Esc` | Close modal / exit inter-entity |

### Journal Entry Lifecycle

```
Draft ──[post]──► Posted ──[reverse]──► Posted (is_reversed=true) + New reversal JE
```

Post validates: balanced debits=credits, ≥2 lines, all accounts active & non-placeholder, fiscal period open. Posting a cash receipt auto-triggers envelope fills.

### Envelope Budgeting

On posting a JE that debits a cash/bank account:
1. Sum all cash debit lines
2. Skip if any debit line is Owner's Draw (Equity + contra)
3. For each envelope allocation: `fill_amount = cash_amount × allocation_percentage`
4. Record fills in `envelope_ledger` (no GL impact)

Transfers move earmarked dollars between envelopes (two paired rows with shared UUID). No journal entries created.

Year-end close does NOT affect envelope balances.

### Inter-Entity Transactions

Split-pane modal with two JeForm instances (one per entity). Each entity's lines must independently balance. Write protocol:
1. Create Draft in Entity A
2. Create Draft in Entity B (rollback A's draft on failure)
3. Post Entity A (rollback both drafts on failure)
4. Post Entity B (manual recovery needed — startup detects this)

**Startup recovery**: On app launch, scans for orphaned inter-entity drafts (matched by `inter_entity_uuid`). Prompts user to complete or roll back. Runs before the main UI loads.

### Fiscal Period Management

- Monthly periods with close/reopen
- Period lock: no JE create/post/reverse/reconcile in closed periods (enforced at both tab and repo layers)
- Year-end close: zeroes Revenue/Expense accounts via closing JEs posted to Retained Earnings. Does NOT clear envelope balances.

### Fixed Assets & Depreciation

- CIP (Construction-In-Progress) accounts detected by name containing "construction" (case-insensitive)
- Place-in-service: transfers CIP balance to target asset account via auto-generated JE
- Straight-line depreciation: `cost_basis / useful_life_months` per month
- Final month absorbs rounding remainder so total depreciation exactly equals cost basis
- Depreciation JEs generated as Drafts for user review

### Recurring Entries

Templates reference a source JE. On schedule (Monthly/Quarterly/Annually), generates a new Draft copying the source JE's lines. Checked at startup.

### Audit Log

Append-only. Records every mutation: JE create/post/reverse, account changes, period close/reopen, year-end close, envelope changes, inter-entity posts. Human-readable descriptions. Same SQLite file as entity data for transactional consistency.

---

## Workspace Configuration

```toml
# workspace.toml
report_output_dir = "~/accounting/reports"

[[entities]]
name = "Acme Land LLC"
db_path = "~/accounting/database/acme_land.sqlite"

[[entities]]
name = "Acme Rentals LLC"
db_path = "~/accounting/database/acme_rentals.sqlite"
```

Tilde expansion happens at config load time. Reports output as `[ReportName][MM-DD-YYYY].txt`.

---

## Rust Coding Conventions

These are enforced by the project and must be followed for any new code:

- **No `.unwrap()` in production code.** Use `?` for propagation. `.expect("reason")` only in init code with clear invariant.
- **Iterators over loops** for transformation/aggregation.
- **Immutability by default.** `mut` only when genuinely needed.
- **Borrow before own.** Prefer `&T`/`&mut T` over taking ownership.
- **No `unsafe`** without `// SAFETY:` comment.
- **No `async` or `tokio`.** Synchronous application.
- **Logging:** `tracing` crate. No `println!` in library code.
- **SQL:** parameterized queries only (`params![]` / `named_params!{}`). Never string interpolation.
- **Money:** always `Money(i64)` newtype. Never raw `i64` or `f64` in signatures.
- **Enums:** all state values are Rust enums with `FromStr`/`Display`. Never raw strings.

### Verification (run after every change)

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings   # zero warnings required
cargo test
```

All three must pass before committing. A git pre-commit hook at `.githooks/pre-commit` enforces this. Set with `git config core.hooksPath .githooks`.

---

## Key Design Patterns

### Adding a New Tab

1. Create `src/tabs/your_tab.rs` implementing the `Tab` trait
2. Add the variant to `TabId` enum in `src/tabs/mod.rs`
3. Register the tab in `EntityContext::new()` in `src/app.rs`
4. The tab gets keyboard dispatch and rendering automatically

### Adding a New Repository

1. Create `src/db/your_repo.rs` with a struct borrowing `&'conn Connection`
2. Add an accessor method to `EntityDb` in `src/db/mod.rs`
3. Use parameterized SQL, return typed `Result<T>` with `thiserror` errors

### Adding a New Report

1. Create `src/reports/your_report.rs` implementing the `Report` trait (`name()` + `generate()`)
2. Use shared formatting: `format_header()`, `format_table()`, `format_money()`
3. Register in the Reports tab's report list

### Cross-Tab Navigation

Return `TabAction::NavigateTo(TabId::Target, RecordId::Variant(id))` from `handle_key()`. App switches tabs and calls `navigate_to()` on the target.

### Data Mutations

1. Tab calls repo method(s) via `&EntityDb`
2. For cross-repo operations, use `db.conn().transaction()` in a service function
3. Return `TabAction::RefreshData` — App calls `refresh()` on all tabs
4. Next render cycle shows updated state

---

## Gotchas & Lessons Learned

### Money & Precision
- `$1 = 100,000,000 internal units` (8 decimal places). Test values: `$100 = 10_000_000_000`.
- Percentages: `1% = 1,000,000`, `10% = 10_000_000`.
- Final depreciation month absorbs remainder so `SUM(all months) == cost_basis` exactly.

### Architecture
- `EntityDb` wraps `rusqlite::Connection` and hands out repo objects. Repos borrow `&Connection`.
- `InterEntityMode` receives primary DB as `&EntityDb` parameter — does NOT store a reference. Secondary `EntityDb` is owned (drops when mode exits).
- `Tab::handle_key` returns `TabAction`; tabs never mutate App state directly.
- `TabAction::ShowMessage` → green status bar (3s). `App::set_error` → red status bar (5s).

### Cash Account Detection (envelope fill)
- Cash = `account_type == Asset && !is_placeholder && name.to_lowercase().contains("cash|bank|checking|savings")`.
- Owner's Draw suppression: `account_type == Equity && is_contra` → skip fill.
- Multiple cash debit lines: envelope fill amount = sum of all cash debits.

### CIP Account Detection
- Place-in-service form opens only when selected account name contains "construction" (case-insensitive).

### Fiscal Periods
- `create_draft` rejects closed periods at creation time (avoids orphaned un-postable entries).
- `generate_pending_depreciation` returns `(Vec<NewJournalEntry>, Option<String>)`. Warning fires when a depreciation month has no fiscal period.

### Cross-Module Test Access
- Private struct fields can't be set from cross-module tests. Use `#[cfg(test)] pub(crate) fn set_test_state(...)` helpers.

### Ratatui Specifics
- `Table::highlight_style` is deprecated in ratatui 0.29 — use `row_highlight_style`.
- `TableState` must be cloned for immutable `render()` since `render_stateful_widget` requires `&mut TableState`.
- Tab labels abbreviate on narrow terminals.

### SQLite
- `row_to_account` is a free function (not method) to satisfy rusqlite's `FnMut(&Row)` callback signature.
- WAL mode + foreign_keys=ON set on every connection open.
- Schema migrations via `ALTER TABLE ADD COLUMN` in `EntityDb::open()` for columns added after initial release.

---

## Seed Account Hierarchy

Default accounts created for new entities (small business LLC):

```
Assets (1000–1521)
  Cash & Bank Accounts (1000) [placeholder]
    Checking Account (1100)
    Savings Account (1200)
  Accounts Receivable (1200)
  Fixed Assets
    Land (1400)
    Buildings (1500)
    Accum. Depreciation - Buildings (1521) [contra]
    Construction in Progress (1510)

Liabilities (2000–2400)
  Accounts Payable (2100)
  Mortgage Payable (2200)
  Other Liabilities (2400)

Equity (3000–3300)
  Owner's Capital (3000)
  Owner's Draw (3200) [contra]
  Retained Earnings (3300)

Revenue (4000–4200)
  Rental Income (4000)
  Other Income (4200)

Expenses (5000–5800)
  Repairs & Maintenance (5000)
  Insurance (5100)
  Property Taxes (5200)
  Utilities (5300)
  Management Fees (5400)
  Depreciation Expense (5500)
  Mortgage Interest (5600)
  Professional Fees (5700)
  Miscellaneous Expense (5800)
```

---

## V1 Out of Scope (potential future features)

These were explicitly deferred from V1. Extension points exist (enum variants can be added, tables can be added, new tabs implement the Tab trait):

- Multi-user access and authentication
- Network features (HTTP, APIs, WebSockets)
- Inventory / materials management
- Accelerated depreciation methods (MACRS, double-declining balance)
- Full invoice management (line items, invoice numbers, customer/vendor records)
- Consolidated multi-entity financial reports
- PDF report output
- Automated backup
- Bank feed / import (OFX, CSV, QFX)
- Budgeting by period (actuals vs. budget variance)
- More than 2 entities in inter-entity modal
- Formal bank reconciliation UI workflow
- Mouse input

---

## Development History

Built across 7 phases, 85 tasks, 80+ commits between 2026-03-15 and 2026-03-16.

### Phase 1: Foundation (20 tasks)
Project setup, all newtypes (`Money`, `Percentage`, IDs), all enums, workspace config, SQLite schema + seed data, `EntityDb`, `FiscalRepo`, `Tab` trait + stub tabs, `StatusBar`, `App` event loop, entity creation/open flow, pre-commit hook.

### Phase 2a: Chart of Accounts (6 tasks)
`AccountRepo` (full CRUD + balances), `AuditRepo` (append-only), CoA tab (hierarchical list, search, expand/collapse), CoA CRUD modals, `AccountPicker` widget, `Confirmation` widget.

### Phase 2b: Journal Entries (8 tasks)
`JournalRepo` (JE + lines), post/reverse orchestration in `services/journal.rs`, `JeForm` widget (dynamic lines, running totals, embedded AccountPicker), JE tab (list + detail + filter), post/reverse actions, reconciliation state toggle, account balance verification, CoA→JE cross-tab navigation.

### Phase 3: GL, AR/AP, Fiscal Periods (12 tasks)
General Ledger tab (running balance, date filter), CoA→GL navigation, `ArRepo` + `ApRepo` (payments, status transitions), AR/AP tabs (status filter, payment recording, overdue highlighting, JE navigation), fiscal period close/reopen, year-end close (closing entries to Retained Earnings), fiscal modal UI, period lock enforcement on all mutations, JE→GL navigation.

### Phase 4: Envelopes + Fixed Assets (10 tasks)
`EnvelopeRepo` (allocations, fills, transfers, balances), envelope fill wired into JE post, envelope reversal on JE reverse, Envelopes tab (allocation config + FY-filtered balances), envelope transfers, envelope indicators on CoA tab, `AssetRepo` (place-in-service, depreciation generation), Fixed Assets tab, place-in-service action on CoA tab.

### Phase 5: Reports, Recurring, Startup (14 tasks)
Report formatting utilities (box-drawing, tables, headers), all 8 reports, Reports tab, `RecurringRepo` + recurring template creation in JE tab, startup sequence (recurring check, depreciation check), Audit Log tab, `?` help overlay.

### Phase 6: Inter-Entity + Polish (15 tasks)
`InterEntityMode`, inter-entity form (split-pane with two JeForms), write protocol (Draft→Post two-phase), startup recovery for orphaned drafts, App wiring, auto-create intercompany accounts, JE form validation polish, AR/AP payment JE auto-creation, edge cases (multi-cash envelope fill, year-end envelope persistence, cross-FY depreciation), cross-tab navigation audit, status bar polish, full integration test.

---

## Spec Files Reference

All specs live in `specs/`. These were the source of truth during development:

| File | Contents |
|------|----------|
| `specs/SPEC.md` | Master spec: overview, success criteria, tech stack, out-of-scope |
| `specs/data-model.md` | SQLite schema — all 14 tables, design decisions, integrity invariants |
| `specs/type-system.md` | Rust newtypes, enums, state machines, transition rules, algorithms |
| `specs/architecture.md` | Module structure, Tab trait, EntityDb, repos, event loop, data flow |
| `specs/implementation-protocols.md` | Session management, commit rules, rollback protocol, progress tracking |
| `specs/boundaries.md` | Always Do / Ask First / Never Do guardrails |
| `specs/progress.md` | Final state: all phases complete, decisions log, review fixes |
| `specs/phase-1.md` through `specs/phase-6.md` | Task-by-task implementation plans |

---

## How to Add Features

1. Read this document for orientation
2. Read `CLAUDE.md` for coding conventions
3. Read `specs/architecture.md` for component design
4. Read `specs/data-model.md` if touching the database
5. Read `specs/type-system.md` if touching types or state machines
6. Follow the verification cycle: `cargo fmt` → `cargo clippy -D warnings` → `cargo test`
7. Follow established patterns (repo pattern, tab trait, TabAction for communication)
8. No `.unwrap()`, no `async`, no `println!` in library code, parameterized SQL only
