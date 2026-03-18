# Handoff — Double-Entry Bookkeeping TUI

> Living orientation document. Generated from actual code, not specs.
> If this document says one thing and the code says another, the code wins.
>
> Last updated: 2026-03-18

---

## What This Is

A terminal-based double-entry bookkeeping application for small businesses. Single-user,
single-entity (with inter-entity modal for transfers), fully synchronous. Includes an
AI Accountant chat panel (Claude API), a CSV bank import pipeline, a startup screen with
entity management, and an update checker.

---

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Language | Rust (stable, edition 2024) |
| TUI framework | Ratatui + Crossterm |
| Database | SQLite via rusqlite (WAL mode, FK enabled) |
| HTTP client | ureq (synchronous, blocking, `json` feature) |
| AI | Claude API (Anthropic) |
| CSV parsing | csv crate |
| Error handling | thiserror (domain), anyhow (CLI boundary) |
| Logging | tracing crate |
| Serialization | serde + serde_json + toml + toml_edit |

**Crate name:** `bursar` (binary: `bursar`). Config directory: `~/.config/bursar/`.

**Hard constraints:** No async. No tokio. No threading. No `unsafe`. No `.unwrap()`.

---

## Codebase Overview

**71 Rust source files, ~43,116 lines of code, 638 tests.**

```
src/
├── main.rs                          205 lines   — Terminal setup, AppState wrapper loop
├── lib.rs                            16 lines   — Module declarations
├── app/                           4,241 lines   — Application core (directory module)
│   ├── mod.rs                       927 lines   — App struct, EntityContext, render, tick, run
│   ├── key_dispatch.rs              687 lines   — handle_key, help overlay, entity picker
│   ├── ai_handler.rs                494 lines   — handle_ai_request, tool-use loop
│   └── import_handler.rs          2,133 lines   — CSV import flow, bank detection, review
├── config.rs                        722 lines   — Config loading (workspace, entity, secrets)
├── startup.rs                       588 lines   — DB/config initialization, entity selection
├── startup_screen.rs                676 lines   — Startup screen: entity picker, add/edit/delete
├── update.rs                         72 lines   — GitHub release update checker (semver)
├── integration_tests.rs             478 lines   — Cross-module integration tests
│
├── ai/                            3,047 lines   — AI Accountant
│   ├── mod.rs                       283 lines   — Wire types (ApiMessage, ToolCall, RoundResult, etc.)
│   ├── client.rs                    832 lines   — HTTP client, request/response, classify_round
│   ├── context.rs                   197 lines   — Context file loading for system prompts
│   ├── csv_import.rs                810 lines   — CSV parsing, 3-pass matching pipeline
│   └── tools.rs                     925 lines   — 10 read-only tool definitions + fulfillment
│
├── db/                            9,452 lines   — Database layer
│   ├── mod.rs                       263 lines   — EntityDb wrapper, migrations
│   ├── schema.rs                    633 lines   — CREATE TABLE statements (15 tables)
│   ├── account_repo.rs            1,096 lines   — Chart of accounts CRUD
│   ├── journal_repo.rs             1,955 lines   — Journal entries, lines, import queries
│   ├── asset_repo.rs              1,225 lines   — Fixed assets, depreciation
│   ├── fiscal_repo.rs               654 lines   — Fiscal years and periods
│   ├── envelope_repo.rs             662 lines   — Envelope allocations and ledger
│   ├── ar_repo.rs                   689 lines   — Accounts receivable
│   ├── ap_repo.rs                   535 lines   — Accounts payable
│   ├── recurring_repo.rs            685 lines   — Recurring entry templates
│   ├── audit_repo.rs                560 lines   — Audit log
│   └── import_mapping_repo.rs       495 lines   — Learned CSV mappings
│
├── inter_entity/                  2,156 lines   — Inter-entity transfers
│   ├── mod.rs                       427 lines   — Mode management
│   ├── form.rs                      742 lines   — Transfer form
│   ├── recovery.rs                  453 lines   — Orphan detection and recovery
│   └── write_protocol.rs           534 lines   — Atomic two-DB write
│
├── reports/                       3,081 lines   — Report generation
│   ├── mod.rs                       539 lines   — Report trait, shared rendering
│   ├── trial_balance.rs             286 lines
│   ├── balance_sheet.rs             272 lines
│   ├── income_statement.rs          260 lines
│   ├── cash_flow.rs                 295 lines
│   ├── account_detail.rs            288 lines
│   ├── ar_aging.rs                  311 lines
│   ├── ap_aging.rs                  249 lines
│   ├── fixed_asset_schedule.rs      227 lines
│   └── envelope_budget.rs           354 lines
│
├── services/                      2,036 lines   — Business logic
│   ├── mod.rs                         2 lines
│   ├── journal.rs                 1,370 lines   — Posting, reversal, depreciation, year-end
│   └── fiscal.rs                    664 lines   — Period management, close/reopen
│
├── tabs/                          9,587 lines   — TUI tabs (one file each)
│   ├── mod.rs                       136 lines   — Tab trait, TabId, TabAction enums
│   ├── chart_of_accounts.rs       1,759 lines   — Account tree with CRUD
│   ├── journal_entries.rs         1,894 lines   — JE list/detail, recurring, import triggers
│   ├── accounts_receivable.rs     1,277 lines   — AR items and payments
│   ├── accounts_payable.rs        1,186 lines   — AP items and payments
│   ├── envelopes.rs               1,060 lines   — Allocations and balance views
│   ├── general_ledger.rs            646 lines   — Per-account transaction ledger
│   ├── reports.rs                   692 lines   — Report menu and parameter entry
│   ├── fixed_assets.rs              409 lines   — Asset register, depreciation schedule
│   └── audit_log.rs                 528 lines   — Filterable audit event viewer
│
├── types/                         1,025 lines   — Domain types
│   ├── mod.rs                        17 lines
│   ├── enums.rs                     663 lines   — 15 enums (10 persisted, 5 in-memory)
│   ├── ids.rs                        72 lines   — 12 ID newtypes (macro-generated)
│   ├── money.rs                     179 lines   — Money(i64), 8 decimal places
│   └── percentage.rs                 94 lines   — Percentage(i64), 6 decimal places
│
└── widgets/                       5,734 lines   — Reusable UI components
    ├── mod.rs                        44 lines   — centered_rect helper
    ├── chat_panel.rs                936 lines   — AI chat interface
    ├── je_form.rs                 1,338 lines   — Journal entry create/edit form
    ├── user_guide.rs                773 lines   — Embedded user guide viewer
    ├── fiscal_modal.rs              719 lines   — Fiscal year/period management
    ├── file_picker.rs               487 lines   — CSV file browser
    ├── account_picker.rs            464 lines   — Account selection widget
    ├── text_input_modal.rs          337 lines   — Reusable single-line text input dialog
    ├── status_bar.rs                264 lines   — Status messages + hotkey hints
    ├── confirmation.rs              215 lines   — Y/N confirmation dialog
    └── existing_db_modal.rs         157 lines   — Restore/fresh/cancel for existing DB files
```

