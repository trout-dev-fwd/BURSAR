# V4 Feature Specification — Tax Workstation

## Overview

Add a Tax tab that serves as a tax preparation workstation built on top of existing
journal entries. Users review posted JEs one by one, flag them with tax form
classifications (manually or via AI batch review), write reasons for each classification,
and export a categorized tax summary report for their accountant.

The tab also integrates an IRS publication reference library that provides sourced
tax guidance through the AI chat panel when opened from the Tax tab.

**Who**: A small business owner or individual doing their own tax prep or organizing
records for an accountant.

**Why**: Currently, preparing taxes requires manually reviewing every transaction and
deciding what's deductible. This tab brings that workflow into the app with AI assistance,
so tax prep happens alongside the accounting rather than as a separate annual scramble.

---

## Success Criteria

- [ ] Audit Log tab moved from position 9 to position 0
- [ ] Tax tab added at position 9 (hotkey `9`, right after Reports)
- [ ] Tax tab shows all posted JEs for the selected fiscal year
- [ ] Users can flag JEs with a tax form classification via `f` key (with optional reason)
- [ ] Users can re-flag any JE regardless of current status (corrections always allowed)
- [ ] Users can queue JEs for AI review via `a` key
- [ ] Users can mark JEs as non-deductible via `n` key
- [ ] Users can edit JE memos from both the Tax tab (`m`) and JE tab (`m`)
- [ ] AI batch review classifies queued JEs against user's enabled forms
- [ ] AI provides a reason for each classification, saved to the database
- [ ] AI batch review uses prompt caching for the system prompt
- [ ] Users review AI suggestions one by one (accept/override/reject)
- [ ] Form configuration screen (`c` key) toggles forms on/off
- [ ] All forms enabled by default
- [ ] Tax Form Guide available via Ctrl+H user guide
- [ ] `?` help overlay shows `Ctrl+H` as "Open user guide (& form guide)"
- [ ] Tax reference library ingested via `u` hotkey in Tax tab
- [ ] AI chat from Tax tab includes IRS reference chunks + accounting context
- [ ] Tax Summary report exportable from Reports tab, includes reasons per entry
- [ ] Per-line `line_memo` field hidden from JE UI (memo field only)
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass

---

## Tab Restructuring

| Position | Before | After |
|----------|--------|-------|
| 0 | — | Audit Log |
| 1 | Chart of Accounts | Chart of Accounts |
| 2 | General Ledger | General Ledger |
| 3 | Journal Entries | Journal Entries |
| 4 | Accounts Receivable | Accounts Receivable |
| 5 | Accounts Payable | Accounts Payable |
| 6 | Envelopes | Envelopes |
| 7 | Fixed Assets | Fixed Assets |
| 8 | Reports | Reports |
| 9 | Audit Log | **Tax** |

Audit Log moves to `0` key. Tax tab takes `9` key. All other tabs keep their positions.

---

## Schema Changes

### New Table

```sql
CREATE TABLE IF NOT EXISTS tax_tags (
    id INTEGER PRIMARY KEY,
    journal_entry_id INTEGER NOT NULL UNIQUE REFERENCES journal_entries(id),
    form_tag TEXT,
    status TEXT NOT NULL DEFAULT 'unreviewed',
    ai_suggested_form TEXT,
    reason TEXT,
    reviewed_at TEXT
);
```

**Status values:** `unreviewed`, `ai_pending`, `ai_suggested`, `confirmed`, `non_deductible`

**reason field:** Stores either the AI's explanation for its suggestion, or the user's
manually entered reason when flagging with `f`. Included in the Tax Summary report
so the accountant sees context for each classification.

### JE Memo Simplification

Per-line `line_memo` field is hidden from the JE tab UI. The JE-level `memo` field
becomes the sole user-facing description. The `line_memo` column remains in the schema
for backward compatibility but is no longer populated or displayed.

The `m` key for memo editing is available on both the JE tab and Tax tab.

---

## Tax Form Classifications

All forms are enabled by default. Users can disable forms they don't need via the
configuration screen (`c` key in Tax tab).

