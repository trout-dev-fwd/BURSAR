# V2 Type System & State Machines — AI Accounting Assistant

This document defines all new types, enums, state machines, and algorithms introduced in V2 Phase 1. It supplements the existing V1 type system — all V1 types remain unchanged.

---

## New Enums

### AuditAction (extend existing)

The existing `AuditAction` enum gains five new variants. Stored as `TEXT` in the `audit_log.action` column.

```rust
// Existing variants unchanged: JeCreated, JePosted, JeReversed, AccountCreated,
// AccountUpdated, AccountDeactivated, AccountReactivated, AccountDeleted,
// PeriodClosed, PeriodReopened, YearEndClose, EnvelopeAllocChanged,
// EnvelopeTransfer, InterEntityPost, AssetPlacedInService, RecurringGenerated

// New variants:
AiPrompt,         // User sent a question to Claude
AiResponse,       // Claude returned a response (description = single-line summary)
AiToolUse,        // Claude used a tool (description = "Used {tool}({params})")
CsvImport,        // CSV import completed (description = summary stats)
MappingLearned,   // New import mapping created (description = pattern → account)
```

`FromStr` / `Display` implementations follow the existing pattern (exact string match, e.g., `"AiPrompt"`).

### ImportMatchType

Stored as `TEXT` in the `import_mappings.match_type` column.

```rust
enum ImportMatchType {
    Exact,      // Full string equality match
    Substring,  // Pattern is contained within the bank description
}
```

**Display:** `"exact"`, `"substring"` (lowercase to match CHECK constraint in schema).

### ImportMatchSource

Stored as `TEXT` in the `import_mappings.source` column.

```rust
enum ImportMatchSource {
    Confirmed,    // User explicitly confirmed this mapping
    AiSuggested,  // Claude suggested this mapping, user didn't reject
}
```

**Display:** `"confirmed"`, `"ai_suggested"` (lowercase with underscore to match CHECK constraint).

### AiRequestState

In-memory only. Tracks the current state of an AI API interaction for UI display.

```rust
enum AiRequestState {
    Idle,              // No active AI request
    CallingApi,        // First API call in progress — "Calling Accountant ☏"
    FulfillingTools,   // Tool results gathered, follow-up API call in progress — "Checking the books 🕮"
}
```

Not stored in the database. Lives as a field on `App`.

### AiResponse

In-memory only. Represents the parsed result of a Claude API response.

```rust
enum AiResponse {
    Text {
        content: String,      // The full display text
        summary: String,      // Single-line summary for audit logging
    },
    ToolUse(Vec<ToolCall>),   // One or more tool invocations requested
}
```

### AiError

In-memory only. Represents AI API failures.

```rust
enum AiError {
    Timeout,                          // Request exceeded 10-second timeout
    ApiError { status: u16, body: String },  // HTTP 4xx/5xx response
    ParseError(String),               // Response couldn't be deserialized
    NoApiKey,                         // No API key configured
    MaxToolDepth,                     // Exceeded 5 tool use rounds
}
```

All variants surface to the user as "The Call Dropped ☹" via `set_error`. The specific error is logged via `tracing::warn!` for debugging.

### ChatRole

In-memory only. Identifies the sender of a chat message.

```rust
enum ChatRole {
    User,       // User's typed input
    Assistant,  // Claude's response
    System,     // System notifications ([Context refreshed], [Conversation cleared], etc.)
}
```

### SlashCommand

In-memory only. Parsed from user input in the chat panel.

```rust
enum SlashCommand {
    Clear,                    // /clear — reset conversation
    Context,                  // /context — refresh system prompt
    Compact,                  // /compact — summarize conversation
    Persona(Option<String>),  // /persona [text] — view or update persona
    Match,                    // /match — re-match selected draft
    Unknown(String),          // Unrecognized command name
}
```

**Parsing algorithm:**
1. If input does not start with `/`, it is not a command — return `None`
2. Split on first space: command name = part before space, argument = part after space (or empty)
3. Match command name (case-insensitive):
   - `"/clear"` → `Clear`
   - `"/context"` → `Context`
   - `"/compact"` → `Compact`
   - `"/persona"` → `Persona(Some(arg))` if arg non-empty, `Persona(None)` if empty
   - `"/match"` → `Match`
   - anything else → `Unknown(name)`
