# Phase 6: Inter-Entity Transactions & Polish

**Goal**: The inter-entity journal entry modal with split-pane UI, the two-phase write protocol
with failure recovery, edge case handling, and final polish.

**Depends on**: Phase 5 (everything else complete).

**Estimated tasks**: 15

---

## Tasks

### Task 1: Create InterEntityMode struct
**Context**: `specs/architecture.md` (Inter-Entity Modal section), `src/app.rs`,
`src/db/mod.rs`.
**Action**: Create `src/inter_entity/mod.rs`. Define `InterEntityMode`:
- Opens a second `EntityDb` connection to the selected entity
- Holds: primary_db ref, secondary_db, both entity names, account lists for both entities
- Lifecycle: open → form → submit/cancel → close (drops secondary connection)
**Verify**: Opens and closes second DB connection without resource leaks.
Test with a second entity's database file.
**Do NOT**: Implement the form or write protocol yet (Tasks 2-3).

---

### Task 2: Create inter-entity form
**Context**: `src/inter_entity/mod.rs`, `src/widgets/je_form.rs`, `src/widgets/account_picker.rs`,
`src/db/envelope_repo.rs`.
**Action**: Create `src/inter_entity/form.rs`. Split-pane entry form:
- **Top pane**: shared entry form (date, memo)
  - Entity A line items section (debit/credit rows with account picker from Entity A)
  - Entity B line items section (debit/credit rows with account picker from Entity B)
- **Bottom-left pane**: Entity A chart of accounts with earmarked dollar amounts
- **Bottom-right pane**: Entity B chart of accounts with earmarked dollar amounts
- Per-entity validation: each entity's lines must independently balance (debits = credits)
- Running totals per entity
- `Tab` moves between form sections and fields
- `Esc` exits (with unsaved-changes prompt if form has content)
**Verify**: Form renders in ┬ layout. Can add lines for both entities. Per-entity balance
validation shows error if one side doesn't balance. Account pickers pull from correct entity.
**Do NOT**: Implement the write protocol (Task 3). Form should return the validated data structure.

---

### Task 3: Implement inter-entity write protocol **[TEST-FIRST]**
**Context**: `src/inter_entity/mod.rs`, `src/db/journal_repo.rs`, `src/db/envelope_repo.rs`,
`specs/type-system.md` (Inter-Entity Transaction Protocol).
**Action**: On form submit:
1. Validate both sides balance independently.
2. Generate UUID for `inter_entity_uuid`.
3. Write Draft JE to Entity A's database (with Entity B's name in `source_entity_name`).
4. Write Draft JE to Entity B's database (with Entity A's name in `source_entity_name`).
5. Post Entity A's JE (triggers envelope fills if applicable).
6. Post Entity B's JE (triggers envelope fills if applicable).
7. If any step fails after step 3: execute rollback (delete drafts, reverse any posted entry).
Both entries share the same `inter_entity_uuid`.
**Verify**:
- Successful post → entries in both databases with matching UUIDs, both Posted
- Simulate failure after step 4 (both Draft) → both rolled back
- Simulate failure after step 5 (A Posted, B Draft) → A reversed, B deleted
- Envelope fills triggered correctly on both sides
**Do NOT**: Handle the startup recovery case (Task 4). This is the happy path + immediate failure handling.

---

### Task 4: Implement inter-entity startup recovery **[TEST-FIRST]**
**Context**: `src/inter_entity/recovery.rs`, `src/startup.rs` (structure from Phase 5),
`src/config.rs` (to find the other entity's DB path),
`specs/type-system.md` (Inter-Entity failure recovery).
**Action**: Create `src/inter_entity/recovery.rs`. Full recovery logic:
- Query active entity: `SELECT ... WHERE inter_entity_uuid IS NOT NULL AND status = 'Draft'`
- For each orphaned Draft, open the other entity's DB (matched by `source_entity_name`
  from workspace config) and check the paired entry's status
- **Both Draft** → prompt: "Post both entries?" or "Delete both drafts?"
- **One Posted, one Draft** → prompt: "Complete (post the draft)?" or "Roll back (reverse
  posted, delete draft)?"