---

## Architecture

### Three-State Wrapper Loop (`main.rs`)

The application lifecycle is managed by an `AppState` enum in `main.rs`:

```rust
enum AppState {
    Splash,                      // ASCII banner + optional update check
    Startup(Box<StartupScreen>), // Entity picker (before any DB is opened)
    Running(Box<App>),           // Main application (entity DB is open)
}
```

The wrapper loop in `main()` owns the terminal and drives whichever state is active:

```
Splash ──(1s minimum)──→ Startup ──(user selects entity)──→ Running
                                                                │
                                                           q → Quit
```

**Splash** renders the ASCII banner and version. If `[updates]` is configured, it checks
GitHub for a newer release (3s timeout). Enforces a minimum 1-second display.

**Startup** shows `StartupScreen` — the entity picker with add/edit/delete flows. Returns
`StartupAction::OpenEntity { name, db_path }` when the user selects an entity. On
transition to Running, the wrapper re-reads `workspace.toml` (to pick up entity changes),
persists `last_opened_entity` via `toml_edit`, opens the `EntityDb`, runs startup checks,
and creates `App`.

**Running** delegates to `App`'s extracted methods:

```
loop {
    app.render(&mut terminal)?;       // draw one frame
    poll(500ms) → app.handle_event(&evt);  // process key input
    app.process_pending(&mut terminal);     // dispatch AI/import/slash
    app.tick();                             // typewriter, status bar, unsaved
    if app.should_quit() { break; }
}
```

### Extracted App Methods

| Method | Signature | Purpose |
|--------|-----------|---------|
| `render` | `(&mut self, terminal: &mut Terminal<B>) -> Result<()>` | Draws one frame |
| `handle_event` | `(&mut self, event: &Event)` | Routes key events to `handle_key` |
| `tick` | `(&mut self)` | Typewriter, status expiry, unsaved indicator |
| `process_pending` | `(&mut self, terminal: &mut Terminal<B>)` | Dispatches pending AI/import/slash |
| `should_quit` | `(&self) -> bool` | True when app should exit |
| `run` | `(&mut self) -> Result<()>` | Convenience: sets up terminal and runs its own event loop |

`App::run` still exists as a self-contained convenience method (sets up its own terminal,
runs the event loop, restores on exit). The extracted methods allow `main.rs` to drive
the loop externally.

### Key Dispatch Order (`handle_key` in `app/key_dispatch.rs`)

Priority from highest to lowest:

1. **Ctrl+H** → toggle user guide (always)
2. **User guide open** → all keys to guide; Esc closes
3. **Help overlay open** → Esc/`?` dismiss; all others consumed
4. **File picker open** → all keys to file picker
5. **Import flow active** → all keys to import handler
6. **Chat panel visible + focus=ChatPanel** → Tab switches focus; all else to panel
7. **Chat panel visible + focus=MainTab** → Tab/Ctrl+K switch to panel; else fall through
8. **InterEntity mode** → all keys to inter-entity handler
9. **InterEntityAccountSetup** → all keys to setup handler
10. **SecondaryEntityPicker** → all keys to picker
11. **Fiscal modal open** → all keys to modal
12. **Tab `wants_input()` = true** → all keys to active tab (suppresses globals)
13. **Global hotkeys** → q, ?, f, Ctrl+K, 1-9, Ctrl+Left/Right
14. **Fallback** → delegate to active tab's `handle_key`

**Note:** Startup screen key handling is in `StartupScreen::handle_event` (called from
`main.rs`), not in `App::handle_key`.

### App Struct (key fields)

```rust
pub struct App {
    entity: EntityContext,              // DB + name + 9 tab instances
    config: WorkspaceConfig,
    active_tab: usize,                  // 0-8
    mode: AppMode,                      // Normal | InterEntity | etc.
    status_bar: StatusBar,
    fiscal_modal: Option<FiscalModal>,
    show_help: bool,
    inter_entity_help: bool,
    user_guide: Option<UserGuide>,
    should_quit: bool,
    chat_panel: ChatPanel,
    focus: FocusTarget,                 // MainTab | ChatPanel
    ai_state: AiRequestState,          // Idle | CallingApi | FulfillingTools
    ai_client: Option<AiClient>,       // lazily initialized
    pending_ai_messages: Option<Vec<ApiMessage>>,
    pending_slash_command: Option<SlashCommand>,
    file_picker: Option<FilePicker>,
    import_flow: Option<ImportFlowState>,
    pending_bank_detection: bool,
    pending_pass1: bool,
    pending_pass2: bool,
    pending_draft_creation: bool,
}
```

### StartupScreen Struct (key fields)

```rust
pub struct StartupScreen {
    pub entities: Vec<EntityEntry>,     // parsed from workspace.toml
    pub selected_index: usize,          // currently highlighted entity
    pub update_notice: Option<String>,  // "New version v1.2.0 available..."
    pub workspace_path: PathBuf,        // path to workspace.toml
    text_input: Option<TextInputModal>, // active add/edit modal
    pending_action: Option<PendingEntityAction>,  // Add | Edit(index)
    confirm_delete: Option<Confirmation>,
    existing_db_modal: Option<ExistingDbModal>,
    pending_add: Option<PendingAdd>,    // deferred add awaiting modal decision
    status_message: Option<String>,     // status/error below entity list
}
```

### Tab Trait

```rust
pub trait Tab {
    fn title(&self) -> &str;
    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction;
    fn render(&self, frame: &mut Frame, area: Rect);
    fn refresh(&mut self, db: &EntityDb);
    fn wants_input(&self) -> bool { false }
    fn navigate_to(&mut self, record_id: RecordId, db: &EntityDb) { }
    fn has_unsaved_changes(&self) -> bool { false }
    fn hotkey_help(&self) -> Vec<(&'static str, &'static str)> { vec![] }
    fn selected_draft_import_ref(&self) -> Option<String> { None }
}
```

