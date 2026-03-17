# V2 Architecture — AI Accounting Assistant

This document defines the architectural additions for V2 Phase 1. All V1 architecture remains unchanged — this document covers only new modules, new traits/structs, new data flows, and modifications to existing components.

---

## Foundational Decisions

### Decision 1: Remain synchronous — no async, no tokio

V2 introduces HTTP calls to the Claude API via `ureq`, which is a blocking synchronous HTTP client. This preserves V1's no-async constraint. The trade-off is that the UI freezes during API calls (typically 2-6 seconds per call). This is mitigated by:
- Forced render flush before each blocking call (user sees loading indicator)
- 10-second per-call timeout (bounded freeze duration)
- Status messages with personality ("Calling Accountant ☏") so the freeze feels intentional

**Why not threading:** A `std::thread` + `mpsc::channel` approach would keep the UI responsive during API calls. This was considered and deferred — the blocking approach is simpler, covers the use case adequately, and can be upgraded to threaded later without architectural changes (the `AiRequestState` enum and forced-render pattern are compatible with either approach).

### Decision 2: New code lives in `src/ai/` module

All AI functionality is isolated in a new `src/ai/` module. This keeps AI concerns separate from the existing accounting logic. No existing module gains AI dependencies. The only touch points between AI and the rest of the app are:
- `App` orchestrates AI requests and manages state
- `ChatPanel` widget renders the chat interface
- Existing repos are called through the tool fulfillment layer (read-only)
- Audit log receives new entry types through the existing `AuditRepo`

### Decision 3: Tools access data read-only

Claude's tools can read data from the entity database but never write. All mutations (creating drafts, writing mappings) are performed by application code after Claude returns its response. This ensures:
- Claude cannot accidentally corrupt data
- All writes go through established repo methods with validation
- The audit trail captures mutations at the application level, not the AI level

### Decision 4: Configuration layering

Configuration is read from multiple sources with a clear precedence order:
1. `~/.config/bookkeeper/secrets.toml` — API key (highest sensitivity, most isolated)
2. `workspace.toml` — global settings (report dir, context dir, global persona, model, entity list)
3. Per-entity `.toml` — entity-specific settings (persona override, bank accounts, last import dir)
4. In-memory state — session-only state (conversation history, chat panel visibility, focus)

Writes flow back to the appropriate level: `/persona` updates entity toml or workspace toml, new bank formats update entity toml, `last_import_dir` updates entity toml. Secrets and workspace entity list are never written by the application.

---

## Module Tree

### New Modules

```
src/
├── ai/
│   ├── mod.rs              Module declarations and shared types
│   │                        (AiError, AiResponse, ToolCall, AiRequestState)
│   ├── client.rs           Claude API client
│   │                        - build_request() — construct API payload
│   │                        - send_message() — make ureq POST, parse response
│   │                        - send_with_tool_loop() — orchestrate multi-round tool use
│   │                        - build_system_prompt() — construct persona + context prompt
│   │                        - parse_summary() — extract SUMMARY line from response
│   ├── tools.rs            Tool definitions and fulfillment
│   │                        - tool_definitions() — return Vec<ToolDefinition> for API
│   │                        - fulfill_tool_call() — dispatch to repo methods
│   │                        - serialize_result() — format repo results as JSON strings
│   ├── context.rs          Entity context file management
│   │                        - read_context() — read .md file, auto-create if missing
│   │                        - context_file_path() — derive path from entity name
│   │                        - slugify_entity_name() — name to filename conversion
│   └── csv_import.rs       CSV import pipeline
│                            - parse_csv() — read and normalize using bank config
│                            - detect_format() — Claude-assisted column detection for new banks
│                            - run_pass1() — local matching against import_mappings
│                            - run_pass2_batch() — prepare AI matching batch request
│                            - check_duplicates() — hash comparison against recent imports
│                            - create_import_drafts() — batch draft JE creation
├── widgets/
│   ├── chat_panel.rs       Chat panel widget (NEW)
│   │                        - ChatPanel struct and state management
│   │                        - render() — draw panel with messages, input, border
│   │                        - handle_key() — input handling, slash command parsing
│   │                        - add_message() — append to history
│   │                        - tick() — advance typewriter animation
│   │                        - build_welcome_message() — initial panel content
│   └── ... (existing widgets unchanged)
├── db/
│   ├── import_mapping_repo.rs  Import mapping CRUD (NEW)
│   │                        - find_exact_match() — exact description lookup
│   │                        - find_substring_match() — substring pattern lookup
│   │                        - create_mapping() — insert new mapping
│   │                        - update_mapping() — change account for existing pattern
│   │                        - record_use() — update last_used_at and use_count
│   │                        - list_mappings() — all mappings for a bank (future UI)
│   └── ... (existing repos unchanged except minor additions)
└── ... (existing modules)
```