- User must resolve each before main UI loads
Wire into `startup.rs` to replace the Phase 5 detection-only stub.
**Verify**: Seed inconsistent states in two entity DBs:
- Both Draft → "Post both" works, "Delete both" works
- One Posted, one Draft → "Complete" posts the draft, "Roll back" reverses and deletes
- After resolution, no orphaned inter-entity drafts remain
**Do NOT**: Skip the prompt — user must explicitly choose the resolution.

---

### Task 5: Wire inter-entity mode into App
**Context**: `src/app.rs`, `src/inter_entity/mod.rs`, `src/config.rs`.
**Action**:
- `i` hotkey from Journal Entries tab:
  - If only one entity in workspace config → show error: "Inter-entity mode requires
    at least two entities in workspace config."
  - If multiple entities → show entity picker for the second entity
  - Open `InterEntityMode` with both DB connections
- `Esc` from inter-entity mode → unsaved-changes check, then close
- On successful post → `TabAction::RefreshData`, return to Normal mode
**Verify**: Full flow: `i` → picker → form → post → both databases updated → return to normal.
**Do NOT**: Allow entering inter-entity mode from any tab other than Journal Entries.

---

### Task 6: Auto-create intercompany accounts
**Context**: `src/inter_entity/mod.rs`, `src/db/account_repo.rs`.
**Action**: When entering inter-entity mode, check if the active entity has "Due To [Entity B]"
and "Due From [Entity B]" accounts. If not, prompt: "Create intercompany accounts for
[Entity B]?" On confirm, create them as sub-accounts under Liabilities (Due To) and
Assets (Due From). Same check for Entity B regarding Entity A.
**Verify**: First inter-entity session between two new entities → prompt for account creation.
Second session → no prompt (accounts already exist).
**Do NOT**: Auto-create without prompting. User confirms account creation.

---

### Task 7: Edge case — JE form validation polish
**Context**: `src/widgets/je_form.rs`, `src/inter_entity/form.rs`.
**Action**:
- Prevent posting with zero-amount lines (both debit and credit are 0)
- Date validation: entry_date must fall within a fiscal period that exists
- Ensure at least 2 lines before submit
- Amount validation: reject negative amounts in debit/credit fields
**Verify**: Each edge case has a test. Zero-amount line → rejected. Date outside fiscal periods → rejected.
**Do NOT**: Add validation for things handled by the post orchestration (like period-closed checks).

---

### Task 8: Edge case — AR/AP payment flow polish
**Context**: `src/tabs/accounts_receivable.rs`, `src/db/ar_repo.rs`, `src/db/journal_repo.rs`.
**Action**: When recording a payment on an AR item, optionally auto-create the payment JE:
- Debit Cash account, Credit AR account, for the payment amount
- User confirms the auto-generated JE before it's created
- Link the payment to the auto-created JE
Same for AP: Debit AP account, Credit Cash account.
**Verify**: Record payment → auto-JE created → AR status updates → GL reflects both entries.
User can still manually link to an existing JE instead of auto-creating.
**Do NOT**: Make auto-creation mandatory. It's an option alongside manual JE linkage.

---

### Task 9: Edge case — envelope fill with multiple Cash accounts
**Context**: `src/db/envelope_repo.rs`, post orchestration function.
**Action**: Verify that if a JE debits multiple Cash/Bank accounts, the fill calculation
sums ALL Cash/Bank debit lines (not just the first one).
**Verify**: JE with debits to two Cash accounts → fill based on total cash received across both.
**Do NOT**: Change the fill logic if it already works correctly. Just verify and add a test.

---

