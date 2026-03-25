# V4 Progress Tracker

## Current State
- **Active Phase**: Phase 4 complete
- **Last Completed Task**: Phase 4, Task 3
- **Next Task**: Phase 5, Task 1
- **Blockers**: None

## Completed Phases

### Phase 1 — Tab Restructuring + Schema (2026-03-24)
All 5 tasks committed. Verification: `cargo fmt && cargo clippy -D warnings && cargo test` all pass (744 tests).

| Task | Commit | Description |
|------|--------|-------------|
| 1 | 6ce306f | Move Audit Log tab to position 0 |
| 2 | d00a30d | tax_tags table, enums, and TaxTagRepo |
| 3 | 93b9003 | Create Tax tab shell at position 9 |
| 4 | d25babd | Form configuration screen and entity TOML |
| 5 | 9894484 | Memo editing on JE tab and hide per-line memo |

## Completed Phases (continued)

### Phase 2 — Tax Reference Library (2026-03-24)
All 3 tasks committed. Verification: `cargo fmt && cargo clippy -D warnings && cargo test` all pass (760 tests).

| Task | Commit | Description |
|------|--------|-------------|
| 1 | (see git log) | tax_reference schema, scraper dependency, TaxRefRepo |
| 2 | (see git log) | HTML fetcher and parser for IRS publications |
| 3 | (see git log) | Wire tax reference ingestion to `u` hotkey |

## Completed Phases (continued)

### Phase 3 — Tax Review Workflow (2026-03-25)
All 3 tasks committed. Verification: `cargo fmt && cargo clippy -D warnings && cargo test` all pass (765 tests).

| Task | Commit | Description |
|------|--------|-------------|
| 1 | (see git log) | Tax tab JE list view with status and fiscal year selector |
| 2 | (see git log) | Manual flagging, reason input, and memo editing |
| 3 | (see git log) | Tax Form Guide in user guide and help overlay updates |

## Completed Phases (continued)

### Phase 4 — AI Batch Review (2026-03-25)
All 3 tasks committed. Verification: `cargo fmt && cargo clippy -D warnings && cargo test` all pass (791 tests).

| Task | Commit | Description |
|------|--------|-------------|
| 1 | 2790a1f | Tax-scoped AI context, tax tag tool, and keyword extraction |
| 2 | 3949512 | AI batch classification with R hotkey and prompt caching |
| 3 | 465fb14 | AI suggestion review — accept, override, reject |

## Current Phase Progress
_(Phase 4 complete — Phase 5 not started)_

## Decisions & Discoveries

- **[Pre-implementation]**: Tax tab at position 9, Audit Log moved to 0.

- **[Pre-implementation]**: All tax forms enabled by default. Users disable via `c` config screen.

- **[Pre-implementation]**: Renamed "Not Taxable" to "Non-Deductible" — clearer terminology.
  Tag name: `non_deductible`. Display: "Non-Deductible".

- **[Pre-implementation]**: Per-line `line_memo` hidden from JE UI. JE-level `memo` is the sole
  description. `m` key for memo editing available on both JE tab and Tax tab.

- **[Pre-implementation]**: `reason` column added to `tax_tags`. Stores AI explanation or user's
  manual note. Included in Tax Summary report for accountant context.

- **[Pre-implementation]**: AI batch response uses pipe-separated format, not JSON/XML:
  `JE-0004: schedule_c | Office supplies are ordinary business expenses`
  Saves tokens, more reliable parsing than JSON from LLM output.

- **[Pre-implementation]**: Prompt caching enabled for batch review system prompt. Same
  `anthropic-beta: prompt-caching-2024-07-31` pattern as chat panel.

- **[Pre-implementation]**: `f` and `n` keys work on ANY status. Re-flagging always allowed.

- **[Pre-implementation]**: Tax Form Guide lives in Ctrl+H user guide, not `?` overlay.
  `?` overlay shows `Ctrl+H` as "Open user guide (& form guide)".

- **[Pre-implementation]**: Per-JE tagging only. Split Draft feature (for mixed business/personal
  JEs) deferred to a future version. Workaround: split at draft stage before posting.

- **[Pre-implementation]**: Tax reference context only in AI chat from Tax tab. Other tabs
  get normal accounting AI context.

- **[Pre-implementation]**: Highlighted JE's tax tag (form, status, reason) auto-included in
  Tax tab AI context. User can ask about any JE by number — Claude uses `get_tax_tag` tool
  to fetch non-highlighted JEs' classifications.

- **[Pre-implementation]**: New `get_tax_tag` AI tool for Tax tab. Read-only, returns form_tag,
  status, reason, ai_suggested_form for any JE number. Consistent with existing tool pattern.

- **[Pre-implementation]**: Keyword mapping table includes form names (e.g., "Schedule C" maps
  to `small_business,business_expense` tags) so asking about a specific form pulls the right
  IRS reference chunks.

- **[Phase 1, Task 1]**: Tab key cycling uses `'0'..='9'` range. `0` → AuditLog (idx 0), `9` → Tax (idx 9).
  Help overlay updated from "1–9" to "0–9".