| Tag | Form | Short Description |
|-----|------|-------------------|
| `schedule_c` | Schedule C | Business income & expenses |
| `schedule_a_medical` | Schedule A | Medical & dental expenses |
| `schedule_a_taxes` | Schedule A | State & local taxes paid |
| `schedule_a_interest` | Schedule A | Mortgage & investment interest |
| `schedule_a_charity` | Schedule A | Charitable contributions |
| `schedule_d` | Schedule D | Capital gains & losses |
| `schedule_e` | Schedule E | Rental income & expenses |
| `schedule_se` | Schedule SE | Self-employment tax |
| `form_4562` | Form 4562 | Depreciation & amortization |
| `form_8829` | Form 8829 | Home office deduction |
| `form_4797` | Form 4797 | Sale of business property |
| `form_1120s` | Form 1120-S | S-Corporation return |
| `estimated_payment` | Form 1040-ES | Estimated tax payments |
| `non_deductible` | — | No deduction applies |

### Form Configuration

Stored in entity TOML config:

```toml
[tax]
enabled_forms = ["schedule_c", "schedule_a_medical", ...]  # all by default
```

When not present in the config, all forms are enabled.

---

## Tax Tab — Main View

Shows all posted JEs for the selected fiscal year with tax review status.

### Columns

```
Date    | JE #    | Memo                        | Amount   | Form          | Status
--------|---------|-----------------------------|---------:|---------------|----------
Jan 15  | JE-0004 | Home Depot building mater.. | $245.00  | Schedule C    | Confirmed
Jan 16  | JE-0005 | Transfer to Ally            | $500.00  |               | Non-Deductible
Jan 20  | JE-0006 | Amazon office supplies      | $67.99   | Schedule C?   | AI Suggested
Jan 22  | JE-0007 | Mortgage payment            | $1,200   |               | Unreviewed
```

**Amount column:** Shows net flow — total debits for expense-type JEs, total credits
for income-type JEs. For JEs with mixed categories (e.g., part business, part personal),
the full JE amount is shown. Per-JE tagging applies to the whole entry. Users who need
to split mixed transactions should do so at the draft stage (split draft feature planned
for a future version).

**Status colors:**
- Unreviewed: default/dim
- AI Pending: yellow
- AI Suggested: cyan (form name shown with `?` suffix, e.g., `Schedule C?`)
- Confirmed: green
- Non-Deductible: gray

### Hotkeys

| Key | Action |
|-----|--------|
| `↑/↓` or `k/j` | Navigate JEs |
| `f` | Flag selected JE with a form + optional reason |
| `a` | Queue selected JE for AI review |
| `n` | Mark selected JE as non-deductible |
| `m` | Edit memo for selected JE |
| `Enter` | View JE detail (lines with accounts, debits, credits) |
| `R` | Run AI batch review on all `ai_pending` JEs |
| `u` | Update tax reference library (fetch IRS publications) |
| `c` | Configure enabled forms |
| `←/→` | Cycle fiscal year |
| `Ctrl+K` | Open AI chat with tax context |

**Flagging behavior:** `f` and `n` work on ANY status — unreviewed, ai_suggested,
confirmed, or non_deductible. Users can always correct a classification. Re-flagging
overwrites the previous form_tag, status, and reason.

### Progress Indicator

Status bar or tab header shows: `"Tax Review: 47/200 reviewed (23%)"` — count of JEs
with any status other than `unreviewed` vs total posted JEs in the fiscal year.

---

## Manual Flagging Flow

When the user presses `f`:

1. **Form picker modal** opens showing only enabled forms
2. User selects a form with arrow keys + Enter
3. **Reason input** opens: "Reason (optional):" — single-line text input
4. User types a reason or presses Enter to skip
5. Tax tag saved: `status = confirmed`, `form_tag = selected`, `reason = user's input`

When the user presses `n`:

1. **Reason input** opens: "Reason (optional):" — single-line text input
2. User types a reason or presses Enter to skip
3. Tax tag saved: `status = non_deductible`, `form_tag = non_deductible`, `reason = user's input`

---

## AI Batch Review Flow

1. User marks JEs with `a` key → status becomes `ai_pending`
2. User presses `R` to run batch review
3. Bursar collects all `ai_pending` JEs (batches of 25)
4. For each batch, sends to Claude with prompt caching enabled:
   - **Cached system prompt:** enabled form list with descriptions + IRS reference chunks
   - **User content:** JE details (date, memo, debit accounts, credit accounts, amounts)
   - **Instruction:** "For each JE, return one line: JE number | form_tag | reason"
5. Parse response — one line per JE, pipe-separated:
   ```
   JE-0004: schedule_c | Office supplies are ordinary business expenses
   JE-0005: non_deductible | Transfer between personal accounts
   ```
