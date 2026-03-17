# V2 Phase 2 — AI Client & Chat Panel

## Overview

Build the Claude API client, tool fulfillment layer, chat panel widget, focus management system, and all slash commands. By the end of this phase, Ctrl+K opens a working AI accountant that can answer questions using tool use.

**Tasks:** 12
**Depends on:** V2 Phase 1 complete
**Produces:** Working AI chat panel with tool use, slash commands, typewriter rendering, focus model

---

## Completion Criteria

- [ ] `ureq` dependency added and API client functional
- [ ] All 10 tools defined and fulfillable against a test database
- [ ] Ctrl+K opens a 30% width right-side chat panel
- [ ] Tab switches focus between panel and main tab area
- [ ] Chat panel focused: all keys go to input (no hotkey leaks)
- [ ] Main tab focused: all hotkeys work normally
- [ ] Escape and Ctrl+K close the panel when it's focused
- [ ] User can type a question, see "Calling Accountant ☏", get a response
- [ ] Tool use works: "Checking the books 🕮" appears during tool fulfillment
- [ ] Timeout shows "The Call Dropped ☹"
- [ ] Typewriter animation renders response progressively, Enter skips
- [ ] `/clear`, `/context`, `/compact`, `/persona`, `/match` all work
- [ ] AI interactions logged to audit_log (AiPrompt, AiResponse, AiToolUse)
- [ ] Help overlay updated with Ctrl+K and chat commands section
- [ ] All existing tests pass plus new tests
- [ ] `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test` clean

## Does NOT Cover

- CSV import flow (Phase 3)
- `U` or `Shift+U` hotkeys (Phase 3)
- Bank format detection (Phase 3)
- Import review screen (Phase 3)

---

## Task 1: Add ureq Dependency

**Context:** Read `Cargo.toml`

**Action:**
1. Add `ureq` to `Cargo.toml` dependencies (latest stable, with `json` feature disabled — we handle JSON via `serde_json`)
2. Verify it builds

**Verify:**
- `cargo build` succeeds
- `use ureq;` compiles in a test
- No new warnings from clippy

**Do NOT:**
- Write any API client code yet
- Add any other dependencies

---

## Task 2: AI Client — Core Request/Response [TEST-FIRST]

**Context:** Read `src/ai/mod.rs` (types from Phase 1), `specs/v2/v2-architecture.md` (AiClient struct)

**Action:**
1. Create `src/ai/client.rs`
2. Implement `AiClient::new(api_key, model) -> Self` — stores key, model, 10s timeout
3. Implement `build_system_prompt(persona, entity_name, context_contents) -> String` — constructs the system prompt including the SUMMARY instruction
4. Implement `parse_summary(response_text: &str) -> (String, String)` — extracts `SUMMARY: ...` line, returns (display_text_without_summary, summary_line). Fallback: first sentence truncated to 100 chars if no SUMMARY line found.
5. Implement internal `send_request(system, messages, tools_opt) -> Result<RawApiResponse>`:
   - Build JSON payload with model, max_tokens (4096), system, messages, tools (if any)
   - POST to `https://api.anthropic.com/v1/messages`
   - Headers: `x-api-key`, `anthropic-version: 2023-06-01`, `content-type: application/json`
   - 10-second timeout via `ureq`
   - Parse response JSON
   - Map HTTP errors to `AiError::ApiError`
   - Map timeout to `AiError::Timeout`
   - Map parse failures to `AiError::ParseError`
6. Implement `send_simple(system, messages) -> Result<String>`:
   - Calls `send_request` without tools
   - Extracts text content from response
   - Returns the text (no SUMMARY parsing — caller decides)

**Verify:**
- `build_system_prompt` includes persona, entity name, context contents, and SUMMARY instruction
- `parse_summary` extracts summary line and strips it from display text
- `parse_summary` fallback on missing SUMMARY line
- `send_request` builds correct JSON structure (test by inspecting serialized payload, not making real calls)
- Error mapping: test that known error shapes produce correct `AiError` variants
- Note: actual API calls are NOT tested here (use mocks or payload inspection)

**Do NOT:**
- Implement `send_with_tools` yet (next task)
- Make any real API calls in tests

---

## Task 3: AI Client — Tool Use Loop [TEST-FIRST]

**Context:** Read `src/ai/client.rs` (from Task 2), `specs/v2/v2-type-system.md` (AI Request Lifecycle state machine)

