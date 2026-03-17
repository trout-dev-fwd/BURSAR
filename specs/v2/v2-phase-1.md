# V2 Phase 1 — Foundation

## Overview

Establish all new types, configuration parsing, database schema changes, and repository methods that Features 2 and 3 build on. No UI changes, no API calls, no chat panel. Everything in this phase is testable in isolation.

**Tasks:** 12
**Depends on:** V1 complete, draft editing feature merged
**Produces:** All new types, config structs, database tables/columns, repos, context file management

---

## Completion Criteria

- [ ] All new enums (`AiRequestState`, `ImportMatchType`, `ImportMatchSource`, `ChatRole`, `SlashCommand`, `FocusTarget`, `MatchSource`, `MatchConfidence`) compile with `FromStr`/`Display`
- [ ] `AuditAction` has five new variants with `FromStr`/`Display`
- [ ] `workspace.toml` parses with and without `[ai]` section (backwards compatible)
- [ ] Per-entity TOML files parse with all bank account format variations
- [ ] Secrets file loads from `~/.config/bookkeeper/secrets.toml`
- [ ] `import_mappings` table created on schema init, migrated on existing DBs
- [ ] `import_ref` column added to `journal_entries` via migration
- [ ] `ImportMappingRepo` passes all CRUD and matching tests
- [ ] Context file auto-creation and read works
- [ ] Envelopes tab uses `v` for view toggle (Tab removed)
- [ ] JE form navigable with arrow keys + Enter
- [ ] All 372 existing tests still pass
- [ ] `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test` clean

## Does NOT Cover

- No `ureq` dependency yet (added in Phase 2)
- No API calls
- No chat panel widget
- No import flow UI
- No changes to `app.rs` event loop (except Tab key interception prep)

---

## Task 1: New Enums and Types [TEST-FIRST]

**Context:** Read `src/types/enums.rs`, `specs/v2/v2-type-system.md` (New Enums section)

**Action:**
1. Add five new variants to the existing `AuditAction` enum: `AiPrompt`, `AiResponse`, `AiToolUse`, `CsvImport`, `MappingLearned`. Update `FromStr`/`Display` impls.
2. Create `ImportMatchType` enum (`Exact`, `Substring`) with `FromStr`/`Display` (lowercase: `"exact"`, `"substring"`)
3. Create `ImportMatchSource` enum (`Confirmed`, `AiSuggested`) with `FromStr`/`Display` (lowercase: `"confirmed"`, `"ai_suggested"`)
4. Create `AiRequestState` enum (`Idle`, `CallingApi`, `FulfillingTools`) — no serialization needed, in-memory only
5. Create `ChatRole` enum (`User`, `Assistant`, `System`) — in-memory only
6. Create `FocusTarget` enum (`MainTab`, `ChatPanel`) — in-memory only
7. Create `MatchSource` enum (`Local`, `Ai`, `UserConfirmed`, `Unmatched`) — in-memory only
8. Create `MatchConfidence` enum (`High`, `Medium`, `Low`) — in-memory only

**Verify:**
- All new enums round-trip through `FromStr`/`Display` (for those that need it)
- Existing `AuditAction` variants still parse correctly
- `cargo test` — all existing tests pass

**Do NOT:**
- Add `SlashCommand` enum yet (it has parsing logic, deferred to Phase 2)
- Add `ImportFlowStep` enum yet (complex, deferred to Phase 3)
- Add struct types (separate task)

---

## Task 2: New Struct Types [TEST-FIRST]

**Context:** Read `specs/v2/v2-type-system.md` (New Structs section), `specs/v2/v2-architecture.md` (Key Structs)

**Action:**
1. In `src/ai/mod.rs` (create the `ai` module), define:
   - `ToolCall { id: String, name: String, input: serde_json::Value }`
   - `ToolDefinition { name: String, description: String, input_schema: serde_json::Value }`
   - `AiResponse` enum (`Text { content, summary }`, `ToolUse(Vec<ToolCall>)`)
   - `AiError` enum (`Timeout`, `ApiError { status, body }`, `ParseError(String)`, `NoApiKey`, `MaxToolDepth`)
   - `ToolResult { tool_use_id: String, content: String }`
   - `ApiMessage`, `ApiRole`, `ApiContent` structs/enums