### Modified Existing Files

| File | Changes |
|------|---------|
| `src/app.rs` | Add `ChatPanel`, `AiRequestState`, `FocusTarget` fields to `App`. Modify event dispatch for focus model. Add Ctrl+K handler. Add AI request orchestration methods. Modify render to support split layout. Update help overlay. |
| `src/config.rs` | Add `WorkspaceAiConfig`, `EntityTomlConfig`, `BankAccountConfig` structs and parsing. Add secrets file loading. Add entity toml read/write. Add tilde expansion for new paths. |
| `src/db/mod.rs` | Add `import_mappings()` accessor to `EntityDb`. Add `import_ref` migration in `open()`. Add `import_mappings` table creation in schema init. |
| `src/db/schema.rs` | Add `CREATE TABLE import_mappings` statement. |
| `src/db/journal_repo.rs` | Add `import_ref` to insert/select queries. Add query for incomplete imports (Shift+U). Add query for duplicate detection. |
| `src/db/audit_repo.rs` | No schema changes — new action type strings are handled by existing `action TEXT` column. May add convenience methods for querying AI-specific entries. |
| `src/tabs/journal_entries.rs` | Add `U` hotkey (import flow), `Shift+U` hotkey (batch re-match), `e` hotkey (edit draft — V2 prerequisite). Add import flow state management. |
| `src/tabs/envelopes.rs` | Replace Tab with `v` for view toggle. |
| `src/widgets/je_form.rs` | Add arrow key + Enter field navigation as alternative to Tab. Add `from_existing()` constructor for edit mode. |
| `src/widgets/status_bar.rs` | Add AI state rendering ("Calling Accountant ☏", "Checking the books 🕮"). |
| `src/lib.rs` | Add `mod ai;` declaration. |
| `src/types/enums.rs` | Add new `AuditAction` variants, new enums (`ImportMatchType`, `ImportMatchSource`). |

---

## Key Structs and Signatures

### AiClient

The API client. Stateless — all state (messages, system prompt) is passed in per call.

```rust
// src/ai/client.rs

pub struct AiClient {
    api_key: String,
    model: String,
    timeout: Duration,  // 10 seconds
}

impl AiClient {
    /// Create a new client. Called once on first AI interaction.
    pub fn new(api_key: String, model: String) -> Self;

    /// Send a single message exchange (no tool loop).
    /// Used for /compact and bank format detection.
    pub fn send_simple(
        &self,
        system: &str,
        messages: &[ApiMessage],
    ) -> Result<String, AiError>;

    /// Send a message with tool use support. Loops up to max_depth rounds.
    /// Returns the final text response and a list of tool calls made.
    pub fn send_with_tools(
        &self,
        system: &str,
        messages: &[ApiMessage],
        tools: &[ToolDefinition],
        db: &EntityDb,
        max_depth: usize,           // typically 5
        on_stage_change: &mut dyn FnMut(AiRequestState),  // callback for UI state updates
    ) -> Result<AiResponse, AiError>;

    /// Build the system prompt from config and context.
    pub fn build_system_prompt(
        persona: &str,
        entity_name: &str,
        context_contents: &str,
    ) -> String;
}

/// Message format for the API conversation history.
pub struct ApiMessage {
    pub role: ApiRole,
    pub content: ApiContent,
}

pub enum ApiRole { User, Assistant }

pub enum ApiContent {
    Text(String),
    ToolUse(Vec<ToolCall>),
    ToolResult(Vec<ToolResult>),
}

pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,  // JSON-serialized result
}
```

