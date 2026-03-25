# HANDOFF.md — Bursar Double-Entry Bookkeeping TUI

> **Living orientation document.** Regenerated from codebase review, not memory.
> If this document says one thing and the code says another, the code wins.
> Last updated: 2026-03-18

## What This Is

Bursar is a terminal-based double-entry bookkeeping application built with Rust, Ratatui, and SQLite. It supports chart of accounts, journal entries, AR/AP, envelope budgeting, fixed assets with depreciation, fiscal period management, inter-entity transactions, AI-assisted accounting via Claude, and CSV bank statement import. Single-user, fully synchronous, no async/tokio.

## Tech Stack

| Layer | Technology |
|-------|------------|
| Language | Rust (stable) |
| TUI framework | Ratatui 0.29 + Crossterm 0.28 |
| Database | SQLite via rusqlite 0.32 (bundled) |
| Serialization | serde + serde_json, toml, toml_edit |
| HTTP | ureq 2 (synchronous, blocking) |
| Date/time | chrono 0.4 |
| IDs | uuid 1 (v4) |
| Errors | thiserror 2 (domain), anyhow 1 (CLI boundary) |
| Logging | tracing 0.1 + tracing-subscriber 0.3 |
| CSV | csv 1 |

**Crate name:** `bursar` v0.1.0

**Hard constraints:** No async. No tokio. No `unsafe` without `// SAFETY:`. No `.unwrap()` in production code. No `println!` in library code. Parameterized SQL only.

## Codebase Overview

- **71 Rust source files**
- **43,116 lines of code**
- **638 tests** (all passing)

### File Tree

```
src/
├── main.rs                          205
├── lib.rs                            16
├── config.rs                        722
├── startup.rs                       588
├── startup_screen.rs                676
├── update.rs                         72
├── integration_tests.rs             478
│
├── app/
│   ├── mod.rs                       927    # App struct, EntityContext, AppMode, creation wizard
│   ├── key_dispatch.rs              687    # Key routing priority chain, help overlay
│   ├── ai_handler.rs                494    # AI request flow, slash commands
│   └── import_handler.rs           2133    # CSV import pipeline, bank detection, review UI
│
├── ai/
│   ├── mod.rs                       283    # Wire types (ApiMessage, ToolCall, RoundResult, etc.)
│   ├── client.rs                    832    # AiClient, HTTP calls, prompt caching, SUMMARY parsing
│   ├── tools.rs                     925    # 10 read-only AI tools, fulfillment handlers
│   ├── context.rs                   197    # Per-entity context file loading
│   └── csv_import.rs                810    # CSV parsing, 3-pass matching, money parsing
│
├── db/
│   ├── mod.rs                       263    # EntityDb wrapper, migrations
│   ├── schema.rs                    633    # 15 CREATE TABLE statements, seed accounts
│   ├── account_repo.rs             1096
│   ├── journal_repo.rs             1955
│   ├── fiscal_repo.rs               654
│   ├── envelope_repo.rs             662
│   ├── asset_repo.rs               1225
│   ├── ar_repo.rs                   689
│   ├── ap_repo.rs                   535
│   ├── audit_repo.rs                560
│   ├── recurring_repo.rs            685
│   └── import_mapping_repo.rs       495
│
├── tabs/
│   ├── mod.rs                       136    # Tab trait, TabAction, TabId
│   ├── chart_of_accounts.rs        1759
│   ├── general_ledger.rs            646
│   ├── journal_entries.rs          1894
│   ├── accounts_receivable.rs      1277
│   ├── accounts_payable.rs         1186
│   ├── envelopes.rs                1060
│   ├── fixed_assets.rs              409
│   ├── reports.rs                   692
│   └── audit_log.rs                 528
│
├── widgets/
│   ├── mod.rs                        44    # Exports, centered_rect()
│   ├── je_form.rs                  1338    # Journal entry form (shared by JE tab + inter-entity)
│   ├── chat_panel.rs                936    # AI chat panel with typewriter animation
│   ├── user_guide.rs                773    # 3-level drill-down guide viewer
│   ├── fiscal_modal.rs              719    # Fiscal year/period management + year-end close
│   ├── file_picker.rs               487    # File browser for CSV import
│   ├── account_picker.rs            464    # Live-query account search
│   ├── text_input_modal.rs          337    # Single-line text input
│   ├── status_bar.rs                264    # Entity name, fiscal period, messages, AI status
│   ├── confirmation.rs              215    # Yes/No dialog
│   └── existing_db_modal.rs         157    # Restore/Fresh/Cancel for existing DB files
│
├── inter_entity/
│   ├── mod.rs                       427    # InterEntityMode, intercompany account helpers
│   ├── form.rs                      742    # Split-pane JE form (Entity A + Entity B)
│   ├── write_protocol.rs            534    # Two-phase draft+post with rollback matrix
│   └── recovery.rs                  453    # Orphan draft detection and resolution
│
├── services/
│   ├── mod.rs                         2
│   ├── journal.rs                  1370    # Post, reverse, payment JE, envelope fills
│   └── fiscal.rs                    664    # Year-end close workflow
│
├── reports/
│   ├── mod.rs                       539    # Report trait, formatting, file I/O
│   ├── trial_balance.rs             286
│   ├── balance_sheet.rs             272
│   ├── income_statement.rs          260
│   ├── cash_flow.rs                 295
│   ├── account_detail.rs            288
│   ├── ar_aging.rs                  311
│   ├── ap_aging.rs                  249
│   ├── fixed_asset_schedule.rs      227
│   └── envelope_budget.rs           354
│
└── types/
    ├── mod.rs                        17
    ├── enums.rs                     663    # All persisted + in-memory enums
    ├── ids.rs                        72    # 12 ID newtypes
    ├── money.rs                     179    # Money(i64), scale 10^8
    └── percentage.rs                 94    # Percentage(i64), scale 10^6
```

