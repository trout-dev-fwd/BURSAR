# V4 Phase 4: AI Batch Review

## Overview

Implement AI-powered batch classification. Users press `R` to send queued JEs to
Claude in batches of 25 with prompt caching. Results include form suggestions and
reasons. Users accept, override, or reject each suggestion.

## Context Files

| File | Why |
|------|-----|
| `CLAUDE.md` | Coding style, verification |
| `specs/v4/v4-SPEC.md` | AI Batch Review Flow, Prompt Caching sections |
| `specs/v4/v4-progress.md` | Decisions log |
| `src/tabs/tax.rs` | Tax tab with `R` hotkey |
| `src/db/tax_tag_repo.rs` | Status transitions |
| `src/ai/client.rs` | AI client, prompt caching pattern |
| `src/app/ai_handler.rs` | AI request patterns |
| `src/db/tax_ref_repo.rs` | Tax reference retrieval |

## Tasks

### Task 1: Tax-Scoped AI Context + Tax Tag Tool

**Files:** `src/ai/tax_context.rs` (new), `src/ai/tools.rs`

**Part A: Keyword extraction and reference retrieval**

Keyword-to-tag mapping table (~25 entries). Must include both topic terms AND
form names so asking "why Schedule C?" pulls the right reference material:

```rust
// Topic keywords
(&["deduct", "deduction", "deductible"], "deduction"),
(&["depreciat", "depreciation", "macrs", "section 179"], "depreciation"),
(&["home office", "home business"], "home_office"),
// ... etc (see v4-SPEC.md)

// Form name keywords → map to relevant topics
(&["schedule c", "business expense", "self-employ"], "small_business,business_expense"),
(&["schedule a", "itemize", "itemized"], "deduction"),
(&["schedule d", "capital gain", "stock sale"], "capital_gains"),
(&["schedule e", "rental income", "rental property"], "rental"),
(&["schedule se", "self-employment tax"], "small_business"),
(&["form 4562", "depreciation schedule"], "depreciation"),
(&["form 8829", "home office deduction"], "home_office"),
// ... etc
```

```rust
pub fn extract_tax_tags(text: &str) -> Vec<&'static str>
pub fn get_relevant_chunks(db: &EntityDb, tags: &[&str], max_chunks: usize, max_chars: usize) -> Vec<TaxRefChunk>
pub fn build_tax_context(db: &EntityDb, message: &str, selected_je: Option<&SelectedJeContext>) -> Option<String>
```

**Part B: Selected JE context with tax tag**

`build_tax_context` accepts an optional `SelectedJeContext` — the highlighted JE's
details plus its tax tag (if any). When present, auto-include in the context block:

```
## Selected Journal Entry
JE-0004 | Jan 15, 2026 | Home Depot building materials
  Debit: 5100 Repairs & Maintenance  $245.00
  Credit: 1110 Chase Checking        $245.00

Tax Classification: Schedule C (AI Suggested)
Reason: Building supplies are ordinary business expenses
```

This is auto-included so Claude sees it without a tool call. But the user can
ask about ANY JE by number — Claude uses tools for non-highlighted JEs.

**Part C: New `get_tax_tag` AI tool**

Add to `src/ai/tools.rs`:
- Tool name: `get_tax_tag`
- Input: `je_number` (string, e.g., "JE-0004")
- Output: form_tag, status, reason, ai_suggested_form (or "no tax tag" if unreviewed)
- Tool is read-only (consistent with all other AI tools)

This lets Claude answer "why was JE-0012 classified as Schedule A?" even when
JE-0012 isn't the highlighted row.

**Part D: Wire into handle_ai_request()**

When active tab is Tax:
1. Get the highlighted JE's details + tax tag → `SelectedJeContext`
2. Call `build_tax_context(db, &message, Some(&selected))` → reference block
3. Append to system prompt with citation instructions
4. Include `get_tax_tag` in the tool definitions

When active tab is NOT Tax: skip all of the above, normal AI behavior.

**Tests:**
- Keyword extraction: topic terms and form names both work
- build_tax_context with selected JE includes tax tag details
- build_tax_context without selected JE still returns reference chunks
- build_tax_context with empty tax_reference table returns None
- get_tax_tag tool returns correct data for tagged JEs
- get_tax_tag tool returns "no tax tag" for unreviewed JEs
- Respects max_chars limit

**Commit:** `V4 Phase 4, Task 1: tax-scoped AI context, tax tag tool, and keyword extraction`

---

### Task 2: AI Batch Classification

**Files:** `src/tabs/tax.rs`, `src/app/ai_handler.rs` or `src/app/tax_handler.rs`

Handle `R` key:

1. Collect `ai_pending` JEs via `tax_tag_repo.get_pending()`
2. If empty → "No JEs queued for AI review"
3. Get enabled forms from config
4. Batch into groups of 25
5. Build system prompt (enabled forms + descriptions + tax reference chunks).
   Mark with `cache_control: { type: "ephemeral" }` for prompt caching.
6. For each batch:
   - User content: JE details (date, memo, accounts, amounts)
   - Instruction: "For each JE, return one line: `JE-XXXX: form_tag | reason`"
   - Force render "Classifying batch {n}/{total}..." before blocking call
   - Parse response: split lines, split by `|`, extract form_tag and reason
   - `tax_tag_repo.set_ai_suggested(je_id, form, reason)` for each
   - Log `AuditAction::AiTaxReview`
7. Show summary: "Reviewed {count} JEs: {breakdown}"

Parse failures for individual JEs leave them as `ai_pending` with a warning.

**Tests:** Parse sample pipe-separated response, batch sizing, empty pending list.

**Commit:** `V4 Phase 4, Task 2: AI batch classification with R hotkey and prompt caching`

---

### Task 3: AI Suggestion Review

**File:** `src/tabs/tax.rs`

JEs with `ai_suggested` status show form name with `?` suffix in cyan.

User interaction:
- `Enter` on ai_suggested → accept: `accept_suggestion()`, status → confirmed,
  reason preserved from AI
- `f` on ai_suggested → override with different form + new reason
- `n` on ai_suggested → non-deductible + optional reason

`ai_suggested_form` preserved in database even after override (audit trail).

**Tests:** Accept preserves reason, override preserves ai_suggested_form, re-flag works.

**Commit:** `V4 Phase 4, Task 3: AI suggestion review — accept, override, reject`

---

## Phase Completion Checklist

- [ ] Tax AI context includes IRS chunks only from Tax tab
- [ ] Highlighted JE's tax tag auto-included in context
- [ ] `get_tax_tag` tool allows querying any JE's classification
- [ ] Form name keywords (e.g., "Schedule C") map to relevant topic tags
- [ ] `R` sends ai_pending JEs in batches of 25
- [ ] Prompt caching enabled for system prompt across batches
- [ ] Response parsed as pipe-separated lines with form_tag and reason
- [ ] Suggestions stored with ai_suggested status and reason
- [ ] Enter accepts, `f` overrides, `n` rejects
- [ ] ai_suggested_form preserved after override
- [ ] Parse failures per-JE, not per-batch
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass
