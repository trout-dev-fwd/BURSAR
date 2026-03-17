# Phase 1: Foundation

**Goal**: Project scaffolding, core types, database schema, workspace config, entity
creation/opening, and a minimal TUI shell that starts, displays tabs, and quits.

**Depends on**: Nothing (this is the root).

**Estimated tasks**: 20

---

## Tasks

### Task 1: Initialize Cargo project
**Context**: None (greenfield).
**Action**: Create project at `~/coding-projects/accounting/`. Add to `Cargo.toml`:
`ratatui`, `crossterm`, `rusqlite` (with `bundled` feature), `serde` + `toml`, `chrono`,
`uuid`, `thiserror`, `anyhow`, `tracing`, `tracing-subscriber`.
Create `src/main.rs` with a minimal `fn main() {}`.
**Verify**: `cargo build` succeeds with no errors.
**Do NOT**: Write any application logic. This is purely dependency setup.

---

### Task 2: Create Money(i64) newtype **[TEST-FIRST]**
**Context**: `specs/type-system.md` (Money section).
**Action**: Create `src/types/money.rs`. Implement `Money(i64)` with:
- `Add`, `Sub`, `Neg`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`, `Clone`, `Copy`
- `Money::from_dollars(f64) -> Money`
- `Money::cents_rounded() -> i64`
- `Money::is_zero() -> bool`
- `Money::abs() -> Money`
- `Money::apply_percentage(Percentage) -> Money`
- `Display` impl: 2 decimal places, thousands separator
**Verify**: Unit tests:
- `Money::from_dollars(1234.56).to_string() == "1,234.56"`
- `Money::from_dollars(0.0).is_zero() == true`
- Arithmetic: `Money(100) + Money(200) == Money(300)`
- Percentage application matches expected result
- Negative amounts display with `-` prefix
**Do NOT**: Create `Percentage` type here (that's Task 3). Do not add database serialization yet.

---

### Task 3: Create Percentage(i64) newtype **[TEST-FIRST]**
**Context**: `src/types/money.rs` (for `apply_percentage` integration), `specs/type-system.md`.
**Action**: Create `src/types/percentage.rs`. Implement `Percentage(i64)` with:
- `Percentage::from_display(f64) -> Percentage` (parses "15.5" into 15_500_000)
- `Display` impl: 2 decimal places with `%` suffix
**Verify**: Unit tests:
- `Percentage::from_display(15.5).to_string() == "15.50%"`
- Round-trip: `from_display(x).to_string()` parses back correctly
- Integration: `Money::from_dollars(1000.0).apply_percentage(Percentage::from_display(15.5))`
  produces the correct result
**Do NOT**: Implement any envelope logic. This is just the type.

---

### Task 4: Create ID newtypes via macro **[TEST-FIRST]**
**Context**: `specs/type-system.md` (ID Newtypes section).
**Action**: Create `src/types/ids.rs`. Define a `newtype_id!` macro that generates a newtype
wrapping `i64` with `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `Hash`, `From<i64>`, `Into<i64>`.
Generate: `AccountId`, `JournalEntryId`, `JournalEntryLineId`, `FiscalYearId`, `FiscalPeriodId`,
`ArItemId`, `ApItemId`, `EnvelopeAllocationId`, `EnvelopeLedgerId`, `FixedAssetDetailId`,
`RecurringTemplateId`, `AuditLogId`.
**Verify**: Compile-time test: a function accepting `AccountId` rejects `JournalEntryId`.
Runtime test: round-trip `AccountId::from(42_i64)` → `i64::from(id) == 42`.
**Do NOT**: Add database serialization traits yet.

---

