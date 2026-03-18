# V2 Summary — AI Accountant & CSV Import

## Overview

V2 added two major features to the double-entry bookkeeping TUI: an AI Accountant chat
panel powered by Claude, and a CSV import pipeline for bank transactions. Development
spanned 64 commits across three phases plus pre-phase work.

## Commit Counts by Phase

| Phase | Task Commits | Fix / Polish | Total |
|-------|-------------|-------------|-------|
| Pre-phase | 2 | 0 | 2 |
| Specs | 1 | 0 | 1 |
| Phase 1: Foundation | 12 | 1 | 13 |
| Phase 2: AI Chat | 12 | 6 | 18 |
| Phase 3: CSV Import | 13 | 16 | 29 |
| **Total** | **40** | **23** | **64** |

## Test Progression

| Milestone | Tests |
|-----------|-------|
| V1 complete | 372 |
| After Phase 1 | 487 (+115) |
| After Phase 2 | 582 (+95) |
| After Phase 3 | 582 (+0) |
| Current (with fixes) | 609 (+27) |

---

## Phase 1: Foundation (13 commits)

Laid the groundwork for AI integration and CSV import.

**Type system extensions:**
- New enums: `AiRequestState`, `ChatRole`, `FocusTarget`, `MatchSource`, `MatchConfidence`,
  `ImportMatchType`, `ImportMatchSource`
- New wire types: `ApiMessage`, `ApiContent`, `ToolCall`, `ToolResult`, `AiResponse`, `RoundResult`
- New import types: `NormalizedTransaction`, `ImportMatch`, `ColumnMapping`

**Database changes:**
- New table: `import_mappings` (learned CSV-to-account mappings)
- New column: `journal_entries.import_ref` (deduplication key)
- New repo: `ImportMappingRepo` with CRUD operations
- New journal queries: `find_by_import_ref`, `get_incomplete_imports`

**Configuration system:**
- `workspace.toml`: added `[ai]` section (persona, model) and `context_dir`
- Per-entity `.toml`: `ai_persona`, `last_import_dir`, `[[bank_accounts]]` config
- `~/.config/bookkeeper/secrets.toml`: API key storage
- Context file loading for AI system prompts

**Key remapping (Tab key conflict resolution):**
- Envelopes: `v` replaces Tab for view toggle
- JE form: Arrow keys + Enter as alternative to Tab for field navigation
- Tab key freed for focus switching between chat panel and main content

**Audit logging:** Added `AiPrompt`, `AiResponse`, `AiToolUse` convenience methods.

---

## Phase 2: AI Chat Panel (18 commits)

Built the complete AI Accountant feature.

**AI Client (`src/ai/client.rs`):**
- Synchronous HTTP via `ureq` — no async, no threading
- Stateless design: all conversation state passed per call
- Prompt caching support (`anthropic-beta: prompt-caching-2024-07-31`)
- SUMMARY line parsing: stripped from display, logged to audit
- 10-second timeout on all API calls

**Tool Use (10 read-only tools):**
- `get_account`, `get_account_children`, `search_accounts`
- `get_gl_transactions`, `get_journal_entry`
- `get_open_ar_items`, `get_open_ap_items`
- `get_envelope_balances`, `get_trial_balance`, `get_audit_log`
- All tools query repos but never write to the database

**Tool Fulfillment Loop:**
- Up to 5 rounds per request
- `handle_ai_request` in `app.rs` drives rounds via `send_single_round`
- Between rounds: log `AiToolUse` to audit, set `ai_state = FulfillingTools`, force render
- `take()`/replace pattern on `ai_client` to split borrows

**Chat Panel Widget (`src/widgets/chat_panel.rs`):**
- Does NOT own an `AiClient` — returns `ChatAction` for App to handle
- Typewriter animation (80 chars/tick)
- Message history with word-wrapped rendering
- Slash commands: `/clear`, `/context`, `/compact`, `/persona [text]`, `/match`

**Focus Model:**
- `FocusTarget` enum: `MainTab` | `ChatPanel`
- Tab key switches focus when panel is visible
- Ctrl+K toggles panel visibility (focus goes to panel on open)
- Panel intercepts all keys when focused except Tab/Esc/Ctrl+K