## Architecture

### Three-State Wrapper Loop

`main.rs` runs a state machine with three states:

```rust
enum AppState {
    Splash,                        // Logo + version, 1s minimum, checks for updates
    Startup(Box<StartupScreen>),   // Entity picker (add/edit/delete/open)
    Running(Box<App>),             // Main application with tabs
}
```

Transitions:
- `Splash` → `Startup` (always, after 1s + optional update check)
- `Startup` → `Running` (on entity open: re-reads config, runs startup checks, creates App)
- `Running` → quit (on `q` or should_quit)

### App Struct

```rust
pub struct App {
    entity: EntityContext,                        // DB + name + 9 tabs
    config: WorkspaceConfig,                      // Parsed workspace.toml
    active_tab: usize,                            // 0-8 tab index
    mode: AppMode,                                // Normal | SecondaryEntityPicker | InterEntityAccountSetup | InterEntity
    status_bar: StatusBar,                        // Bottom bar widget
    fiscal_modal: Option<FiscalModal>,            // Fiscal period management overlay
    show_help: bool,                              // ? help overlay visible
    inter_entity_help: bool,                      // ? help in inter-entity mode
    user_guide: Option<UserGuide>,                // Ctrl+H guide viewer
    should_quit: bool,                            // Exit flag
    chat_panel: ChatPanel,                        // AI chat panel widget
    focus: FocusTarget,                           // MainTab | ChatPanel
    ai_state: AiRequestState,                     // Idle | CallingApi | FulfillingTools
    ai_client: Option<AiClient>,                  // Lazy-initialized Claude client
    pending_ai_messages: Option<Vec<ApiMessage>>,  // Queued AI request
    pending_slash_command: Option<SlashCommand>,   // Queued slash command
    file_picker: Option<FilePicker>,              // CSV file browser
    import_flow: Option<ImportFlowState>,          // CSV import wizard state
    pending_bank_detection: bool,                 // Trigger bank detection step
    pending_pass1: bool,                          // Trigger Pass 1 matching
    pending_pass2: bool,                          // Trigger Pass 2 AI matching
    pending_draft_creation: bool,                 // Trigger draft creation step
}
```

### EntityContext

```rust
pub struct EntityContext {
    pub db: EntityDb,
    pub name: String,
    pub tabs: Vec<Box<dyn Tab>>,  // 9 tabs in order: CoA, GL, JE, AR, AP, Envelopes, FixedAssets, Reports, AuditLog
}
```

### AppMode

```rust
pub enum AppMode {
    Normal,
    SecondaryEntityPicker { selected: usize, candidates: Vec<usize> },
    InterEntityAccountSetup { mode: Box<InterEntityMode>, confirm: Confirmation },
    InterEntity(Box<InterEntityMode>),
}
```

### StartupScreen Struct

```rust
pub struct StartupScreen {
    pub entities: Vec<EntityEntry>,
    pub selected_index: usize,
    pub update_notice: Option<String>,
    pub workspace_path: PathBuf,
    text_input: Option<TextInputModal>,
    pending_action: Option<PendingEntityAction>,
    confirm_delete: Option<Confirmation>,
    existing_db_modal: Option<ExistingDbModal>,
    pending_add: Option<PendingAdd>,
    status_message: Option<String>,
}
```

### Key Dispatch Priority Order

`src/app/key_dispatch.rs` — highest to lowest:

| Priority | Condition | Routes To |
|----------|-----------|-----------|
| 1 | Ctrl+H | Toggle user guide |
| 2 | User guide visible | Guide handles all keys; Esc dismisses |
| 3 | Help overlay visible (`?`) | Consumes all keys; Esc/? dismisses |
| 4 | File picker visible | `handle_file_picker_key()` |
| 5 | Import wizard visible | `handle_import_key()` |
| 6 | Chat panel visible + focused | Panel gets all keys (Tab switches focus to MainTab) |
| 6b | Chat panel visible + main focused | Tab/Ctrl+K switches focus to ChatPanel; others continue |
| 7 | Inter-entity mode | `?` shows help; Ctrl+K toggles panel; others to `handle_inter_entity_key()` |
| 8 | Account setup confirmation | `handle_account_setup_key()` |
| 9 | Secondary entity picker | `handle_secondary_picker_key()` |
| 10 | Fiscal modal open | Routes to modal |
| 11 | Active tab `wants_input()` | Suppresses globals; routes to tab |
| 12 | Global hotkeys | `q`, `?`, `f`, `Ctrl+K`, `1-9`, `Ctrl+←/→`, then delegate to tab |

### Tab Trait

```rust
pub trait Tab {
    fn title(&self) -> &str;
    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction;
    fn render(&self, frame: &mut Frame, area: Rect);
    fn refresh(&mut self, db: &EntityDb);
    fn wants_input(&self) -> bool;                              // default: false
    fn navigate_to(&mut self, record_id: RecordId, db: &EntityDb); // default: no-op
    fn has_unsaved_changes(&self) -> bool;                      // default: false
    fn hotkey_help(&self) -> Vec<(&'static str, &'static str)>; // default: empty
    fn selected_draft_import_ref(&self) -> Option<String>;      // default: None
}
```

**TabAction variants:** `None`, `SwitchTab(TabId)`, `NavigateTo(TabId, RecordId)`, `ShowMessage(String)`, `RefreshData`, `StartInterEntityMode`, `StartImport`, `StartRematch`, `Quit`

**TabId:** `ChartOfAccounts(0)`, `GeneralLedger(1)`, `JournalEntries(2)`, `AccountsReceivable(3)`, `AccountsPayable(4)`, `Envelopes(5)`, `FixedAssets(6)`, `Reports(7)`, `AuditLog(8)`

### EntityDb Pattern

`EntityDb` owns a `rusqlite::Connection` and hands out repo objects via accessor methods. Repos borrow `&Connection`:

```rust
db.accounts()        → AccountRepo
db.journals()        → JournalRepo
db.fiscal()          → FiscalRepo
db.envelopes()       → EnvelopeRepo
db.assets()          → AssetRepo
db.ar()              → ArRepo
db.ap()              → ApRepo
db.audit()           → AuditRepo
db.recurring()       → RecurringRepo
db.import_mappings() → ImportMappingRepo
```

For cross-repo transactions: `db.conn()` returns `&Connection` directly.

### ChatPanel → App Communication

The chat panel returns `ChatAction` variants that `App` processes:

- `SendMessage(Vec<ApiMessage>)` → queued as `pending_ai_messages`, processed in `process_pending()`
- `SlashCommand(SlashCommand)` → queued as `pending_slash_command`, executed in `process_pending()`
- `Close` → hides panel, resets focus
- `SkipTypewriter` → completes typewriter animation instantly

### AI Request Flow

`src/app/ai_handler.rs` — `handle_ai_request()`:

1. Lazy-init AI client via `ensure_ai_client()` (loads `~/.config/bookkeeper/secrets.toml`)
2. Load entity context from context directory
3. Build system prompt (persona + entity name + context)
4. Log user message as `AiPrompt` to audit
5. Set `ai_state = CallingApi`, render "Calling Accountant ☏", force `terminal.draw()`
6. **Tool-use loop** (max 5 rounds):
   - Call `client.send_single_round()` (blocking ureq POST)
   - `RoundResult::Done(response)` → break with text
   - `RoundResult::NeedsToolCall { tool_calls, .. }` → log each as `AiToolUse`, set `ai_state = FulfillingTools`, render "Checking the books 🕮", fulfill via `fulfill_tool_call()`, append tool results to messages, continue loop
   - Error → break with error
7. AI client drops (was taken via `.take()` at start)
8. On success: parse SUMMARY line, log `AiResponse`, add content to chat panel
9. On error: show error in status bar

### CSV Import Pipeline

`src/app/import_handler.rs` + `src/ai/csv_import.rs`:

1. **File picker** → user selects CSV file
2. **Bank selection** → pick existing bank config or add new
3. **Bank detection** (new bank only) → Claude analyzes first 4 CSV rows, detects columns + date format, saves config to entity TOML
4. **Duplicate check** → flag transactions with matching `import_ref` in journal_entries
5. **Pass 1 (Local)** → match against learned mappings in `import_mappings` table (exact then substring)
6. **Pass 2 (AI)** → batch unmatched transactions (25/batch) to Claude with account lookup tools
7. **Pass 3 (Clarification)** → user confirms Low-confidence AI matches
8. **Review screen** → user reviews all matches, can edit accounts, reject rows
9. **Draft creation** → batch-create Draft JEs in SAVEPOINT, learn new mappings, refresh tabs

## Data Model

### Tables (15 total)

