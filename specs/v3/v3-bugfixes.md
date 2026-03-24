# V3 Bug Fixes — Post-Release Testing Issues

## Overview

Issues discovered during manual testing of v0.4.0 with real bank data. All are
independent fixes — no architectural changes, no schema changes, no new dependencies.
Will be committed incrementally but tagged as a single patch release when all testing
is complete.

## Fixes

### Fix 1: Parent Picker Shows Non-Placeholder Accounts

**Bug:** When adding an account in Chart of Accounts (`a` key), the parent field's
account picker shows all accounts including non-placeholders. Only placeholder accounts
should be valid parents.

**File:** `src/tabs/chart_of_accounts.rs`

**Fix:** Find where the AccountPicker is created for the parent field in the Add Account
form. It should be using `AccountPicker::with_placeholders()` (or equivalent flag that
filters to `is_placeholder = true` only). If the method exists but isn't being called,
switch to it. If the Add form creates its own picker instance, ensure it passes the
placeholder filter.

Check the Edit Account form too — same bug may exist there.

**Verification:**
- Press `a` on CoA tab, tab to Parent field, open picker — only accounts with P flag visible
- Press `e` on CoA tab, tab to Parent field — same, only placeholders
- `cargo fmt && cargo clippy -D warnings && cargo test`

**Commit:** `fix: parent picker in Add/Edit account only shows placeholder accounts`

---

### Fix 2: AI Chat Panel Auto-Opens During CSV Import

**Bug:** The AI Accountant chat panel opens automatically when running CSV import
(during Pass 2 AI matching). The panel should remain closed — the AI calls happen
in the background and results appear in the review screen.

**File:** `src/app/import_handler.rs` or `src/app/ai_handler.rs`

**Fix:** Find where the import flow sets `self.focus = FocusTarget::ChatPanel` or
opens the chat panel. The AI matching calls during import should not affect the chat
panel visibility or focus state. The panel state before import should be preserved.

**Verification:**
- Close the chat panel (Esc or Ctrl+K if open)
- Press `u` to import a CSV, complete through to review screen
- Chat panel should remain closed throughout the import
- `cargo fmt && cargo clippy -D warnings && cargo test`

**Commit:** `fix: CSV import AI matching no longer opens chat panel`

---

### Fix 3: Draft Preview Text Truncation

**Bug:** In the import review screen, the Draft Preview pane at the bottom cuts off
long memo/description text from bank transactions.

**File:** `src/app/import_handler.rs`

**Fix:** The Draft Preview pane likely uses a fixed-width `Paragraph` without wrapping.
Enable `Wrap { trim: false }` on the paragraph widget, or increase the allocated area
for the preview pane. The memo text from bank CSVs can be very long (e.g., full ACH
descriptions with payroll details).

**Verification:**
- Import a CSV with long transaction descriptions
- Select a transaction in the review screen
- Draft Preview should show the full memo text, wrapped if needed
- `cargo fmt && cargo clippy -D warnings && cargo test`

**Commit:** `fix: draft preview wraps long memo text in import review`

---

### Fix 4: "Approve All" Row Position

**Bug:** The "Approve All & Create Drafts [Enter]" row appears in the middle of the
review screen instead of always being at the top of the list.

**File:** `src/app/import_handler.rs`

**Fix:** In `build_review_rows()`, the ApproveAction row should always be the first
row after the Transfer Matches section (if present). Check that ApproveAction is
pushed before the Unmatched/Matched sections. If the issue is with rendering (the
row is in the right position but scrolls off screen), ensure the list scroll offset
keeps ApproveAction visible or pins it.

**Verification:**
- Import a CSV with both transfer matches and normal transactions
- ApproveAction row should appear directly after transfer matches, before Unmatched
- With no transfer matches, ApproveAction should be the very first row
- `cargo fmt && cargo clippy -D warnings && cargo test`

**Commit:** `fix: approve all row always appears at top of review list`

---

### Fix 5: JE Detail Memo Cutoff