- **[Phase 1, Task 2]**: `TaxFormTag` has 14 variants (13 IRS forms + `NonDeductible`).
  `TaxTagRepo` uses UPSERT for `set_manual` and `set_non_deductible` — re-flagging always
  overwrites regardless of current status. 19 tests covering all CRUD and status transitions.

- **[Phase 1, Task 3]**: `TaxTab::set_enabled_forms_from_strings()` is a non-trait public method
  (not added to Tab trait). Called optionally during entity load to restore persisted config.
  Tab starts with all-enabled default when config absent.

- **[Phase 1, Task 4]**: `TaxConfig { enabled_forms: Option<Vec<String>> }` added to
  `EntityTomlConfig`. `TabAction::SaveTaxFormConfig(Vec<String>)` handled in `key_dispatch.rs`
  via entity TOML load → update → save pattern, same as other entity config actions.

- **[Phase 1, Task 5]**: `Focus::LineNote` removed entirely from `je_form.rs`. Tab order is now
  Account → Debit → Credit (wraps to next row). Note column hidden from both JE form and detail
  view. `note_input`/`line_memo` data preserved in storage and roundtripped via `from_existing`.
  `m` key opens `TextInputModal` pre-filled with current memo; works on any selected entry.

## Decisions & Discoveries (Phase 2)

- **[Phase 2, Task 2]**: `split_by_heading_level` uses a `while let` loop (not `loop/break`) per
  clippy's `while_let_loop` lint. Uses lowercase comparison for HTML tag matching to handle
  mixed-case tags from real IRS pages.

- **[Phase 2, Task 2]**: `scraper::Html::parse_fragment` used for tag stripping — returns clean
  normalized text. Called per-section, not per-document, so performance is acceptable.

- **[Phase 2, Task 3]**: Transaction isolation: network fetching (Phase 1) and DB writes (Phase 2)
  are separated so `terminal.draw()` can be called between fetches without borrow conflicts.
  The closure-based transaction block lets `conn` borrow end before final `set_message` call.

- **[Phase 2, Task 3]**: `chrono::Local::now().year()` returns `i32` directly (no cast needed).
  The `tax_year` column in `tax_reference` is INTEGER, stored as `i32`.

## Decisions & Discoveries (Phase 3)

- **[Phase 3, Task 1]**: `PostedJeWithTag` struct added to `tax_tag_repo.rs`. Uses LEFT JOIN from
  `journal_entries` so all posted JEs appear (tag=None means Unreviewed). `list_all_posted_for_date_range`
  collects raw row data first, then parses dates/enums post-collection — avoids rusqlite closure type
  constraints.

- **[Phase 3, Task 1]**: `selected_row()` not added in Task 1 to avoid dead_code warning. Added in Task 2
  when the flagging keys were implemented.

- **[Phase 3, Task 2]**: `TaxModal` enum covers FormPicker, FlagReason, NonDeductibleReason, MemoEdit.
  `TaxDetailState` is a separate field (not a modal) — it splits the area vertically like the JE tab's
  detail panel. Detail + modal can't coexist; modals have priority in handle_key dispatch.

- **[Phase 3, Task 2]**: Detail panel navigation (↑/↓) intercepts keys before base hotkeys when detail
  is open, but `f`, `n`, `a`, `m` fall through — flagging works even while detail is visible.

- **[Phase 3, Task 3]**: Guide's "Tab 9: Audit Log" header corrected to "Tab 0: Audit Log" (was missed in
  Phase 1 guide update). Global Controls table updated from `1`–`9` to `0`–`9`. Ctrl+H description updated
  in both the guide and the `?` overlay.

## Decisions & Discoveries (Phase 4)

- **[Phase 4, Task 1]**: `KEYWORD_MAP` has 24 entries covering topic terms and form names. Both
  paths tested: topic keywords pull topic tags, form name keywords (e.g., "schedule c") map to
  multi-topic strings like `small_business,business_expense`. `build_tax_context` returns `None`
  when tax_refs table is empty and no selected JE — avoids empty system prompt injection.

- **[Phase 4, Task 2]**: `TaxReviewStatus` import removed from `tax_handler.rs` (unused).
  `TAX_TAB_INDEX` const removed (dead code). Used `audit().append()` not `.log()` for audit
  logging — AuditRepo method is `append`. `send_cached_simple` added to AiClient for single-round
  cached requests without tool use.

- **[Phase 4, Task 3]**: `Enter` on `ai_suggested` JEs accepts suggestion (copies
  `ai_suggested_form` → `form_tag`, status → confirmed, reason preserved). `ai_suggested_form`
  is NOT in the UPSERT SET clause for `set_manual` or `set_non_deductible`, so it's automatically
  preserved as an audit trail after any override. Three new tests added to tax_tag_repo.rs:
  accept_preserves_reason, override_preserves_ai_suggested_form, re_flag_preserves_ai_suggested_form.

## Known Issues
- None currently.