| Table | Key Columns |
|-------|-------------|
| `accounts` | id, number (UNIQUE), name, account_type (TEXT), parent_id, is_active, is_contra, is_placeholder |
| `fixed_asset_details` | id, account_id (UNIQUE FK), cost_basis, in_service_date, useful_life_months, is_depreciable, source_cip_account_id, accum_depreciation_account_id, depreciation_expense_account_id |
| `fiscal_years` | id, start_date, end_date, is_closed, closed_at |
| `fiscal_periods` | id, fiscal_year_id (FK), period_number, start_date, end_date, is_closed, closed_at, reopened_at |
| `journal_entries` | id, je_number (UNIQUE), entry_date, memo, status (Draft/Posted), is_reversed, reversed_by_je_id, reversal_of_je_id, inter_entity_uuid, source_entity_name, fiscal_period_id (FK), import_ref |
| `journal_entry_lines` | id, journal_entry_id (FK), account_id (FK), debit_amount, credit_amount, line_memo, reconcile_state (Uncleared/Cleared/Reconciled), sort_order |
| `ar_items` | id, account_id (FK), customer_name, description, amount, due_date, status (Open/Partial/Paid), originating_je_id (FK) |
| `ar_payments` | id, ar_item_id (FK), je_id (FK), amount, payment_date |
| `ap_items` | id, account_id (FK), vendor_name, description, amount, due_date, status, originating_je_id (FK) |
| `ap_payments` | id, ap_item_id (FK), je_id (FK), amount, payment_date |
| `envelope_allocations` | id, account_id (UNIQUE FK), percentage |
| `envelope_ledger` | id, account_id (FK), entry_type (Fill/Transfer/Reversal), amount (signed), source_je_id, related_account_id, transfer_group_id (UUID), memo |
| `recurring_entry_templates` | id, source_je_id (FK), frequency (Monthly/Quarterly/Annually), next_due_date, is_active, last_generated_date |
| `audit_log` | id, action_type (TEXT), entity_name, record_type, record_id, description, created_at |
| `import_mappings` | id, description_pattern, account_id (FK), match_type (exact/substring), source (confirmed/ai_suggested), bank_name, use_count, UNIQUE(description_pattern, bank_name) |

### Money Representation

```rust
pub struct Money(pub i64);  // $1 = 100,000,000 (10^8 scale)
```

- Max: ~$92.2 billion
- Display: rounds to 2 decimal places with comma separators (`$1,234.56`)
- Arithmetic: `Add`, `Sub`, `Mul<i64>`, `Neg`
- Key methods: `from_dollars(f64)`, `cents_rounded() -> i64`, `abs()`, `apply_percentage(Percentage)`

### Percentage Representation

```rust
pub struct Percentage(pub i64);  // 1% = 1,000,000 (10^6 scale)
```

- Display: `"15.50%"`
- Key methods: `from_display(f64)`, `as_multiplier() -> f64`

### ID Newtypes (all wrap `i64`)

`AccountId`, `JournalEntryId`, `JournalEntryLineId`, `FiscalYearId`, `FiscalPeriodId`, `ArItemId`, `ApItemId`, `EnvelopeAllocationId`, `EnvelopeLedgerId`, `FixedAssetDetailId`, `RecurringTemplateId`, `AuditLogId`

### Enums

**Persisted (DB TEXT):** `AccountType` (5), `BalanceDirection` (2), `ReconcileState` (3), `JournalEntryStatus` (2), `ArApStatus` (3), `EntryFrequency` (3), `EnvelopeEntryType` (3), `ImportMatchType` (2), `ImportMatchSource` (2), `AuditAction` (24)

**In-memory only:** `AiRequestState` (3), `ChatRole` (3), `FocusTarget` (2), `MatchSource` (4), `MatchConfidence` (3)

### AuditAction Variants (24)

V1: `JournalEntryCreated`, `JournalEntryPosted`, `JournalEntryReversed`, `AccountCreated`, `AccountModified`, `AccountDeactivated`, `AccountReactivated`, `AccountDeleted`, `PeriodClosed`, `PeriodReopened`, `YearEndClose`, `EnvelopeAllocationChanged`, `EnvelopeTransfer`, `PlaceInService`, `InterEntityEntryPosted`, `ArItemCreated`, `ArPaymentRecorded`, `ApItemCreated`, `ApPaymentRecorded`

V2: `AiPrompt`, `AiResponse`, `AiToolUse`, `CsvImport`, `MappingLearned`

## Feature Summary

### Startup Screen

- **Add entity:** validates name uniqueness, derives filenames via `slugify()`, checks for existing DB files (offers Restore/Fresh), writes to workspace.toml via `toml_edit`
- **Edit entity:** changes display name in workspace.toml only (does NOT rename files)
- **Delete entity:** removes entry from workspace.toml only (does NOT delete .sqlite or .toml files)
- **Open entity:** re-reads config, runs startup checks (orphan recovery, recurring entries, pending depreciation)

### Startup Checks

Run after opening an entity, before the main event loop:

1. **Orphaned inter-entity drafts** — Detects Draft JEs with `inter_entity_uuid`. Classifies peer status (Draft/Posted/NotFound). Offers resolution: post both, delete both, complete, rollback, or delete orphan.
2. **Recurring entries due** — Templates with `next_due_date ≤ today`. Offers to generate Draft JEs.
3. **Pending depreciation** — Depreciable assets with ungenerated months through today. Offers to generate Draft JEs.

### 9 Tabs

| # | Tab | Key Features |
|---|-----|-------------|
| 1 | Chart of Accounts | Hierarchical tree, add/edit/delete accounts, search, activate/deactivate, place-in-service for fixed assets |
| 2 | General Ledger | Account picker, GL rows with running balance, reconcile state display (✓/✓✓), date range filter, Enter navigates to JE |
| 3 | Journal Entries | New/edit draft, post/reverse, status filter cycle, scheduled entries sub-view, CSV import (u), re-match (U), inter-entity (i), fiscal period filter (f) |
| 4 | Accounts Receivable | New AR item, record payment (auto-creates JE), status filter, search, open in GL |
| 5 | Accounts Payable | New AP item, record payment (auto-creates JE), status filter, search, open in GL |
| 6 | Envelopes | Two views (Allocations / Balances) toggled with `v`, edit allocation %, distribute funds, transfer funds, fiscal year selector |
| 7 | Fixed Assets | Register view (asset list), Schedule view (depreciation ledger), generate pending depreciation |
| 8 | Reports | 9 report types, date parameter forms, account picker for Account Detail, F9 generates + saves to file |
| 9 | Audit Log | Chronological event log, filter by action type (←/→), date range filter |

### AI Accountant (Ctrl+K)

- Chat panel with typewriter animation (80 chars per tick advance)
- 10 read-only tools for querying accounting data
- Tool-use loop (max 5 rounds per request)
- Prompt caching via `anthropic-beta: prompt-caching-2024-07-31`
- SUMMARY line convention: AI ends responses with `SUMMARY: [one sentence]`, stripped from display, logged to audit
- Slash commands: `/clear`, `/context`, `/compact`, `/persona [name]`, `/match`

### AI Tools (10)

| Tool | Description |
|------|-------------|
| `get_account` | Look up account by number/name/substring |
| `get_account_children` | Get child accounts under a placeholder |
| `search_accounts` | Search by substring with balances |
| `get_gl_transactions` | GL lines with debit/credit and running balance, optional date range |
| `get_journal_entry` | Get JE by number with all lines |
| `get_open_ar_items` | AR items with optional status filter |
| `get_open_ap_items` | AP items with optional status filter |
| `get_envelope_balances` | All envelope allocations with available amounts |
| `get_trial_balance` | Trial balance, optional as-of date |
| `get_audit_log` | Search audit by action type and/or date range |

### CSV Import

- 3-pass matching pipeline (local mappings → AI batch → user clarification)
- Bank detection via Claude (columns, date format)
- Duplicate detection against existing `import_ref` values
- Draft JE creation with learned mapping persistence
- Re-match existing incomplete drafts via Shift+U

### Reports (9 types)

| Report | Type | Description |
|--------|------|-------------|
| Trial Balance | As-of | All accounts with non-zero balances, total debits = total credits |
| Balance Sheet | As-of | Assets = Liabilities + Equity, hierarchical by type |
| Income Statement | Date range | Revenue − Expenses with subtotals, net income |
| Cash Flow | Date range | Direct method, cash/bank account inflows/outflows |
| Account Detail | Date range + account | All posted GL transactions with running balance |
| AR Aging | As-of | Open items in aging buckets (Current, 1-30, 31-60, 61-90, 90+) |
| AP Aging | As-of | Open items in aging buckets |
| Fixed Asset Schedule | As-of | Cost basis, accumulated depreciation, book value |
| Envelope Budget Summary | As-of | Allocations, earmarked vs GL balance, available amounts |

## All Hotkeys

### Startup Screen

| Key | Action |
|-----|--------|
| `↑/↓` | Navigate entity list |
| `Enter` | Open selected entity |
| `a` | Add new entity |
| `e` | Edit entity name |
| `d` | Delete entity (from config only) |
| `q` / `Esc` | Quit |

### Global (Running)

| Key | Action |
|-----|--------|
| `1–9` | Switch to tab by index |
| `Ctrl+←` / `Ctrl+→` | Previous / next tab |
| `Ctrl+K` | Toggle AI chat panel + focus |
| `Ctrl+H` | Open/close user guide |
| `f` | Open fiscal period management modal |
| `?` | Show/hide help overlay |
| `q` | Quit |

### Chart of Accounts

| Key | Action |
|-----|--------|
| `↑/↓` or `k/j` | Navigate |
| `/` | Search |
| `a` | Add account |
| `e` | Edit account |
| `d` | Toggle active/inactive |
| `x` | Delete account |
| `s` | Place in service as fixed asset |