**Action:**
1. Implement `send_with_tools(system, messages, tools, db, max_depth, on_stage_change) -> Result<AiResponse>`:
   - Call `send_request` with tools
   - If response contains `tool_use` blocks:
     - Call `on_stage_change(AiRequestState::FulfillingTools)`
     - For each tool_use block, call `fulfill_tool_call` (imported from tools.rs)
     - Append assistant tool_use message and user tool_result messages to history
     - Call `send_request` again with updated history
     - Repeat up to `max_depth` rounds
   - If response contains text block:
     - Parse SUMMARY line
     - Return `AiResponse::Text { content, summary }`
   - If max depth exceeded:
     - Return whatever text exists, or a fallback message
2. Handle mixed responses (text + tool_use in same response) — process tool calls first, text accumulates

**Verify:**
- Single-round text response → returns immediately
- Tool use → fulfillment → text response → correct final text
- Multi-round tool use (2-3 rounds) → terminates correctly
- Max depth (5) reached → returns fallback message
- Timeout on any round → returns AiError::Timeout
- `on_stage_change` callback called at correct points
- Tool results correctly formatted in follow-up request

**Do NOT:**
- Create real tool handlers (use a mock `fulfill_tool_call` for testing)
- Make real API calls

---

## Task 4: Tool Definitions [TEST-FIRST]

**Context:** Read `specs/v2/v2-type-system.md` (Tool Schema Definitions), `specs/v2/v2-architecture.md` (Tool Fulfillment signatures)

**Action:**
1. Create `src/ai/tools.rs`
2. Implement `tool_definitions() -> Vec<ToolDefinition>` — returns JSON schemas for all 10 tools:
   - `get_account`, `get_account_children`, `search_accounts`
   - `get_gl_transactions`, `get_journal_entry`
   - `get_open_ar_items`, `get_open_ap_items`
   - `get_envelope_balances`, `get_trial_balance`
   - `get_audit_log`
3. Each tool definition includes: name, description, input_schema (JSON Schema with parameter types and required fields)
4. Tool descriptions should be clear enough for Claude to understand when to use each tool

**Verify:**
- `tool_definitions()` returns 10 definitions
- Each definition has a valid JSON Schema as input_schema
- Each schema's required fields match the non-optional parameters
- Serializes to valid JSON (for API request payload)

**Do NOT:**
- Implement fulfillment handlers yet (next task)
- Include any tools that write data

---

## Task 5: Tool Fulfillment Handlers

**Context:** Read `src/ai/tools.rs` (definitions from Task 4), all `src/db/*_repo.rs` files for method signatures