Tabs never mutate App state. They return `TabAction` values:
`None`, `SwitchTab`, `NavigateTo`, `ShowMessage`, `RefreshData`, `StartInterEntityMode`,
`StartImport`, `StartRematch`, `Quit`.

### EntityDb Pattern

```rust
pub struct EntityDb { conn: Connection }
```

Owns the rusqlite `Connection`. Hands out repo objects via accessors:
`db.accounts()`, `db.journals()`, `db.fiscal()`, `db.envelopes()`, `db.assets()`,
`db.ar()`, `db.ap()`, `db.recurring()`, `db.audit()`, `db.import_mappings()`.

Each repo borrows `&Connection` — no ownership, no Arc, no Mutex.

### ChatPanel → App Communication

ChatPanel makes no API calls and writes no data. It returns `ChatAction`:
- `SendMessage(Vec<ApiMessage>)` → App calls `handle_ai_request`
- `SlashCommand(SlashCommand)` → App calls `execute_slash_command`
- `Close` → App hides panel
- `SkipTypewriter` → instant-reveal animation
- `None` → no-op

### AI Request Flow

1. `ensure_ai_client()` — lazy-loads API key from secrets.toml
2. Build system prompt with persona + entity name + context files
3. Log `AiPrompt` to audit
4. Set `ai_state = CallingApi`, force render
5. Loop up to 5 rounds:
   - `send_single_round(system, messages, tools, accumulated_text, use_cache)`
   - `RoundResult::Done` → break with response text
   - `RoundResult::NeedsToolCall` → log each tool to audit, set `FulfillingTools`,
     force render, fulfill each tool, append results, next round
6. Parse SUMMARY line, add response to chat panel, log to audit

### CSV Import Pipeline

Three-pass matching triggered by `u` in Journal Entries tab:

1. **File picker** → select `.csv` file
2. **Bank detection** → match to configured bank or create new
3. **Column mapping** → confirm/edit date, description, amount columns
4. **Duplicate check** → warn if `import_ref` already exists
5. **Pass 1 (local)** → match against `import_mappings` table
6. **Pass 2 (AI)** → send unmatched to Claude for categorization
7. **Pass 3 (clarification)** → resolve ambiguous matches
8. **Review** → accept/reject/edit each match
9. **Draft creation** → create single-line draft JEs, learn confirmed mappings

---

## Data Model

**15 SQLite tables.** All money stored as `INTEGER` (i64, 8 decimal places).
Enums stored as `TEXT`. Foreign keys enforced. WAL journal mode.

### Core Tables

| Table | Purpose | Key Columns |
|-------|---------|-------------|
| `accounts` | Chart of accounts | number (unique), name, account_type, parent_id, is_placeholder, is_contra |
| `journal_entries` | Transaction headers | je_number (unique), entry_date, status (Draft/Posted), fiscal_period_id, import_ref |
| `journal_entry_lines` | Debit/credit lines | journal_entry_id, account_id, debit_amount, credit_amount, reconcile_state |
| `fiscal_years` | Annual periods | start_date, end_date, is_closed |
| `fiscal_periods` | Monthly periods | fiscal_year_id, period_number, start_date, end_date, is_closed |

### Domain Tables

| Table | Purpose |
|-------|---------|
| `fixed_asset_details` | Depreciation config per asset account |
| `ar_items` / `ar_payments` | Accounts receivable tracking |
| `ap_items` / `ap_payments` | Accounts payable tracking |
| `envelope_allocations` | Budget percentage per account |
| `envelope_ledger` | Earmark history (Fill, Transfer, Reversal) |
| `recurring_entry_templates` | Auto-generation config (Monthly/Quarterly/Annually) |
| `audit_log` | All system events (23 action types) |
| `import_mappings` | Learned CSV description→account mappings |

### Money Representation

- `Money(i64)`: 1 dollar = 100,000,000 units. Display rounds to 2 decimal places.
- `Percentage(i64)`: 1% = 1,000,000 units.
- **Never use f64 for money.** Parse CSV amounts directly to Money via integer arithmetic.