4. Trailing whitespace in arguments is trimmed

### FocusTarget

In-memory only. Tracks which UI element has keyboard focus when the chat panel is open.

```rust
enum FocusTarget {
    MainTab,    // Active tab receives keyboard input
    ChatPanel,  // Chat panel receives keyboard input
}
```

Lives as a field on `App`, only relevant when the chat panel is visible. When the panel is not visible, focus is implicitly `MainTab` and this field is ignored.

### ImportFlowStep

In-memory only. Tracks the current step in the CSV import wizard modal.

```rust
enum ImportFlowStep {
    FilePathInput,              // User typing a file path
    BankSelection,              // User choosing from known banks or "New"
    NewBankName,                // User typing a new bank name
    NewBankDetection,           // Claude analyzing CSV headers (loading state)
    NewBankConfirmation,        // User confirming detected column mapping
    NewBankAccountPicker,       // User selecting linked CoA account
    DuplicateWarning,           // Showing duplicate detection results
    Pass1Matching,              // Local matching in progress ("Importing ☺")
    Pass2AiMatching,            // AI matching batches in progress
    Pass3Clarification,         // Individual ambiguous items in chat panel
    ReviewScreen,               // User reviewing all matches before approval
    Creating,                   // Draft JEs being created
    Complete,                   // Import finished, summary displayed
    Failed(String),             // Import failed with error message
}
```

---

## New Structs

### ToolCall

Represents a single tool invocation requested by Claude.

```rust
struct ToolCall {
    id: String,           // Tool use ID from the API (needed for tool_result response)
    name: String,         // Tool name (e.g., "get_account")
    input: serde_json::Value,  // Tool parameters as JSON
}
```

### ToolDefinition

Describes a tool for the Claude API request.

```rust
struct ToolDefinition {
    name: String,
    description: String,
    input_schema: serde_json::Value,  // JSON Schema for parameters
}
```

### ChatMessage

A single message in the chat panel conversation history.

```rust
struct ChatMessage {
    role: ChatRole,
    content: String,
    is_fully_rendered: bool,  // False while typewriter animation is active
}
```

### TypewriterState

Tracks the animation state for progressively revealing an AI response.

```rust
struct TypewriterState {
    full_text: String,
    display_position: usize,  // Characters revealed so far (byte offset, aligned to char boundary)
    message_index: usize,     // Index into ChatPanel.messages for the message being animated
}
```

### ChatPanel

Full state of the AI chat panel widget.

```rust
struct ChatPanel {
    messages: Vec<ChatMessage>,
    input_buffer: String,
    cursor_pos: usize,
    scroll_offset: usize,
    system_prompt: String,
    is_visible: bool,
    typewriter: Option<TypewriterState>,
}
```

Note: `is_focused` is tracked via `App.focus` as `FocusTarget`, not on the panel itself. This keeps focus management centralized.

### NormalizedTransaction

A bank statement line normalized to a common internal format regardless of bank CSV structure.

```rust
struct NormalizedTransaction {
    date: NaiveDate,
    description: String,
    amount: Money,         // Positive = deposit/credit to bank, negative = withdrawal/debit from bank
    import_ref: String,    // Composite: "{bank_name}|{date}|{description}|{amount}"
    raw_row: String,       // Original CSV line for debugging
}
```

### ImportMatch

A proposed mapping of a transaction to an account, with metadata.

```rust
struct ImportMatch {
    transaction: NormalizedTransaction,
    matched_account_id: Option<AccountId>,  // None if unmatched
    matched_account_display: Option<String>, // "5100 - Insurance" for display
    match_source: MatchSource,
    confidence: Option<MatchConfidence>,     // Only for AI matches
    reasoning: Option<String>,              // Claude's explanation (AI matches only)
    rejected: bool,                         // User rejected this match in review
}

enum MatchSource {
    Local,          // Pass 1 — matched via import_mappings table
    Ai,             // Pass 2 — matched by Claude
    UserConfirmed,  // Pass 3 — user manually confirmed
    Unmatched,      // No match found or AI unavailable
}

enum MatchConfidence {
    High,
    Medium,
    Low,
}
```