**Bug fixes (6):**
- Tool logging and yield-between-rounds restructuring
- Typewriter speed tuning, input word wrap
- Three separate chat panel scroll fixes (direction, auto-scroll, offset)

---

## Phase 3: CSV Import Pipeline (29 commits)

Built the multi-step import wizard — the most complex and bug-prone phase.

**Import Flow State Machine:**
- File picker → Bank detection → Column mapping → Duplicate check →
  Pass 1 (local) → Pass 2 (AI) → Pass 3 (clarification) → Review → Draft creation

**Three-Pass Matching:**
1. **Local matching**: Check `import_mappings` table for known description→account mappings
2. **AI matching**: Send unmatched transactions to Claude for categorization
3. **Clarification**: Handle ambiguous matches with user prompts

**File Browser Widget (`src/widgets/file_picker.rs`):**
- Modal file browser for `.csv` selection (replaced text input)
- Directories first, then `.csv` files, hidden entries excluded
- Parent directory (`..`) navigation, Esc cancels, Enter selects

**Bank Configuration:**
- Auto-detection of bank format from CSV headers
- Configurable column mappings (date, description, amount or debit/credit split)
- Date format cycle selector (replaced free-text)
- Bank add/edit/delete from the selection modal

**Draft Creation:**
- Single-line draft strategy: unmatched imports create a JE with only the bank account line
- Contra line added after matching resolves
- `import_ref` format: `"{bank_name}|{date}|{description}|{amount}"`

**Re-match Features:**
- `U` (Shift+U): batch re-match all incomplete import drafts
- `/match` slash command: re-match the currently selected draft

**Bug fixes (16):**
- Schema initialization order for fresh databases
- Seed account population on fresh databases
- Account picker memory leak
- Import flow cleanup after draft creation
- Parenthetical negative amount parsing `(1234.56)`
- Status bar help indicator visibility
- Tracing display bleed into TUI
- `get_journal_entry` tool lookup (flexible ID parsing)
- Import config staleness, parse warnings
- Bank editing, overlay cleanup
- Various file picker improvements

---

## Architecture Changes

| Area | V1 | V2 |
|------|----|----|
| Module count | 8 modules | 9 modules (+`src/ai/`) |
| Dependencies | ratatui, rusqlite, crossterm | +ureq, serde_json, csv |
| Config files | workspace.toml | +entity.toml, secrets.toml |
| DB tables | 14 | 15 (+import_mappings) |
| Audit actions | 19 | 23 (+AiPrompt, AiResponse, AiToolUse, CsvImport, MappingLearned) |
| Key dispatch | 8 priority levels | 12 priority levels |
| Tab key | Tab-specific use | Global focus switch when panel open |

## Key Design Decisions

1. **Synchronous HTTP** — `ureq` chosen over async alternatives to maintain the no-async invariant.
   Trade-off: UI freezes during API calls, mitigated by forced renders before blocking.

2. **Read-only AI tools** — Claude can query the books but never write. All mutations happen in
   application code after Claude returns. Prevents AI-initiated data corruption.

3. **Imports always create Drafts** — Never Posted entries. User must review and post manually.
   Safety net for incorrect categorizations.

4. **Single-line draft strategy** — Unmatched imports create a JE with only the bank account line.
   Simpler than alternatives (sentinel accounts, nullable columns). Balance enforcement at post time.

5. **Lazy API key loading** — Not loaded at startup. First Ctrl+K or `u` import triggers the load.
   Keeps the app usable without AI configuration.

6. **ChatPanel returns actions, App handles I/O** — Panel never makes API calls or writes to DB.
   Maintains the pattern where tabs/widgets return actions and App processes them.

## Lessons Learned

- **Phase 3 required 16 fix commits vs Phase 2's 6** — multi-step wizards with file I/O, parsing,
  and AI integration are significantly harder to get right than chat interfaces.
- **Three separate scroll fixes** needed for the chat panel — scroll behavior is deceptively complex.
- **Tab key conflicts** required careful resolution across three contexts (envelopes, JE form, focus).
- **Fresh database initialization** needed both migration paths AND correct CREATE TABLE schemas
  (in-memory test DBs don't run migrations).
- **Forced render before blocking calls** is essential in a synchronous TUI — without it the user
  sees nothing while the API call executes.
