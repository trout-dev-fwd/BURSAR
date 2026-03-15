# Architecture

## Overview

A synchronous, single-threaded Ratatui TUI application. One entity is active at a time. A second
entity's database is opened only inside the inter-entity journal entry modal. No async runtime.
No network calls. All I/O is terminal events + local SQLite reads/writes.

---

## Design Decisions

### Event Loop: Synchronous
- Standard `crossterm` event polling with a **500ms tick rate**.
- No `tokio`. No async. Ratatui's `Terminal::draw()` is synchronous, `rusqlite` is synchronous,
  and the I/O profile (terminal input + local SQLite) is inherently blocking/single-threaded.
- **WHY**: Async adds complexity (spawn_blocking wrappers, runtime overhead) with no benefit for
  this workload. Every production Ratatui app uses this pattern.

### Terminal Backend: Crossterm
- `ratatui` with `crossterm` backend.
- **WHY**: Most popular, best maintained, largest ecosystem of examples and community support.

### Database Access: Repository Per Domain
- Each domain (accounts, journal entries, AR, AP, envelopes, etc.) gets its own repository struct.
- Repositories hold a reference to the `rusqlite::Connection` and expose domain-specific query methods.
- **WHY**: Keeps the agent's context focused. When working on AR features, only `ar_repo.rs` and
  the AR tab need to be in context — not a 2,000-line god struct.

### State Management: Tab Trait
- Each of the 9 tabs implements a `Tab` trait with `handle_key`, `render`, and `title` methods.
- Each tab is its own file under `src/tabs/`.
- Tabs communicate outward via `TabAction` return values, not by mutating shared state.
- **WHY**: Isolates each tab's logic. Agent works on one file per tab. Adding a new tab means one
  new file + one line to register it. Cross-tab navigation is explicit via `TabAction::NavigateTo`.

### Entity Management: Single Active Entity
- The app is fundamentally **single-entity**. `App` holds one `EntityContext` (database, tab set, name).
- The second entity is **only** opened inside the inter-entity journal entry modal overlay.
- When the modal opens, it creates a temporary second database connection. When it closes, the
  connection is dropped.
- No entity-switching hotkeys. No dual tab sets. No dual-entity state management.
- **WHY**: The multi-entity feature exists solely to post inter-company journal entries to both
  databases simultaneously. There is no use case for browsing Entity B's ledger or reports.

### Fuzzy Search: Substring/Prefix Match
- Account selection in entry forms uses simple substring and prefix matching.
- No external fuzzy matching crate.
- **WHY**: The account list is small (dozens of accounts). Substring matching is sufficient and
  avoids an extra dependency. Can upgrade to a fuzzy crate later if needed.

### Report Output: Global Configuration
- Report output directory configured once in `workspace.toml`, shared by all entities.
- Reports are `.txt` files named `[ReportName][MM-DD-YYYY].txt`.

---

## Tech Stack

| Component          | Choice                                        |
|--------------------|-----------------------------------------------|
| Language           | Rust (stable toolchain)                       |
| TUI framework      | `ratatui` + `crossterm`                       |
| Database           | SQLite via `rusqlite` (bundled feature)        |
| Config             | `serde` + `toml` for `workspace.toml`          |
| Dates              | `chrono` (`NaiveDate`, `NaiveDateTime`)        |
| UUIDs              | `uuid` (inter-entity transaction IDs, transfer group IDs) |
| Error handling     | `thiserror` (library/domain errors), `anyhow` (CLI/main) |
| Logging            | `tracing` + `tracing-subscriber`               |
| Secrets            | N/A (no auth, no API keys in V1)               |
| Async runtime      | **None** (synchronous event loop)              |

---

## Module Structure