### BankAccountConfig

Deserialized from per-entity TOML `[[bank_accounts]]` entries.

```rust
struct BankAccountConfig {
    name: String,
    linked_account: String,           // CoA account number (TEXT)
    date_column: String,
    description_column: String,
    amount_column: Option<String>,     // Single-amount format
    debit_column: Option<String>,      // Split format
    credit_column: Option<String>,     // Split format
    debit_is_negative: bool,           // Default true; for single-amount format
    date_format: String,               // chrono format string
}
```

**Validation:** Either `amount_column` must be `Some` (single-amount format) or both `debit_column` and `credit_column` must be `Some` (split format). Both being `None` or mixing is invalid.

### EntityConfig (extend existing)

The existing entity configuration struct adds new optional fields.

```rust
// Existing fields unchanged: name, db_path

// New fields:
config_path: Option<String>,          // Path to per-entity TOML (relative or absolute)
```

### EntityTomlConfig

New struct for per-entity TOML file contents.

```rust
struct EntityTomlConfig {
    ai_persona: Option<String>,
    last_import_dir: Option<String>,
    bank_accounts: Vec<BankAccountConfig>,
}
```

### WorkspaceAiConfig

New struct for the `[ai]` section of workspace.toml.

```rust
struct WorkspaceAiConfig {
    persona: String,       // Default: "Professional Tax Accountant"
    model: String,         // Default: "claude-sonnet-4-20250514"
}
```

---

## State Machines

### AI Request Lifecycle

```
                    ┌─────────────────────────────────────┐
                    │                                     │
                    ▼                                     │
    ┌───────┐  user sends  ┌────────────┐  tool_use    ┌──────────────────┐
    │ Idle  │──message────▶│ CallingApi  │─response───▶│ FulfillingTools   │
    │       │              │     ☏      │              │       🕮          │
    └───────┘              └────────────┘              └──────────────────┘
        ▲                      │    │                        │    │
        │                      │    │                        │    │
        │         text response│    │timeout/error           │    │timeout/error
        │                      │    │                        │    │
        │                      ▼    ▼                        ▼    ▼
        │              ┌──────────────────┐          ┌──────────────────┐
        │              │ Display response │          │ "The Call        │
        │              │ (typewriter)     │          │  Dropped ☹"     │
        │              └────────┬─────────┘          └────────┬─────────┘
        │                       │                             │
        └───────────────────────┴─────────────────────────────┘
```

**Transitions:**

| From | Event | To | Action |
|------|-------|----|--------|
| Idle | User sends message | CallingApi | Force render, make ureq POST |
| CallingApi | Response is text | Idle | Log AiPrompt + AiResponse to audit, start typewriter |
| CallingApi | Response is tool_use | FulfillingTools | Log AiToolUse to audit, fulfill tools locally, force render, make follow-up ureq POST |
| CallingApi | Timeout or HTTP error | Idle | Log AiPrompt to audit (no AiResponse), set_error "The Call Dropped ☹" |
| FulfillingTools | Response is text | Idle | Log AiResponse to audit, start typewriter |
| FulfillingTools | Response is tool_use | FulfillingTools | Log AiToolUse, fulfill tools, make another follow-up POST (if under depth limit) |
| FulfillingTools | Timeout or HTTP error | Idle | set_error "The Call Dropped ☹" |
| FulfillingTools | Max depth (5 rounds) reached | Idle | Return whatever partial text exists, log warning |

**Tool use depth:** Maximum 5 API round-trips per user message. Each round may include multiple tool calls (Claude can request several tools at once). The depth counter increments per API call, not per tool. After 5 rounds, the loop terminates and returns any text content from the last response, or a fallback message: "I wasn't able to complete my analysis within the lookup limit. Try asking a more specific question."

**Forced render:** Before each blocking `ureq` call, the app calls `terminal.draw()` to update the display with the current loading state. This ensures the user sees "Calling Accountant ☏" or "Checking the books 🕮" before the UI freezes.

### Typewriter Animation

