# V4 Phase 3: Tax Review Workflow

## Overview

Build the Tax tab's main functionality: JE list view with status, manual flagging
with form picker and reason input, memo editing, fiscal year selector, and the
Tax Form Guide in the Ctrl+H user guide.

## Context Files

| File | Why |
|------|-----|
| `CLAUDE.md` | Coding style, verification |
| `specs/v4/v4-SPEC.md` | Tax Tab Main View, Manual Flagging Flow, Tax Form Guide |
| `specs/v4/v4-progress.md` | Decisions log |
| `src/tabs/tax.rs` | Tax tab shell from Phase 1 |
| `src/db/tax_tag_repo.rs` | TaxTagRepo from Phase 1 |
| `src/tabs/envelopes.rs` | Fiscal year selector pattern |
| `src/widgets/text_input_modal.rs` | For reason input |
| `specs/guide/user-guide.md` | For Tax Form Guide section |

## Tasks

### Task 1: JE List View with Tax Status

**File:** `src/tabs/tax.rs`

Replace placeholder with a table of posted JEs for the selected fiscal year.
Join JE data with tax_tags (LEFT JOIN — unreviewed have no row).

Columns: Date, JE #, Memo (truncated), Amount, Form, Status.
Status colors: Unreviewed=dim, AiPending=yellow, AiSuggested=cyan (with `?`
suffix on form name), Confirmed=green, NonDeductible=gray.

Fiscal year selector: `←/→` cycle years (same as Envelopes). Default to current.

Progress indicator: `"Tax Review: 47/200 (23%)"` in header.

**Commit:** `V4 Phase 3, Task 1: Tax tab JE list view with status and fiscal year selector`

---

### Task 2: Manual Flagging, Reason Input, and Memo Editing

**File:** `src/tabs/tax.rs`

**`f` key — Flag with form:**
1. Open form picker (enabled forms only, show name + description)
2. On select → open reason input: "Reason (optional):" via TextInputModal
3. On submit → `tax_tag_repo.set_manual(je_id, form, reason)`, refresh

**`n` key — Non-deductible:**
1. Open reason input: "Reason (optional):"
2. On submit → `tax_tag_repo.set_non_deductible(je_id, reason)`, refresh

**`a` key — Queue for AI:**
- `tax_tag_repo.set_ai_pending(je_id)`, refresh, show success message

**`m` key — Edit memo:**
- Open TextInputModal pre-filled with current memo
- On submit → update `journal_entries.memo`, refresh

**`Enter` key — View JE detail:**
- Same detail view as JE tab

All flagging keys (`f`, `n`) work on ANY current status. Re-flagging overwrites.

**Tests:** Status transitions, re-flagging from confirmed, reason stored correctly.

**Commit:** `V4 Phase 3, Task 2: manual flagging, reason input, and memo editing`

---

### Task 3: Tax Form Guide in User Guide + Help Overlay Update

**Files:** `specs/guide/user-guide.md`, `src/app/key_dispatch.rs`

Add "Tax Form Guide" section to the user guide with full descriptions for each
form (see v4-SPEC.md Tax Form Guide section). Covers: what the form is, who needs
it, what transactions go on it.

Update `?` help overlay:
- `Ctrl+H` description changes to `"Open user guide (& form guide)"`
- Tax tab hotkeys added to Tab-specific section:
  `f` Flag with form, `a` Queue for AI, `n` Non-deductible, `m` Edit memo,
  `R` Run AI review, `u` Update tax reference, `c` Configure forms, `←/→` Fiscal year

**Commit:** `V4 Phase 3, Task 3: Tax Form Guide in user guide and help overlay updates`

---

## Phase Completion Checklist

- [ ] Tax tab shows posted JEs with correct status colors
- [ ] Fiscal year selector works (←/→)
- [ ] Progress indicator shows reviewed/total
- [ ] `f` opens form picker → reason input → saves with status confirmed
- [ ] `n` opens reason input → saves as non_deductible
- [ ] `a` queues as ai_pending
- [ ] `m` edits memo
- [ ] All keys work on any status (re-flagging allowed)
- [ ] Reason stored and displayed correctly
- [ ] Tax Form Guide in Ctrl+H user guide
- [ ] `?` overlay updated with Tax hotkeys and form guide reference
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass
