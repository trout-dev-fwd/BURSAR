# V4 Bug Fixes — Post-Release Testing Issues (Round 3)

## Overview

Additional fixes from continued manual testing of v0.5.0 + previous bug fixes.

## Fixes

### Fix 1: Replace F2/F3 with Ctrl+Down/Ctrl+Up for JE Line Management

**Bug:** F2 (insert line) and F3/Delete (remove line) in the JE form don't work
in some terminal emulators that intercept function keys.

**Files:** `src/widgets/je_form.rs`, `src/app/key_dispatch.rs`, `specs/guide/user-guide.md`

**Fix:** Replace the key bindings:
- F2 (insert line below) → Ctrl+Down Arrow
- F3 (remove line) → Ctrl+Up Arrow

In `je_form.rs`, change the key match arms:
- `KeyCode::F(2)` → `KeyCode::Down` with `KeyModifiers::CONTROL`
- `KeyCode::F(3)` → `KeyCode::Up` with `KeyModifiers::CONTROL`

Also keep Delete as an alternative for remove line if it's currently there.

Update the help overlay's JE form section and the user guide to reflect the new
key bindings. Remove all F2/F3 references.

**Commit:** `fix: replace F2/F3 with Ctrl+Down/Ctrl+Up for JE line add/remove`

---

### Fix 2: Enter Opens Detail on AI-Suggested JEs, Space Accepts

**Bug:** Enter on an AI-suggested JE in the Tax tab auto-accepts the suggestion
instead of opening the detail view. Users want to inspect before accepting.

**File:** `src/tabs/tax.rs`

**Fix:** Change the behavior:
- `Enter` → always opens JE detail view regardless of status (consistent behavior)
- `Space` → accepts AI suggestion (only on `ai_suggested` status JEs)

For non-ai_suggested JEs, Space does nothing (or could toggle through statuses —
but keeping it simple, Space only works on ai_suggested).

Update the help overlay and user guide:
- `Enter`: "View JE detail"
- `Space`: "Accept AI suggestion"

**Commit:** `fix: Enter opens detail view, Space accepts AI suggestion in Tax tab`

---

### Fix 3: Show Reason in JE Detail View (Tax Tab)

**Bug:** When viewing a JE's detail in the Tax tab (pressing Enter), the reason
for the form assignment is not visible.

**File:** `src/tabs/tax.rs`

**Fix:** In the Tax tab's detail view rendering, below the `Memo:` line, add a
`Form Reason:` line showing the tax tag's reason if one exists. Only show this
line if a tax tag with a reason exists for the JE.

Format:
```
Memo: Import: DEPOSIT ACH ALLIANT CU TYPE: NEWACCDEP
Form Reason: Interest income from savings account
```

Both lines styled in Yellow (see Fix 4).

If no tax tag or no reason, the `Form Reason:` line is omitted.

**Commit:** `fix: show Form Reason in Tax tab JE detail view`

---

### Fix 4: Yellow Color for Memo and Reason Lines

**Bug:** Memo text is hard to distinguish from other content in the JE detail view
and Tax tab.

**Files:** `src/tabs/journal_entries.rs`, `src/tabs/tax.rs`

**Fix:** Style the following lines in Yellow (`Color::Yellow`):

**JE tab detail view:**
- The `Memo: {text}` line above the JE lines table

**Tax tab detail view:**
- The `Memo: {text}` line
- The `Form Reason: {text}` line (added in Fix 3)

Both the label prefix ("Memo:", "Form Reason:") and the content text should be
Yellow. This makes them visually distinct from the white/default table content.

**Commit:** `fix: style Memo and Form Reason lines in yellow`

---

## Task Order

Work through fixes 1-4 sequentially. For each fix:

1. Implement the changes
2. Run: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
3. Fix any issues until all three pass
4. Commit with the message specified above

Do NOT tag a release after these fixes.