### General Ledger

| Key | Action |
|-----|--------|
| `↑/↓` or `k/j` | Scroll entries |
| `p` | Pick account |
| `f` | Set date range filter |
| `Enter` | Navigate to JE |

### Journal Entries

| Key | Action |
|-----|--------|
| `↑/↓` or `k/j` | Navigate |
| `n` | New journal entry |
| `e` | Edit draft entry |
| `p` | Post selected entry |
| `r` | Reverse posted entry |
| `s` | Scheduled entries sub-view |
| `i` | New inter-entity entry |
| `g` | Go to General Ledger |
| `f` | Cycle fiscal period filter |
| `t` | Create scheduled entry |
| `u` | Import CSV statement |
| `U` (Shift) | Re-match incomplete imports |

### Journal Entries — Scheduled Sub-view

| Key | Action |
|-----|--------|
| `↑/↓` | Navigate |
| `Enter` | Jump to source JE |
| `g` | Generate due entries |
| `d` | Toggle active/inactive |
| `Esc` | Back to Journal Entries |

### Accounts Receivable / Accounts Payable

| Key | Action |
|-----|--------|
| `↑/↓` or `k/j` | Navigate |
| `n` | New item |
| `p` | Record payment |
| `o` | Open in General Ledger |
| `s` / `f` | Search / filter |

### Envelopes

| Key | Action |
|-----|--------|
| `v` | Switch view (Allocations ↔ Balances) |
| `↑/↓` | Navigate |
| `d` | Distribute funds to envelope |
| `t` | Transfer between envelopes |

### Fixed Assets

| Key | Action |
|-----|--------|
| `↑/↓` | Navigate assets |
| `Enter` | View depreciation schedule |
| `Esc` | Back to asset list |
| `g` | Generate pending depreciation |

### Reports

| Key | Action |
|-----|--------|
| `↑/↓` or `k/j` | Select report |
| `Enter` | Configure parameters |
| `Tab` | Next parameter field |
| `F9` | Generate and save report |
| `Esc` | Back to menu |

### Audit Log

| Key | Action |
|-----|--------|
| `↑/↓` or `k/j` | Scroll entries |
| `←/→` | Cycle action type filter |
| `d` | Set date range filter |
| `c` | Clear all filters |

### JE Form (shared by JE tab + inter-entity)

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Move between fields |
| `Enter` | Open account picker (account fields); advance (text fields); add line (from last field) |
| `Arrow keys + Enter` | Alternative navigation (fallback when Tab intercepted by chat panel) |
| `Ctrl+S` | Validate and submit |
| `Esc` | Cancel |
| `Ctrl+↓` | Insert new line below |
| `Ctrl+↑` / `Delete` | Remove focused line |

### Chat Panel

| Key | Action |
|-----|--------|
| `Ctrl+K` / `Esc` | Open / close panel |
| `Tab` | Switch focus (panel ↔ tab) |
| `Enter` | Submit message (or skip typewriter if active) |
| `↑/↓` | Scroll message history (when input empty) |
| `/clear` | Reset conversation |
| `/context` | Refresh tab data |
| `/compact` | Compress history |
| `/persona [name]` | View / change persona |
| `/match` | Re-match selected draft |

### Fiscal Modal

| Key | Action |
|-----|--------|
| `↑/↓` or `k/j` | Navigate |
| `a` | Add fiscal year |
| `c` | Close period |
| `o` | Reopen period |
| `y` | Year-end close |
| `Esc` | Close modal |

## Key Design Patterns

### How to Add a Tab

1. Create `src/tabs/my_tab.rs` implementing the `Tab` trait
2. Add variant to `TabId` in `src/tabs/mod.rs`
3. Add to `EntityContext::new()` in `src/app/mod.rs` (push to `tabs` vec)
4. Tab index is position in the vec (0-based)
5. Implement `refresh()` to load data from `EntityDb`
6. Return `TabAction` variants from `handle_key()` — never mutate App state directly

### How to Add a Repo

1. Create `src/db/my_repo.rs` with `pub struct MyRepo<'a> { conn: &'a Connection }`
2. Add accessor method to `EntityDb` in `src/db/mod.rs`: `pub fn my_repo(&self) -> MyRepo { MyRepo { conn: &self.conn } }`
3. Add CREATE TABLE to `src/db/schema.rs` in `initialize_schema()`
4. Use `params![]` / `named_params!{}` for all SQL — never string interpolation

### How to Add an AI Tool

1. Add tool definition to `tool_definitions()` in `src/ai/tools.rs`
2. Add handler function `handle_my_tool(params: &Value, db: &EntityDb) -> Result<String>`
3. Add dispatch arm to `fulfill_tool_call()` match
4. Tools are read-only — never write through tools

### How to Add a Slash Command

