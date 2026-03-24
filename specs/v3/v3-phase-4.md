# V3 Phase 4: Wiring and Integration

## Overview

Connect confirmed transfer matches to the junction table writes during draft creation.
Rejected matches re-enter the normal import flow. End-to-end testing of the full pipeline.

## Context Files

| File | Why |
|------|-----|
| `CLAUDE.md` | Coding style, verification commands |
| `specs/v3/v3-SPEC.md` | Confirmed Match Behavior section |
| `specs/v3/v3-progress.md` | Decisions log |
| `src/app/import_handler.rs` | Draft creation step, review screen state |
| `src/db/import_ref_repo.rs` | Junction table writes |
| `src/db/journal_repo.rs` | Transfer match query |

## Tasks

### Task 1: Process Confirmed Matches on Draft Creation

**File:** `src/app/import_handler.rs`

During the draft creation step (when user submits the review screen):

1. **Confirmed transfer matches**: for each row where `confirmed == true`:
   - Call `import_ref_repo.insert(matched_je_id, &row.import_ref)` to store the
     second import_ref on the existing draft
   - Do NOT create a new draft JE
   - Log to audit: `AuditAction::CsvImport` with description indicating transfer
     match (e.g., "Transfer match: skipped import, linked to JE #47")

2. **Rejected transfer matches**: collect these rows and add them back to the
   unmatched transaction pool. They need to go through Pass 2 (AI categorization)
   before draft creation.
   - If there are rejected matches, run Pass 2 on them, then add results to the
     normal review list
   - For simplicity in V3: rejected matches can be treated as unmatched after
     rejection — they get a default "Uncategorized" account and the user edits
     them in the review screen before submitting. This avoids re-running Pass 2
     mid-review. Document this simplification.

3. **Normal transactions**: process as before (create draft JEs, store import_refs)

**Tests:**
- Confirmed match → import_ref stored on existing JE, no new draft created
- Rejected match → new draft created via normal flow
- Mix of confirmed and rejected → correct behavior for each
- Confirmed match's import_ref detected as duplicate on next import

**Commit:** `V3 Phase 4, Task 1: process confirmed matches during draft creation`

---

### Task 2: End-to-End Integration Test

**File:** `src/app/import_handler.rs` or `src/integration_tests.rs`

Write an integration test covering the full cross-bank transfer scenario:

1. Import CSV from "Bank A" with a -$500 transaction on Jan 15
   → Draft JE created with import_ref "BankA|2026-01-15|Transfer|..."
2. Import CSV from "Bank B" with a +$500 transaction on Jan 16
   → Transfer detection finds the Bank A draft as a match
3. Confirm the match
   → No new draft created
   → Junction table has two import_refs for the same JE
4. Re-import Bank B CSV
   → Transaction detected as duplicate (import_ref already exists)
5. Re-import Bank A CSV
   → Transaction detected as duplicate (original import_ref exists)

Additional test cases:
- Amount within $3 tolerance matches (e.g., -$502 vs +$500)
- Amount outside $3 tolerance does not match
- Date within 3 days matches; date at 4 days does not
- Multiple potential matches → transaction stays unmatched

**Commit:** `V3 Phase 4, Task 2: end-to-end integration test for transfer detection`

---

### Task 3: Update Documentation

**Files:** `CLAUDE.md`, `specs/guide/user-guide.md`, `specs/v3/v3-progress.md`

1. **CLAUDE.md** — add V3 key decisions and gotchas:
   - Junction table replaces import_ref column
   - Transfer detection rule (±$3, 3 days)
   - Migration runs on EntityDb::open()

2. **User guide** — update the CSV Import section:
   - New "Transfer Matches" section in import review
   - How to confirm/reject matches
   - What happens when a match is confirmed

3. **v3-progress.md** — mark all phases and tasks complete

**Commit:** `V3 Phase 4, Task 3: update documentation for transfer detection`

---

## Phase Completion Checklist

- [ ] Confirmed matches store import_ref on existing JE via junction table
- [ ] Confirmed matches create no new drafts
- [ ] Rejected matches proceed through normal import flow
- [ ] End-to-end test covers full cross-bank scenario
- [ ] Duplicate detection works for both import_refs on a merged JE
- [ ] Amount tolerance and date range tested at boundaries
- [ ] CLAUDE.md updated with V3 decisions
- [ ] User guide updated with transfer match documentation
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass

## Post-Review

After Opus review and fixes, cut a release: `v0.4.0` (minor bump — new feature).
