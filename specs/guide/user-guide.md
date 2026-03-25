# User Guide — Double-Entry Bookkeeping TUI

## How This Guide Works

This guide is organized by tab. Each tab section lists its features, and each feature
explains the step-by-step process. Access this guide in-app via `Ctrl+H` from any screen.

---

## Understanding Double-Entry Accounting (Start Here)

Every financial transaction is recorded as a **journal entry** with at least two lines.
One line is a **debit** and the other is a **credit**. Debits must always equal credits.

**Quick rules:**
- **Spending money**: Debit the expense account, Credit the cash account
- **Receiving money**: Debit the cash account, Credit the revenue account
- **Someone owes you**: Debit Accounts Receivable, Credit the revenue account
- **You owe someone**: Debit the expense account, Credit Accounts Payable
- **Paying off a debt**: Debit the liability account, Credit the cash account

**Account types and their normal balances:**
- **Assets** (what you own): normal debit balance — debits increase, credits decrease
- **Liabilities** (what you owe): normal credit balance — credits increase, debits decrease
- **Equity** (owner's stake): normal credit balance — credits increase, debits decrease
- **Revenue** (money earned): normal credit balance — credits increase
- **Expenses** (money spent): normal debit balance — debits increase

If this is confusing, don't worry. The app validates every entry — it won't let you post
something that doesn't balance.

---

## Global Controls

These work from any tab:

| Key | Action |
|-----|--------|
| `1`–`9` / `0` | Jump to tab by number (`0` = Audit Log; only when no form/modal is open) |
| `Ctrl+←` / `Ctrl+→` | Cycle to the previous / next tab (wraps around) |
| `Ctrl+K` | Open / close the AI Accountant panel |
| `y` | Open fiscal period management (close/reopen periods, year-end close) |
| `?` | Show hotkey quick-reference for the current tab; also shows Feedback section (`b` / `f`) |
| `Ctrl+H` | Open this user guide (& form guide) |
| `q` | Quit the application |

---

## Tab 1: Chart of Accounts (CoA)

The Chart of Accounts is your master list of categories for tracking money. Think of each
account as a bucket: Cash, Rent, Revenue, Equipment, etc.

### Browsing Accounts
- Use `↑↓` or `j/k` to scroll through the list
- Accounts are organized in a hierarchy — parent accounts (marked `P`) group related sub-accounts
- Press `Enter` on a parent account to expand or collapse its children
- Press `Enter` on a leaf account (no children) to jump to its General Ledger
- Press `/` to search accounts by name or number

### Adding a New Account
1. Press `a` to open the Add Account form
2. Fill in: account number, name, type (Asset/Liability/Equity/Revenue/Expense)
3. Optionally select a parent account (press Enter on the parent field to open the account picker)
4. Set flags: Placeholder (category-only, can't post to it), Contra (opposite normal balance)
5. Press `Enter` to save

**Tip:** Use a consistent numbering scheme. The default seed uses: 1000s = Assets,
2000s = Liabilities, 3000s = Equity, 4000s = Revenue, 5000s = Expenses.

### Editing an Account
1. Select the account and press `e`
2. Change the name or number (account type cannot be changed after creation)
3. Press `Enter` to save

### Deactivating / Reactivating an Account
1. Select the account and press `d`
2. Confirm when prompted
3. Deactivated accounts show an `x` flag and are hidden from account pickers
4. Press `d` again on a deactivated account to reactivate it

### Deleting an Unused Account
1. Select the account and press `x`
2. This only works if the account has never been used (no journal entries, no AR/AP items,
   no child accounts, no envelope allocations, no fixed asset details)
3. Confirm when prompted — deletion is permanent

### Place in Service (CIP → Fixed Asset)
This is how you convert a construction/purchase project into a depreciable asset.
1. First, post journal entries to the **Construction in Progress** account to record costs
2. Select the CIP account (1400) and press `s`
3. Fill in: target fixed asset account, accumulated depreciation account, depreciation
   expense account, in-service date, useful life in months
4. The app generates a transfer journal entry moving the cost from CIP to the fixed asset
5. The asset now appears on the Fixed Assets tab and begins generating depreciation

### Understanding the Columns
- **Number**: Your account numbering code
- **Name**: Account description (indented to show hierarchy)
- **Type**: Asset, Liab(ility), Equity, Rev(enue), Exp(ense)
- **Balance**: Current account balance from posted journal entries
- **Earmarked/Avail**: If envelope budgeting is configured for this account, shows remaining
  budget available (only appears when envelopes are configured)
- **Flags**: P = Placeholder, C = Contra, x = Deactivated

---

## Tab 2: General Ledger (GL)

The General Ledger shows the complete transaction history for a single account.

### Viewing an Account's History
1. Navigate here from the CoA tab by pressing `Enter` on a leaf account, or switch to
   tab `2` and select an account
2. The ledger shows every posted journal entry line that affected this account
3. Columns: Date, JE Number, Memo, Debit, Credit, Running Balance

### Filtering by Date
- Use the date filter to narrow the view to a specific period

### Navigating to Source Entries
- Press `Enter` on any ledger row to jump to that journal entry on the Journal Entries tab

### Understanding the Running Balance
The running balance accumulates from top to bottom. For expense accounts (debit-normal),
debits increase the balance. For revenue accounts (credit-normal), credits increase the balance.

---

## Tab 3: Journal Entries

Journal Entries are the core of double-entry accounting. Every financial transaction
is recorded here.

### Viewing Entries
- Scroll through all entries with `↑↓`
- Press `Enter` to view an entry's line items (the individual debits and credits)
- Press `f` to cycle the filter: All → Draft → Posted
- In the detail view, press `g` on a line to jump to that account's General Ledger

### Creating a New Entry
1. Press `a` to open the entry form
2. Fill in the date (YYYY-MM-DD format) and memo
3. For each line:
   - Press `Enter` on the Account field to open the account picker
   - Type the debit OR credit amount (not both — one side per line)
   - Optionally add a line note
4. Press `Ctrl+↓` to add a row below the current line, `Ctrl+↑` to remove the current row
5. Watch the totals at the bottom — Debits must equal Credits (shows green ✓ when balanced)
6. Press `Ctrl+S` to submit the entry as a Draft

**Form navigation:** Use `Tab` / `Shift+Tab` to move forward/backward through fields. You can
also use arrow keys: `↑`/`↓` move between rows, `←`/`→` move between columns (Account, Debit,
Credit, Note) within the same row. `Enter` advances to the next field (same as `Tab`).

**The Avail column** shows your remaining envelope budget for that account. This helps you
see if you're about to overspend a budgeted category.

### Editing a Draft Entry
Draft entries can be corrected before posting.
1. Select a Draft entry and press `e`
2. The form opens pre-populated with the existing date, memo, and line items
3. Make your changes — the form works exactly like creating a new entry
4. Press `Ctrl+S` to save; press `Esc` to cancel without saving

Only **Draft** entries can be edited. Posted and reversed entries cannot be changed
(use the reverse-and-repost workflow for those — see "Reversing a Posted Entry" below).

### Posting a Draft Entry
1. Select a Draft entry and press `p`
2. Confirm when prompted
3. The entry becomes permanent — balances update, envelope fills trigger (if it's a cash receipt)

**The app validates before posting:**
- Debits must equal credits
- All accounts must be active and not placeholders
- The fiscal period must be open
- At least 2 lines required

### Reversing a Posted Entry
Posted entries cannot be edited. To correct a mistake:
1. Select the posted entry and press `r`
2. Enter the reversal date
3. Confirm when prompted
4. The app creates a new entry with all debits and credits swapped, linked to the original

### Marking Lines as Cleared
In the entry detail view (press `Enter` on an entry):
1. Select a line and press `c` to mark it as Cleared (shows ✓)
2. Press `c` again to un-clear it
3. Cleared marks help you track which transactions you've verified against bank statements

### Creating a Scheduled Entry
1. Select a posted entry and press `t`
2. Choose the frequency: Monthly, Quarterly, or Annually
3. Set the start date for the first recurrence
4. The app will prompt you to generate entries when they're due (at startup and on-demand)

### Viewing Scheduled Entries
1. Press `s` to open the scheduled entries sub-view
2. See all entries with their source JE number, memo, frequency, next due date, and status
3. Color coding: **red** = overdue, **yellow** = due today, **gray** = inactive
4. Press `Enter` to jump to the source journal entry in the main list
5. Press `g` to generate all due entries (creates them as Drafts for review)
6. Press `d` to toggle a template active/inactive
7. Press `Esc` to return to the main entry list

### Inter-Entity Journal Entry
If you manage two entities (e.g., two LLCs), you can post a transaction that affects both:
1. Press `i` from the Journal Entries tab (requires two entities in workspace config)
2. Select the second entity when prompted
3. The split-pane form opens: entry form on top, both entities' accounts on the bottom
4. Fill in lines for both entities — each side must balance independently
5. Submit — the app writes to both entity databases with linked entries

**Example — LLC A pays LLC B $1,000 for rent:**
- Entity A: Debit Rent Expense $1,000, Credit Checking $1,000
- Entity B: Debit Checking $1,000, Credit Service Revenue $1,000

### Importing from Bank Statements
Press `u` to import a bank statement CSV file and automatically categorize transactions.

**The import wizard walks you through:**
1. **File browser** — navigate to your CSV file using the arrow keys; press Enter to open a
   directory or select a `.csv` file, Backspace to go up to the parent directory, and Esc to
   cancel. The browser opens in the last directory you imported from (or your home directory).
2. **Bank selection** — pick a known bank or set up a new one (the app detects column layout)
3. **Account linking** — confirm which Chart of Accounts entry is your bank account
4. **Duplicate check** — transactions already imported are skipped automatically
5. **Matching** — three passes categorize transactions:
   - Pass 1: exact matches from previously learned patterns + transfer detection (instant)
   - Pass 2: AI categorization for remaining transactions (requires API key)
   - Pass 3: conversational clarification for low-confidence items (chat panel)
6. **Review** — inspect all matches; the review screen has two sections:
   - **Transfer Matches** (top, if any) — inter-account transfers detected automatically
   - **All other transactions** — normal categorized and uncategorized items
7. **Confirm** — press `Enter` to create Draft journal entries for all approved items

**After importing**, review the Draft entries on the Journal Entries tab, make any corrections
via `e` (edit), then post them with `p`.

**Re-matching incomplete imports:** Press `Shift+U` to re-run AI matching on Draft entries
that have only one journal line (the bank line was created but the contra account was not
matched). This is useful when Pass 2 was skipped (e.g., no API key) or failed.

### Transfer Matches

When you import bank statements from multiple accounts (e.g., checking and credit card), the
same transfer appears twice: once as a withdrawal from Account A and once as a deposit to
Account B. Without detection, both sides get imported as separate journal entries,
double-counting the transfer.

**How it works:** During Pass 1, each new transaction is compared against existing Draft
journal entries. If a Draft is found with a negated amount within $3 and a date within 3
calendar days, it is flagged as a transfer match.

**Transfer Matches section in the review screen:**
```
─── Transfer Matches (2) ──────────────────────────────────────────
  ✓  Jan 14  +$500.00   "ACH Deposit Chase"  →  JE #47  (-$500, Jan 14)
  ✓  Jan 18  +$2000.00  "Payment Thank You"  →  JE #62  (-$2000, Jan 17)
```

Each row shows:
- `✓` (green) — will be confirmed (skip import, link to existing JE)
- `✗` (red) — rejected (will be imported as a new draft instead)

**Navigation:**
| Key | Action |
|-----|--------|
| `↑` / `↓` | Move between transfer match rows and the rest of the review |
| `Enter` or `Space` | Toggle confirm (`✓`) / reject (`✗`) for the selected match |
| `Enter` (on Approve button) | Submit the review screen |

**When a match is confirmed:**
- No new Draft JE is created for this transaction
- The current transaction's import reference is linked to the existing Draft JE
- Future imports from either bank will detect this transaction as a duplicate (no re-import)
- The existing Draft's accounts are unchanged — fix any incorrect accounts during normal draft review

**When a match is rejected:**
- A new Draft JE is created with the bank line only (no contra account)
- Edit it on the Journal Entries tab to add the correct offsetting account before posting

---

## Tab 4: Accounts Receivable (AR)

Accounts Receivable tracks money that customers owe you.

### The AR Workflow
1. **Record the revenue** — Post a journal entry: Debit AR, Credit Revenue
2. **Create the AR item** — Press `n` on the AR tab: customer name, description, amount,
   due date, and the JE number from step 1
3. **Receive payment** — Post a journal entry: Debit Cash, Credit AR
4. **Record the payment** — Press `p` on the AR item, link it to the payment JE
5. Status updates automatically: Open → Partial (if not fully paid) → Paid

### Creating an AR Item
1. Press `n`
2. Enter: customer name, description, amount owed, due date
3. Enter the originating journal entry number (the one that created the receivable)

### Recording a Payment
1. Select the AR item and press `p`
2. Choose: auto-create the payment JE (the app builds it for you) or manually link to
   an existing JE
3. Enter the payment amount and date
4. If the payment is less than the total, status becomes Partial
5. When fully paid, status becomes Paid (permanent — cannot be reopened)

### Viewing Payment History
- Press `Enter` on an AR item to see all payments received

### Navigating to the Source Entry
- Press `o` on an AR item to jump to its originating journal entry

### Overdue Items
Items past their due date are highlighted. The **Days** column shows how many days outstanding.

---

## Tab 5: Accounts Payable (AP)

Accounts Payable tracks money you owe to vendors. It works identically to AR but from
the other side.

### The AP Workflow
1. **Record the expense** — Post a journal entry: Debit Expense, Credit AP
2. **Create the AP item** — Press `n`: vendor name, description, amount, due date, JE number
3. **Make payment** — Post a journal entry: Debit AP, Credit Cash
4. **Record the payment** — Press `p`, link to the payment JE

### Keys
Same as AR: `n` (new), `p` (payment), `Enter` (payment history), `o` (open source JE),
`s` (cycle status filter).

---

## Tab 6: Envelopes

Envelope budgeting lets you earmark portions of incoming cash for specific purposes.
Think of it as putting money into labeled envelopes: "this much for rent, this much
for insurance, this much for repairs."

### How Envelopes Work
1. You set a percentage for each account you want to budget (e.g., Rent = 15%)
2. When you post a journal entry that brings cash in (debit to a Cash/Bank account),
   the app automatically calculates: 15% of that cash → earmarked for Rent
3. As you spend money (post expense entries), the Available amount decreases
4. If Available goes negative, you've overspent that envelope

**Important:** Envelopes are budgetary only. They don't create journal entries or
affect your actual account balances. They're a planning layer on top of your real accounting.

### Setting Allocations (Allocation Config view)
1. On the Envelopes tab, you start in the Allocation Config view
2. Select an account and press `Enter` to edit its percentage
3. Type the percentage (e.g., "15.5") and press `Enter` to save
4. Press `d` to remove an allocation
5. Percentages don't need to add up to 100% — unallocated cash simply stays unearmarked

### Viewing Balances (Balances view)
1. Press `v` to switch from Allocation Config to the Balances view
2. Columns: Account Name, Allocation %, GL Balance (amount spent), Earmarked (budgeted amount),
   Available (budget remaining)
3. Use `←→` arrow keys to switch between fiscal years
4. The header shows which fiscal year you're viewing (e.g., "FY 2026")

### Transferring Between Envelopes
Sometimes you need to move budget from one envelope to another:
1. In the Balances view, press `t`
2. Select the source envelope (where to take money from)
3. Select the destination envelope (where to put it)
4. Enter the amount
5. Confirm — this is purely budgetary, no journal entry is created

### What Triggers Envelope Fills
- **Cash receipts** (journal entries that debit a Cash/Bank account): fills occur
- **Owner's Capital contributions**: fills occur
- **Owner's Draw**: fills do NOT occur
- **Non-cash entries** (e.g., accrual adjustments): no fills
- **Reversals**: if you reverse a cash receipt, the fills are automatically undone

---

## Tab 7: Fixed Assets

The Fixed Assets tab shows your depreciable property and tracks depreciation over time.

### The Fixed Asset Workflow
1. **Record the purchase** — Post a JE: Debit Construction in Progress, Credit Cash
2. **Place in service** — On the CoA tab, select the CIP account, press `s`, fill in the
   target asset account, depreciation accounts, in-service date, and useful life
3. **Generate depreciation** — On the Fixed Assets tab, press `g` to create monthly
   depreciation entries (created as Drafts for your review)
4. **Post depreciation** — Go to Journal Entries tab, review and post the depreciation drafts

### Viewing the Asset Register
The register shows for each asset:
- Asset name and account number
- Cost basis (original purchase price)
- In-service date
- Useful life (months)
- Monthly depreciation amount
- Accumulated depreciation to date
- Current book value (cost − accumulated depreciation)

**Land** is flagged as non-depreciable and shows no depreciation fields.

### Generating Depreciation
1. Press `g` to generate pending depreciation entries
2. The app calculates monthly straight-line depreciation: Cost ÷ Useful Life in Months
3. Entries are created as Drafts from the in-service date through the current fiscal period
4. Review and post them on the Journal Entries tab
5. The final month of an asset's life absorbs any rounding remainder so total depreciation
   exactly equals the cost basis

### Understanding Depreciation
Depreciation spreads the cost of an asset over its useful life. A $12,000 piece of equipment
with a 12-month useful life depreciates at $1,000/month. Each month:
- Depreciation Expense increases by $1,000 (your cost of using the asset that month)
- Accumulated Depreciation increases by $1,000 (total wear recorded)
- Book Value decreases by $1,000 (what the asset is "worth" on paper)

---

## Tab 8: Reports

Generate formatted accounting reports as `.txt` files.

### Available Reports

| Report | What It Shows | Parameters |
|--------|---------------|------------|
| Trial Balance | All accounts with balances — proves debits = credits | As-of date |
| Balance Sheet | Assets = Liabilities + Equity snapshot | As-of date |
| Income Statement | Revenue − Expenses for a period | Date range |
| Cash Flow Statement | Cash in/out for a period (direct method) | Date range |
| Account Detail | Full transaction history for one account | Date range + account |
| AR Aging | Open receivables by age (current, 30, 60, 90+ days) | As-of date |
| AP Aging | Open payables by age | As-of date |
| Fixed Asset Schedule | All assets with cost, depreciation, book value | As-of date |
| Envelope Budget Summary | Earmarked vs. GL spending vs. available for each allocated account | Date range |
| Tax Summary | Confirmed JEs grouped by tax form with reasons; counts of non-deductible and unreviewed | Date range |

### Generating a Report
1. Select a report from the list
2. Enter the required parameters (date or date range, account if needed)
3. Press `Enter` to generate
4. The report is saved as a `.txt` file in your configured reports directory
5. A confirmation message shows the file path

### Understanding Key Reports

**Trial Balance** — The fundamental check. If Total Debits ≠ Total Credits, something is wrong.
Run this regularly.

**Balance Sheet** — Shows your financial position at a point in time. The equation
Assets = Liabilities + Equity should hold (it will balance perfectly after year-end close;
mid-year, current earnings appear as the gap).

**Income Statement** — Shows profitability over a period. Revenue minus Expenses = Net Income.

**Envelope Budget Summary** — Shows each account with an envelope allocation: the percentage
allocated, how much has been earmarked (fills/transfers in the period), how much has been
spent (GL balance change in the period), and what remains available (Earmarked − GL Balance).
An "Unallocated" line at the bottom shows what percentage of revenue is not yet assigned to
any envelope.

---

## Tab 9: Tax Workstation

The Tax tab is your tax preparation workspace. It shows all posted journal entries for
the selected fiscal year with their review status, so you can classify each transaction
to the right IRS form — manually or with AI assistance.

### Navigating the Tax List

| Key | Action |
|-----|--------|
| `↑/↓` or `k/j` | Move up / down through entries |
| `←/→` | Switch fiscal year |
| `Enter` | View full JE detail (lines, accounts, amounts) — press again or `Esc` to close |

### Classifying Entries

| Key | Action |
|-----|--------|
| `f` | Flag selected entry with a tax form + optional reason |
| `n` | Mark selected entry as non-deductible + optional reason |
| `a` | Queue selected entry for AI batch review |
| `m` | Edit the journal entry memo |

**`f` — Flag with form:**
1. A form picker opens listing your enabled forms
2. Select a form with arrow keys + `Enter`
3. Enter an optional reason (or press `Enter` to skip)
4. Entry status becomes **Confirmed** (shown in green)

**`n` — Non-deductible:**
1. Enter an optional reason (or press `Enter` to skip)
2. Entry status becomes **Non-Deductible** (shown in gray)

**Re-flagging:** Both `f` and `n` work on any status — you can always correct a
classification. Re-flagging overwrites the previous form, status, and reason.

### AI Batch Review

| Key | Action |
|-----|--------|
| `a` | Queue selected entry for AI review (status → AI Pending, shown in yellow) |
| `R` | Run AI batch review on all queued entries |

After batch review, AI-suggested entries appear in **cyan** with a `?` suffix on the
form name (e.g., `Schedule C?`). Press `Enter` to accept the suggestion, `f` to
override with a different form, or `n` to mark as non-deductible.

### Tax Reference Library

| Key | Action |
|-----|--------|
| `u` | Fetch/update IRS publication text (requires internet connection) |

When the AI chat panel (`Ctrl+K`) is opened from the Tax tab, it automatically includes
relevant IRS publication excerpts and the selected entry's full details. Ask questions
like "Is this home office expense deductible?" and get sourced answers citing IRS pubs.

### Form Configuration

| Key | Action |
|-----|--------|
| `c` | Configure which tax forms are enabled |

Press `Space` to toggle a form on/off. Only enabled forms appear in the `f` picker.
All forms are enabled by default. Your configuration is saved per-entity.

### Status Colors

| Color | Status | Meaning |
|-------|--------|---------|
| Dim/gray | Unreviewed | Entry has not been classified |
| Yellow | AI Pending | Queued for AI batch review |
| Cyan | AI Suggested | AI has classified this entry (review pending) |
| Green | Confirmed | Classification confirmed |
| Gray | Non-Deductible | Entry marked as non-deductible |

### Progress Indicator

The tab header shows `Tax Review: 47/200 (23%)` — the count of reviewed entries
(any status other than Unreviewed) vs total posted entries in the fiscal year.

### Tax Summary Report

Once entries are classified, generate the Tax Summary report from the **Reports** tab.
It lists confirmed entries grouped by form with their reasons, plus counts of
non-deductible and unreviewed entries. Share this with your accountant.

### Tax Disclaimer

This tool organizes your financial records to assist with tax preparation. It does not
constitute tax advice. Tax laws vary by jurisdiction, change frequently, and depend on
your specific circumstances. Always consult a qualified tax professional before filing.
The AI guidance and IRS publication excerpts provided are for informational purposes only.

---

## Tab 0: Audit Log

Every change in the system is recorded here. The audit log is read-only and cannot be
modified or deleted.

### Viewing the Log
- Scroll through events chronologically
- Each entry shows: timestamp, action type, and a description of what happened

### Filtering
- Filter by date range
- Filter by action type (cycle with `←→`): journal entries posted, accounts created,
  periods closed, envelope changes, etc.

### What Gets Logged
- Journal entries: created, posted, reversed
- Accounts: created, modified, deactivated, reactivated, deleted
- Fiscal periods: closed, reopened
- Year-end close
- Envelope allocations changed, transfers
- Fixed assets placed in service
- Inter-entity entries posted

---

## Fiscal Period Management

Access from any tab by pressing `f`.

### What Are Fiscal Years and Periods?

A **fiscal year** spans January 1 – December 31 and is divided into 12 monthly **periods**
named P01 through P12:

| Period | Month     | Date range           |
|--------|-----------|----------------------|
| P01    | January   | Jan 1 – Jan 31       |
| P02    | February  | Feb 1 – Feb 28/29    |
| P03    | March     | Mar 1 – Mar 31       |
| P04    | April     | Apr 1 – Apr 30       |
| P05    | May       | May 1 – May 31       |
| P06    | June      | Jun 1 – Jun 30       |
| P07    | July      | Jul 1 – Jul 31       |
| P08    | August    | Aug 1 – Aug 31       |
| P09    | September | Sep 1 – Sep 30       |
| P10    | October   | Oct 1 – Oct 31       |
| P11    | November  | Nov 1 – Nov 30       |
| P12    | December  | Dec 1 – Dec 31       |

Each period has a status of **Open** or **Closed**. Journal entries can only be created in
open periods. **You must create a fiscal year before entering or importing transactions for
that year.**

### Hotkeys

**Global (any tab):**

| Key  | Action                  |
|------|-------------------------|
| `y`  | Open fiscal period manager |

**Inside the fiscal manager:**

| Key  | Action                                           |
|------|--------------------------------------------------|
| `a`  | Add a new fiscal year (creates all 12 periods)   |
| `c`  | Close the selected period                        |
| `o`  | Reopen a closed period                           |
| `y`  | Year-end close (zeroes revenue/expense accounts) |
| `Esc`| Close fiscal manager                             |

### Typical Workflow

1. Press `y` to open the fiscal period manager.
2. Press `a` to add a new fiscal year — type the year (e.g. `2026`) and press `Enter`.
3. All 12 periods (P01–P12) are created in **Open** status immediately.
4. Enter or import transactions for that year normally.
5. After reconciling a month, select it and press `c` to close it. Closing a period locks
   all journal entries in that month — no new postings or reversals are allowed.
6. At year-end, after all December entries are posted, press `y` to run the year-end close.
   The app generates closing entries and resets Revenue and Expense balances to zero.

### Adding a Fiscal Year

1. Press `y` to open the fiscal period manager
2. Press `a` — a prompt asks for the fiscal year number
3. Type the four-digit year (e.g. `2026`) and press `Enter`
4. Twelve periods (P01–P12) are created automatically, all in Open status

### Closing a Period

1. Press `y` to open the fiscal period manager
2. Select a period using the arrow keys and press `c` to close it
3. Confirm — all journal entries in that period become locked (cannot be posted, reversed,
   or modified)

### Reopening a Period

1. Select a closed period and press `o`
2. Confirm — entries in that period become editable again
3. Use with caution — reopening a period you've already reported on can cause discrepancies

### Year-End Close

1. Press `y` in the fiscal period manager
2. The app generates closing entries:
   - All Revenue accounts are zeroed out
   - All Expense accounts are zeroed out
   - Net income (Revenue − Expenses) posts to Retained Earnings
3. Review the generated entries
4. Confirm to post — the fiscal year is marked as closed

**When to do this:** At the end of your fiscal year, after all entries for the year are posted
and reviewed. This resets Revenue and Expense accounts to zero for the new year.

---

## AI Accountant Panel

Press `Ctrl+K` from any tab to open the AI Accountant panel on the right side of the screen.
The panel gives you a conversational AI assistant with read-only access to your books.

### Opening and Closing
- `Ctrl+K` — toggle the panel open or closed from any screen
- `Esc` or `Ctrl+K` while the panel is focused — close the panel

### Focus Switching
When the panel is open, the keyboard focus can be on either the panel or the main tab:
- `Tab` — switch focus between the panel and the main tab
- When focus is on the panel, all typing goes to the chat input
- When focus is on the main tab, all hotkeys work normally

### Sending Messages
1. Type your question or request in the input area at the bottom of the panel
2. Press `Enter` to send
3. If a response is still being typed out (typewriter animation), press `Enter` to skip to the end

### Slash Commands
Type a slash command in the input area and press `Enter`:

| Command | Action |
|---------|--------|
| `/clear` | Reset the conversation history |
| `/context` | Refresh the context snapshot sent with messages (re-reads current tab data) |
| `/compact` | Compress conversation history to save space while keeping key context |
| `/persona [text]` | View or update the AI persona for this entity |
| `/match` | Re-match the selected draft journal entry against import rules |

### What the AI Can Do
- Answer questions about your accounts, balances, and transactions
- Explain journal entries or account history
- Help with accounting questions specific to your books
- Suggest how to categorize a transaction

### What the AI Cannot Do
- Post, edit, or delete journal entries (read-only access)
- Modify accounts, envelopes, or any other data
- Access data outside the currently open entity

---

## Feedback

You can report bugs or request features without leaving Bursar. Feedback is only available
after opening an entity (not from the startup screen).

### Reporting a Bug or Requesting a Feature

1. Press `?` to open the help overlay
2. Press `b` to report a bug, or `f` to request a feature
3. The help overlay closes and a text input modal appears
4. Type your description (multi-line; use Enter for new lines)
5. Press `Ctrl+S` to submit, or `Esc` to cancel
6. Bursar opens your browser with a pre-filled GitHub issue containing your description,
   system information, and (for bugs) your recent audit log entries
7. Review and submit the issue on GitHub

### Keys in the Feedback Modal

| Key | Action |
|-----|--------|
| `Ctrl+S` | Submit and open issue in browser |
| `Esc` | Cancel without submitting |
| `Enter` | New line |
| `←` / `→` | Move cursor (wraps across lines) |
| `↑` / `↓` | Move cursor up/down (column clamped to line length) |
| `Home` / `End` | Start / end of current line |
| `Backspace` | Delete character before cursor (merges lines at line start) |

---

## Common Workflows

### Recording a Sale
1. Journal Entry: Debit AR (or Cash if paid immediately), Credit Revenue
2. If on credit: create an AR item to track the receivable
3. When paid: Journal Entry Debit Cash, Credit AR → record payment on AR item

### Paying a Bill
1. Journal Entry: Debit Expense (e.g., Utilities), Credit Cash (or AP if paying later)
2. If paying later: create an AP item to track the payable
3. When paying: Journal Entry Debit AP, Credit Cash → record payment on AP item

### Owner Putting Money into the Business
1. Journal Entry: Debit Cash, Credit Owner's Capital
2. This triggers envelope fills (cash is coming in)

### Owner Taking Money Out
1. Journal Entry: Debit Owner's Draw, Credit Cash
2. This does NOT trigger envelope fills

### Buying Equipment
1. Journal Entry: Debit Construction in Progress, Credit Cash
2. CoA tab → select CIP → press `s` → Place in Service
3. Fixed Assets tab → press `g` → generate depreciation → post the drafts

### Correcting a Mistake
Posted entries cannot be edited. Instead:
1. Select the entry on the Journal Entries tab
2. Press `r` to reverse it (creates a mirror entry that cancels it out)
3. Create a new, correct entry

### Month-End Routine
1. Generate and post any pending depreciation (Fixed Assets tab → `g`)
2. Generate and post any due scheduled entries (Journal Entries tab → `R` → `g`)
3. Review the Trial Balance report (debits should equal credits)
4. Close the month (press `f` → select the period → `c`)

### Year-End Routine
1. Complete all month-end routines for the final month
2. Review the Income Statement for the full year
3. Review the Balance Sheet
4. Press `f` → `y` to run year-end close
5. Review and post the closing entries
6. Create the new fiscal year when prompted

---

## Tax Form Guide

This section explains each IRS tax form available in the Tax tab classifier.
Access via `Ctrl+H` from any tab.

### Schedule C — Profit or Loss from Business
For sole proprietors and single-member LLCs. Report all business revenue
and deductible expenses: supplies, rent, utilities, advertising, insurance,
contract labor, vehicle expenses, home office, and more. Net profit flows
to your Form 1040 and is also subject to self-employment tax (Schedule SE).

### Schedule A — Itemized Deductions
Use instead of the standard deduction if your itemized total is higher.
Four subcategories in this app:
- **Medical & Dental**: expenses exceeding 7.5% of AGI
- **State & Local Taxes**: income tax, property tax (capped at $10,000)
- **Interest**: mortgage interest, investment interest
- **Charitable Contributions**: cash/property donations to 501(c)(3) organizations

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