```
src/
├── main.rs                        # Entry point: load config, open entity, run event loop
├── app.rs                         # App struct, event loop, global hotkey dispatch
├── config.rs                      # workspace.toml parsing
├── types/
│   ├── mod.rs
│   ├── money.rs                   # Money(i64) newtype
│   ├── percentage.rs              # Percentage(i64) newtype
│   ├── ids.rs                     # All ID newtypes via macro
│   └── enums.rs                   # AccountType, JournalEntryStatus, ReconcileState, etc.
├── db/
│   ├── mod.rs                     # EntityDb struct (holds Connection, provides repo access)
│   ├── schema.rs                  # CREATE TABLE statements, schema init, seed data
│   ├── account_repo.rs            # CRUD for accounts + fixed_asset_details
│   ├── journal_repo.rs            # JE + JE lines: create, post, reverse, list, search
│   ├── ar_repo.rs                 # AR items + AR payments
│   ├── ap_repo.rs                 # AP items + AP payments
│   ├── envelope_repo.rs           # Allocations + ledger: fill, transfer, balance queries
│   ├── fiscal_repo.rs             # Fiscal years + periods: create, close, reopen
│   ├── asset_repo.rs              # Fixed asset register + depreciation generation
│   ├── recurring_repo.rs          # Recurring templates: list upcoming, generate entries
│   └── audit_repo.rs              # Audit log: append-only writes, filtered reads
├── tabs/
│   ├── mod.rs                     # Tab trait, TabAction enum, TabId enum
│   ├── chart_of_accounts.rs       # Account list, balances, envelope indicators, placeholders
│   ├── general_ledger.rs          # Per-account transaction history, date filtering
│   ├── journal_entries.rs         # Entry list, new entry (delegates to JeForm widget), recurring
│   ├── accounts_receivable.rs     # Open items, mark paid, payment recording
│   ├── accounts_payable.rs        # Open items, mark paid, payment recording
│   ├── envelopes.rs               # Allocation config, balances, transfers
│   ├── fixed_assets.rs            # Asset register, depreciation schedule, place-in-service
│   ├── reports.rs                 # Report selection menu, parameter input, generation trigger
│   └── audit_log.rs               # Immutable event list, date/action filtering
├── inter_entity/
│   ├── mod.rs                     # Inter-entity modal overlay controller
│   ├── form.rs                    # Split-pane entry form (top: form, bottom-L/R: account lists)
│   └── recovery.rs                # Startup consistency check for orphaned inter-entity drafts
├── reports/
│   ├── mod.rs                     # Report trait, shared formatting utilities (box-drawing)
│   ├── trial_balance.rs
│   ├── balance_sheet.rs
│   ├── income_statement.rs
│   ├── cash_flow.rs
│   ├── account_detail.rs
│   ├── ar_aging.rs
│   ├── ap_aging.rs
│   └── fixed_asset_schedule.rs
├── widgets/
│   ├── mod.rs
│   ├── account_picker.rs          # Substring-match account selector (reused across tabs)
│   ├── confirmation.rs            # Yes/No confirmation modal
│   ├── status_bar.rs              # Entity name, fiscal period, unsaved changes indicator
│   └── je_form.rs                 # Journal entry form widget (reused in JE tab + inter-entity)
└── startup.rs                     # Startup sequence: recurring entries, inter-entity recovery,
                                   #   depreciation check
```

---

## Key Components

### `App` (app.rs)

The top-level application struct. Owns the event loop and all state.

```rust
struct App {
    entity: EntityContext,          // active entity: db handle, name, tabs
    config: WorkspaceConfig,       // parsed workspace.toml
    active_tab: usize,             // index into entity.tabs
    mode: AppMode,                 // Normal, InterEntity, Modal
    status_bar: StatusBar,
    should_quit: bool,
}

struct EntityContext {
    db: EntityDb,                  // database handle + repos
    name: String,                  // display name from workspace config
    tabs: Vec<Box<dyn Tab>>,       // the 9 tab instances
}

enum AppMode {
    Normal,                        // standard single-entity tab navigation
    InterEntity(InterEntityMode),  // modal overlay with second DB connection
    Modal(ModalKind),              // confirmation prompts, entity picker, etc.
}
```

**Event loop pseudocode:**

```
fn run(app: &mut App, terminal: &mut Terminal) -> Result<()> {
    loop {
        // 1. Render
        terminal.draw(|frame| {
            render_status_bar(frame, &app.status_bar);
            match &app.mode {
                Normal => app.entity.tabs[app.active_tab].render(frame, area),
                InterEntity(mode) => mode.render(frame, area),
                Modal(modal) => {
                    // render active tab underneath, then modal on top
                    app.entity.tabs[app.active_tab].render(frame, area);
                    modal.render(frame, centered_area);
                }
            }
        });

        // 2. Poll for input (500ms timeout = tick rate)
        if crossterm::event::poll(Duration::from_millis(500))? {
            if let Event::Key(key) = crossterm::event::read()? {
                // 3. Handle input
                match &app.mode {
                    Modal(modal) => handle_modal_key(app, key),
                    InterEntity(mode) => handle_inter_entity_key(app, key),
                    Normal => {
                        // Global hotkeys first (tab switch, quit, inter-entity)
                        if let Some(action) = handle_global_key(app, key) {
                            process_action(app, action);
                        } else {
                            // Delegate to active tab
                            let action = app.entity.tabs[app.active_tab]
                                .handle_key(key, &app.entity.db);
                            process_action(app, action);
                        }
                    }
                }
            }
        }

        // 4. Tick: update status bar clock, check for pending items
        app.status_bar.tick();

        if app.should_quit { break; }
    }
    Ok(())
}
```