**Design note on `on_stage_change`:** This callback lets `App` update `AiRequestState` and force a render between stages of the tool loop, even though the overall call is blocking. The pattern:
1. App sets CallingApi, force renders, calls `send_with_tools`
2. Inside the tool loop, `send_with_tools` calls `on_stage_change(FulfillingTools)` between rounds
3. App's callback updates state and calls `terminal.draw()` inside the callback
4. Loop continues with the next blocking ureq call

This is not true concurrency — it's cooperative yielding at well-defined points within a blocking call.

### Tool Fulfillment

```rust
// src/ai/tools.rs

/// Returns all tool definitions for the Claude API request.
pub fn tool_definitions() -> Vec<ToolDefinition>;

/// Dispatch a tool call to the appropriate repo method.
/// Returns a JSON string of the result.
pub fn fulfill_tool_call(
    tool_call: &ToolCall,
    db: &EntityDb,
) -> Result<String, AiError>;

/// Individual tool handlers (private, called by fulfill_tool_call):
fn handle_get_account(params: &serde_json::Value, db: &EntityDb) -> Result<String, AiError>;
fn handle_get_account_children(params: &serde_json::Value, db: &EntityDb) -> Result<String, AiError>;
fn handle_search_accounts(params: &serde_json::Value, db: &EntityDb) -> Result<String, AiError>;
fn handle_get_gl_transactions(params: &serde_json::Value, db: &EntityDb) -> Result<String, AiError>;
fn handle_get_journal_entry(params: &serde_json::Value, db: &EntityDb) -> Result<String, AiError>;
fn handle_get_open_ar_items(params: &serde_json::Value, db: &EntityDb) -> Result<String, AiError>;
fn handle_get_open_ap_items(params: &serde_json::Value, db: &EntityDb) -> Result<String, AiError>;
fn handle_get_envelope_balances(params: &serde_json::Value, db: &EntityDb) -> Result<String, AiError>;
fn handle_get_trial_balance(params: &serde_json::Value, db: &EntityDb) -> Result<String, AiError>;
fn handle_get_audit_log(params: &serde_json::Value, db: &EntityDb) -> Result<String, AiError>;
```

### ChatPanel

```rust
// src/widgets/chat_panel.rs

pub struct ChatPanel {
    messages: Vec<ChatMessage>,
    input_buffer: String,
    cursor_pos: usize,
    scroll_offset: usize,
    system_prompt: String,
    is_visible: bool,
    typewriter: Option<TypewriterState>,
    entity_name: String,        // for display header
    current_persona: String,    // for display in welcome message
}

impl ChatPanel {
    pub fn new(entity_name: &str, persona: &str) -> Self;

    /// Process a key event. Returns a ChatAction for App to handle.
    pub fn handle_key(&mut self, key: KeyEvent) -> ChatAction;

    /// Advance typewriter animation. Called on each 500ms tick.
    pub fn tick(&mut self);

    /// Render the panel into the given area.
    pub fn render(&self, frame: &mut Frame, area: Rect);

    /// Add a user message and return the full API message history.
    pub fn submit_input(&mut self) -> Option<Vec<ApiMessage>>;

    /// Add an assistant response with typewriter animation.
    pub fn add_response(&mut self, content: String);

    /// Add a system notification (e.g., "[Context refreshed]").
    pub fn add_system_note(&mut self, note: &str);

    /// Replace all messages with a compacted summary.
    pub fn replace_with_summary(&mut self, summary: String, original_count: usize);

    /// Build the welcome message for first open.
    pub fn build_welcome(&mut self);

    /// Toggle visibility. Returns new visibility state.
    pub fn toggle_visible(&mut self) -> bool;

    /// Get the current API message history (for building requests).
    pub fn api_messages(&self) -> Vec<ApiMessage>;

    /// Rebuild system prompt from fresh config/context.
    pub fn rebuild_system_prompt(&mut self, persona: &str, entity_name: &str, context: &str);

    pub fn is_visible(&self) -> bool;
}

/// Actions the chat panel requests from App.
pub enum ChatAction {
    None,
    SendMessage(Vec<ApiMessage>),   // Send to Claude API
    SlashCommand(SlashCommand),      // Execute a slash command
    Close,                           // Close the panel (Escape/Ctrl+K)
    SkipTypewriter,                  // Enter pressed during animation
}
```