```
    ┌──────────┐  response received  ┌───────────┐  tick (500ms)  ┌───────────┐
    │ Inactive │────────────────────▶│ Animating │───────────────▶│ Animating │
    │          │                     │ pos=0     │                │ pos+=20   │
    └──────────┘                     └───────────┘                └───────────┘
         ▲                                │    │                       │
         │                     Enter key  │    │  pos >= len           │
         │                                ▼    ▼                       │
         │                          ┌──────────────┐                   │
         └──────────────────────────│   Complete   │◀──────────────────┘
                                    │ full text    │   pos >= len
                                    └──────────────┘
```

**Transitions:**

| From | Event | To | Action |
|------|-------|----|--------|
| Inactive | AI response received | Animating (pos=0) | Create TypewriterState, mark message as not fully rendered |
| Animating | 500ms tick | Animating (pos+=20) | Advance display_position by 20 characters (aligned to char boundary) |
| Animating | display_position >= full_text.len() | Complete | Set is_fully_rendered = true, clear TypewriterState |
| Animating | Enter key pressed | Complete | Set display_position to full_text.len(), set is_fully_rendered = true |
| Complete | (automatic) | Inactive | TypewriterState becomes None |

**Character boundary alignment:** When advancing by 20, scan forward to the next char boundary to avoid splitting a multi-byte UTF-8 character. Use `str::is_char_boundary()`.

**Enter key dual purpose:** When typewriter is active, Enter skips animation. When typewriter is inactive (or None), Enter sends the input buffer as a message. Check typewriter state first.

### Chat Panel Visibility & Focus

```
    ┌─────────────────┐   Ctrl+K   ┌─────────────────────────┐
    │ Panel Hidden    │──────────▶│ Panel Visible            │
    │ Focus: MainTab  │           │ Focus: ChatPanel         │
    └─────────────────┘           └─────────────────────────┘
           ▲                         │         │          │
           │              Ctrl+K or  │   Tab   │    Tab   │
           │              Escape     │         ▼          │
           │ (when focused)          │  ┌──────────────┐  │
           │                         │  │ Panel Visible│  │
           └─────────────────────────┘  │ Focus: Main  │──┘
                                        └──────────────┘
                                         │          ▲
                                  Ctrl+K │          │ Escape
                                  (opens │          │ (closes whatever
                                  panel  │          │  has focus — if
                                  focus) │          │  modal open,
                                         ▼          │  closes modal
                                        ┌──────────┐│  first)
                                        │ Panel    ││
                                        │ Visible  │┘
                                        │ Focus:   │
                                        │ ChatPanel│
                                        └──────────┘
```

**Transitions:**

| Current State | Event | New State | Notes |
|--------------|-------|-----------|-------|
| Hidden | Ctrl+K | Visible, Focus: ChatPanel | Panel opens and immediately receives focus |
| Visible, Focus: ChatPanel | Ctrl+K | Hidden | Panel closes |
| Visible, Focus: ChatPanel | Escape | Hidden | Panel closes (unless a modal is open — modal closes first) |
| Visible, Focus: ChatPanel | Tab | Visible, Focus: MainTab | Focus moves to main tab area |
| Visible, Focus: MainTab | Tab | Visible, Focus: ChatPanel | Focus moves to chat panel |
| Visible, Focus: MainTab | Ctrl+K | Visible, Focus: ChatPanel | Focus moves to chat panel (panel stays open) |
| Visible, Focus: MainTab | Escape | Visible, Focus: MainTab | Normal Escape behavior in the tab (close modals, cancel forms, etc.) |

**Tab key rules when panel is visible:**
- Tab is intercepted at the App level BEFORE reaching any tab or widget
- This overrides Tab usage in the JE form (field navigation) and Envelopes tab (view toggle)
- JE form alternative: arrow keys + Enter for field navigation (always available)
- Envelopes tab alternative: `v` key for view toggle (always available, added as part of this work)
- When panel is NOT visible, Tab works normally in all tabs and widgets (no behavior change from V1)

### Import Flow