### ID Types

12 newtype wrappers over `i64`: `AccountId`, `JournalEntryId`, `JournalEntryLineId`,
`FiscalYearId`, `FiscalPeriodId`, `ArItemId`, `ApItemId`, `EnvelopeAllocationId`,
`EnvelopeLedgerId`, `FixedAssetDetailId`, `RecurringTemplateId`, `AuditLogId`.

### Enums (15 total)

**Persisted (TEXT in SQLite, with FromStr/Display):**
`AccountType` (5), `BalanceDirection` (2), `ReconcileState` (3), `JournalEntryStatus` (2),
`ArApStatus` (3), `EntryFrequency` (3), `EnvelopeEntryType` (3), `AuditAction` (23),
`ImportMatchType` (2), `ImportMatchSource` (2).

**In-memory only:**
`AiRequestState` (3), `ChatRole` (3), `FocusTarget` (2), `MatchSource` (4), `MatchConfidence` (3).

---

## Feature Summary

### Startup Screen

- ASCII banner splash with version display
- Optional GitHub release update check (`[updates]` config section)
- Entity picker with pre-selection of last-opened entity
- Add entity (auto-generates db/config filenames, handles existing DB files)
- Edit entity name (renames db/config files on disk)
- Delete entity (with confirmation, removes db/config files)
- Entity changes are persisted to `workspace.toml` via `toml_edit` (format-preserving)

### 9 Tabs

| # | Tab | Key Features |
|---|-----|-------------|
| 1 | Chart of Accounts | Hierarchical tree, expand/collapse, add/edit/delete/deactivate, search, place-in-service |
| 2 | General Ledger | Per-account transaction list, running balance, date filter, navigate to JE |
| 3 | Journal Entries | Create/edit/post/reverse, reconcile, recurring templates, CSV import, re-match, inter-entity |
| 4 | Accounts Receivable | Create receivables, record payments, payment history, status filter |
| 5 | Accounts Payable | Create payables, record payments, payment history, status filter |
| 6 | Envelopes | Allocation percentages, balance tracking, transfers, fiscal year scoping |
| 7 | Fixed Assets | Asset register, depreciation schedule, bulk depreciation generation |
| 8 | Reports | 9 report types with parameter entry, file output |
| 9 | Audit Log | Filterable by action type and date range |

### AI Accountant (Ctrl+K)

- Chat panel with typewriter animation
- 10 read-only tools for querying the books
- Up to 5 tool-use rounds per request
- Prompt caching for efficiency
- Slash commands: `/clear`, `/context`, `/compact`, `/persona`, `/match`

### CSV Import (u in JE tab)

- File browser for `.csv` selection
- Bank auto-detection and configuration
- Three-pass matching: local → AI → clarification
- Draft creation with learned mappings
- Batch re-match (Shift+U) for incomplete imports

### Reports (9 types)

Trial Balance, Balance Sheet, Income Statement, Cash Flow Statement,
Account Detail, AR Aging, AP Aging, Fixed Asset Schedule, Envelope Budget Summary.

---

## All Hotkeys

### Startup Screen

| Key | Action |
|-----|--------|
| `Up/k`, `Down/j` | Navigate entity list |
| `Enter` | Open selected entity |
| `a` | Add new entity |
| `e` | Edit selected entity name |
| `d` | Delete selected entity |
| `q` | Quit |

### Global (Running state)

| Key | Action |
|-----|--------|
| `1`–`9` | Switch to tab by number |
| `Ctrl+Left/Right` | Cycle tabs |
| `Ctrl+K` | Toggle AI chat panel |
| `Ctrl+H` | Toggle user guide |
| `f` | Fiscal period modal |
| `?` | Help overlay |
| `q` | Quit |

### Chart of Accounts (Tab 1)

| Key | Action |
|-----|--------|
| `Up/k`, `Down/j` | Navigate |
| `Enter` | Expand/collapse or navigate to GL |
| `/` | Search |
| `a` | Add account |
| `e` | Edit account |
| `d` | Toggle active/inactive |
| `x` | Delete account |
| `s` | Place in service |

### General Ledger (Tab 2)

| Key | Action |
|-----|--------|
| `Up/k`, `Down/j` | Navigate |
| `Enter` | Go to journal entry |
| `p` | Account picker |
| `f` | Date filter |