**Design note:** `ChatPanel` does NOT own the `AiClient` or make API calls. It prepares messages and returns `ChatAction::SendMessage` — `App` handles the actual API call and feeds the response back via `add_response()`. This keeps the widget pure (no I/O) and lets App manage the loading state.

### ImportMappingRepo

```rust
// src/db/import_mapping_repo.rs

pub struct ImportMappingRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> ImportMappingRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self;

    /// Find an exact description match for the given bank.
    pub fn find_exact_match(
        &self,
        bank_name: &str,
        description: &str,
    ) -> Result<Option<AccountId>>;

    /// Find a substring match, returning the longest (most specific) pattern.
    pub fn find_substring_match(
        &self,
        bank_name: &str,
        description: &str,
    ) -> Result<Option<AccountId>>;

    /// Create a new mapping. Returns error if pattern+bank already exists.
    pub fn create(
        &self,
        description_pattern: &str,
        account_id: AccountId,
        match_type: ImportMatchType,
        source: ImportMatchSource,
        bank_name: &str,
    ) -> Result<i64>;

    /// Update the account for an existing mapping (when user corrects a mapping).
    pub fn update_account(
        &self,
        id: i64,
        account_id: AccountId,
        source: ImportMatchSource,
    ) -> Result<()>;

    /// Record a successful use of a mapping (updates last_used_at and use_count).
    pub fn record_use(&self, id: i64) -> Result<()>;

    /// List all mappings for a bank (for future management UI).
    pub fn list_by_bank(&self, bank_name: &str) -> Result<Vec<ImportMapping>>;
}

/// Row struct for import_mappings table.
pub struct ImportMapping {
    pub id: i64,
    pub description_pattern: String,
    pub account_id: AccountId,
    pub match_type: ImportMatchType,
    pub source: ImportMatchSource,
    pub bank_name: String,
    pub created_at: NaiveDateTime,
    pub last_used_at: NaiveDateTime,
    pub use_count: i64,
}
```

### EntityDb Extensions

```rust
// src/db/mod.rs — additions to existing EntityDb impl

impl EntityDb {
    // New accessor (follows existing pattern: accounts(), journals(), etc.)
    pub fn import_mappings(&self) -> ImportMappingRepo<'_>;
}
```

### Config Extensions

```rust
// src/config.rs — new structs

/// The [ai] section of workspace.toml
pub struct WorkspaceAiConfig {
    pub persona: String,    // Default: "Professional Tax Accountant"
    pub model: String,      // Default: "claude-sonnet-4-20250514"
}

/// Contents of a per-entity .toml file
pub struct EntityTomlConfig {
    pub ai_persona: Option<String>,
    pub last_import_dir: Option<String>,
    pub bank_accounts: Vec<BankAccountConfig>,
}

/// A single [[bank_accounts]] entry in entity toml
pub struct BankAccountConfig {
    pub name: String,
    pub linked_account: String,
    pub date_column: String,
    pub description_column: String,
    pub amount_column: Option<String>,
    pub debit_column: Option<String>,
    pub credit_column: Option<String>,
    pub debit_is_negative: bool,    // Default: true
    pub date_format: String,
}

/// Secrets configuration
pub struct SecretsConfig {
    pub anthropic_api_key: String,
}

// New functions
pub fn load_secrets() -> Result<SecretsConfig>;
pub fn load_entity_toml(config_path: &str, workspace_dir: &Path) -> Result<EntityTomlConfig>;
pub fn save_entity_toml(config_path: &str, workspace_dir: &Path, config: &EntityTomlConfig) -> Result<()>;
pub fn secrets_file_path() -> PathBuf;  // ~/.config/bookkeeper/secrets.toml
```

### App Extensions

```rust
// src/app.rs — new fields on App struct

pub struct App {
    // ... existing fields unchanged ...

    // New AI fields
    ai_client: Option<AiClient>,          // Lazily initialized on first AI use
    ai_state: AiRequestState,             // Current API call state
    chat_panel: ChatPanel,                // Chat panel widget
    focus: FocusTarget,                   // MainTab or ChatPanel
    entity_toml: Option<EntityTomlConfig>, // Loaded entity config
    import_flow: Option<ImportFlowState>,  // Active import wizard state (None when not importing)
}
```