```
    ┌──────────────┐          ┌────────────────┐         ┌──────────────┐
    │ FilePathInput│─confirm─▶│ BankSelection  │─known──▶│ DuplicateWarn│
    │              │          │                │         │  (if dupes)  │
    └──────────────┘          └────────────────┘         └──────────────┘
                                     │                         │
                                     │new                      │confirm/skip
                                     ▼                         ▼
                              ┌──────────────┐          ┌──────────────┐
                              │ NewBankName  │          │ Pass1Matching│
                              └──────┬───────┘          │  "Importing  │
                                     │                  │     ☺"       │
                                     ▼                  └──────┬───────┘
                              ┌──────────────┐                 │
                              │ NewBank      │                 │ if unmatched
                              │ Detection    │                 │ items exist
                              │"Initializing │                 ▼
                              │     ↻"       │          ┌──────────────┐
                              └──────┬───────┘          │ Pass2        │
                                     │                  │ AiMatching   │
                                     │success           │"Calling      │
                                     ▼                  │ Accountant ☏"│
                              ┌──────────────┐          └──────┬───────┘
                              │ NewBank      │                 │
                              │ Confirmation │                 │ if low-conf
                              └──────┬───────┘                 │ items exist
                                     │                         ▼
                                     │confirm           ┌──────────────┐
                                     ▼                  │ Pass3        │
                              ┌──────────────┐          │ Clarification│
                              │ NewBank      │          └──────┬───────┘
                              │ AccountPicker│                 │
                              └──────┬───────┘                 │ all resolved
                                     │                         ▼
                                     │select            ┌──────────────┐
                                     │                  │ ReviewScreen │
                                     ▼                  └──────┬───────┘
                              ┌──────────────┐                 │
                              │ DuplicateWarn│                 │ approve
                              │  (if dupes)  │                 ▼
                              └──────────────┘          ┌──────────────┐
                                                        │ Creating     │
                                                        └──────┬───────┘
                                                               │
                                                               ▼
                                                        ┌──────────────┐
                                                        │ Complete     │
                                                        └──────────────┘
```

**Error/cancel transitions (from any step):**
- Escape at any step → return to JE tab list view (cancel import)
- API timeout during NewBankDetection → `Failed("Failed ⨂")`
- API timeout during Pass2 → fallback to unmatched (one-sided drafts), continue to ReviewScreen
- File not found in FilePathInput → status bar error, stay in FilePathInput

**Step-specific behavior:**

| Step | Input | Output | Next |
|------|-------|--------|------|
| FilePathInput | Text input, tilde expansion, pre-filled with last_import_dir | Validated file path | BankSelection |
| BankSelection | Selection list from entity toml bank_accounts, plus "New" | Selected bank name | DuplicateWarning (known) or NewBankName (new) |
| NewBankName | Text input | Bank display name | NewBankDetection |
| NewBankDetection | Automatic (Claude API call) | Column mapping proposal | NewBankConfirmation (success) or Failed (timeout) |
| NewBankConfirmation | Y/N confirmation of detected columns | Confirmed mapping | NewBankAccountPicker |
| NewBankAccountPicker | AccountPicker widget | Linked CoA account | DuplicateWarning (then Pass1) |
| DuplicateWarning | Y/N (skip dupes or include all) | Filtered transaction list | Pass1Matching |
| Pass1Matching | Automatic (local matching) | Matched + unmatched lists | ReviewScreen (all matched) or Pass2 (unmatched exist) |
| Pass2AiMatching | Automatic (Claude API batches) | Updated match list | Pass3 (low-conf exist) or ReviewScreen (all resolved) |
| Pass3Clarification | Per-item in chat panel | User confirmations | ReviewScreen (all resolved) |
| ReviewScreen | Browse list, `r` to reject, Enter to approve | Approved match list | Creating |
| Creating | Automatic (batch draft creation) | Draft JEs | Complete |
| Complete | Dismiss | Status bar summary | JE tab list view |

### Debit/Credit Mapping Algorithm

Determines which side of a journal entry is debited vs. credited based on the linked account type and transaction sign.

**Inputs:**
- `amount: Money` — from normalized transaction (positive = deposit, negative = withdrawal)
- `linked_account_type: AccountType` — from the CoA entry for the linked bank account
- `matched_account_id: Option<AccountId>` — the other side (may be None for unmatched)

**Algorithm:**

