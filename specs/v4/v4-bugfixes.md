# V4 Bug Fixes — Post-Release Testing Issues (Round 2)

## Overview

Issues discovered during manual testing of v0.5.0. Includes key conflict resolution,
UI polish, and usability improvements. Committed incrementally, tagged as a patch
release when all testing is complete.

## Fixes

### Fix 1: Default to Chart of Accounts Tab on Startup

**Bug:** App opens on Audit Log tab (position 0) instead of Chart of Accounts
(position 1) after the V4 tab reordering.

**File:** `src/app/mod.rs`

**Fix:** Change `active_tab` default from `0` to `1` in `App::new()` or wherever
the initial tab index is set.

**Commit:** `fix: default to Chart of Accounts tab on startup instead of Audit Log`

---

### Fix 2: Move Audit Log to End of Tab Bar Visually

**Bug:** Audit Log appears first in the tab bar visually. While it's mapped to the
`0` key, having it first is visually confusing since it's a low-frequency tab.

**Files:** `src/app/mod.rs`, `src/tabs/mod.rs`

**Fix:** Reorder the tab vector so Audit Log is last visually (position 9 in the
vec), Tax is second-to-last (position 8). Keep the KEY BINDINGS as they are — `0`
still opens Audit Log, `9` still opens Tax. This means the tab bar renders as:

```
CoA | GL | Journal | AR | AP | Envelopes | Assets | Reports | Tax | Audit Log
```

But the key mappings are: `1`=CoA, `2`=GL, `3`=Journal, `4`=AR, `5`=AP, `6`=Envelopes,
`7`=Assets, `8`=Reports, `9`=Tax, `0`=Audit Log.

This requires decoupling the visual position (vec index) from the key binding. The
key dispatch needs to map `0` → Audit Log's vec index (9), and `9` → Tax's vec index (8).
Update `tab_id_to_index()` or the key dispatch accordingly.

Update the help overlay: `"0: Audit Log  1-8: Other tabs  9: Tax"` or similar.

**Commit:** `fix: move Audit Log to end of tab bar visually, keep 0 key binding`

---

### Fix 3: Change Global Fiscal Period Hotkey from 'f' to 'y'

**Bug:** The `f` key is a global hotkey for fiscal period management, which intercepts
it before the Tax tab can use it for form flagging. Since `f` is specified for flagging
in the Tax tab, the global fiscal hotkey needs to move.

**Files:** `src/app/key_dispatch.rs`, `specs/guide/user-guide.md`

**Fix:** Change the global fiscal period management hotkey from `f` to `y` (for "year").
Update the help overlay, user guide, and any references.

Verify `y` is not used as a tab-specific key on any tab. Currently used keys:
`a`, `e`, `d`, `x`, `s`, `p`, `r`, `i`, `g`, `f`, `t`, `u`, `U`, `n`, `m`, `v`, `c`,
`R`, `k`, `j`, `/`. The `y` key is free.

**Commit:** `fix: change fiscal period hotkey from 'f' to 'y' to free 'f' for Tax tab`

---

### Fix 4: "Approve All" Row at Top of Review Screen

**Bug:** The "Approve All & Create Drafts" row still appears between the Transfer
Matches section and the Unmatched section, instead of at the very top.

**File:** `src/app/import_handler.rs`

**Fix:** In `build_review_rows()`, push the `ApproveAction` row BEFORE the Transfer
Matches section. Order should be: ApproveAction → Transfer Matches header + rows →
Unmatched header + rows → Matched sections.

**Commit:** `fix: approve all row at top of import review screen above transfer matches`

---

### Fix 5: Tax Tab Memo Column Width / Wrapping

**Bug:** In the Tax tab, the Memo column is too narrow and gets cut off. The Amount
and Form columns could be pushed further right to give the memo more space.

**File:** `src/tabs/tax.rs`

**Fix:** Adjust column width constraints:
- Memo: use `Constraint::Min(30)` or `Constraint::Percentage(40-50)` to give it
  more room
- Amount: `Constraint::Length(12)` (fixed width, right-aligned)
- Form: `Constraint::Length(18)` (fixed width)
- Status: `Constraint::Length(14)` (fixed width)
- Date and JE# stay fixed width

The fixed-width columns for Amount/Form/Status get pushed to the right, and Memo
fills the remaining space.

**Commit:** `fix: widen Tax tab memo column by using flexible layout`

---

### Fix 6: Memo Edit Modal Text Wrapping

**Bug:** When editing a memo via `m` key, long text runs off the right edge of the
modal instead of wrapping.

**File:** `src/widgets/text_input_modal.rs`

**Fix:** The `TextInputModal` likely renders the text as a single line. For memo
editing, the visible text should be scrolled horizontally (showing a window into
the text with the cursor visible) or the modal should be wider. Since this is a
single-line input, horizontal scrolling is the correct approach — show the portion
of the text around the cursor position, with the modal width as the viewport.

If the `TextInputModal` already handles this for long input and the issue is just
modal width, increase the modal's `centered_rect` width percentage (e.g., from 50%
to 80%).

**Commit:** `fix: memo edit modal handles long text with proper scrolling or wider layout`

---

### Fix 7: Tax Reference Library Update Confirmation

**Bug:** After pressing `u` to update the tax reference library, there's no visible
confirmation of what was ingested. The user can't tell if it worked.

**File:** `src/tabs/tax.rs` or wherever the `u` key result is displayed

**Fix:** After ingestion completes, show a summary in the status bar:
`"Tax reference updated: {n} sections from {m} publications"`

If the status bar message isn't persistent enough (disappears after 3 seconds),
also show the result in the Tax tab's header area:
`"Tax Workstation — FY 2026 | Tax Reference: 342 sections | Tax Review: 0/1 (0%)"`

Alternatively, add a line to the tab header showing the tax reference status:
`"Last updated: {date} ({n} sections)"` queried from `tax_reference` table's
max `ingested_at` and count.

**Commit:** `fix: show tax reference library status in Tax tab header`

---

## Task Order

Work through fixes 1-7 sequentially. For each fix:

1. Implement the changes
2. Run: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
3. Fix any issues until all three pass
4. Commit with the message specified above

Do NOT tag a release after these fixes. More fixes may be added from continued testing.