1. Add variant to `SlashCommand` enum in `src/widgets/chat_panel.rs`
2. Add parse arm to `SlashCommand::parse()` in `src/widgets/chat_panel.rs`
3. Add execution arm to `execute_slash_command()` in `src/app/ai_handler.rs`

### How to Add a Widget

1. Create `src/widgets/my_widget.rs` with struct + action enum
2. Export from `src/widgets/mod.rs`
3. Widget returns its own action type; caller in tab or App handles the action
4. Use `centered_rect()` from `src/widgets/mod.rs` for modal positioning

## Gotchas

### Money & Precision

- **$1 = 100,000,000 internal units** (8 decimal places). Test values: `$100 = 10_000_000_000`.
- **Percentages**: `1% = 1,000,000 units`, `10% = 10,000,000`.
- **Rounding**: final depreciation month absorbs remainder so `SUM(all months) == cost_basis` exactly.
- **Money from CSV**: `parse_money_str()` in `src/ai/csv_import.rs` uses pure i64 arithmetic — parses integer and decimal parts separately, pads/truncates decimals to 8 digits, multiplies integer part by 10^8 and adds. **No f64 intermediary ever touches the conversion path.** Handles `$`, commas, parentheses for negatives, empty strings.

### Architecture

- **`EntityDb` is a wrapper** that owns the `rusqlite::Connection` and hands out repo objects via accessor methods. Repos borrow `&Connection`.
- **`Tab::handle_key`** returns `TabAction`; tabs never mutate `App` state directly.
- **`TabAction::ShowMessage`** routes to `StatusBar::set_success`. Use `App::set_error` directly for explicit error paths.
- **AI client ownership transfer**: `handle_ai_request()` calls `self.ai_client.take()` to move the client out of the Option, uses it through the tool-use loop, then it drops. The client is stateless (all state passed per call). This avoids borrow conflicts since the method also needs `&mut self` for other fields.
- **Forced render before blocking calls**: Must call `terminal.draw()` before any `ureq` call so the user sees the loading state before the UI freezes. Appears in `src/app/ai_handler.rs` before the first API call and before tool fulfillment.

### Cash Account Detection (Envelope Fill)

- Cash = `account_type == Asset && !is_placeholder && name.to_lowercase().contains("cash|bank|checking|savings")`.
- Owner's Draw suppression: `account_type == Equity && is_contra` → skip fill.
- If JE has **multiple** cash debit lines, envelope fill amount is the **sum of all** cash debits.

### Fiscal Periods

- `create_draft` rejects closed periods at creation time (avoids orphaned un-postable entries).
- `generate_pending_depreciation` returns `(Vec<JournalEntryId>, Option<String>)`. The warning fires when a depreciation month has no fiscal period; generation stops for that asset (not error).
- Year-end close zeroes GL balances for revenue/expense; **does NOT** clear envelope earmarks.
- Retained Earnings account is identified by account number "3300".

### CIP Account Detection

- `PlaceInService` form opens only when selected account name contains "construction" (case-insensitive). Tested via substring match, not account type.

### Status Bar

- `set_message` / `set_success` → success (green, 3s). `set_error` → error (red, 5s).
- `[*]` unsaved indicator: driven by `Tab::has_unsaved_changes()`; App polls each tick.
- JournalEntriesTab overrides `has_unsaved_changes()` to reflect form content — returns true only when user has typed something (auto-filled date alone doesn't count), or when a confirmation modal is open.
- AI status takes priority over normal messages when both are set.

### Confirmation Widget

- **Confirmation widget handles its own centering** via `centered_rect()`. Never call `centered_rect()` on the area before passing it to `Confirmation::render()` — this causes double-centering that makes the content area too small to display anything.

### Tab Key Conflict (V2)

- Tab key is intercepted at App level when chat panel is open (switches focus between panel and main tab).
- JE form uses arrow keys + Enter as fallback navigation.
- Envelopes uses `v` for view toggle always (never Tab).
- Reports tab uses Tab for parameter field navigation — works when chat panel is closed.

### import_ref Format

- Format: `"{bank_name}|{date}|{description}|{amount_raw}"` where amount_raw is the Money i64 internal representation.
- Date format in import_ref: `%Y-%m-%d`.
- If descriptions contain pipe characters, `parse_import_ref()` parses from the ends: bank_name is first segment, amount is last, date is second, description is everything in between.

### SUMMARY Line Convention

- System prompt instructs Claude to end responses with `SUMMARY: [one sentence]`.
- `parse_summary()` in `src/ai/client.rs` searches for the **last** `SUMMARY:` line using `rfind`.
- Strips the SUMMARY line from display text, logs it to audit.
- Fallback if missing: first sentence truncated to 100 chars via `extract_first_sentence()`.

### Entity Path Resolution

- All paths in workspace.toml support `~/` expansion and relative paths.
- Relative paths are resolved against the workspace directory (parent of workspace.toml).
- Paths are expanded and made absolute at config load time via `expand_config_paths()` in `src/config.rs`.
- Entity TOML `config_path` resolved via `resolve_config_path()` — same `~/` + relative logic.

### Secrets File Location

- Secrets live at `~/.config/bookkeeper/secrets.toml` — **not** `~/.config/bursar/`.
- The `bookkeeper` name is a legacy artifact from the config directory naming.
- Auto-creates `~/.config/bookkeeper/` directory if missing on first access.
- Loaded lazily: not at startup. First `Ctrl+K` or `u` (CSV import) triggers load. Missing key shows specific error with the file path.

### toml_edit for Entity Management

- `src/startup_screen.rs` uses `toml_edit::DocumentMut` (not serde serialization) for workspace.toml mutations.
- This preserves formatting, comments, and whitespace in the user's TOML file.
- Entities are an array-of-tables (`[[entities]]`). Add pushes to the array, edit modifies in-place by index, delete removes by index.

### Inter-Entity Write Protocol

- Two-phase: create Drafts in both entities, then Post both.
- Shared UUID links the pair (`inter_entity_uuid` column).
- Rollback matrix: Step 5 fail → delete A draft. Step 6 fail → delete both drafts. Step 7 fail → reverse A (posted) + delete B (draft).
- At startup, orphaned inter-entity drafts are detected and user is prompted to resolve.

### Tests

- **No `expect()` in production code** — all `expect()` calls are in `#[cfg(test)]` blocks with clear invariant messages.
- **In-memory DBs** (`:memory:`) used for unit tests; temp file DBs for persistence tests.
- **Only one code comment** (`// NOTE` in `src/db/audit_repo.rs`) about sentinel query pattern — the codebase has no TODO/HACK/FIXME markers.
- **Integration test** (`src/integration_tests.rs`): single comprehensive test covering full lifecycle — fiscal years, JE creation/posting, AR/AP flows, assets, envelopes, period close, all reports, year-end close.

### AI & Import Details

- Max 5 rounds of tool-use per AI request.
- `accumulated_text` carries partial text from prior rounds (some rounds return both text and tool calls).
- Prompt caching enabled for chat panel and Pass 2 batch requests (`use_cache: true`), disabled for one-offs like `/match`.
- API timeout: 120 seconds.
- HTTP headers: `x-api-key`, `anthropic-version: 2023-06-01`, `anthropic-beta: prompt-caching-2024-07-31`.
- Default model: `claude-sonnet-4-20250514`. Max tokens: 4096.
- Bank detection sends first 4 CSV rows to Claude.
- Pass 2 batches unmatched transactions 25 per batch.
- `determine_debit_credit()` in `src/ai/csv_import.rs` determines bank entry side: Asset + positive (deposit) → debit bank; Asset + negative (withdrawal) → credit bank.
- Draft creation uses `SAVEPOINT` for atomicity — rolls back all if any creation fails.
- Learned mappings saved to `import_mappings` table after successful draft creation.
- `last_import_dir` saved to entity TOML for convenience.
- Context files auto-created at `{context_dir}/{slugified_entity_name}.md` with skeleton content on first access.

## Configuration Reference

### workspace.toml

```toml
report_output_dir = "~/bursar/reports"   # default

[updates]
github_repo = "owner/repo"              # optional, enables update checks

[ai]
persona = "Professional Tax Accountant" # default
model = "claude-sonnet-4-20250514"        # default

last_opened_entity = "My Farm LLC"       # auto-set on entity open

[[entities]]
name = "My Farm LLC"
db_path = "my-farm-llc.sqlite"           # relative to workspace dir, or absolute, or ~/
config_path = "my-farm-llc.toml"         # optional
```

### Per-entity TOML (e.g., my-farm-llc.toml)

```toml
ai_persona = "Agricultural Tax Expert"  # optional, overrides workspace persona
last_import_dir = "/home/user/downloads" # optional, remembered from last import

[[bank_accounts]]
name = "Chase Checking"
linked_account = "1010"                  # account number in CoA
date_column = "Post Date"
description_column = "Description"
amount_column = "Amount"                 # single-column mode
# debit_column = "Debit"               # split-column mode (alternative)
# credit_column = "Credit"
debit_is_negative = true                 # default
date_format = "%m/%d/%Y"
```

### secrets.toml (`~/.config/bookkeeper/secrets.toml`)

```toml
anthropic_api_key = "sk-ant-..."
```

## Dependencies

```toml
[dependencies]
ratatui = { version = "0.29", features = ["unstable-rendered-line-info"] }
crossterm = "0.28"
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
thiserror = "2"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde_json = "1"
csv = "1"
ureq = { version = "2", features = ["json"] }
toml_edit = "0.25"
```

## Out of Scope

- Multi-user / concurrent access
- Async / tokio
- Networking beyond Claude API + GitHub update check
- AI write operations (AI tools are read-only)
- Direct posting from CSV import (always creates Drafts)
- File deletion on entity delete (only removes from workspace.toml)
- File renaming on entity edit (only changes display name)