```
IF linked_account_type is Asset (checking, savings):
    IF amount > 0 (deposit):
        Line 1: DEBIT  linked_account  |  amount
        Line 2: CREDIT matched_account |  amount   (or blank if unmatched)
    IF amount < 0 (withdrawal):
        Line 1: CREDIT linked_account  |  |amount|
        Line 2: DEBIT  matched_account |  |amount|  (or blank if unmatched)

IF linked_account_type is Liability (credit card):
    IF amount > 0 (purchase/charge increases liability):
        Line 1: CREDIT linked_account  |  amount
        Line 2: DEBIT  matched_account |  amount   (or blank if unmatched)
    IF amount < 0 (payment decreases liability):
        Line 1: DEBIT  linked_account  |  |amount|
        Line 2: CREDIT matched_account |  |amount|  (or blank if unmatched)
```

**Amounts are always stored as positive values in debit/credit columns.** The sign of the normalized transaction determines which column each amount goes in — the `Money` value in `journal_entry_lines.debit_amount` or `credit_amount` is always positive.

**Unmatched transactions:** If `matched_account_id` is None, only Line 1 is created. The draft has one complete line (the bank side) and needs manual editing to add Line 2.

### Pass 1 Matching Algorithm

Local deterministic matching against the `import_mappings` table.

**Inputs:**
- `transactions: Vec<NormalizedTransaction>`
- `bank_name: String`
- `db: &EntityDb`

**Algorithm:**

```
FOR each transaction in transactions:
    1. Query import_mappings for exact match:
       WHERE bank_name = {bank_name}
       AND match_type = 'exact'
       AND description_pattern = {transaction.description}

    2. IF exact match found:
       → Mark as matched (MatchSource::Local)
       → UPDATE last_used_at and use_count on the mapping
       → CONTINUE to next transaction

    3. Query import_mappings for substring match:
       WHERE bank_name = {bank_name}
       AND match_type = 'substring'
       AND {transaction.description} LIKE '%' || description_pattern || '%'
       ORDER BY LENGTH(description_pattern) DESC
       LIMIT 1

    4. IF substring match found:
       → Mark as matched (MatchSource::Local)
       → UPDATE last_used_at and use_count on the mapping
       → CONTINUE to next transaction

    5. ELSE:
       → Mark as unmatched (MatchSource::Unmatched)
```

**Ordering:** Exact matches always take priority over substring matches. Among substring matches, longer patterns take priority (more specific). This is enforced by the `ORDER BY LENGTH(description_pattern) DESC`.

### System Prompt Construction Algorithm

Builds the system prompt for Claude API calls.

**Inputs:**
- `workspace_config: &WorkspaceConfig` — global AI config
- `entity_toml: &EntityTomlConfig` — entity-specific config
- `entity_name: &str`
- `context_file_contents: &str` — from the entity's context .md file

**Algorithm:**

```
1. Determine persona:
   IF entity_toml.ai_persona is Some → use it
   ELSE → use workspace_config.ai.persona

2. Construct system prompt:
   """
   You are a {persona}.
   You are advising on the books for {entity_name}.
   You have access to the entity's accounting data via tools.
   Provide clear, actionable guidance. Keep responses concise — 3 paragraphs maximum.
   When referencing specific accounts or amounts, be precise.
   If you need more information to answer accurately, use the available tools before
   asking the user.

   IMPORTANT: At the end of every response, include a line formatted exactly as:
   SUMMARY: [one sentence summarizing what you analyzed and concluded]
   This summary will be logged for future reference. It should be a neutral,
   factual description of what was discussed, not a greeting or pleasantry.

   --- Entity Context ---
   {context_file_contents}
   """

3. Return constructed string
```

**The SUMMARY line** is parsed out of Claude's response by the client. It is stripped from the display text (the user never sees it) and stored as the `AiResponse` audit log entry description. If Claude omits the SUMMARY line, the client falls back to truncating the first sentence of the response to 100 characters.

### Duplicate Detection Algorithm

Identifies previously imported transactions to prevent double-entry.

**Inputs:**
- `transactions: Vec<NormalizedTransaction>` — parsed from CSV
- `db: &EntityDb`

**Algorithm:**

