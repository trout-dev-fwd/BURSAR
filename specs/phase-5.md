# Phase 5: Reports, Recurring Entries, Startup Checks

**Goal**: All 8 reports with box-drawing formatting, recurring entry templates and generation,
startup sequence (recurring prompts, depreciation prompts, inter-entity recovery structure),
audit log tab, and help overlay.

**Depends on**: Phase 4 (all domain features that reports need to query).

**Estimated tasks**: 14

---

## Tasks

### Task 1: Create report formatting utilities **[TEST-FIRST]**
**Context**: `specs/architecture.md` (Report trait section), feature spec Section 12 (report formatting).
**Action**: Create `src/reports/mod.rs`. Define:
- `Report` trait: `fn name(&self) -> &str`, `fn generate(&self, db: &EntityDb, params: &ReportParams) -> Result<String>`
- `ReportParams` struct: entity_name, as_of_date, date_range, account_id
- Shared formatting functions:
  - Box-drawing character constants
  - `format_header(entity: &str, title: &str, date_info: &str) -> String`
  - `format_table(headers: &[&str], rows: &[Vec<String>], alignments: &[Align]) -> String`
  - `format_money(amount: Money) -> String` — right-aligned, 2 decimal places
  - Column width auto-calculation
- File output: `write_report(content: &str, name: &str, output_dir: &Path) -> Result<PathBuf>`
  — writes to `{output_dir}/{ReportName}{MM-DD-YYYY}.txt`
**Verify**: `format_header` produces correct box-drawn header matching feature spec example.
`format_table` aligns columns correctly. `format_money` right-aligns amounts.
**Do NOT**: Implement any specific report yet (Tasks 2-9).

---

### Task 2: Implement Trial Balance report
**Context**: `src/reports/mod.rs`, `src/db/account_repo.rs`.
**Action**: Create `src/reports/trial_balance.rs`. Lists all accounts with balances at a
point in time. Final row: total debits, total credits. Must balance (debits = credits).
**Verify**: Generate against test data. Totals balance. Format matches spec.
**Do NOT**: Include zero-balance inactive accounts unless they have activity in the period.

### Task 3: Implement Balance Sheet report
**Context**: `src/reports/mod.rs`, `src/db/account_repo.rs`.
**Action**: Create `src/reports/balance_sheet.rs`. Assets = Liabilities + Equity as of a date.
Hierarchical display with subtotals per account type.
**Verify**: Fundamental equation holds. Subtotals are correct.

### Task 4: Implement Income Statement report
**Context**: `src/reports/mod.rs`.
**Action**: Create `src/reports/income_statement.rs`. Revenue − Expenses over a date range.
**Verify**: Net income = Revenue total − Expense total.

### Task 5: Implement Cash Flow Statement report
**Context**: `src/reports/mod.rs`.
**Action**: Create `src/reports/cash_flow.rs`. Cash inflows/outflows, direct method.
**Verify**: Net cash change matches change in Cash account balance over the period.

### Task 6: Implement Account Detail report
**Context**: `src/reports/mod.rs`, `src/db/journal_repo.rs`.
**Action**: Create `src/reports/account_detail.rs`. All entries for one account with running balance.
**Verify**: Running balance matches GL tab display for same account and date range.

### Task 7: Implement AR Aging report
**Context**: `src/reports/mod.rs`, `src/db/ar_repo.rs`.
**Action**: Create `src/reports/ar_aging.rs`. Open receivables grouped by aging buckets:
Current, 1-30, 31-60, 61-90, 90+ days.
**Verify**: Items sort into correct buckets based on days outstanding from due date.

### Task 8: Implement AP Aging report
**Context**: `src/reports/mod.rs`, `src/db/ap_repo.rs`.
**Action**: Create `src/reports/ap_aging.rs`. Same structure as AR Aging.
**Verify**: Same pattern.

### Task 9: Implement Fixed Asset Schedule report
**Context**: `src/reports/mod.rs`, `src/db/asset_repo.rs`.
**Action**: Create `src/reports/fixed_asset_schedule.rs`. All fixed assets with cost,
accumulated depreciation, book value.
**Verify**: Book value = cost − accumulated depreciation for each asset.

---

### Task 10: Implement Reports tab
**Context**: `src/tabs/reports.rs` (stub), `src/reports/` (all report implementations).
**Action**: Replace stub:
- Menu listing all 8 reports
- Parameter input: date/date range picker (reuse or create date input widget),
  account selector (for Account Detail)
