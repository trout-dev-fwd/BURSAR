# V2 Phase 3 — CSV Import Pipeline

## Overview

Build the full CSV import workflow: `U` hotkey, import wizard modals, bank format detection, three-pass matching pipeline, review screen, draft creation, and re-match capabilities.

**Tasks:** 13
**Depends on:** V2 Phase 2 complete (AI client, chat panel, tool use all working)
**Produces:** Complete bank statement import with AI-assisted transaction matching

---

## Completion Criteria

- [ ] `U` in JE tab opens import wizard
- [ ] File path input with tilde expansion and last_import_dir default
- [ ] Bank selection from entity toml, with "New" option
- [ ] New bank: name input → Claude format detection → column confirmation → account picker
- [ ] "Initializing ↻" during format detection, "Failed ⨂" on timeout
- [ ] Duplicate detection against last 90 days of imports
- [ ] Pass 1: local matching against import_mappings ("Importing ☺ N/M")
- [ ] Pass 2: AI matching in batches of 25 via tool use
- [ ] Pass 3: clarification dialog in chat panel for low-confidence items
- [ ] Review screen with grouped matches, detail pane, `r` to reject
- [ ] Enter to approve → batch draft creation with correct debit/credit
- [ ] All drafts have import_ref for traceability
- [ ] AI failure fallback: one-sided drafts for unmatched items
- [ ] Shift+U re-matches incomplete imports
- [ ] `/match` re-matches single selected draft
- [ ] Learned mappings saved to import_mappings table
- [ ] last_import_dir updated in entity toml
- [ ] CsvImport and MappingLearned audit log entries
- [ ] Help overlay updated with U, Shift+U, /match
- [ ] All existing tests pass plus new tests
- [ ] `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test` clean

## Does NOT Cover

- Multiple CSV import in one session (one import at a time)
- Editing bank format configs after creation (manual toml editing)
- Import mapping management UI (future feature)
- CSV export

---

## Task 1: Import Flow State and TabAction Extension

**Context:** Read `src/tabs/mod.rs` (TabAction enum), `specs/v2/v2-type-system.md` (ImportFlowStep)

**Action:**
1. Define `ImportFlowStep` enum in `src/ai/csv_import.rs` (all variants from type system spec)
2. Define `ImportFlowState` struct to track the full wizard state:
   ```rust
   struct ImportFlowState {
       step: ImportFlowStep,
       file_path: Option<PathBuf>,
       bank_config: Option<BankAccountConfig>,
       is_new_bank: bool,
       new_bank_name: Option<String>,
       detected_config: Option<BankAccountConfig>,
       transactions: Vec<NormalizedTransaction>,
       duplicates: Vec<NormalizedTransaction>,
       matches: Vec<ImportMatch>,
       input_buffer: String,
       selected_index: usize,
       scroll_offset: usize,
   }
   ```
3. Add `TabAction::StartImport` variant to `TabAction` enum
4. Add `import_flow: Option<ImportFlowState>` field to `App`

**Verify:**
- `ImportFlowStep` enum compiles with all variants
- `ImportFlowState` struct holds all necessary state
- `TabAction::StartImport` dispatches correctly in App (placeholder — just creates the state)
- Existing TabAction handling unchanged

**Do NOT:**
- Implement any modal rendering yet
- Wire up the full flow yet

---

## Task 2: Import Wizard — File Path Input Modal

**Context:** Read `src/widgets/confirmation.rs` (for modal rendering pattern), `src/config.rs` (tilde expansion)

**Action:**
1. In `App`, when `import_flow` is `Some` and step is `FilePathInput`:
   - Render a centered modal with title "Import CSV Statement"
   - Text input for file path
   - Pre-fill from entity_toml.last_import_dir (if set), appending `/` so user just types filename
   - Tilde expansion on the entered path
   - Enter → validate file exists
     - If exists: update `last_import_dir` in entity toml, save, advance to BankSelection
     - If not: show "File not found" error in modal, stay on FilePathInput
   - Escape → cancel import (set `import_flow = None`)
2. Handle key dispatch: when `import_flow` is Some, the modal handles keys before anything else (same priority as other modals)

**Verify:**
- Modal renders centered with input field
- Pre-filled path from last_import_dir
- Tilde expansion works (`~/Downloads/file.csv` resolves correctly)
- Valid file → advances to next step
- Invalid file → error message, stays on input
- Escape → cancels, returns to JE list
- `last_import_dir` updated in entity toml after successful file selection

**Do NOT:**
- Implement file path tab-completion
- Validate CSV format at this step (just check file exists)

---

## Task 3: Import Wizard — Bank Selection Modal

**Context:** Read `src/config.rs` (entity toml loading), entity toml bank_accounts structure

**Action:**
1. When step is `BankSelection`:
   - Render modal with title "Select Bank Account"
   - List all `bank_accounts` from entity toml by name
   - Add "➕ New Bank Account" as last option
   - Arrow keys to navigate, Enter to select
   - Default selection: first bank account (or last used, if tracked)
   - On select known bank: store the `BankAccountConfig`, advance to DuplicateCheck (via parsing step)
   - On select "New": advance to NewBankName
   - Escape → cancel import
2. If entity toml has no bank_accounts, skip selection and go directly to NewBankName

**Verify:**
- Lists all configured bank accounts
- "New Bank Account" option always present at bottom
- Arrow keys navigate, Enter selects
- Known bank → proceeds with stored config
- New → advances to name input
- Empty bank list → goes directly to new bank flow
- Escape cancels

**Do NOT:**
- Allow editing existing bank configs
- Allow deleting bank configs

---

## Task 4: Import Wizard — New Bank Setup (Name + Detection)

**Context:** Read `src/ai/client.rs`, `src/ai/csv_import.rs`, `specs/v2/v2-type-system.md` (Import Flow)

**Action:**
1. When step is `NewBankName`:
   - Render modal with title "New Bank Account"
   - Text input: "Bank account name:"
   - Enter → store name, advance to NewBankDetection
   - Escape → cancel import
2. When step is `NewBankDetection`:
   - Render "Initializing ↻" in modal
   - Read first 4 lines of CSV file (header + 3 data rows)
   - Lazy-init AiClient if needed
   - Send to Claude with format detection prompt:
     "Analyze this CSV header and sample rows from a bank statement. Identify: date column name, date format (chrono-compatible like %m/%d/%Y), description/memo column name, and either a single amount column (with sign convention) or separate debit/credit columns. Respond in JSON only with fields: date_column, date_format, description_column, amount_column (or null), debit_column (or null), credit_column (or null), debit_is_negative (boolean)."
   - Parse response into `BankAccountConfig` (with name from previous step, linked_account empty for now)
   - On success → advance to NewBankConfirmation
   - On timeout/error → show "Failed ⨂" in modal, Enter to retry, Escape to cancel

**Verify:**
- Name input renders and captures text
- "Initializing ↻" displays during API call
- Successful detection → parsed config displayed in next step
- Timeout → "Failed ⨂" displayed
- Escape cancels at any point

**Do NOT:**
- Save the bank config yet (happens after account picker)
- Parse the full CSV yet (just first 4 lines for detection)

---

## Task 5: Import Wizard — Confirmation + Account Picker

**Context:** Read `src/widgets/account_picker.rs`, entity toml save functions

**Action:**
1. When step is `NewBankConfirmation`:
   - Render modal showing detected column mapping:
     ```
     Detected Format:
       Date:        "Date" (MM/DD/YYYY)
       Description: "Description"
       Amount:      "Amount" (negative = withdrawal)
     
     Is this correct? [Y/N]
     ```
   - Y → advance to NewBankAccountPicker
   - N → go back to NewBankDetection (retry) or cancel
2. When step is `NewBankAccountPicker`:
   - Render the existing `AccountPicker` widget with prompt "Which account is this bank account?"
   - On selection: complete the `BankAccountConfig` with linked_account
   - Save new `[[bank_accounts]]` entry to entity toml
   - Advance to DuplicateCheck (which triggers parsing)

**Verify:**
- Confirmation modal shows detected columns clearly
- Y advances, N retries or cancels
- AccountPicker works within the import modal context
- Selected account number stored in config
- Entity toml updated with new bank_accounts entry
- Saved config round-trips correctly

**Do NOT:**
- Save to workspace.toml (entity toml only)
- Allow editing the detected columns manually (rely on Claude or retry)

---

## Task 6: Duplicate Detection Step