### Journal Entries (Tab 3)

| Key | Action |
|-----|--------|
| `Up/Down` | Navigate entries or lines |
| `Enter/Esc` | Open/close detail view |
| `n` | New entry |
| `e` | Edit draft |
| `p` | Post draft |
| `r` | Reverse posted entry |
| `s` | Scheduled entry templates |
| `t` | Create recurring template |
| `c` | Toggle reconcile state |
| `g` | Go to GL for line's account |
| `f` | Cycle status filter |
| `i` | Inter-entity mode |
| `u` | CSV import |
| `U` | Re-match incomplete imports |

### Accounts Receivable (Tab 4) / Accounts Payable (Tab 5)

| Key | Action |
|-----|--------|
| `Up/k`, `Down/j` | Navigate |
| `n` | New item |
| `p` | Record payment |
| `Enter` | Payment history |
| `o` | Go to originating JE |
| `s/f` | Cycle status filter |

### Envelopes (Tab 6)

| Key | Action |
|-----|--------|
| `v` | Toggle view (Allocations ↔ Balances) |
| `Up/Down` | Navigate |
| `Enter` | Edit allocation |
| `d` | Remove allocation |
| `t` | Transfer (Balances view) |
| `Left/Right` | Change fiscal year (Balances view) |

### Fixed Assets (Tab 7)

| Key | Action |
|-----|--------|
| `Up/Down` | Navigate |
| `Enter` | Depreciation schedule |
| `Esc` | Back to register |
| `g` | Generate depreciation drafts |

### Reports (Tab 8)

| Key | Action |
|-----|--------|
| `Up/k`, `Down/j` | Navigate menu |
| `Enter` | Select/configure/generate |
| `Tab` | Next parameter field |
| `F9` | Generate report |
| `Esc` | Back to menu |

### Audit Log (Tab 9)

| Key | Action |
|-----|--------|
| `Up/k`, `Down/j` | Navigate |
| `Left/Right` | Cycle action filter |
| `d` | Date filter |
| `c` | Clear filters |

### Chat Panel (when focused)

| Key | Action |
|-----|--------|
| `Tab` | Switch focus to main tab |
| `Esc/Ctrl+K` | Close panel |
| `Enter` | Send message / skip typewriter / run slash command |
| `Up/Down` | Scroll history (when input empty) |
| `Left/Right/Home/End` | Cursor movement |
| `Backspace/Delete` | Delete text |

---

## Key Design Patterns

### How to Add a New Tab

1. Create `src/tabs/my_tab.rs` implementing the `Tab` trait
2. Add variant to `TabId` enum in `src/tabs/mod.rs`
3. Register in `EntityContext::new()` in `src/app.rs` (tabs are in fixed order)
4. Add tab number hotkey in `handle_key` global section
5. Add `hotkey_help()` implementation for the `?` overlay

### How to Add a New Repo

1. Create `src/db/my_repo.rs` with a struct borrowing `&'a Connection`
2. Add accessor method to `EntityDb` in `src/db/mod.rs`
3. Add CREATE TABLE to `src/db/schema.rs`
4. If adding columns to existing tables, add a migration function in `EntityDb::open`

### How to Add a New AI Tool

1. Add `ToolDefinition` to `tool_definitions()` in `src/ai/tools.rs`
2. Add handler function `handle_my_tool(input, db) -> Result<String, AiError>`
3. Add match arm in `fulfill_tool_call`
4. Tools must be **read-only** — query repos, never write

### How to Add a Slash Command

1. Add variant to `SlashCommand` enum in `src/ai/mod.rs`
2. Add parse case in `SlashCommand::parse()` in `src/widgets/chat_panel.rs`
3. Add execution logic in `execute_slash_command()` in `src/app.rs`

### How to Add a Widget

1. Create `src/widgets/my_widget.rs` with struct + action enum
2. Widget returns actions; App processes them (same as Tab pattern)
3. Add to App struct as `Option<MyWidget>` if modal
4. Add key dispatch priority level in `handle_key` if it captures input
5. Register module and re-exports in `src/widgets/mod.rs`

---

## Gotchas