- Generate button: runs the report, writes file, shows confirmation with file path in status bar
**Verify**: Generate each report type from the TUI. Files appear in configured output directory
with correct names and formatting.
**Do NOT**: Implement PDF output (out of scope).

---

### Task 11: Create RecurringRepo and wire into JE tab
**Context**: `src/db/mod.rs`, `specs/data-model.md` (recurring_entry_templates table).
**Action**: Create `src/db/recurring_repo.rs`. Implement `RecurringRepo<'conn>`:
- `create_template(source_je_id: JournalEntryId, frequency: EntryFrequency, start_date: NaiveDate) -> Result<RecurringTemplateId>`
- `list_upcoming() -> Result<Vec<RecurringTemplate>>` — active, ordered by next_due_date
- `generate_entries(as_of: NaiveDate) -> Result<Vec<JournalEntryId>>` — creates draft JEs,
  advances next_due_date
- `deactivate(id: RecurringTemplateId) -> Result<()>`

Wire into Journal Entries tab:
- `f` on a posted JE → create recurring template (prompt for frequency and start date)
- Sub-view or indicator showing upcoming recurring entries
**Verify**: Flag JE as recurring → template created. Generate → draft JE with same line items
and correct date. next_due_date advances. Generate again with same date → no duplicates.
Deactivate → no longer generates.
**Do NOT**: Auto-post generated entries. They are always created as Draft for user review.

---

### Task 12: Create startup sequence
**Context**: `specs/architecture.md` (Startup Sequence section),
`src/db/recurring_repo.rs`, `src/db/asset_repo.rs`.
**Action**: Create `src/startup.rs`. Implement `run_startup_checks()`:
1. **Inter-entity recovery** (structure only — query for orphaned drafts, report findings.
   Full resolution logic is Phase 6). If orphaned drafts found, warn user.
2. **Recurring entries due**: if `next_due_date <= today` for any active template,
   prompt user to generate and review.
3. **Pending depreciation**: if any asset has un-generated depreciation months,
   prompt user to generate.
Present prompts sequentially before main UI loads. User must resolve each.
Wire into `main.rs` after entity is opened, before `App::run()`.
**Verify**: Create a recurring template with past due date → startup prompts to generate.
Create an asset with un-generated depreciation → startup prompts.
**Do NOT**: Implement full inter-entity recovery resolution (Phase 6). Detection only.

---

### Task 13: Implement Audit Log tab
**Context**: `src/tabs/audit_log.rs` (stub), `src/db/audit_repo.rs`.
**Action**: Replace stub:
- Read-only list of all audit events
- Columns: Timestamp, Action Type, Description
- Filter by date range, action type (dropdown/selector)
- No edit/delete actions (append-only)
**Verify**: All mutations from previous phases appear in the log. Filters work.
**Do NOT**: Add any mutation capabilities. The audit log is strictly read-only.

---

### Task 14: Implement `?` help overlay
**Context**: `src/app.rs`, all tab files.
**Action**: Global hotkey `?` shows a modal overlay with:
- Global hotkeys (tab switching, quit, fiscal period management)
- Active tab's hotkeys (context-dependent)
- `Esc` or `?` dismisses
Each tab should expose a method `fn hotkey_help(&self) -> Vec<(&str, &str)>` returning
(key, description) pairs.
**Verify**: Help overlay appears on each tab, shows correct keys for that tab's context.
**Do NOT**: Make this a tab. It's a modal overlay.

---

## Phase 5 Complete When

- [ ] All Phase 4 checks still pass
- [ ] All 8 reports generate correctly formatted .txt files with box-drawing
- [ ] Report files appear in configured output directory with correct naming
- [ ] Recurring entry templates can be created from posted JEs
- [ ] Recurring generation creates correct draft entries and advances schedule
- [ ] Startup checks detect due recurring entries and pending depreciation
- [ ] Audit log tab displays all historical mutations with filtering
- [ ] Help overlay shows context-appropriate hotkeys
- [ ] `cargo clippy -D warnings` and `cargo test` pass
- [ ] `progress.md` updated

## Phase 5 Does NOT Cover

- Inter-entity transaction execution (Phase 6)
- Inter-entity startup recovery resolution (Phase 6)
- AR/AP auto-JE creation (Phase 6 polish)
- Edge case handling (Phase 6)

**After completing Phase 5**: Developer reviews all code and signs off before Phase 6 begins.