### Task 5: Create all enums with FromStr/Display **[TEST-FIRST]**
**Context**: `specs/type-system.md` (Enums section).
**Action**: Create `src/types/enums.rs`. Implement all enums:
`AccountType`, `BalanceDirection`, `ReconcileState`, `JournalEntryStatus`, `ArApStatus`,
`EntryFrequency`, `EnvelopeEntryType`, `AuditAction`.
Each gets `FromStr` and `Display` impls. Include `AccountType::normal_balance()` method.
**Verify**: Unit tests for round-trip `Display → FromStr` on every variant of every enum.
Test `AccountType::Asset.normal_balance() == BalanceDirection::Debit`.
Test `AccountType::Revenue.normal_balance() == BalanceDirection::Credit`.
**Do NOT**: Implement state transition logic (that's in the repos/services).

---

### Task 6: Create types/mod.rs
**Context**: `src/types/money.rs`, `src/types/percentage.rs`, `src/types/ids.rs`, `src/types/enums.rs`.
**Action**: Create `src/types/mod.rs` that re-exports all types.
**Verify**: `use crate::types::*` brings all newtypes and enums into scope. `cargo build` passes.
**Do NOT**: Add anything beyond re-exports.

---

### Task 7: Create workspace config (config.rs)
**Context**: `specs/architecture.md` (Workspace Config section).
**Action**: Create `src/config.rs`. Define `WorkspaceConfig` and `EntityConfig` structs with
`serde::Deserialize` and `serde::Serialize`. Functions:
- `load_config(path: &Path) -> Result<WorkspaceConfig>`
- `save_config(path: &Path, config: &WorkspaceConfig) -> Result<()>`
- Create default config if file doesn't exist.
**Verify**: Round-trip test: create config with 2 entities, save to temp file, load, assert equal.
Verify TOML format matches the example in architecture.md.
**Do NOT**: Create any entity databases. This is config only.

---

### Task 8: Create database schema (db/schema.rs) **[TEST-FIRST]**
**Context**: `specs/data-model.md` (all CREATE TABLE statements).
**Action**: Create `src/db/schema.rs`. Function: `initialize_schema(conn: &Connection) -> Result<()>`.
Runs all CREATE TABLE statements in a single transaction. Also sets:
`PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;`
**Verify**: Call `initialize_schema` on an in-memory SQLite database, then query `sqlite_master`
to confirm all 14 tables exist. Verify foreign key pragma is enabled.
**Do NOT**: Seed any data (that's Task 9). Do not create the EntityDb wrapper yet (Task 10).

---

### Task 9: Create default account seeding
**Context**: `src/db/schema.rs`, `specs/data-model.md`, feature spec Section 2.2 (Standard Built-in Account Categories).
**Action**: Add `seed_default_accounts(conn: &Connection) -> Result<()>` to `src/db/schema.rs`.
Inserts the default chart of accounts hierarchy: top-level placeholder parents (Assets,
Liabilities, Equity, Revenue, Expense) and standard sub-accounts from the feature spec.
Set `is_placeholder = 1` on category parents, `is_contra = 1` on contra accounts
(Accumulated Depreciation, Owner's Draw).
**Verify**: After seeding, query accounts table:
- 5 top-level accounts exist with `is_placeholder = 1`
- Sub-accounts have correct `parent_id` references
- Account types are correct
- Contra accounts flagged correctly
**Do NOT**: Create intercompany accounts (Due To/Due From) — those are created dynamically
when inter-entity mode is entered (Phase 6).

---

### Task 10: Create EntityDb struct
**Context**: `src/db/schema.rs`, `specs/architecture.md` (EntityDb section).
**Action**: Create `src/db/mod.rs`. Define `EntityDb` struct holding `rusqlite::Connection`.
Implement:
- `EntityDb::open(path: &Path) -> Result<Self>` (opens existing file, enables WAL + FK pragmas)
- `EntityDb::create(path: &Path, entity_name: &str, fiscal_year_start_month: u32) -> Result<Self>`
  (creates file, calls `initialize_schema`, `seed_default_accounts`, creates initial fiscal year)
- `fn conn(&self) -> &Connection` (direct connection access for transactions)
- Stub repo accessor methods that return empty repo structs (to be filled in later phases)
**Verify**: Create a new entity DB at a temp path, reopen it with `open`, confirm schema exists
and seeded accounts are present.
**Do NOT**: Implement any repo logic beyond stubs.

---

### Task 11: Create FiscalRepo with year/period creation **[TEST-FIRST]**
**Context**: `src/db/mod.rs`, `specs/data-model.md` (fiscal_years, fiscal_periods tables).
**Action**: Create `src/db/fiscal_repo.rs`. Implement `FiscalRepo<'conn>` with:
- `create_fiscal_year(start_month: u32, year: i32) -> Result<FiscalYearId>` — creates the
  fiscal year row and all 12 monthly period rows.
- `get_period_for_date(date: NaiveDate) -> Result<FiscalPeriod>`
- `list_periods(fiscal_year_id: FiscalYearId) -> Result<Vec<FiscalPeriod>>`
Wire into `EntityDb` as `fn fiscal(&self) -> FiscalRepo<'_>`.
**Verify**: Create fiscal year starting January 2025:
- 12 periods exist with correct start/end dates
- `get_period_for_date(2025-03-15)` returns period #3 (March)
- Period dates are contiguous (no gaps, no overlaps)
**Do NOT**: Implement close/reopen logic (that's Phase 3).

---

### Task 12: Wire fiscal year into EntityDb::create
**Context**: `src/db/mod.rs`, `src/db/fiscal_repo.rs`.
**Action**: Update `EntityDb::create` to call `fiscal_repo.create_fiscal_year()` with the
provided start month and current year.
**Verify**: Newly created entity DB has a fiscal year and 12 periods.
**Do NOT**: Add TUI prompts for fiscal year setup (that's Task 18).

---

### Task 13: Create Tab trait and enums
**Context**: `specs/architecture.md` (Tab trait, TabAction, TabId, RecordId sections).
**Action**: Create `src/tabs/mod.rs`. Define:
- `Tab` trait with `title()`, `handle_key()`, `render()`, `refresh()`, `navigate_to()`
- `TabAction` enum (None, SwitchTab, NavigateTo, ShowMessage, RefreshData, StartInterEntityMode, Quit)
- `TabId` enum (all 9 tabs)
- `RecordId` enum (Account, JournalEntry, ArItem, ApItem)
**Verify**: Compiles. All variants exist.
**Do NOT**: Implement any tab logic.

---

### Task 14: Create stub tab implementations
**Context**: `src/tabs/mod.rs`.
**Action**: Create one file per tab under `src/tabs/`:
`chart_of_accounts.rs`, `general_ledger.rs`, `journal_entries.rs`, `accounts_receivable.rs`,
`accounts_payable.rs`, `envelopes.rs`, `fixed_assets.rs`, `reports.rs`, `audit_log.rs`.
Each implements `Tab` with:
- `title()` returns the tab name
- `render()` displays the title centered in the area
- `handle_key()` returns `TabAction::None`
- `refresh()` is a no-op
**Verify**: All compile. Each returns the correct title string.
**Do NOT**: Add any real functionality. These are placeholders.

---

### Task 15: Create StatusBar widget
**Context**: `specs/architecture.md` (StatusBar section).
**Action**: Create `src/widgets/status_bar.rs` and `src/widgets/mod.rs`.
`StatusBar` renders: entity name (left), current fiscal period (center), message area (right).
`StatusBar::set_message(msg: String)` stores a message.
`StatusBar::tick()` clears message after 5-second timeout.
**Verify**: Construct a StatusBar, call `set_message`, verify message is stored.
Call `tick()` enough times to exceed timeout, verify message is cleared.
**Do NOT**: Implement unsaved-changes indicator (that's Phase 6).

---

### Task 16: Create App struct and event loop
**Context**: `specs/architecture.md` (App, EntityContext, AppMode, event loop pseudocode),
`src/tabs/mod.rs`, `src/widgets/status_bar.rs`.
**Action**: Create `src/app.rs`. Implement:
- `App` struct with `EntityContext`, `WorkspaceConfig`, `active_tab`, `AppMode`, `StatusBar`
- `EntityContext` with `EntityDb`, entity name, `Vec<Box<dyn Tab>>`
- `AppMode` enum (Normal, InterEntity placeholder, Modal placeholder)
- `App::run()` — the synchronous event loop:
  - Initialize terminal (crossterm raw mode, alternate screen)
  - Render: tab bar at top, active tab content below, status bar at bottom
  - 500ms poll timeout
  - Global hotkeys: `1`–`9` for tab switching, `q` for quit
  - Delegate unhandled keys to active tab
  - Restore terminal on exit (including on panic — use a drop guard)
**Verify**: Manually test: app starts, shows 9 tab titles, switches tabs with number keys,
quits with `q`, terminal restores cleanly. Status bar shows entity name.
**Do NOT**: Implement inter-entity mode or modals beyond placeholder enum variants.
Do NOT implement `?` help overlay (Phase 5).

---

### Task 17: Create main.rs entry point
**Context**: `src/app.rs`, `src/config.rs`.
**Action**: Update `src/main.rs`:
- Parse command-line args: optional path to workspace.toml
  (default: `~/.config/accounting/workspace.toml`)
- Load or create workspace config
- If no entities in config: print message directing user to create one (TUI creation in Task 18)
- If entities exist: open the first one
- Initialize `tracing_subscriber`
- Run `App::run()`
- Use `anyhow` for top-level error handling
**Verify**: `cargo run` with no existing config → creates default config, prints message.
`cargo run` with an entity in config → launches TUI.
**Do NOT**: Implement entity creation prompts in the terminal (that's Task 18).

---

### Task 18: Entity creation flow in TUI
**Context**: `src/app.rs`, `src/config.rs`, `src/db/mod.rs`.
**Action**: When the app launches with no entities (or user requests new entity via a hotkey),
show modal prompts for:
- Entity name (text input)
- Database file path (text input with default)
- Fiscal year start month (1–12 selector)
Create SQLite file via `EntityDb::create`, register in workspace config via `save_config`.
Load the new entity into the app.
**Verify**: Launch with empty config → create entity through TUI → quit → relaunch →
entity loads from config. Database file exists at specified path with full schema.
**Do NOT**: Implement entity deletion or editing.

---

### Task 19: Entity open/picker flow
**Context**: `src/app.rs`, `src/config.rs`.
**Action**: On startup, if multiple entities exist in config, show an entity picker modal
(list of entity names, arrow keys to select, Enter to confirm).
If only one entity, open it directly.
**Verify**: With two entities in config, picker appears. Selection loads the chosen entity.
Tab bar and status bar reflect the selected entity.
**Do NOT**: Implement entity switching after startup (single-entity design — second entity
only opens in inter-entity mode, Phase 6).

---

### Task 20: Set up pre-commit hook
**Context**: None.
**Action**: Create `.githooks/pre-commit` script that runs:
```bash
#!/bin/sh
cargo fmt -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test
```
Add a note to `CLAUDE.md` to configure git: `git config core.hooksPath .githooks`
**Verify**: Attempt to commit code with a clippy warning → commit rejected.
Fix the warning → commit succeeds.
**Do NOT**: Modify any application code in this task.

---

## Phase 1 Complete When

- [ ] `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test` all pass
- [ ] App launches, displays 9 placeholder tabs with correct titles
- [ ] Tab switching via `1`–`9` works
- [ ] Status bar shows entity name and current fiscal period
- [ ] Quit with `q` restores terminal cleanly (including on panic)
- [ ] Entity creation produces a valid SQLite file with full schema and seeded accounts
- [ ] Workspace config persists across restarts
- [ ] All types (Money, Percentage, IDs, enums) have passing unit tests
- [ ] Pre-commit hook rejects non-passing code
- [ ] `progress.md` is updated with all tasks marked complete

## Phase 1 Does NOT Cover

- Any real tab content (lists, forms, data display)
- Journal entries, AR/AP, envelopes, or any domain logic
- Reports, audit log entries, inter-entity features
- Startup checks (recurring, depreciation, inter-entity recovery)
- Account CRUD operations
- The `?` help overlay

**After completing Phase 1**: Developer reviews all code and signs off before Phase 2a begins.