**Context:** Read `src/ai/csv_import.rs` (parse_csv, check_duplicates from Phase 1), `src/db/journal_repo.rs` (get_recent_import_refs)

**Action:**
1. On entering DuplicateCheck step (from bank selection or account picker):
   - Parse the full CSV using `parse_csv()` with the selected/created bank config
   - Call `get_recent_import_refs(90)` from journal repo
   - Call `check_duplicates()` to split transactions
   - If duplicates found and duplicates.len() > 0:
     - Render DuplicateWarning modal: "{N} of {M} transactions appear to already be imported. Skip duplicates?"
     - Y → proceed with unique transactions only
     - N → proceed with all transactions
   - If no duplicates → proceed directly to Pass1Matching
2. Store the transaction list in `ImportFlowState.transactions`

**Verify:**
- CSV parsed correctly into NormalizedTransactions
- Duplicates identified against recent import_refs
- Warning shown only when duplicates exist
- Y skips duplicates, N includes all
- No duplicates → skips directly to Pass 1
- CSV parse error → show error, cancel import

**Do NOT:**
- Implement matching yet (next tasks)
- Create any drafts yet

---

## Task 7: Pass 1 — Local Matching

**Context:** Read `src/db/import_mapping_repo.rs`, `specs/v2/v2-type-system.md` (Pass 1 Matching Algorithm)

**Action:**
1. Implement `run_pass1(transactions: &[NormalizedTransaction], bank_name: &str, db: &EntityDb) -> Vec<ImportMatch>` in `csv_import.rs`:
   - For each transaction: try exact match, then substring match via ImportMappingRepo
   - On match: create `ImportMatch` with `MatchSource::Local`, record use on the mapping
   - On no match: create `ImportMatch` with `MatchSource::Unmatched`
2. When step is Pass1Matching:
   - Display "Importing ☺ {completed}/{total}" in the modal, updated as each transaction is processed
   - Run Pass 1 synchronously (this is local DB queries, should be fast)
   - Store results in `ImportFlowState.matches`
   - If all matched → advance to ReviewScreen
   - If unmatched exist AND API key available → advance to Pass2AiMatching
   - If unmatched exist AND no API key → advance to ReviewScreen (unmatched become one-sided)

**Verify:**
- Exact match found → MatchSource::Local, correct account
- Substring match found → MatchSource::Local, correct account
- No match → MatchSource::Unmatched
- Exact match takes priority over substring
- Progress indicator updates correctly
- All matched → goes to review
- Unmatched with API → goes to Pass 2
- Unmatched without API → goes to review

**Do NOT:**
- Implement AI matching (next task)
- Create drafts (later task)

---

## Task 8: Pass 2 — AI Matching

**Context:** Read `src/ai/client.rs`, `src/ai/tools.rs`, `specs/v2/v2-type-system.md` (tool use)

**Action:**
1. When step is Pass2AiMatching:
   - Auto-open chat panel if not visible
   - Collect all unmatched items from `ImportFlowState.matches`
   - Batch into groups of 25
   - For each batch:
     - Build prompt: "Match these bank transactions to accounts. Use tools to search accounts, review GL history, and check envelope balances. For each transaction, respond with JSON: account_number, confidence (high/medium/low), reasoning (one sentence). Flag any that would exceed envelope allocations."
     - Include transaction list in the prompt: date, description, amount for each
     - Send via `send_with_tools()` (Claude uses tools to look up accounts, etc.)
     - Parse response: extract matches with confidence and reasoning
     - Update `ImportMatch` entries with matched account, source = `Ai`, confidence, reasoning
     - Display progress in chat: "Matching transactions... {completed}/{total}"
   - On API failure at any point: remaining unmatched stay as `MatchSource::Unmatched`
   - After all batches: if any Low confidence items → advance to Pass3Clarification, else → ReviewScreen

**Verify:**
- Unmatched items batched correctly (max 25 per batch)
- Chat panel opens automatically
- Progress displayed in chat
- Successful matches update the ImportMatch entries
- Confidence levels parsed correctly
- API failure → remaining items stay unmatched, flow continues
- All high/medium → skips Pass 3
- Some low → goes to Pass 3

**Do NOT:**
- Create drafts yet
- Write mappings to DB yet (happens at draft creation)

---

## Task 9: Pass 3 — Clarification Dialog