---

## Data Flows

### Flow 1: User asks a question via Ctrl+K

```
User presses Ctrl+K
    │
    ▼
App.handle_key()
    ├── If panel not visible: open panel, set focus to ChatPanel
    ├── If panel visible + focused: close panel
    └── If panel visible + unfocused: set focus to ChatPanel
    │
    ▼
User types question, presses Enter
    │
    ▼
ChatPanel.handle_key(Enter)
    ├── If typewriter active: return ChatAction::SkipTypewriter
    └── If input non-empty: return ChatAction::SendMessage(api_messages)
    │
    ▼
App receives ChatAction::SendMessage
    │
    ▼
App.handle_ai_request()
    ├── 1. Lazy-init AiClient (load secrets, load model from config)
    ├── 2. Log AiPrompt to audit_log
    ├── 3. Set ai_state = CallingApi
    ├── 4. Force render (terminal.draw()) → user sees "Calling Accountant ☏"
    ├── 5. Call ai_client.send_with_tools(system_prompt, messages, tools, db, 5, callback)
    │       │
    │       ├── ureq POST to Claude API (blocks up to 10s)
    │       ├── If tool_use response:
    │       │     ├── Callback: set ai_state = FulfillingTools, force render
    │       │     │   → user sees "Checking the books 🕮"
    │       │     ├── fulfill_tool_call() for each tool (instant, local SQLite)
    │       │     ├── Log AiToolUse to audit_log for each tool
    │       │     ├── ureq POST with tool results (blocks up to 10s)
    │       │     └── Repeat if another tool_use (up to 5 rounds)
    │       └── Return AiResponse::Text or AiError
    │
    ├── 6a. On success:
    │     ├── Parse SUMMARY line from response
    │     ├── Log AiResponse to audit_log (summary only)
    │     ├── Strip SUMMARY from display text
    │     ├── ChatPanel.add_response(display_text) → starts typewriter
    │     └── Set ai_state = Idle
    │
    └── 6b. On error:
          ├── Set ai_state = Idle
          └── App.set_error("The Call Dropped ☹")
```

### Flow 2: CSV import via U hotkey

```
User presses U in JE tab (list view, not in form)
    │
    ▼
JournalEntriesTab.handle_key('U')
    ├── Return TabAction::StartImport
    │
    ▼
App receives TabAction::StartImport
    ├── Create ImportFlowState at step FilePathInput
    ├── Pre-fill path from entity_toml.last_import_dir
    │
    ▼
App renders import modal overlay (steps 1-3 are modal popups)
    │
    ▼
[FilePathInput] → user enters path → validate file exists
    ├── Update entity_toml.last_import_dir, save entity toml
    │
    ▼
[BankSelection] → user selects bank or "New"
    ├── If known bank → skip to DuplicateCheck
    │
    ▼
[NewBankName] → user types name
    │
    ▼
[NewBankDetection] → render "Initializing ↻" in modal
    ├── Read first 4 lines of CSV
    ├── Lazy-init AiClient
    ├── ai_client.send_simple() with column detection prompt
    ├── On success → parse into BankAccountConfig → NewBankConfirmation
    └── On failure → render "Failed ⨂" in modal, user presses Esc to cancel
    │
    ▼
[NewBankConfirmation] → display detected columns → user confirms Y/N
    │
    ▼
[NewBankAccountPicker] → AccountPicker widget → user selects linked account
    ├── Save new BankAccountConfig to entity toml
    │
    ▼
[DuplicateCheck]
    ├── parse_csv() → Vec<NormalizedTransaction>
    ├── check_duplicates() against last 90 days of import_refs
    ├── If duplicates found → DuplicateWarning popup
    │     └── User: skip dupes or include all
    └── If no duplicates → proceed directly
    │
    ▼
[Pass1Matching] → render "Importing ☺ N/M" in modal
    ├── run_pass1() → query import_mappings for each transaction
    ├── Split into matched + unmatched
    ├── If all matched → ReviewScreen
    │
    ▼
[Pass2AiMatching] → if API key available and unmatched exist
    ├── Auto-open chat panel if not visible
    ├── Batch unmatched (max 25 per batch)
    ├── For each batch:
    │     ├── send_with_tools() with matching prompt
    │     ├── Parse matches from response
    │     ├── Display progress in chat: "Matching... N/M"
    │     └── Categorize by confidence
    ├── On API failure at any point:
    │     └── Remaining unmatched → MatchSource::Unmatched (one-sided drafts)
    │
    ▼
[Pass3Clarification] → if low-confidence items exist
    ├── For each low-confidence item:
    │     ├── Display in chat panel with Claude's reasoning
    │     ├── User confirms, redirects, or skips
    │     └── Confirmed → write to import_mappings + update match
    │
    ▼
[ReviewScreen] → scrollable list grouped by match source
    ├── Top: auto-matched (dimmed)
    ├── Middle: AI-matched (with reasoning, selectable)
    ├── Bottom: user-confirmed + unmatched
    ├── Detail pane at bottom (JE preview on highlight)
    ├── 'r' to reject individual AI match
    ├── Enter to approve and create drafts
    ├── Escape to cancel entire import
    │
    ▼
[Creating] → batch create draft JEs
    ├── For each approved match:
    │     ├── Apply debit/credit mapping rules
    │     ├── create_draft() with import_ref
    │     ├── Set memo: "Import: {description}"
    │     └── Write AI-suggested mappings to import_mappings table
    ├── Return TabAction::RefreshData
    │
    ▼
[Complete]
    ├── Status bar: "Created N draft JEs from {bank_name} import"
    ├── Log CsvImport to audit_log
    └── Return to JE list view
```