### `Tab` Trait (tabs/mod.rs)

The contract every tab implements.

```rust
trait Tab {
    /// Display name for the tab bar.
    fn title(&self) -> &str;

    /// Handle a key press. Returns an action for App to process.
    /// Receives a read reference to the database for queries.
    /// For mutations, the tab calls repo methods directly and returns
    /// TabAction::RefreshData so App re-queries affected data.
    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction;

    /// Render this tab's content into the given area.
    fn render(&self, frame: &mut Frame, area: Rect);

    /// Called by App after any data mutation (RefreshData action).
    /// The tab re-queries whatever data it displays.
    fn refresh(&mut self, db: &EntityDb);

    /// Called when navigating to this tab with a specific record to focus.
    /// Default implementation does nothing (tabs that don't support it ignore it).
    fn navigate_to(&mut self, record_id: RecordId, db: &EntityDb) {
        let _ = (record_id, db); // default: no-op
    }
}
```

### `TabAction` Enum (tabs/mod.rs)

How tabs communicate outward to the App.

```rust
enum TabAction {
    /// Nothing happened, no state change.
    None,
    /// Switch to another tab.
    SwitchTab(TabId),
    /// Switch to another tab and focus a specific record.
    /// Example: AR tab → "view originating journal entry" → NavigateTo(JournalEntries, je_id)
    NavigateTo(TabId, RecordId),
    /// Display a message in the status bar.
    ShowMessage(String),
    /// Data was mutated. App should call refresh() on all tabs.
    RefreshData,
    /// Enter inter-entity journal entry mode (requires 2nd entity).
    /// If only one entity is configured, App shows entity picker first.
    StartInterEntityMode,
    /// Quit the application.
    Quit,
}

enum TabId {
    ChartOfAccounts,    // index 0
    GeneralLedger,      // index 1
    JournalEntries,     // index 2
    AccountsReceivable, // index 3
    AccountsPayable,    // index 4
    Envelopes,          // index 5
    FixedAssets,        // index 6
    Reports,            // index 7
    AuditLog,           // index 8
}

/// Used for cross-tab navigation. Wraps the relevant ID type.
enum RecordId {
    Account(AccountId),
    JournalEntry(JournalEntryId),
    ArItem(ArItemId),
    ApItem(ApItemId),
}
```

### `EntityDb` (db/mod.rs)

Holds the database connection and provides access to all repositories.

```rust
struct EntityDb {
    conn: rusqlite::Connection,
}

impl EntityDb {
    /// Open an existing entity database file.
    fn open(path: &Path) -> Result<Self>;

    /// Create a new entity database: create file, run schema, seed default accounts.
    fn create(path: &Path, entity_name: &str) -> Result<Self>;

    /// Repository accessors. Each borrows &self.conn.
    fn accounts(&self) -> AccountRepo<'_>;
    fn journals(&self) -> JournalRepo<'_>;
    fn ar(&self) -> ArRepo<'_>;
    fn ap(&self) -> ApRepo<'_>;
    fn envelopes(&self) -> EnvelopeRepo<'_>;
    fn fiscal(&self) -> FiscalRepo<'_>;
    fn assets(&self) -> AssetRepo<'_>;
    fn recurring(&self) -> RecurringRepo<'_>;
    fn audit(&self) -> AuditRepo<'_>;

    /// Direct connection access for transactions that span multiple repos.
    fn conn(&self) -> &rusqlite::Connection;
}
```

**Cross-repo operations** (e.g., posting a journal entry that triggers envelope fills) use
`self.conn()` directly to wrap multiple repo calls in a single SQLite transaction:

```rust
fn post_journal_entry(db: &EntityDb, je_id: JournalEntryId) -> Result<()> {
    let tx = db.conn().transaction()?;
    // 1. Validate and update JE status via journal_repo
    // 2. Check if this is a cash receipt, trigger envelope fills via envelope_repo
    // 3. Write audit log via audit_repo
    tx.commit()?;
    Ok(())
}
```

These cross-repo orchestration functions live in a `services` pattern — either as free functions
in the relevant tab file, or in a thin `services/` module if they grow complex. The agent should
start with free functions and extract to a services module only if needed.