**Context:** Read `src/widgets/chat_panel.rs`, `specs/v2/v2-type-system.md` (Import Flow — Pass3Clarification step)

**Action:**
1. When step is Pass3Clarification:
   - Collect all Low confidence items
   - For each item, display in chat panel:
     ```
     Transaction: 2026-03-15 | HOME DEPOT #4847 | -$247.32
     Best guess: 5000 - Repairs & Maintenance (Low confidence)
     Reason: Vendor name suggests repairs, but could also be supplies.
     
     Confirm this match? Type an account number/name to redirect, or 'skip' to leave unmatched.
     ```
   - Wait for user input in chat panel:
     - User types account number/name → search accounts, update match, source = UserConfirmed
     - User types "confirm" or "y" → accept Claude's suggestion, source = UserConfirmed
     - User types "skip" or "s" → leave as Unmatched
   - Advance through each low-confidence item sequentially
   - After all resolved → advance to ReviewScreen
2. Write confirmed mappings to `import_mappings` table immediately (so they're available for future imports even if the user cancels the review)

**Verify:**
- Each low-confidence item presented in chat
- User can confirm Claude's suggestion
- User can redirect to a different account
- User can skip
- Confirmed items update match and source
- Mappings written to import_mappings table
- After all items → advances to review

**Do NOT:**
- Allow batch operations on clarification items (one at a time)
- Modify the chat panel's normal conversation flow (these are interleaved)

---

## Task 10: Review Screen

**Context:** Read `src/tabs/journal_entries.rs` (for JE list + detail rendering pattern), `specs/v2/v2-architecture.md` (Render Layout — review screen)

**Action:**
1. When step is ReviewScreen:
   - Take over the full tab area (not a small modal — this is a full-screen view)
   - Render a scrollable list of all matches grouped by source:
     - Header: "Import Review — {bank_name} — {total} transactions"
     - Section: "Auto-Matched ({count})" — dimmed, collapsed by default
     - Section: "AI-Matched ({count})" — each row shows: description → account (confidence) [reasoning]
     - Section: "User-Confirmed ({count})" — each row shows: description → account
     - Section: "Unmatched ({count})" — each row shows: description → (bank account only)
   - Arrow keys to navigate, Enter to expand/collapse sections
   - Bottom detail pane (when a match is highlighted): show the proposed draft JE with both lines, debit/credit amounts, import_ref, and memo
   - `r` key on an AI-matched item → reject it (moves to Unmatched, reasoning cleared)
   - Enter on the header bar or a dedicated "Approve All" prompt → advance to Creating
   - Escape → cancel entire import (no drafts created, but learned mappings from Pass 3 are kept)
2. Apply debit/credit rules for the detail pane preview using `determine_debit_credit()` from Phase 1

**Verify:**
- All match groups displayed with correct counts
- Navigation between items works
- Detail pane shows correct JE preview (debit/credit correct for asset and liability accounts)
- `r` rejects AI-matched item (moves to Unmatched)
- Cannot reject auto-matched or user-confirmed items
- Approve → advances to creation
- Escape → returns to JE tab with no drafts created

**Do NOT:**
- Create drafts (next task)
- Allow editing individual match details (just approve or reject)

---

## Task 11: Draft Creation

**Context:** Read `src/db/journal_repo.rs` (create_draft), `specs/v2/v2-type-system.md` (Debit/Credit Mapping Algorithm)

**Action:**
1. When step is Creating:
   - Wrap entire batch in a database transaction
   - For each approved match (not rejected, not skipped-to-unmatched-by-user-in-review):
     - Determine debit/credit using `determine_debit_credit(amount, linked_account_type)`
     - Build draft JE:
       - Date: transaction date
       - Memo: "Import: {description}" (truncated to 200 chars if longer)
       - import_ref: transaction's import_ref string
       - Line 1: bank account side (always filled)
       - Line 2: matched account side (filled if matched, blank account_id if unmatched)
     - Call `create_draft()` with import_ref
   - For unmatched items:
     - Create one-sided draft (only bank account line, no second line, or second line with null account)
   - Write AI-suggested mappings (high/medium confidence, not rejected) to import_mappings table:
     - description_pattern = transaction description
     - match_type = Exact
     - source = AiSuggested
   - On transaction success:
     - Log CsvImport to audit_log
     - Log MappingLearned for each new mapping
     - Set step to Complete
   - On transaction failure:
     - Rollback entire batch
     - Show error in status bar
     - Return to ReviewScreen

**Verify:**
- Drafts created with correct debit/credit amounts
- Asset account deposit → debit bank, credit other
- Asset account withdrawal → credit bank, debit other
- Liability account charge → credit liability, debit expense
- Liability account payment → debit liability, credit other
- All amounts positive in debit/credit columns
- import_ref stored on each draft
- Memo formatted correctly, truncated if too long
- Unmatched items create one-sided drafts
- AI-suggested mappings written to import_mappings
- Audit log entries created
- Transaction rollback on failure

**Do NOT:**
- Auto-post any entries (drafts only, always)
- Create entries for rejected items

---

## Task 12: Batch Re-Match (Shift+U) and /match Completion

**Context:** Read `src/db/journal_repo.rs` (get_incomplete_imports), `src/ai/csv_import.rs`

**Action:**
1. Add `Shift+U` handler in JE tab:
   - Call `get_incomplete_imports()` from journal repo
   - If empty: show "No incomplete imports to re-match" in status bar
   - If found:
     - Auto-open chat panel
     - Collect transaction info from each draft's import_ref (parse back into description, amount, etc.)
     - Run through Pass 2 → Pass 3 → ReviewScreen flow (reusing existing pipeline)
     - On approval: update existing drafts in-place (use update_draft from the edit feature)
2. Complete the `/match` slash command (stubbed in Phase 2):
   - Get the currently selected JE from the JE tab
   - Validate: is draft, has import_ref, has incomplete lines
   - Parse import_ref back into transaction description
   - Send single-item match request via send_with_tools
   - Display Claude's suggestion in chat
   - On user confirmation: update draft, write mapping
   - On skip: no change

**Verify:**
- Shift+U with no incomplete imports → status bar message
- Shift+U with incomplete imports → re-match flow starts
- Re-matched drafts updated in-place (not duplicated)
- `/match` with valid draft → sends to Claude, shows suggestion
- `/match` with non-draft → error message
- `/match` with draft without import_ref → error message
- `/match` confirmed → draft updated, mapping written
- Import_ref parsing handles the composite format correctly

**Do NOT:**
- Create new drafts (update existing ones)
- Allow re-matching posted entries

---

## Task 13: Help Overlay + Final Polish

**Context:** Read `src/app.rs` (help overlay)

**Action:**
1. Add to Journal Entries tab section in help overlay:
   ```
   U            Import CSV statement
   Shift+U      Re-match incomplete imports
   ```
2. Add `/match` to Chat Panel section:
   ```
   /match       Re-match selected draft
   ```
3. Add "Importing ☺", "Initializing ↻", "Failed ⨂" to status bar rendering (if not already handled by modal rendering)
4. Final review pass:
   - Verify all import flow steps render correctly at various terminal sizes
   - Verify chat panel + import flow interactions (Pass 2/3 use chat panel while import modal may be in background)
   - Verify Escape cancels cleanly at every step
   - Verify audit log entries for the complete import flow

**Verify:**
- Help overlay shows all new hotkeys and commands
- All status messages render correctly
- Full import flow works end-to-end (manual test with sample CSV)
- All tests pass
- `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test` clean

---

## Developer Review Gate

Before declaring V2 Phase 1 complete:
1. Run full verification: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
2. **Full manual test with real bank statement:**
   - Import a SoFi or Alliant CSV as a new bank (format detection)
   - Verify column mapping detection
   - Verify Pass 1 matches (after second import with learned mappings)
   - Verify Pass 2 AI matching
   - Verify Pass 3 clarification dialog
   - Review screen: verify grouping, detail pane, reject function
   - Approve → verify drafts created with correct debit/credit
   - Shift+U → verify re-match of incomplete imports
   - `/match` → verify single-draft re-match
3. **Import the same CSV again** → verify duplicate detection
4. **Test AI failure scenario** (disconnect network or use invalid key):
   - New bank detection → "Failed ⨂"
   - Pass 2 → fallback to one-sided drafts
5. Review learned mappings in import_mappings table
6. Review audit log for complete import trail
7. Count total tests across all three phases
8. Review overall code quality and consistency with V1 patterns