**Action:**
1. Implement `fulfill_tool_call(tool_call: &ToolCall, db: &EntityDb) -> Result<String>`:
   - Match on `tool_call.name`
   - Deserialize `tool_call.input` into expected parameters
   - Call appropriate repo method
   - Serialize result to JSON string
   - Unknown tool name → return error message as tool result (don't panic)
2. Implement individual handlers for each of the 10 tools:
   - `get_account` → `db.accounts().search()` or `get_by_number()`
   - `get_account_children` → `db.accounts().get_children()`
   - `search_accounts` → `db.accounts().search()`
   - `get_gl_transactions` → `db.journals().get_account_transactions()` with optional date filter
   - `get_journal_entry` → `db.journals().get_by_number()` + `get_lines()`
   - `get_open_ar_items` → `db.ar().list()` with optional status filter
   - `get_open_ap_items` → `db.ap().list()` with optional status filter
   - `get_envelope_balances` → `db.envelopes().get_balances()`
   - `get_trial_balance` → `db.accounts().get_all_balances()`
   - `get_audit_log` → `db.audit().get_ai_entries()` or general query with filters
3. Serialize Money values as formatted strings (e.g., "$1,234.56") in tool results for Claude readability
4. Serialize dates as YYYY-MM-DD strings

**Verify:**
- Each tool handler returns valid JSON when given valid parameters
- Each tool handler returns a descriptive error string for invalid parameters
- Unknown tool name returns error (not panic)
- Money formatting is human-readable in output
- Test against a seeded test database with accounts, JEs, AR/AP items

**Do NOT:**
- Add any write-capable tools
- Call `db.conn().execute()` directly — always go through repos

---

## Task 6: Chat Panel Widget — Structure and Rendering

**Context:** Read `src/widgets/chat_panel.rs` stub (if created), `specs/v2/v2-architecture.md` (ChatPanel struct, Render Layout)

**Action:**
1. Implement `ChatPanel` struct with all fields from the architecture spec
2. Implement `ChatPanel::new(entity_name, persona) -> Self` — initializes with empty messages, invisible
3. Implement `build_welcome() -> ()` — populates messages with the welcome/help text
4. Implement `render(frame, area)`:
   - Header with entity name
   - Scrollable message area with word wrapping at panel width
   - Different styling per role: User messages right-aligned or prefixed with `You:`, Assistant messages prefixed with `Accountant:`, System messages dim/italic
   - Typewriter: render only `full_text[..display_position]` when active
   - Input line at bottom with `> ` prompt and cursor
   - Border with focus indicator (bright when focused, dim when not — `is_focused` passed as parameter or read from a flag)
5. Implement `tick()` — advance typewriter by 20 characters per tick (char-boundary aligned)
6. Implement `toggle_visible() -> bool`, `is_visible() -> bool`

**Verify:**
- Renders correctly at various widths (30% of 80, 120, 200 columns)
- Word wrapping doesn't break mid-word
- Long messages scroll correctly
- Welcome message displays on first open
- Typewriter reveals text progressively
- Border changes appearance based on focus state
- Empty panel (no messages) renders cleanly

**Do NOT:**
- Implement key handling yet (next task)
- Connect to App yet
- Make any API calls

---

## Task 7: Chat Panel Widget — Key Handling

**Context:** Read `src/widgets/chat_panel.rs` (from Task 6), `specs/v2/v2-type-system.md` (SlashCommand enum, Typewriter state machine)

**Action:**
1. Implement `SlashCommand` enum and parsing function (from type system spec)
2. Implement `handle_key(key: KeyEvent) -> ChatAction`:
   - Character keys → insert into `input_buffer` at `cursor_pos`
   - Backspace → delete character before cursor
   - Delete → delete character at cursor
   - Left/Right arrows → move cursor position
   - Home/End → move cursor to start/end of input
   - Enter:
     - If typewriter active → return `ChatAction::SkipTypewriter`
     - If input starts with `/` → parse as SlashCommand, return `ChatAction::SlashCommand`
     - If input non-empty → call `submit_input()`, return `ChatAction::SendMessage`
     - If input empty → return `ChatAction::None`
   - Up/Down arrows (when input empty) → scroll message history
   - Escape → return `ChatAction::Close`
   - Ctrl+K → return `ChatAction::Close`
   - Tab → not handled here (intercepted at App level)
3. Implement `submit_input() -> Option<Vec<ApiMessage>>`:
   - Add input as a User ChatMessage
   - Clear input buffer and cursor
   - Build and return API message history
4. Implement `add_response(content: String)` — creates ChatMessage with typewriter
5. Implement `add_system_note(note: &str)` — creates System ChatMessage (fully rendered, no typewriter)
6. Implement `replace_with_summary(summary, original_count)` — for /compact
7. Implement `rebuild_system_prompt(persona, entity_name, context)` — rebuilds the system prompt
8. Implement `api_messages() -> Vec<ApiMessage>` — converts ChatMessages to API format (excludes System messages)

**Verify:**
- Typing characters → appear in input buffer at cursor position
- Backspace → deletes correctly
- Enter with typewriter active → returns SkipTypewriter
- Enter with `/clear` → returns SlashCommand(Clear)
- Enter with text → returns SendMessage with correct API messages
- Enter with empty input → returns None
- `/persona some text` → returns SlashCommand(Persona(Some("some text")))
- `/persona` → returns SlashCommand(Persona(None))
- `/unknown` → returns SlashCommand(Unknown("unknown"))
- Escape → returns Close
- `api_messages()` correctly formats User and Assistant messages, excludes System

**Do NOT:**
- Execute slash commands (App does that)
- Make API calls
- Handle focus switching (App does that)

---

## Task 8: App Integration — Focus Model and Layout

**Context:** Read `src/app.rs`, `specs/v2/v2-architecture.md` (Flow 4: Focus and key dispatch, Render Layout)

**Action:**
1. Add fields to `App`: `chat_panel: ChatPanel`, `focus: FocusTarget`, `ai_client: Option<AiClient>`, `ai_state: AiRequestState`
2. Initialize in `App::new()`: `ChatPanel::new(entity_name, persona)`, `FocusTarget::MainTab`, `None`, `Idle`
3. Modify `App.render()`:
   - If `chat_panel.is_visible()`: split main content area into 70% left (tab) + 30% right (panel)
   - Pass focus state to panel render (for border styling)
   - Pass focus state to tab area render (for border styling, if applicable)
   - Tab bar and status bar remain full width
4. Modify `App.handle_key()` following the dispatch order from the architecture spec:
   - Step 1: Modal priority (unchanged)
   - Step 2: If panel visible + focused → delegate to panel (except Tab/Escape/Ctrl+K)
   - Step 3: If panel visible + unfocused → Tab and Ctrl+K switch focus
   - Step 4: Inter-entity mode (unchanged)
   - Step 5: Global hotkeys — add Ctrl+K to open panel
   - Step 6: Active tab (unchanged)
5. Tab key interception: when panel is visible, intercept `KeyCode::Tab` at step 2/3 BEFORE it reaches any tab or widget

**Verify:**
- Ctrl+K opens panel, focus moves to ChatPanel
- Ctrl+K when panel focused → closes panel
- Tab toggles focus between panel and main tab
- When panel focused: pressing `q` types "q" in input (not quit)
- When panel focused: pressing `n` types "n" in input (not new JE)
- When main tab focused with panel open: `q` quits, `n` opens new JE, etc.
- Escape when panel focused → closes panel
- Escape when main tab focused with panel open → normal tab Escape behavior
- Panel renders at 30% width, tab at 70%
- Tab key in JE form still works when panel is NOT open
- Tab key switches focus when panel IS open (even in JE form context)

**Do NOT:**
- Wire up AI API calls yet (next task)
- Implement slash commands yet (later task)

---

## Task 9: App Integration — AI Request Orchestration

**Context:** Read `src/app.rs` (from Task 8), `src/ai/client.rs`, `specs/v2/v2-architecture.md` (Flow 1: User asks a question)

**Action:**
1. Implement `App.handle_ai_request(messages: Vec<ApiMessage>)`:
   - Lazy-init `AiClient`: load secrets (show error if missing), load model from config, create client
   - Load entity context file
   - Build system prompt
   - Log `AiPrompt` to audit_log (user's question)
   - Set `ai_state = CallingApi`
   - Force render via `terminal.draw()`
   - Call `ai_client.send_with_tools()` with stage change callback
   - The callback: updates `ai_state`, calls `terminal.draw()`, logs `AiToolUse` entries
   - On success: parse summary, log `AiResponse`, strip summary from display, call `chat_panel.add_response()`
   - On error: call `set_error("The Call Dropped ☹")`
   - Set `ai_state = Idle`
2. Connect `ChatAction::SendMessage` in the event loop to `handle_ai_request()`
3. Connect `ChatAction::SkipTypewriter` → set typewriter to complete
4. Add `ai_state` rendering in status bar: display the appropriate message for CallingApi and FulfillingTools
5. Add typewriter tick to the existing tick handler (500ms cycle)

**Verify:**
- End-to-end flow: type question → see "Calling Accountant ☏" → response appears with typewriter
- Tool use: question requiring data → see "Checking the books 🕮" → response with data
- Timeout: API not responding → "The Call Dropped ☹" in status bar
- No API key: → "No API key found — see ~/.config/bookkeeper/secrets.toml"
- Audit log: AiPrompt and AiResponse entries created
- Audit log: AiToolUse entries for each tool call
- Typewriter: text reveals over time, Enter skips
- Note: this requires a real API key for integration testing, or careful mocking

**Do NOT:**
- Implement slash commands (next task)
- Wire up import flow

---

## Task 10: Slash Command Execution

**Context:** Read `src/app.rs` (from Task 9), `specs/v2/v2-architecture.md` (Flow 3: Slash command processing)

**Action:**
1. Handle `ChatAction::SlashCommand(cmd)` in the event loop
2. Implement each command:
   - `/clear` → clear messages, rebuild system prompt, build welcome, add "[Conversation cleared]" note
   - `/context` → re-read context file, rebuild system prompt, add "[Context refreshed from {tab} tab]" note
   - `/compact`:
     - If < 5 messages: add "Not enough conversation to compact" note
     - Else: set CallingApi, force render, send compaction request via `send_simple`, on success call `replace_with_summary`, on failure `set_error`
   - `/persona` (no args) → add "Current persona: {persona}" note
   - `/persona {text}` → update entity toml (or create if needed), save, rebuild system prompt, add "[Persona updated]" note
   - `/match`:
     - Check if JE tab is active and a draft with import_ref is selected
     - If not applicable: add "Select an incomplete import draft in the Journal Entries tab first" note
     - If applicable: build single-match prompt, send via `send_with_tools`, display suggestion
     - (Full /match implementation may need Phase 3 infrastructure — implement the check and error message now, defer the actual matching to Phase 3)
   - Unknown → add "Unknown command. Available: /clear, /context, /compact, /persona, /match" note
3. For `/persona` with args: implement entity toml write (update `ai_persona` field, save file)

**Verify:**
- `/clear` resets messages and shows welcome
- `/context` rebuilds prompt without clearing messages
- `/compact` summarizes conversation (requires API call)
- `/compact` with < 5 messages shows error note
- `/compact` failure shows "The Call Dropped ☹"
- `/persona` shows current persona
- `/persona new text` updates entity toml and rebuilds prompt
- `/match` with no applicable draft shows error note
- Unknown command shows help text

**Do NOT:**
- Fully implement `/match` matching logic (defer to Phase 3)
- Modify the JE tab to expose selection info yet (Phase 3)

---

## Task 11: Help Overlay Update

**Context:** Read the help overlay rendering section in `src/app.rs`

**Action:**
1. Add `Ctrl+K       AI Accountant` to the Global Hotkeys section (always visible)
2. Add a new "Chat Panel" section (visible only when `chat_panel.is_visible()`):
```
── Chat Panel ──────────────
Ctrl+K       Open/close AI panel
Tab          Switch focus (panel ↔ tab)
/clear       Reset conversation
/context     Refresh tab data
/compact     Compress history
/persona     View/change persona
/match       Re-match selected draft
```
3. Update Envelopes tab section: change Tab reference to `v` for view toggle
4. Add `e            Edit selected draft entry` to Journal Entries section (if not already there from the prerequisite feature)

**Verify:**
- Help overlay shows Ctrl+K in global section
- Chat Panel section appears only when panel is open
- Chat Panel section hidden when panel is closed
- Envelopes section shows `v` not Tab
- JE section shows `e` for edit

**Do NOT:**
- Add import hotkeys yet (Phase 3)

---

## Task 12: Loading State Status Bar Messages

**Context:** Read `src/widgets/status_bar.rs`, `specs/v2/v2-type-system.md` (AiRequestState)

**Action:**
1. Modify status bar rendering to check `AiRequestState` (passed from App)
2. When `CallingApi`: display "Calling Accountant ☏" in the status message area (use a distinct style — perhaps the same green as success messages, or a new info color)
3. When `FulfillingTools`: display "Checking the books 🕮"
4. When `Idle`: normal status bar behavior (existing messages, errors, etc.)
5. AI state messages take priority over normal status messages while active (they're replaced by normal messages once Idle)
6. Ensure the Unicode characters (☏ and 🕮) render correctly in the terminal

**Verify:**
- CallingApi state → "Calling Accountant ☏" visible
- FulfillingTools state → "Checking the books 🕮" visible
- Idle state → normal status bar
- AI message disappears when state returns to Idle
- Unicode characters render without corruption

**Do NOT:**
- Add "Importing ☺" or other import-specific messages (Phase 3)
- Modify the error message display ("The Call Dropped ☹" uses existing `set_error`)

---

## Developer Review Gate

Before proceeding to Phase 3:
1. Run full verification: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
2. **Manual testing with a real API key:**
   - Open Ctrl+K panel → see welcome message
   - Ask a question about accounts → see tool use, get accurate answer
   - Ask a follow-up → conversation context maintained
   - `/clear` → conversation reset
   - `/context` → context refreshed
   - `/compact` → conversation summarized
   - `/persona` → shows current persona
   - Tab switching → focus model works correctly
   - Type `q` in chat → doesn't quit app
   - Timeout simulation (disconnect network) → "The Call Dropped ☹"
3. Review audit log entries for AI interactions
4. Review focus model edge cases: modal + panel open, inter-entity mode + panel open
5. Count total tests