6. Each JE: `status = 'ai_suggested'`, `ai_suggested_form = parsed_tag`, `reason = parsed_reason`
7. Show summary: "Reviewed 25 JEs: 18 Schedule C, 3 Non-Deductible, 4 need review"

### AI Suggestion Review

After batch review, JEs with `ai_suggested` status show the suggestion in cyan with
`?` suffix. The user can:
- `Enter` → accept (status → `confirmed`, form_tag = ai_suggested_form, reason preserved)
- `f` → override with a different form + new reason
- `n` → mark as non-deductible + optional reason (overrides suggestion)

The `ai_suggested_form` is preserved in the database even after override, for audit trail.

### Prompt Caching

The batch review system prompt (enabled forms + descriptions + IRS reference chunks) is
identical across all batches in a single `R` run. Use `anthropic-beta: prompt-caching-2024-07-31`
header and mark the system prompt block with `cache_control: { type: "ephemeral" }`,
same pattern as the existing chat panel's prompt caching.

---

## IRS Tax Reference Library

`tax_reference` table with chunked IRS publication text. Triggered by `u` key in Tax tab.

### Ingestion

Same as previously specified — fetch HTML from `irs.gov/publications/p{number}`, parse
by `<h2>` headings, store chunks in SQLite. Full replace inside transaction. ~20 publications.

### AI Context (Tax Tab Only)

When Ctrl+K is opened from the Tax tab, the AI context includes:
- **Auto-included** (no tool call needed):
  - The highlighted JE's full details (accounts, debits, credits, memo)
  - The highlighted JE's tax tag (form_tag, status, reason) if one exists
  - IRS reference chunks matching the conversation keywords (keyword-to-tag matching)
  - The entity's chart of accounts and financial context
  - Citation instructions: "Cite as (Pub XXX, Section Name)"
- **Available via tools** (Claude can look up anything):
  - `get_tax_tag` — fetch any JE's tax classification by JE number (new tool)
  - `get_journal_entry` — fetch any JE's full details (existing tool)
  - All other existing read-only tools (accounts, GL, trial balance, etc.)