### Task 10: Edge case — year-end close with envelope balances
**Context**: `src/db/envelope_repo.rs`, year-end close logic from Phase 3.
**Action**: After year-end close, Revenue/Expense accounts zero out. Verify that envelope
earmarks for those accounts **persist** (they're budgetary, not GL).
**Verify**: Set envelope allocations on expense accounts. Post cash receipts (fills occur).
Year-end close → expense account GL balances zero, but envelope earmarked amounts remain.
**Do NOT**: Clear envelope balances on year-end close. They are independent of the GL.

---

### Task 11: Edge case — depreciation across fiscal year boundary
**Context**: `src/db/asset_repo.rs`, `src/db/fiscal_repo.rs`.
**Action**: Asset placed in service in December, generate depreciation for Jan-Mar of next year.
Verify entries reference the correct fiscal_period_id (next year's periods, not current year's).
**Verify**: Depreciation entries land in correct fiscal periods across year boundary.
Requires that a second fiscal year has been created.
**Do NOT**: Auto-create fiscal years. If the next year doesn't exist, depreciation generation
should stop at the last existing period and warn the user.

---

### Task 12: Comprehensive cross-tab navigation audit
**Context**: All tab files, `src/app.rs`.
**Action**: Verify all NavigateTo paths work end-to-end:
- CoA → GL (view account ledger)
- GL → JE (view source entry)
- JE detail → GL (view affected account)
- AR → JE (view originating entry)
- AP → JE (view originating entry)
**Verify**: Each path tested. Navigation lands on the correct record.
**Do NOT**: Add new navigation paths beyond what's specified.

---

### Task 13: Status bar polish
**Context**: `src/widgets/status_bar.rs`, `src/app.rs`.
**Action**:
- Unsaved changes indicator: show `[*]` or similar when a Draft JE form has content
- Error messages: red text, auto-clear after 5 seconds
- Success messages: green text, auto-clear after 3 seconds
- Entity name + current period always visible
**Verify**: Visual review of all status bar states.
**Do NOT**: Add animation or blinking. Simple color-coded text.

---

### Task 14: Update CLAUDE.md for project specifics
**Context**: Existing `CLAUDE.md`, all spec files.
**Action**: Update the project `CLAUDE.md` to reflect finalized decisions:
- Remove `tokio`/async references (synchronous event loop)
- Add project-specific conventions discovered during implementation
- Reference the spec files for architecture and data model
- Add any gotchas discovered during implementation (from `progress.md` Decisions & Discoveries)
**Verify**: `CLAUDE.md` is accurate and complete for any future development sessions.
**Do NOT**: Remove general Rust style rules (no unwrap, iterators over loops, etc.) — those still apply.

---

### Task 15: Full integration test
**Context**: All modules.
**Action**: Write a comprehensive integration test that exercises the full lifecycle:
Create entity → seed accounts → create fiscal year → create JEs → post → view GL →
create AR item → partial payment → set envelope allocations → post cash receipt (verify fills) →
create CIP account → place in service → generate depreciation → close period →
generate all 8 reports → verify report files exist → year-end close → verify closing entries.
**Verify**: Single test passes end-to-end. All 8 report files generated. All balances correct.
**Do NOT**: Test inter-entity features in this integration test (they require two DB files
and are tested separately in Tasks 3-4).

---

## Phase 6 Complete When

- [ ] All previous phase checks still pass
- [ ] Inter-entity journal entry modal works end-to-end with ┬ layout
- [ ] Both entities receive correct journal entries with matching UUIDs
- [ ] Failure recovery detects and resolves all inconsistent states on startup
- [ ] Intercompany accounts auto-created when needed (with user confirmation)
- [ ] All edge cases handled with appropriate error messages
- [ ] Cross-tab navigation works for all defined paths
- [ ] Status bar polish complete
- [ ] CLAUDE.md updated for the project
- [ ] Full integration test passes
- [ ] `cargo clippy -D warnings` and `cargo test` pass
- [ ] `progress.md` marks all phases complete

## Out of Scope (Future Versions)

- Multi-user access and authentication
- Inventory / materials management
- Accelerated depreciation methods (MACRS, double-declining)
- Full invoice management (line items, invoice numbers, customer/vendor records)
- Consolidated multi-entity financial reports
- PDF report output
- Automated backup (user manages SQLite file backups manually)
- Bank feed / import (OFX, CSV statement import)
- Budgeting by period (actuals vs. budget variance reports)
- More than 2 entities open simultaneously
- Formal bank reconciliation workflow (Cleared → Reconciled transition)

**After completing Phase 6**: The application is ready for daily use.