### Repository Pattern (db/*_repo.rs)

Each repository is a lightweight struct borrowing the connection.

```rust
struct AccountRepo<'conn> {
    conn: &'conn rusqlite::Connection,
}

impl<'conn> AccountRepo<'conn> {
    fn list_active(&self) -> Result<Vec<Account>>;
    fn get_by_id(&self, id: AccountId) -> Result<Account>;
    fn create(&self, new: &NewAccount) -> Result<AccountId>;
    fn update(&self, id: AccountId, changes: &AccountUpdate) -> Result<()>;
    fn deactivate(&self, id: AccountId) -> Result<()>;
    fn get_balance(&self, id: AccountId) -> Result<Money>;
    fn get_children(&self, parent_id: AccountId) -> Result<Vec<Account>>;
    fn search(&self, query: &str) -> Result<Vec<Account>>;  // substring match
}
```

Each repo method:
- Takes typed parameters (newtypes, not raw i64)
- Returns `Result<T>` using `thiserror`-defined errors
- Uses parameterized SQL queries (no string interpolation)
- Deserializes TEXT columns to enums via `FromStr`

### Report Trait (reports/mod.rs)

Shared contract and formatting utilities for all 8 reports.

```rust
trait Report {
    /// Human-readable report name (used in filename and header).
    fn name(&self) -> &str;

    /// Generate the report content. Returns the full formatted text.
    fn generate(&self, db: &EntityDb, params: &ReportParams) -> Result<String>;
}

struct ReportParams {
    entity_name: String,
    as_of_date: Option<NaiveDate>,       // for point-in-time reports (Balance Sheet, Trial Balance)
    date_range: Option<(NaiveDate, NaiveDate)>,  // for period reports (Income Statement, Cash Flow)
    account_id: Option<AccountId>,        // for Account Detail report
}
```

**Shared formatting module** provides:
- Box-drawing character table (─, │, ┌, ┐, └, ┘, ├, ┤, ┬, ┴, ┼)
- `fn format_header(entity: &str, title: &str, date_info: &str) -> String`
- `fn format_table(headers: &[&str], rows: &[Vec<String>], alignments: &[Align]) -> String`
- `fn format_money(amount: Money) -> String` — right-aligned, thousands separators, 2 decimal places
- Column width auto-calculation based on content

All reports use these shared utilities. No bespoke formatting per report.

### Inter-Entity Modal (inter_entity/mod.rs)

A modal overlay that temporarily opens a second database connection.

```rust
struct InterEntityMode {
    primary_db: EntityDb,          // reference to App's active entity (Entity A)
    secondary_db: EntityDb,        // temporary connection to Entity B
    primary_name: String,
    secondary_name: String,
    form: InterEntityForm,         // the shared entry form
    primary_accounts: Vec<Account>,   // Entity A's chart of accounts for bottom-left pane
    secondary_accounts: Vec<Account>, // Entity B's chart of accounts for bottom-right pane
}
```

**Layout** (┬ split):
```
┌──────────────────────────────────────────────┐
│  Inter-Entity Journal Entry Form             │
│  Date: ______  Memo: ________________________│
│  Entity A lines:                             │
│    [account picker] [debit] [credit]         │
│  Entity B lines:                             │
│    [account picker] [debit] [credit]         │
├───────────────────────┬──────────────────────┤
│  Entity A             │  Entity B            │
│  Chart of Accounts    │  Chart of Accounts   │
│  + Earmarked amounts  │  + Earmarked amounts │
└───────────────────────┴──────────────────────┘
```

**Lifecycle:**
1. User presses `i` from Journal Entries tab
2. If only one entity in workspace config → show error message
3. If multiple entities available → show entity picker for the second entity
4. Open second entity's database, load its accounts
5. Display the split-pane form
6. On submit: validate, execute the inter-entity write protocol (see type-system.md)
7. On `Esc`: prompt if unsaved changes, then close second DB connection and return to Normal mode

### Startup Sequence (startup.rs)

Runs after the entity database is opened, before the event loop starts.

```rust
fn run_startup_checks(db: &EntityDb, config: &WorkspaceConfig) -> Result<Vec<StartupPrompt>> {
    let mut prompts = Vec::new();

    // 1. Inter-entity recovery (HIGHEST PRIORITY — data integrity)
    //    Check for orphaned inter-entity drafts.
    //    Prompt user immediately if found.
    prompts.extend(check_inter_entity_consistency(db, config)?);

    // 2. Recurring entries due
    //    Check for recurring templates where next_due_date <= today.
    //    Prompt user to review and post.
    prompts.extend(check_recurring_entries(db)?);

    // 3. Depreciation entries
    //    Check for assets with un-generated depreciation months.
    //    Prompt user to review and post.
    prompts.extend(check_pending_depreciation(db)?);

    Ok(prompts)
}
```