2. Add `pub mod ai;` to `src/lib.rs`
3. Add `serde_json` to `Cargo.toml` as explicit dependency
4. In `src/ai/csv_import.rs` (create stub), define:
   - `NormalizedTransaction { date, description, amount, import_ref, raw_row }`
   - `ImportMatch { transaction, matched_account_id, matched_account_display, match_source, confidence, reasoning, rejected }`
5. In `src/ai/mod.rs`, define:
   - `ChatMessage { role: ChatRole, content: String, is_fully_rendered: bool }`
   - `TypewriterState { full_text: String, display_position: usize, message_index: usize }`

**Verify:**
- `src/ai/mod.rs` compiles with all types
- `serde_json::Value` works in struct fields
- Module structure: `src/ai/mod.rs`, `src/ai/csv_import.rs` (stubs for other files)
- `cargo test` — all existing tests pass

**Do NOT:**
- Implement any methods on these structs yet
- Create `ChatPanel` struct (that's a widget, Phase 2)
- Create `BankAccountConfig` (that's config, Task 3)

---

## Task 3: Configuration Extensions [TEST-FIRST]

**Context:** Read `src/config.rs`, `specs/v2/v2-data-model.md` (Configuration File Schemas), `specs/v2/v2-architecture.md` (Config Extensions)

**Action:**
1. Add `WorkspaceAiConfig` struct with `persona` (default: `"Professional Tax Accountant"`) and `model` (default: `"claude-sonnet-4-20250514"`)
2. Add optional `ai: Option<WorkspaceAiConfig>` field to `WorkspaceConfig`
3. Add optional `context_dir: Option<String>` field to `WorkspaceConfig`
4. Add optional `config_path: Option<String>` field to the entity config struct
5. Create `EntityTomlConfig` struct with `ai_persona: Option<String>`, `last_import_dir: Option<String>`, `bank_accounts: Vec<BankAccountConfig>`
6. Create `BankAccountConfig` struct with all fields from the data model (name, linked_account, date_column, description_column, amount_column, debit_column, credit_column, debit_is_negative, date_format)
7. Create `SecretsConfig` struct with `anthropic_api_key: String`
8. Implement `load_secrets() -> Result<SecretsConfig>` — reads from `~/.config/bookkeeper/secrets.toml`, auto-creates directory if missing, returns error if file missing or key empty
9. Implement `secrets_file_path() -> PathBuf`
10. Implement `load_entity_toml(config_path, workspace_dir) -> Result<EntityTomlConfig>` — relative paths resolved against workspace dir, tilde expansion
11. Implement `save_entity_toml(config_path, workspace_dir, config) -> Result<()>` — serialize and write
12. Add validation for `BankAccountConfig`: either `amount_column` is Some, or both `debit_column` and `credit_column` are Some

**Verify:**
- Parse `workspace.toml` with `[ai]` section → all fields populated
- Parse `workspace.toml` without `[ai]` section → defaults applied, no error (backwards compatible)
- Parse entity toml with single-amount bank account → correct struct
- Parse entity toml with split-column bank account → correct struct
- Parse entity toml with no bank accounts → empty vec
- Validation rejects bank account with neither amount_column nor debit/credit columns
- `load_secrets()` returns error on missing file
- `save_entity_toml()` round-trips correctly
- All existing config tests still pass

**Do NOT:**
- Load entity toml at startup (lazy loading happens in Phase 2)
- Write to workspace.toml (entity toml only)

---

## Task 4: Context File Management [TEST-FIRST]

**Context:** Read `specs/v2/v2-data-model.md` (Entity Context Files section), `specs/v2/v2-architecture.md` (ai/context.rs)

**Action:**
1. Create `src/ai/context.rs`
2. Implement `slugify_entity_name(name: &str) -> String` — lowercase, spaces to underscores, strip non-alphanumeric except underscores
3. Implement `context_file_path(entity_name: &str, context_dir: &str) -> PathBuf` — applies slugification, appends `.md`
4. Implement `read_context(entity_name: &str, context_dir: &str) -> Result<String>` — reads file contents, auto-creates with skeleton if missing
5. Auto-creation skeleton:
```markdown
# {Entity Name} — AI Context

## Business Context
<!-- Describe your business here for better AI assistance -->
```
6. Auto-create `context_dir` directory if it doesn't exist

**Verify:**
- `slugify_entity_name("Acme Land LLC")` → `"acme_land_llc"`
- `slugify_entity_name("Bob's Café & Grill")` → `"bobs_caf_grill"` (strips non-alphanumeric except underscore)
- `read_context()` on missing file → creates skeleton, returns skeleton contents
- `read_context()` on existing file → returns existing contents unchanged
- Context dir auto-created if missing

**Do NOT:**
- Implement `append_mapping()` (mappings are in SQLite now, not the context file)
- Read or write anything beyond the business context skeleton

---

## Task 5: Schema — import_mappings Table [TEST-FIRST]

**Context:** Read `src/db/schema.rs`, `src/db/mod.rs`, `specs/v2/v2-data-model.md` (import_mappings table)

**Action:**
1. Add `CREATE TABLE IF NOT EXISTS import_mappings` to `initialize_schema()` in `schema.rs`
2. Include all columns: id, description_pattern, account_id (FK), match_type (CHECK), source (CHECK), bank_name, created_at, last_used_at, use_count (DEFAULT 1)
3. Include UNIQUE constraint on `(description_pattern, bank_name)`
4. Table is created as part of normal schema initialization — no migration needed for new databases

**Verify:**
- New database creation includes `import_mappings` table
- Table has correct columns, types, constraints (inspect via `PRAGMA table_info`)
- Foreign key on `account_id` enforced
- CHECK constraints on `match_type` and `source` enforced
- Unique constraint on `(description_pattern, bank_name)` enforced
- All existing schema tests pass

**Do NOT:**
- Create the repo yet (next task)
- Add the `import_ref` column yet (separate task)

---

## Task 6: ImportMappingRepo [TEST-FIRST]

**Context:** Read `src/db/account_repo.rs` (for pattern reference), `specs/v2/v2-architecture.md` (ImportMappingRepo), `specs/v2/v2-type-system.md` (Pass 1 Matching Algorithm)

**Action:**
1. Create `src/db/import_mapping_repo.rs`
2. Implement `ImportMappingRepo` struct borrowing `&Connection` (follows existing repo pattern)
3. Implement `ImportMapping` row struct
4. Add `import_mappings()` accessor to `EntityDb` in `src/db/mod.rs`
5. Implement methods:
   - `find_exact_match(bank_name, description) -> Result<Option<(i64, AccountId)>>` — returns mapping id + account id
   - `find_substring_match(bank_name, description) -> Result<Option<(i64, AccountId)>>` — longest pattern first
   - `create(description_pattern, account_id, match_type, source, bank_name) -> Result<i64>` — returns new id
   - `update_account(id, account_id, source) -> Result<()>` — change mapping target
   - `record_use(id) -> Result<()>` — update last_used_at and increment use_count
   - `list_by_bank(bank_name) -> Result<Vec<ImportMapping>>` — for future UI

**Verify:**
- Create mapping → retrieve by exact match → correct account returned
- Create mapping → retrieve by substring match → correct account returned
- Substring match returns longest pattern when multiple match
- Exact match takes priority over substring (test with both present)
- `record_use` increments use_count and updates last_used_at
- Duplicate `(description_pattern, bank_name)` returns error
- `update_account` changes the target account
- `list_by_bank` returns all mappings for a bank, empty vec for unknown bank
- Foreign key violation on invalid account_id

**Do NOT:**
- Implement the Pass 1 algorithm yet (that orchestrates the repo, deferred to Phase 3)
- Add any UI for managing mappings

---

## Task 7: Schema — import_ref Column Migration

**Context:** Read `src/db/mod.rs` (look for existing migration pattern with `PRAGMA table_info`), `specs/v2/v2-data-model.md` (import_ref column)

**Action:**
1. In `EntityDb::open()`, after existing migrations, add a check for `import_ref` column on `journal_entries`
2. Use the established pattern: `PRAGMA table_info('journal_entries')` → check if `import_ref` exists → if not, `ALTER TABLE journal_entries ADD COLUMN import_ref TEXT`
3. Add `import_ref` to the `JournalEntry` row struct (as `Option<String>`)
4. Include `import_ref` in journal entry SELECT queries
5. Include `import_ref` as an optional parameter in `create_draft` (defaults to None, existing callers unaffected)

**Verify:**
- Fresh database: `import_ref` column exists on `journal_entries`
- Existing database without column: migration adds it
- Existing database with column: migration is a no-op
- `create_draft` with `import_ref = None` → works as before
- `create_draft` with `import_ref = Some("test|...")` → stored and retrievable
- All existing journal repo tests pass unchanged

**Do NOT:**
- Add duplicate detection queries yet (Phase 3)
- Add incomplete import queries yet (Phase 3)
- Change any existing `create_draft` call sites (they pass None implicitly)

---

## Task 8: Journal Repo — Import Queries

**Context:** Read `src/db/journal_repo.rs`, `specs/v2/v2-data-model.md` (import_ref query patterns)

**Action:**
1. Add `get_recent_import_refs(days: i64) -> Result<HashSet<String>>` — returns all non-null import_ref values from the last N days
2. Add `get_incomplete_imports() -> Result<Vec<JournalEntry>>` — drafts with import_ref that have fewer than 2 lines with non-null account_id

**Verify:**
- `get_recent_import_refs(90)` returns refs from last 90 days, excludes older
- `get_recent_import_refs(90)` returns empty set when no imports exist
- `get_incomplete_imports()` returns drafts with import_ref and incomplete lines
- `get_incomplete_imports()` excludes posted entries
- `get_incomplete_imports()` excludes drafts without import_ref
- `get_incomplete_imports()` excludes drafts where all lines have accounts

**Do NOT:**
- Implement duplicate detection logic (that's in csv_import.rs, Phase 3)
- Implement re-match logic (Phase 3)

---

## Task 9: Envelopes Tab — Replace Tab with V

**Context:** Read `src/tabs/envelopes.rs`, search for `KeyCode::Tab` usage

**Action:**
1. Find the `KeyCode::Tab` handler in the Envelopes tab that toggles between Allocation Config and Balances views
2. Replace `KeyCode::Tab` with `KeyCode::Char('v')`
3. Update any in-code comments or help text that reference Tab for view switching
4. Verify the view toggle works with `v` in both sub-views

**Verify:**
- Pressing `v` in Allocation Config view → switches to Balances view
- Pressing `v` in Balances view → switches to Allocation Config view
- Pressing Tab in Envelopes tab → does nothing (no handler)
- All existing Envelopes tests pass

**Do NOT:**
- Add any chat panel logic
- Modify `app.rs` key dispatch

---

## Task 10: JE Form — Arrow Key Navigation

**Context:** Read `src/widgets/je_form.rs`, search for `KeyCode::Tab` usage

**Action:**
1. Audit all `KeyCode::Tab` usage in `je_form.rs` — document what Tab currently does (expected: advance to next field)
2. Add `KeyCode::Down` / `KeyCode::Up` handlers to navigate between form lines (rows)
3. Add `KeyCode::Left` / `KeyCode::Right` handlers to navigate between columns within a line (date, memo, account, debit, credit)
4. Add `KeyCode::Enter` handler to confirm current field and advance to the next (same behavior as Tab)
5. Tab key behavior remains unchanged — it still works for field navigation
6. Arrow key navigation must not conflict with any existing arrow key usage in the form (e.g., scrolling within a text input field — check carefully)

**Verify:**
- Arrow Down from date field → moves to first line's account field (or appropriate next row)
- Arrow Up reverses direction
- Arrow Left/Right moves between columns within a line
- Enter confirms and advances (same as Tab)
- Tab still works as before
- Account picker (embedded in JE form) arrow keys still work when picker is active
- All existing JE form tests pass

**Do NOT:**
- Remove Tab handling from the form (Tab stays)
- Add chat panel focus interception (that's Phase 2, in app.rs)
- Change any behavior when account picker overlay is active

---

## Task 11: Audit Repo — AI Entry Convenience Methods

**Context:** Read `src/db/audit_repo.rs`, `specs/v2/v2-data-model.md` (audit_log new action types)

**Action:**
1. Add convenience methods to `AuditRepo` for the new action types:
   - `log_ai_prompt(description: &str) -> Result<()>` — truncates to 500 chars
   - `log_ai_response(summary: &str) -> Result<()>`
   - `log_ai_tool_use(tool_name: &str, key_params: &str) -> Result<()>` — formats as "Used {tool}({params})"
   - `log_csv_import(bank_name: &str, total: usize, matched: usize, ai_matched: usize, manual: usize) -> Result<()>`
   - `log_mapping_learned(description: &str, account_number: &str, account_name: &str, source: &str) -> Result<()>`
2. Add `get_ai_entries(start_date: Option<NaiveDate>, end_date: Option<NaiveDate>, limit: Option<usize>) -> Result<Vec<AuditEntry>>` — filters for AiPrompt/AiResponse/AiToolUse actions
3. These methods wrap the existing `append()` or similar method with the correct action variant

**Verify:**
- `log_ai_prompt` with 600-char input → stored description is 500 chars
- `log_ai_prompt` with 400-char input → stored as-is
- `log_ai_tool_use("get_account", "5100")` → description is "Used get_account(5100)"
- `log_csv_import` → correctly formatted description
- `get_ai_entries` filters correctly by action type and date range
- `get_ai_entries` with no filters returns all AI entries
- Existing audit tests unchanged

**Do NOT:**
- Modify the audit_log schema
- Change how existing audit entries are written
- Add any AI logic — these are just logging helpers

---

## Task 12: CSV Parser Stub [TEST-FIRST]

**Context:** Read `specs/v2/v2-architecture.md` (csv_import.rs), `specs/v2/v2-type-system.md` (NormalizedTransaction, Debit/Credit Mapping Algorithm)

**Action:**
1. Add `csv` crate to `Cargo.toml`
2. In `src/ai/csv_import.rs`, implement:
   - `parse_csv(file_path: &Path, bank_config: &BankAccountConfig) -> Result<Vec<NormalizedTransaction>>` — read CSV, map columns, normalize amounts, generate import_refs
   - `check_duplicates(transactions: &[NormalizedTransaction], existing_refs: &HashSet<String>) -> (Vec<NormalizedTransaction>, Vec<NormalizedTransaction>)` — returns (unique, duplicates)
   - `determine_debit_credit(amount: Money, linked_account_type: AccountType) -> (Money, Money, bool)` — returns (debit_amount, credit_amount, bank_side_is_debit). Implements the debit/credit mapping algorithm from the type system spec.
3. Handle both single-amount and split-column CSV formats
4. Handle date parsing using the `date_format` from bank config
5. Generate `import_ref` as `"{bank_name}|{date}|{description}|{amount}"`

**Verify:**
- Parse single-amount CSV (negative = withdrawal) → correct NormalizedTransactions
- Parse split debit/credit CSV → correct NormalizedTransactions
- Date parsing with `%m/%d/%Y` format → correct NaiveDate
- Date parsing with `%Y-%m-%d` format → correct NaiveDate
- `import_ref` format correct
- `check_duplicates` identifies matching refs and separates them
- `check_duplicates` with no duplicates → all in unique list
- Debit/credit for Asset account + positive amount → debit bank, credit other
- Debit/credit for Asset account + negative amount → credit bank, debit other
- Debit/credit for Liability account + positive amount → credit bank, debit other
- Debit/credit for Liability account + negative amount → debit bank, credit other
- Amounts are always positive in the returned debit/credit values
- Malformed CSV row → skipped with warning, not fatal
- Empty CSV → empty vec, no error

**Do NOT:**
- Implement Pass 1 matching (needs repo, deferred to Phase 3)
- Implement Pass 2/3 (needs AI client, deferred to Phase 3)
- Implement bank format detection (needs AI client, deferred to Phase 3)
- Implement draft creation (Phase 3)
- Add the `csv` crate's async features (if any)

---

## Developer Review Gate

Before proceeding to Phase 2:
1. Run full verification: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
2. Confirm all 12 tasks committed (one commit per task)
3. Review the new `src/ai/` module structure
4. Review `ImportMappingRepo` query correctness (especially substring matching)
5. Confirm backwards compatibility: existing `workspace.toml` without `[ai]` section works
6. Confirm existing Envelopes Tab behavior preserved (just `v` instead of Tab)
7. Confirm JE form arrow key navigation doesn't break account picker
8. Count total tests — should be 372 + new tests from this phase
