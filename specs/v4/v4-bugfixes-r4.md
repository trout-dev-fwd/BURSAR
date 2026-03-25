# V4 Bug Fixes — Post-Release Testing Issues (Round 4)

## Overview

UI improvements for JE tab and Tax tab detail views, edit form styling, and
memo field scrolling.

## Fixes

### Fix 1: Always-Visible Detail Panel in JE Tab

**Bug:** The JE detail view (lines with accounts, debits, credits) requires
pressing Enter to open and Esc to close. Users have to keep toggling to see
what's inside each JE.

**File:** `src/tabs/journal_entries.rs`

**Fix:** Convert the JE tab to a master-detail layout:
- Top half: JE list (same as now)
- Bottom half: detail panel showing the selected JE's lines

The detail panel updates automatically as the user arrows through the list.
No Enter to open, no Esc to close — it's always visible.

The detail panel shows:
- Memo line (in Yellow)
- JE lines table: #, Account, Debit, Credit, Rec

Remove the Enter-to-toggle-detail behavior. Enter can be repurposed or left
as a no-op. The `e` key still opens the edit form as before.

Layout split: approximately 60% list, 40% detail. Adjust based on what looks
good — the detail panel needs at least 5-6 rows for the lines table plus the
memo line.

**Commit:** `feat: always-visible detail panel in JE tab (master-detail layout)`

---

### Fix 2: Always-Visible Detail Panel in Tax Tab

**Bug:** Same as Fix 1 but for the Tax tab. Users have to press Enter to see
JE details and the form reason.

**File:** `src/tabs/tax.rs`

**Fix:** Convert the Tax tab to a master-detail layout:
- Top half: tax review list (same as now, with status colors)
- Bottom half: detail panel showing the selected JE's lines + tax info

The detail panel shows:
- Memo line (in Yellow)
- Form Reason line (in Yellow, if a reason exists)
- JE lines table: #, Account, Debit, Credit

Updates automatically as the user arrows through the list.

Remove the Enter-to-toggle-detail behavior. Space still accepts AI suggestions.

**Commit:** `feat: always-visible detail panel in Tax tab (master-detail layout)`

---

### Fix 3: Darker Background for JE Edit Form

**Bug:** The JE edit form (opened via `e` or `a`) blends visually with the list
behind it, making it unclear the user is in edit mode.

**File:** `src/widgets/je_form.rs`

**Fix:** Add a distinct background color to the edit form overlay, matching the
style used by the Ctrl+H user guide panel. Use `Color::Rgb(30, 30, 30)` or
similar dark background, or check what color the user guide uses and match it.

Apply the background to the entire form area (the Block wrapping the form).
This creates a clear visual separation between the edit overlay and the list
behind it.

**Commit:** `fix: darker background for JE edit form overlay`

---

### Fix 4: Memo Field Horizontal Scrolling in Edit Form

**Bug:** In the JE edit form, the memo field text runs off the right edge of
the screen for long bank import descriptions.

**File:** `src/widgets/je_form.rs`

**Fix:** The memo field should scroll horizontally to keep the cursor visible,
same approach as the TextInputModal fix from the previous round. Keep the
field dimensions the same — just add horizontal scroll logic:

1. Calculate the visible width of the memo field
2. Compute a scroll offset based on cursor position
3. Display only the visible portion of the text starting from the scroll offset
4. Cursor position on screen = cursor position in text - scroll offset

This is the same pattern used in `TextInputModal` after the 80% width + scroll fix.

**Commit:** `fix: memo field horizontal scrolling in JE edit form`

---

## Task Order

Work through fixes 1-4 sequentially. For each fix:

1. Implement the changes
2. Run: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
3. Fix any issues until all three pass
4. Commit with the message specified above

Do NOT tag a release after these fixes.