```
1. Query all import_ref values from journal_entries created in the last 90 days:
   SELECT import_ref FROM journal_entries
   WHERE import_ref IS NOT NULL
   AND created_at >= date('now', '-90 days')

2. Collect into a HashSet<String>

3. FOR each transaction:
   IF transaction.import_ref is in the HashSet:
       → Mark as duplicate

4. Return (duplicates: Vec<NormalizedTransaction>, unique: Vec<NormalizedTransaction>)
```

**The 90-day window** is a pragmatic choice. Bank statements typically cover 1 month, and a 90-day lookback catches re-imports of the current month and the two preceding months. Extending further would slow the query on large datasets for diminishing returns.

### /compact Algorithm

Summarizes conversation history to reduce token usage.

**Inputs:**
- `messages: Vec<ChatMessage>` — current conversation history
- `ai_client: &AiClient`

**Algorithm:**

```
1. Count messages (excluding System role messages)
2. IF count < 5: show "Not enough conversation to compact" → return

3. Serialize messages into a single prompt:
   "Summarize this conversation between a user and an AI accountant.
    Preserve all specific numbers, account names, account numbers, dates,
    decisions made, and open questions. Be concise.

    Conversation:
    {for each message: "[{role}]: {content}\n"}
    "

4. Send to Claude API (10s timeout, no tools)
5. ON SUCCESS:
   - Clear messages
   - Insert single System message: "[Compacted from {N} messages]\n\n{summary}"
   - Show system note: "[Context compacted: {N} messages → summary]"
6. ON FAILURE:
   - Show "The Call Dropped ☹" via set_error
   - Messages unchanged
```

---

## Tool Schema Definitions

Each tool is defined as a JSON schema for the Claude API `tools` parameter. The tool names and parameters are listed here; the full JSON schemas are generated in code by `tool_definitions()`.

| Tool | Parameters | Description |
|------|-----------|-------------|
| `get_account` | `query: string` (account number, name, or substring) | Look up an account by number or name. Returns account details and current balance. |
| `get_account_children` | `account_id: integer` | Get all child accounts under a placeholder account. Returns list with balances. |
| `search_accounts` | `query: string` | Search accounts by name or number substring. Returns matching accounts with balances. |
| `get_gl_transactions` | `account_id: integer`, `start_date?: string`, `end_date?: string` | Get general ledger transactions for an account. Optional date range filter. Returns transactions with running balance. |
| `get_journal_entry` | `je_number: integer` | Get full journal entry details including all lines, amounts, accounts, and status. |
| `get_open_ar_items` | `status?: string` (Open, Partial, Paid) | Get accounts receivable items. Optional status filter. |
| `get_open_ap_items` | `status?: string` (Open, Partial, Paid) | Get accounts payable items. Optional status filter. |
| `get_envelope_balances` | (none) | Get all envelope allocations and current available amounts. |
| `get_trial_balance` | `as_of_date?: string` | Get trial balance across all accounts. Optional as-of date. |
| `get_audit_log` | `action?: string`, `start_date?: string`, `end_date?: string`, `limit?: integer` | Search audit log entries. Filter by action type and/or date range. Includes AI interaction history. |

**Note:** `get_audit_log` was not in the original feature spec tool list but is required for Claude to review its own past interactions (logged as AiPrompt/AiResponse/AiToolUse entries). Without it, the audit logging of AI interactions provides no benefit to Claude's continuity.

---

## Tab Key Conflict Resolution

### Affected Components

| Component | V1 Tab Behavior | V2 Alternative | V2 Tab (panel open) |
|-----------|----------------|----------------|---------------------|
| JE Form (`je_form.rs`) | Navigate between fields | Arrow keys (Up/Down between lines, Left/Right between columns) + Enter to confirm and advance | Intercepted by App → switch focus |
| Envelopes Tab (`envelopes.rs`) | Toggle Allocation Config ↔ Balances view | `v` key (mnemonic: view toggle) | Intercepted by App → switch focus |
| All other tabs/widgets | Not used | N/A | Intercepted by App → switch focus |

### Implementation Priority

1. Add `v` key to Envelopes tab as view toggle (works always, panel open or closed)
2. Add arrow key + Enter navigation to JE Form (works always, panel open or closed)
3. Intercept Tab at App level when chat panel is visible
4. Existing Tab behavior unchanged when panel is not visible