Prompts are shown sequentially. Inter-entity recovery always first (data integrity).
User must resolve each prompt before the main UI loads.

### Workspace Config (config.rs)

```rust
/// Parsed from workspace.toml
struct WorkspaceConfig {
    report_output_dir: PathBuf,
    entities: Vec<EntityConfig>,
}

struct EntityConfig {
    name: String,
    db_path: PathBuf,
}
```

**Example `workspace.toml`:**

```toml
report_output_dir = "~/accounting/reports"

[[entities]]
name = "Acme Land LLC"
db_path = "~/accounting/database/acme_land.sqlite"

[[entities]]
name = "Acme Rentals LLC"
db_path = "~/accounting/database/acme_rentals.sqlite"
```

---

## Data Flow

### User Action → Database Mutation → UI Update

```
1. User presses key (e.g., 'p' to post a journal entry)
       ↓
2. App dispatches to active tab's handle_key()
       ↓
3. Tab calls repo method(s) via EntityDb
   (e.g., journal_repo.post(), envelope_repo.fill(), audit_repo.append())
   Wrapped in a SQLite transaction if cross-repo.
       ↓
4. Tab returns TabAction::RefreshData
       ↓
5. App calls refresh() on all tabs
   Each tab re-queries its display data from the DB.
       ↓
6. Next render cycle picks up the updated state.
```

### Cross-Tab Navigation

```
1. User is on AR tab, selects an AR item, presses hotkey to view originating JE
       ↓
2. AR tab returns TabAction::NavigateTo(TabId::JournalEntries, RecordId::JournalEntry(je_id))
       ↓
3. App switches active_tab to JournalEntries
       ↓
4. App calls journal_entries_tab.navigate_to(RecordId::JournalEntry(je_id), &db)
       ↓
5. JE tab scrolls to and highlights the specified entry.
```

---

## Error Handling Strategy

### Error Types

```rust
// Domain errors (thiserror) — specific, matchable
#[derive(Debug, thiserror::Error)]
enum AccountError {
    #[error("Account {0} is a placeholder and cannot receive journal entries")]
    IsPlaceholder(AccountId),
    #[error("Account {0} is deactivated")]
    IsInactive(AccountId),
    #[error("Account number '{0}' already exists")]
    DuplicateNumber(String),
}

#[derive(Debug, thiserror::Error)]
enum JournalError {
    #[error("Entry does not balance: debits={0}, credits={1}")]
    Unbalanced(Money, Money),
    #[error("Fiscal period {0} is closed")]
    PeriodClosed(FiscalPeriodId),
    #[error("Entry {0} is already reversed")]
    AlreadyReversed(JournalEntryId),
    #[error("Cannot modify reconciled line")]
    ReconciledLine,
}

// ... similar for each domain
```

### Error Display in TUI

- Domain errors → displayed as status bar messages (red text, auto-clear after 5 seconds)
- Database errors → displayed as modal error dialog with the technical message
- Fatal errors (cannot open DB, corrupt schema) → print to stderr and exit with non-zero code

### No `.unwrap()` in Production Code

Per `CLAUDE.md` rules:
- `?` for propagation throughout
- `thiserror` for domain-specific errors in repo and service code
- `anyhow` at the `main.rs` / `app.rs` boundary for catch-all error reporting
- `.expect("reason")` permitted ONLY in initialization code with a clear invariant comment

---

## Global Hotkeys

Handled by `App` before delegating to the active tab.

| Key     | Context        | Action                                               |
|---------|----------------|------------------------------------------------------|
| `1`–`9` | Normal mode   | Switch to tab by number                              |
| `n`     | Normal mode    | New journal entry (delegates to JE tab)              |
| `i`     | Normal mode    | Enter inter-entity mode (from JE tab context)        |
| `/`     | Normal mode    | Activate search in current tab                       |
| `?`     | Normal mode    | Show help overlay                                    |
| `q`     | Normal mode    | Quit (with unsaved-changes confirmation if needed)   |
| `Esc`   | Modal/InterEntity | Close modal / exit inter-entity mode             |

Tab-specific hotkeys (`r` for reverse, `p` for post, `c` for clear) are handled by
the individual tab's `handle_key()`, not at the global level.