### Money & Precision
- $1 = 100,000,000 internal units. $100 = 10,000,000,000. Never use f64 for money.
- Percentages: 1% = 1,000,000, 10% = 10,000,000.
- Final depreciation month absorbs rounding remainder.

### Tab Key Conflict
- Tab is intercepted at App level when chat panel is open.
- JE form uses arrow keys + Enter as alternative navigation.
- Envelopes uses `v` for view toggle (not Tab).

### Forced Render Before Blocking Calls
- Must call `terminal.draw()` before any `ureq` call so the user sees loading state.
- The UI freezes during API calls (single-threaded, synchronous).

### Cash Account Detection (Envelope Fill)
- Cash = `Asset && !is_placeholder && name contains "cash|bank|checking|savings"`.
- Owner's Draw (Equity + is_contra) is skipped.
- Multiple cash debit lines: envelope fill = sum of all.

### Fiscal Periods
- `create_draft` rejects closed periods at creation time.
- Year-end close zeroes revenue/expense GL balances but NOT envelope earmarks.

### Import Ref Format
- `"{bank_name}|{date}|{description}|{amount}"` — parse from ends if description has pipes.

### SUMMARY Line
- System prompt tells Claude to end with `SUMMARY: [one sentence]`.
- Client strips it from display, logs to audit. Fallback: truncate first 100 chars.

### Fresh Database Initialization
- In-memory test DBs start fresh (no migrations). Schema CREATE TABLE must include all columns.
- Migrations only run on existing file-based DBs.

### Borrow Splitting for AI Client
- `ai_client` is `.take()`n during `handle_ai_request` to split borrows on `status_bar` (mut)
  and `entity.db` (shared).

### Entity Path Resolution
- Entity `db_path` values in `workspace.toml` are resolved relative to the workspace.toml
  directory (not CWD) via `resolve_relative` in `expand_config_paths`.

### Startup Screen Config Writes
- Entity add/edit/delete use `toml_edit` for format-preserving TOML writes. When working
  with `[[entities]]` array-of-tables, edits must target the correct array index.
- `last_opened_entity` is written to `workspace.toml` on every entity open.
- Config is re-read from disk on transition from Startup → Running, so entity changes
  made during the startup session are picked up.

### Secrets Path
- API key is at `~/.config/bookkeeper/secrets.toml` (note: `bookkeeper`, not `bursar`).
  This is a historical path that has not been renamed.

---

## Configuration Reference

### `workspace.toml` (`~/.config/bursar/workspace.toml` by default)

```toml
report_output_dir = "~/bursar/reports"
context_dir = "context"               # optional, for AI system prompt context files
last_opened_entity = "My Business"    # auto-set on entity open, used for pre-selection

[ai]
persona = "Professional Tax Accountant"
model = "claude-sonnet-4-20250514"

[updates]
github_repo = "owner/bursar"          # optional, enables update check on startup

[[entities]]
name = "My Business"
db_path = "my_business.db"
config_path = "my_business.toml"      # optional, per-entity config
```

### Per-entity `.toml` (same directory as workspace.toml)

```toml
ai_persona = "Small Business Bookkeeper"   # optional, overrides workspace persona
last_import_dir = "/home/user/downloads"   # remembered from last CSV import

[[bank_accounts]]
name = "Chase Checking"
linked_account = "1010"                    # chart of accounts number
date_column = "Post Date"
description_column = "Description"
amount_column = "Amount"                   # OR debit_column + credit_column
date_format = "%m/%d/%Y"
debit_is_negative = true
```

### `~/.config/bookkeeper/secrets.toml`

```toml
anthropic_api_key = "sk-ant-..."
```

Loaded lazily on first AI interaction. Never stored in version control.

---

## Dependencies

```toml
[dependencies]
ratatui = { version = "0.29", features = ["unstable-rendered-line-info"] }
crossterm = "0.28"
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
toml_edit = "0.25"              # format-preserving TOML editing for workspace.toml writes
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
thiserror = "2"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde_json = "1"
csv = "1"
ureq = { version = "2", features = ["json"] }
```

---

## Out of Scope

These are explicitly excluded from the project:

- Async / tokio / threading
- Multi-user or authentication
- Mouse input
- PDF report output
- Invoice management
- Network features beyond Claude API and update check
- Inter-entity with more than 2 entities
- Auto-writing to entity context files without user action