This means the user can ask about any JE by number (e.g., "why was JE-0012
classified as Schedule A?") regardless of which JE is currently highlighted.
Claude uses `get_tax_tag` + `get_journal_entry` to fetch the details, then
references the IRS publication chunks to explain the classification.

**Keyword mapping includes form names.** Asking "why Schedule C?" pulls reference
material tagged with `small_business,business_expense`. Asking "is this depreciation?"
pulls `depreciation,macrs` chunks. Both topic terms and form names are mapped.

When Ctrl+K is opened from any other tab, no tax reference is included and
`get_tax_tag` is not available. Normal accounting AI behavior.

---

## Tax Form Guide (Ctrl+H)

The user guide (opened via Ctrl+H) includes a "Tax Form Guide" section explaining
each form: what it is, who needs it, and what types of transactions go on it.

The `?` help overlay text for Ctrl+H changes to: `"Open user guide (& form guide)"`

### Guide Content

```
## Tax Form Guide

### Schedule C — Profit or Loss from Business
For sole proprietors and single-member LLCs. Report all business revenue
and deductible expenses: supplies, rent, utilities, advertising, insurance,
contract labor, vehicle expenses, home office, and more. Net profit flows
to your Form 1040 and is also subject to self-employment tax (Schedule SE).

### Schedule A — Itemized Deductions
Use instead of the standard deduction if your itemized total is higher.
Four subcategories:
- Medical & Dental: expenses exceeding 7.5% of AGI
- State & Local Taxes: income tax, property tax (capped at $10,000)
- Interest: mortgage interest, investment interest
- Charitable Contributions: cash/property donations to 501(c)(3) organizations

### Schedule D — Capital Gains and Losses
Report gains or losses from selling stocks, bonds, real estate, crypto, or
other capital assets. Short-term gains (held <1 year) taxed as ordinary income.
Long-term gains (held >1 year) taxed at favorable rates (0%, 15%, or 20%).

### Schedule E — Supplemental Income and Loss
Rental property income and expenses, royalties, and pass-through income from
partnerships (K-1) and S-Corps (K-1). Report rental revenue minus deductible
expenses: repairs, insurance, depreciation, property management fees.

### Schedule SE — Self-Employment Tax
Social Security (12.4%) + Medicare (2.9%) on net self-employment income from
Schedule C. Required if net SE income exceeds $400. You can deduct half of SE
tax as an adjustment to income on Form 1040.

### Form 4562 — Depreciation and Amortization
Claim depreciation on business assets: vehicles, equipment, furniture, buildings.
Includes Section 179 immediate expensing (up to $1,220,000 for 2024) and MACRS
depreciation schedules (3, 5, 7, 15, 27.5, or 39 year recovery periods).

### Form 8829 — Expenses for Business Use of Your Home
Home office deduction. Simplified method: $5 per square foot of dedicated workspace,
up to 300 sq ft ($1,500 max). Regular method: calculate actual expenses (mortgage
interest, rent, utilities, insurance, repairs) × business-use percentage.

### Form 4797 — Sales of Business Property
Gains or losses from selling business property (not inventory). Includes Section 1231
gains (favorable long-term capital gains treatment) and depreciation recapture
(taxed as ordinary income up to the amount of depreciation previously claimed).

### Form 1120-S — U.S. Income Tax Return for an S Corporation
S-Corps are pass-through entities: corporate income flows to shareholders via K-1.
Owner-employees must take a reasonable salary (subject to payroll taxes) before
taking distributions (not subject to SE tax). QBI deduction may apply.

### Form 1040-ES — Estimated Tax for Individuals
Quarterly estimated tax payments. Required if you expect to owe $1,000+ when you
file. Due dates: April 15, June 15, September 15, January 15 of the following year.
Underpayment penalties apply if you don't pay enough each quarter.
```

---

## Tax Summary Report

New report type in the Reports tab. Includes the reason for each classification.

```
╔══════════════════════════════════════════════════════════════╗
║                    Tax Summary by Form                       ║
║                    Entity: Trout Home LLC                     ║
║                    Period: Jan 1 – Dec 31, 2026               ║
╠══════════════════════════════════════════════════════════════╣
║                                                                ║
║  Schedule C — Business Income & Expenses                       ║
║  ───────────────────────────────────────────                    ║
║  Jan 15  JE-0004  Home Depot materials      $245.00            ║
║          Reason: Building supplies for rental repairs           ║
║  Jan 20  JE-0006  Office supplies            $67.99            ║
║          Reason: Printer paper, ink cartridges for home office  ║
║                                       Total: $312.99           ║
║                                                                ║
║  Schedule A — Charitable Contributions                         ║
║  ───────────────────────────────────────────                    ║
║  Mar 15  JE-0028  United Way donation       $500.00            ║
║          Reason: Annual charitable donation                    ║
║                                       Total: $500.00           ║
║                                                                ║
║  Non-Deductible: 87 entries (not listed)                       ║
║  Unreviewed:     47 entries                                    ║
║                                                                ║
╚══════════════════════════════════════════════════════════════╝
```

Only `confirmed` entries listed individually with reasons. Non-deductible and
unreviewed shown as counts.

---

## New Dependencies

| Crate | Purpose | Notes |
|-------|---------|-------|
| `scraper` | HTML parsing for IRS publication ingestion | Synchronous, well-maintained |

---

## Out of Scope (V4)

- **Split Draft feature** — splitting a mixed-category JE into multiple JEs for
  separate tax tagging. Planned for a future version. Workaround: manually split
  at the draft stage before posting.
- Per-line tax tagging (tagging is per-JE only)
- Tax form auto-fill or PDF generation
- Tax liability calculation
- State tax form classifications
- Multi-entity consolidated tax view
- Prior-year tax tag comparison
- Draft JE tagging (posted only)
- Automatic re-ingestion of IRS publications (manual trigger only)

---

## Implementation Order

5 phases:

1. **Tab restructuring + schema** — Move Audit Log, create Tax tab shell, tax_tags table (with reason column), form config, memo simplification, `m` key on JE tab
2. **Tax reference library** — scraper dependency, HTML fetch/parse, tax_reference table, `u` hotkey
3. **Tax review workflow** — JE list view, manual flagging with reason input, form picker, memo editing, fiscal year selector, form guide in Ctrl+H
4. **AI batch review** — Queue management, batch send with prompt caching and pipe-separated response, suggestion parsing with reasons, accept/reject flow, tax-scoped AI context
5. **Tax summary report + documentation** — Report with reasons per entry, CLAUDE.md, user guide