**Bug:** When viewing a journal entry's detail (pressing Enter on a JE in the list),
the memo line displayed above the line items is truncated for long descriptions,
especially for imported bank transactions with verbose ACH descriptions.

**File:** `src/tabs/journal_entries.rs`

**Fix:** The memo display area likely uses a single-line `Paragraph` or a fixed-height
area. Either enable text wrapping or allocate more vertical space for the memo. If
the memo area is a single row, expand it to 2-3 rows with `Wrap { trim: false }`.

**Verification:**
- Open a draft JE that was imported from a bank CSV with a long description
- The full memo should be visible (wrapped across multiple lines if needed)
- `cargo fmt && cargo clippy -D warnings && cargo test`

**Commit:** `fix: JE detail view wraps long memo text`

---

### Fix 6: Delete Draft JEs with 'x' Key

**Enhancement:** Allow deleting draft journal entries from the Journal Entries tab
using the `x` key, consistent with the CoA tab's delete behavior. Currently there
is no way to remove unwanted drafts (e.g., bad import guesses) without posting and
reversing them.

**File:** `src/tabs/journal_entries.rs`, `src/db/journal_repo.rs`

**Changes:**

1. Add `delete_draft(&self, je_id: JournalEntryId) -> Result<()>` to `JournalRepo`:
   - Verify the JE has status `Draft` (refuse to delete Posted entries)
   - Delete all lines from `journal_entry_lines` for this JE
   - Delete all import_refs from `journal_entry_import_refs` for this JE
   - Delete the JE from `journal_entries`
   - Log `AuditAction::JournalEntryDeleted` (add this variant if it doesn't exist)

2. Handle `x` key in `JournalEntriesTab`:
   - Only active when a Draft entry is selected
   - Show confirmation dialog: "Delete draft JE-XXXX? This cannot be undone."
   - On confirm: call `delete_draft()`, refresh tab, show success message
   - On Posted entry: show error "Cannot delete posted entries. Use reverse instead."

3. Add to `hotkey_help()`: `("x", "Delete draft")`

**Tests:**
- Delete a draft → JE and its lines removed from DB
- Delete a draft with import_refs → import_refs also removed
- Attempt to delete a posted entry → error returned
- Re-import after deleting a draft with import_ref → transaction is no longer a duplicate

**Verification:**
- Select a draft JE, press `x`, confirm → draft disappears from list
- Select a posted JE, press `x` → error message in status bar
- `cargo fmt && cargo clippy -D warnings && cargo test`

**Commit:** `feat: delete draft JEs with 'x' key on Journal Entries tab`

---

### Fix 7: Change New JE Hotkey from 'n' to 'a'

**Enhancement:** Change the "new journal entry" hotkey from `n` to `a` for consistency
with the Chart of Accounts tab where `a` means "add."

**File:** `src/tabs/journal_entries.rs`

**Changes:**
1. Change the key match from `KeyCode::Char('n')` to `KeyCode::Char('a')`
2. Update `hotkey_help()` from `("n", "New journal entry")` to `("a", "Add journal entry")`
3. Update the user guide (`specs/guide/user-guide.md`) — JE tab hotkeys section

**Check for conflicts:** Verify `a` is not already used for another action on the JE tab.
Currently the JE tab uses: `e` (edit), `p` (post), `r` (reverse), `s` (scheduled),
`i` (inter-entity), `g` (GL), `f` (filter), `t` (template), `u` (import), `U` (re-match),
`n` (new). The `a` key is not in use — safe to reassign.

**Verification:**
- Press `a` on JE tab → new entry form opens
- Press `n` on JE tab → nothing happens (no longer bound)
- Help overlay (`?`) shows `a` not `n` for new entries
- `cargo fmt && cargo clippy -D warnings && cargo test`

**Commit:** `feat: change new JE hotkey from 'n' to 'a' for consistency`

---

## Task Order

Work through fixes 1-7 sequentially. For each fix:

1. Implement the changes
2. Run: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
3. Fix any issues until all three pass
4. Commit with the message specified above

Do NOT tag a release after these fixes. More fixes may be added from continued testing.
