# V4 Phase 5: Tax Summary Report + Documentation

## Overview

Add the Tax Summary report (with reasons) to the Reports tab and update all docs.

## Context Files

| File | Why |
|------|-----|
| `CLAUDE.md` | Needs V4 updates |
| `specs/v4/v4-SPEC.md` | Tax Summary Report section |
| `specs/v4/v4-progress.md` | Task tracking |
| `specs/guide/user-guide.md` | Needs Tax tab section |
| `src/reports/mod.rs` | Report trait |
| `src/tabs/reports.rs` | Report selection |
| `src/db/tax_tag_repo.rs` | Query confirmed JEs by form |

## Tasks

### Task 1: Tax Summary Report

**Files:** `src/reports/tax_summary.rs` (new), `src/reports/mod.rs`, `src/tabs/reports.rs`

New report implementing `Report` trait. Date range parameter.

Query: join `tax_tags` + `journal_entries` + `journal_entry_lines`. Filter to
`status IN ('confirmed', 'non_deductible')` within date range. Group by `form_tag`.

Output: box-drawing style matching existing reports. Each confirmed entry shows
date, JE number, memo, amount, and **reason** on the line below. Non-deductible
shown as count only. Unreviewed count as reminder. Subtotals per form category.

Add "Tax Summary" to report selection list in Reports tab.

**Tests:** Correct grouping, totals, reason display, empty data handling.

**Commit:** `V4 Phase 5, Task 1: Tax Summary by Form report with reasons`

---

### Task 2: Update Documentation

**Files:** `CLAUDE.md`, `specs/guide/user-guide.md`, `specs/v4/v4-progress.md`

**CLAUDE.md:** V4 specs table, key decisions (tax_tags with reason, non_deductible
terminology, pipe-separated AI response, prompt caching, per-JE tagging, tax context
scoped to Tax tab), gotchas (IRS HTML varies, chunk size limits, token budget,
re-flagging always allowed).

**User guide:** Tax tab section covering full workflow — configure forms, review JEs,
flag manually, queue for AI, run batch review, accept/override suggestions, edit memos,
update tax reference library, generate report. Tax disclaimer.

Update tab numbering (Audit Log at 0). Update `?` overlay description for Ctrl+H.

**v4-progress.md:** Mark all complete.

**Commit:** `V4 Phase 5, Task 2: update CLAUDE.md, user guide, and progress tracking`

---

## Phase Completion Checklist

- [ ] Tax Summary report grouped by form with subtotals
- [ ] Reasons displayed per entry in report
- [ ] Non-deductible and unreviewed shown as counts
- [ ] Report in Reports tab selection list
- [ ] CLAUDE.md updated with V4 decisions and gotchas
- [ ] User guide has complete Tax tab documentation with disclaimer
- [ ] Tab numbering updated throughout guide
- [ ] v4-progress.md complete
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass

## Post-Review

After Opus review, cut release: `v0.5.0` (minor — new feature).

Test: configure forms → ingest tax references → review JEs manually → queue + batch AI
review → accept suggestions → generate Tax Summary report → verify Ctrl+H form guide.