### Flow 3: Slash command processing

```
User types /command in chat panel, presses Enter
    │
    ▼
ChatPanel.handle_key(Enter)
    ├── Parse input as SlashCommand
    ├── Return ChatAction::SlashCommand(cmd)
    │
    ▼
App receives ChatAction::SlashCommand
    │
    ├── /clear:
    │     ├── ChatPanel.messages.clear()
    │     ├── Rebuild system prompt from config + context file
    │     ├── ChatPanel.build_welcome()
    │     └── ChatPanel.add_system_note("[Conversation cleared]")
    │
    ├── /context:
    │     ├── Re-read entity context file
    │     ├── ChatPanel.rebuild_system_prompt()
    │     ├── Get current tab name from active tab
    │     └── ChatPanel.add_system_note("[Context refreshed from {tab} tab]")
    │
    ├── /compact:
    │     ├── If < 5 messages: add_system_note("Not enough conversation to compact")
    │     ├── Set ai_state = CallingApi, force render
    │     ├── ai_client.send_simple() with compaction prompt
    │     ├── On success: ChatPanel.replace_with_summary()
    │     └── On failure: set_error("The Call Dropped ☹")
    │
    ├── /persona (no args):
    │     └── ChatPanel.add_system_note("Current persona: {persona}")
    │
    ├── /persona {text}:
    │     ├── Update entity_toml.ai_persona (or workspace ai.persona if no entity config)
    │     ├── Save entity toml
    │     ├── Rebuild system prompt
    │     └── ChatPanel.add_system_note("[Persona updated]")
    │
    ├── /match:
    │     ├── Check if selected JE is a draft with import_ref and blank side
    │     ├── If not: add_system_note("Select an incomplete import draft first")
    │     ├── If yes: send single-match request via send_with_tools()
    │     ├── Display suggestion in chat
    │     └── On confirm: update draft, write mapping
    │
    └── Unknown:
          └── ChatPanel.add_system_note("Unknown command. Available: /clear, /context, /compact, /persona, /match")
```

### Flow 4: Focus and key dispatch

```
KeyEvent arrives in App.handle_key()
    │
    ▼
1. Is there an active modal? (confirmation, import wizard, help overlay, fiscal)
    ├── YES → modal.handle_key() → return (modal has priority)
    │
    ▼
2. Is chat panel visible AND focused?
    ├── YES →
    │     ├── Tab key? → Set focus = MainTab → return
    │     ├── Escape key? → Close chat panel, set focus = MainTab → return
    │     ├── Ctrl+K? → Close chat panel, set focus = MainTab → return
    │     └── Any other key → ChatPanel.handle_key() → process ChatAction → return
    │
    ▼
3. Is chat panel visible AND unfocused (focus = MainTab)?
    ├── Tab key? → Set focus = ChatPanel → return
    ├── Ctrl+K? → Set focus = ChatPanel → return
    │   (panel stays open, just moves focus to it)
    └── Continue to step 4 (normal global hotkey dispatch)
    │
    ▼
4. Is inter-entity mode active?
    ├── YES → inter_entity.handle_key() → return
    │
    ▼
5. Global hotkeys (normal dispatch):
    ├── Ctrl+K → Open chat panel, set focus = ChatPanel
    ├── Ctrl+Left/Right → Cycle tabs
    ├── 1-9 → Switch to tab
    ├── n → New journal entry
    ├── i → Inter-entity mode
    ├── / → Search in current tab
    ├── ? → Help overlay
    ├── f → Fiscal modal
    ├── q → Quit
    │
    ▼
6. Active tab handles remaining keys:
    └── entity.tabs[active_tab].handle_key()
```

---

## Error Handling Strategy

### AI-Specific Errors

All AI errors are represented by `AiError` and handled uniformly:

| Error | Internal Action | User-Facing Display |
|-------|----------------|---------------------|
| `AiError::Timeout` | `tracing::warn!`, set_error | "The Call Dropped ☹" (red, 5s) |
| `AiError::ApiError` | `tracing::warn!` with status + body, set_error | "The Call Dropped ☹" (red, 5s) |
| `AiError::ParseError` | `tracing::error!` with details, set_error | "The Call Dropped ☹" (red, 5s) |
| `AiError::NoApiKey` | set_error | "No API key found — see ~/.config/bookkeeper/secrets.toml" (red, 5s) |
| `AiError::MaxToolDepth` | `tracing::warn!`, return partial response | Partial response displayed + system note in chat |

**Design note:** All AI errors show the same friendly message ("The Call Dropped ☹") rather than technical details. The specific error is logged via `tracing` for debugging. The exception is `NoApiKey` which has an actionable message telling the user where to put the key.

### Import-Specific Errors

| Error | Handling |
|-------|----------|
| File not found | Status bar error, stay in FilePathInput step |
| CSV parse error (malformed row) | Skip row, log warning, continue. Show count of skipped rows in summary. |
| Bank format detection timeout | Show "Failed ⨂" in modal. User can Escape to cancel or retry. |
| Pass 2 API failure | Fall back to unmatched for remaining items. Continue to ReviewScreen. |
| Draft creation failure | Wrap in transaction. Roll back entire batch on any failure. Show error. |

### Config Errors

| Error | Handling |
|-------|----------|
| Entity toml not found | AI features use global defaults. No error shown. |
| Entity toml parse error | `tracing::error!`, show status bar error with filename. AI features use global defaults. |
| Entity toml write failure | `tracing::error!`, show status bar error. Change is lost but app continues. |
| Secrets file not found | Error shown on first AI interaction, not at startup. |

---

## Render Layout

### Normal (no chat panel)

```
┌─────────────────────────────────────────────────────────┐
│ [CoA] [GL] [JE] [AR] [AP] [Env] [FA] [Rep] [Aud]     │ ← tab bar
├─────────────────────────────────────────────────────────┤
│                                                         │
│                  Active Tab Content                      │ ← 100% width
│                                                         │
├─────────────────────────────────────────────────────────┤
│ Acme Land LLC │ FY2026-P03 │ [status message]          │ ← status bar
└─────────────────────────────────────────────────────────┘
```

### With chat panel open

```
┌──────────────────────────────────┬──────────────────────┐
│ [CoA] [GL] [JE] [AR] [AP] ...   │ AI Accountant        │ ← tab bar + panel header
├──────────────────────────────────┤ Acme Land LLC        │
│                                  │──────────────────────│
│                                  │ [Assistant] Your Q1  │
│    Active Tab Content            │ insurance expenses... │ ← 70% / 30% split
│    (compressed but functional)   │                      │
│                                  │ [System] Context     │
│                                  │ refreshed from GL    │
│                                  │──────────────────────│
│                                  │ > _                  │ ← input line
├──────────────────────────────────┴──────────────────────┤
│ Acme Land LLC │ FY2026-P03 │ Calling Accountant ☏      │ ← status bar (full width)
└─────────────────────────────────────────────────────────┘
```

**Layout rules:**
- Tab bar spans full width (tabs may abbreviate more aggressively at 70% width)
- Status bar spans full width (always)
- Active tab content gets 70% of terminal width
- Chat panel gets 30% of terminal width
- Chat panel border is highlighted (bright) when focused, dim when unfocused
- Active tab area border is highlighted when focused, dim when panel is focused
- The 70/30 split is calculated from the terminal width minus borders

### With import wizard modal

```
┌──────────────────────────────────────────────────────────┐
│ [CoA] [GL] [JE] [AR] [AP] [Env] [FA] [Rep] [Aud]      │
├──────────────────────────────────────────────────────────┤
│                                                          │
│          ┌────────────────────────────┐                  │
│          │  Import CSV Statement      │                  │
│          │                            │                  │
│          │  File: ~/Downloads/sofi_   │                  │ ← centered modal overlay
│          │        march_2026.csv      │                  │
│          │                            │                  │
│          │  [Enter] Confirm  [Esc]    │                  │
│          │         Cancel             │                  │
│          └────────────────────────────┘                  │
│                                                          │
├──────────────────────────────────────────────────────────┤
│ Acme Land LLC │ FY2026-P03                               │
└──────────────────────────────────────────────────────────┘
```

Import wizard steps 1-3 (FilePathInput through NewBankAccountPicker) render as centered modal overlays. Steps 4+ transition to the review screen which takes over the full tab area.

---

## Dependencies

### New Crate Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `ureq` | latest stable | Synchronous HTTP client for Claude API |
| `serde_json` | 1.x | JSON serialization for API payloads and tool results |
| `csv` | 1.x | CSV file parsing |

**Note:** `serde` and `toml` are already dependencies. `serde_json` may already be a transitive dependency but should be listed explicitly.

### No New Dependencies For

| Concern | Approach |
|---------|----------|
| Loading animation | Custom — the status bar messages are static text, not animated spinners |
| Typewriter effect | Custom — driven by the existing 500ms tick cycle |
| Markdown parsing | Not needed — context file is read as raw text and passed to Claude as-is |
| TOML writing | `toml::to_string_pretty()` — already available via the `toml` crate |
| File path completion | Not implemented — raw text input with tilde expansion |

---

## Testing Strategy

### Unit Tests (per module)

| Module | Test Focus |
|--------|-----------|
| `ai/client.rs` | Mock HTTP responses: text, tool_use, multi-round, timeout, error. SUMMARY parsing. System prompt construction. |
| `ai/tools.rs` | Each tool handler with a test database. Parameter validation. Unknown tool handling. Result serialization format. |
| `ai/context.rs` | File creation, slugification, read existing, read auto-created. |
| `ai/csv_import.rs` | Parse single-amount format. Parse split-column format. Date format variations. Normalization sign conventions. Duplicate detection. Pass 1 matching (exact, substring, no match). |
| `db/import_mapping_repo.rs` | CRUD operations. Unique constraint. Exact vs substring matching priority. Use count tracking. |
| `widgets/chat_panel.rs` | Slash command parsing. Message management. Typewriter state transitions. API message history construction. |
| `config.rs` (extensions) | Parse workspace.toml with and without [ai] section. Parse entity toml. Parse bank accounts with both formats. Backwards compatibility. Secrets loading. |

### Integration Tests

| Test | Scope |
|------|-------|
| Full AI Q&A cycle | Mock API → tool calls → response → audit log entries |
| Full CSV import cycle | Parse CSV → Pass 1 match → create drafts → verify import_ref |
| Duplicate detection | Import same CSV twice → second import detects duplicates |
| Re-match flow | Create incomplete drafts → Shift+U → mock API → verify draft update |
| Focus model | Ctrl+K open → Tab switch → hotkey reaches correct handler |

### Test Helpers

- Mock `AiClient` that returns pre-defined responses (avoid real API calls in tests)
- Test database factory that creates an in-memory SQLite with schema + seed data + import_mappings
- Test CSV file generator for consistent test fixtures
