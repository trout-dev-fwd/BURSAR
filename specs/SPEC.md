# Double-Entry Bookkeeping TUI — Technical Specification

## Overview

A terminal user interface (TUI) accounting application written in Rust using Ratatui,
with SQLite as the database backend. Each legal entity has its own `.sqlite` file.
The application supports up to two entities open simultaneously via a split-screen
inter-entity transaction workflow. Accrual-basis accounting with envelope budgeting
layered on top via cash receipts. Single-user, no authentication, fully keyboard-driven.

**Who**: A single business owner managing 1–2 LLCs (e.g., a land-holding entity and a
rental entity) who needs proper double-entry accounting with intercompany transaction support.

**Why**: Existing accounting software either lacks intercompany journal entry workflows,
requires cloud subscriptions, or doesn't support envelope budgeting layered on top of
accrual accounting. This tool runs locally, costs nothing, and does exactly what's needed.

---

## Success Criteria

The application is "done" when all of the following are true:

- [ ] Entity creation produces a valid SQLite file with full schema and seeded chart of accounts
- [ ] Journal entries can be created, posted, and reversed with full validation
  (balanced debits/credits, active non-placeholder accounts, open fiscal period)
- [ ] Account balances are correct and verifiable via Trial Balance report (debits = credits)
- [ ] AR/AP items track through their full lifecycle (Open → Partial → Paid) with multiple payments
- [ ] Envelope allocations auto-fill on cash receipts at configured percentages
- [ ] Envelope transfers move earmarked dollars between accounts without GL impact
- [ ] Fixed assets can be placed in service from CIP accounts with auto-generated transfer entries
- [ ] Straight-line depreciation generates correct monthly amounts with final-month rounding
- [ ] Fiscal periods can be closed (locking all entries) and reopened (with confirmation)
- [ ] Year-end close zeroes Revenue/Expense accounts and posts net income to Retained Earnings
- [ ] All 8 reports generate correctly formatted `.txt` files with box-drawing characters
- [ ] Inter-entity journal entries post to both entity databases with matching UUIDs
- [ ] Inter-entity failure recovery detects and resolves orphaned drafts on startup
- [ ] Recurring entry templates generate correct draft entries on schedule
- [ ] Audit log records every mutation with human-readable descriptions
- [ ] All cross-tab navigation paths work (CoA → GL → JE, AR → JE, JE → GL)
- [ ] `cargo fmt && cargo clippy -D warnings && cargo test` all pass with zero warnings
- [ ] Full integration test exercises the complete lifecycle end-to-end

---

## Tech Stack

| Component          | Choice                                         | Notes                                    |
|--------------------|-------------------------------------------------|------------------------------------------|
| Language           | Rust (stable toolchain)                         | Managed via rustup                       |
| TUI framework      | `ratatui` + `crossterm`                         | Synchronous event loop, 500ms tick       |
| Database           | SQLite via `rusqlite` (`bundled` feature)        | One `.sqlite` file per entity            |
| Config             | `serde` + `toml`                                 | `workspace.toml` for entity registry     |
| Dates              | `chrono` (`NaiveDate`, `NaiveDateTime`)           |                                          |
| UUIDs              | `uuid`                                           | Inter-entity IDs, transfer group IDs     |
| Error handling     | `thiserror` (domain) + `anyhow` (CLI boundary)   |                                          |
| Logging            | `tracing` + `tracing-subscriber`                  | No `println!` in library code            |
| Async runtime      | **None**                                         | Synchronous. No tokio.                   |
| Fuzzy search       | **None** (substring/prefix match)                 | Simple, no extra dependency               |

---

## Specification Documents

This is the master document. All technical detail lives in the sub-documents below.
Read the relevant files for your current task — do not try to hold all specs in context
simultaneously.

### Always read (every session)
| File | Purpose |
|------|---------|
| `CLAUDE.md` | Project root. Coding style, verification commands, spec file index. |
| `specs/implementation-protocols.md` | Session management, commit protocol, rollback rules, progress tracking, test-first rules. |
| `specs/boundaries.md` | Always Do / Ask First / Never Do guardrails. |
| `specs/progress.md` | Current state: active phase, completed tasks, next task, decisions log. |

### Read on demand (when the task requires it)
| File | Purpose |
|------|---------|
| `specs/data-model.md` | Complete SQLite schema — all 14 tables, column definitions, design decisions, integrity invariants. |
| `specs/type-system.md` | Rust newtypes (`Money`, `Percentage`, IDs), all enums with `FromStr`/`Display`, state machines with transition rules, algorithms (depreciation, envelope fill, envelope transfer). |
| `specs/architecture.md` | Module tree, `App` struct, `Tab` trait and `TabAction` enum, `EntityDb` and repository pattern, `Report` trait, inter-entity modal, startup sequence, event loop pseudocode, error handling strategy, global hotkeys. |
| `specs/phase-1.md` | Foundation: project setup, types, schema, config, TUI shell. 20 tasks. |
| `specs/phase-2a.md` | Chart of Accounts: account repo, CoA tab, audit log, widgets. 6 tasks. |
| `specs/phase-2b.md` | Journal Entries: JE repo, post/reverse orchestration, JE form, reconciliation. 8 tasks. |
| `specs/phase-3.md` | General Ledger, AR/AP, Fiscal Periods: ledger view, receivables/payables, period close/reopen, year-end close. 12 tasks. |
| `specs/phase-4.md` | Envelopes + Fixed Assets: allocation config, auto-fill on cash receipt, transfers, depreciation generation, place-in-service. 10 tasks. |
| `specs/phase-5.md` | Reports + Recurring + Startup: all 8 reports, recurring templates, startup checks, audit log tab, help overlay. 14 tasks. |
| `specs/phase-6.md` | Inter-Entity + Polish: split-pane modal, write protocol, failure recovery, edge cases, integration test. 15 tasks. |

**Total: 85 tasks across 7 phases.**

---

## Architecture Summary

Full detail in `specs/architecture.md`. Key points for orientation:

**Synchronous event loop.** Crossterm polling at 500ms. No async. Ratatui renders on each cycle.

**Single active entity.** `App` holds one `EntityContext` (database + tabs). The second entity
opens only inside the inter-entity journal entry modal and is dropped when the modal closes.

**Tab trait.** Each of the 9 tabs implements `Tab` with `handle_key()`, `render()`, `refresh()`,
and `navigate_to()`. Tabs communicate outward via `TabAction` return values — they never mutate
`App` state directly. Each tab is its own file under `src/tabs/`.

**Repository per domain.** Each domain (accounts, journal entries, AR, AP, envelopes, fiscal,
assets, recurring, audit) has its own repo struct under `src/db/`. Repos borrow `&Connection`
from `EntityDb`. Cross-repo operations use SQLite transactions via `EntityDb::conn()`.

**Module tree:**
```
src/
├── main.rs              # entry point
├── app.rs               # App struct, event loop
├── config.rs            # workspace.toml
├── startup.rs           # startup checks
├── types/               # Money, Percentage, IDs, enums
├── db/                  # EntityDb + 9 repos + schema
├── tabs/                # Tab trait + 9 tab implementations
├── inter_entity/        # modal, form, recovery
├── reports/             # Report trait + 8 implementations
└── widgets/             # account_picker, confirmation, je_form, status_bar
```

---

## Data Model Summary

Full detail in `specs/data-model.md`. Key decisions:

- **Money**: stored as `INTEGER` (i64). 1 dollar = 100,000,000 units (10^8). Display rounds to 2 decimal places. Prevents tax rounding compounding errors.
- **Percentages**: stored as `INTEGER` (i64). 1% = 1,000,000 units (10^6).
- **Enums**: stored as `TEXT` in SQLite. Human-readable when inspecting via SQLite MCP or CLI.
- **Account hierarchy**: adjacency list with `parent_id`. Placeholder flag prevents posting to category accounts.
- **Journal entries**: header/lines pattern. Two-column debit/credit on lines. Three-state reconciliation (Uncleared → Cleared → Reconciled).
- **AR/AP**: junction table for partial payments. Paid is terminal.
- **Envelopes**: auditable ledger (not just a balance snapshot). Transfer pairs linked by UUID.
- **Fiscal periods**: explicit table with `is_closed` flag. Period lock enforced on all JE mutations.
- **Audit log**: append-only, same SQLite file as entity data for transactional consistency.

14 tables total. See `specs/data-model.md` for complete CREATE TABLE statements.

---

## Implementation Approach

Full detail in `specs/implementation-protocols.md`. Key points:

1. **One task at a time.** Complete it, verify it, commit it, update progress, then proceed.
2. **Fresh sessions.** Start a new Claude Code session for each phase and for heavy tasks within a phase. Read specs and progress at session start, not conversation history.
3. **Test-first on foundations.** Tasks marked `[TEST-FIRST]` write failing tests before implementation.
4. **Pre-commit verification.** `cargo fmt` → `cargo clippy -D warnings` → `cargo test` before every commit. Pre-commit hook enforces this.
5. **Rollback on failure.** If a task fails verification after 2 attempts: `git reset --hard`, fresh session, start the task over.
6. **Developer review gate.** Developer signs off at the end of each phase before the next begins.
7. **Progress tracking.** `specs/progress.md` is updated by the agent after every task and read at every session start.

---

## Out of Scope (V1)

These features are explicitly deferred. Do NOT implement them. Do NOT architect for them
beyond leaving reasonable extension points (e.g., enum variants can be added, tables can be
added, new tabs can implement the Tab trait).

- Multi-user access and authentication
- Network features (HTTP, APIs, WebSockets)
- Inventory / materials management
- Accelerated depreciation methods (MACRS, double-declining balance)
- Full invoice management (line items, invoice numbers, customer/vendor records)
- Consolidated multi-entity financial reports
- PDF report output
- Automated backup (user manages SQLite file backups manually)
- Bank feed / import (OFX, CSV, QFX statement import)
- Budgeting by period (actuals vs. budget variance reports)
- More than 2 entities in the inter-entity modal
- Formal bank reconciliation workflow (the Cleared → Reconciled transition is defined but
  the reconciliation UI is deferred)
- Mouse input (fully keyboard-driven)

---

## Constraints & Gotchas

### SQLite-Specific
- **No distributed transactions.** Inter-entity writes use a two-phase Draft→Post protocol
  with startup recovery, not a cross-database transaction. See `specs/type-system.md`.
- **WAL mode required.** `PRAGMA journal_mode=WAL` set on every database open.
  Prevents locking issues if the SQLite MCP server reads while the app writes.
- **Foreign keys not enforced by default.** `PRAGMA foreign_keys=ON` must be set on every
  connection open. This is done in `EntityDb::open()` and `EntityDb::create()`.

### Money & Precision
- **Never use floating point for money.** Not in Rust, not in SQL, not in display formatting
  until the final `Display` impl. All arithmetic is integer-on-integer.
- **Rounding happens once** — at the display boundary. Internal calculations carry 8 decimal
  places. Reports and UI show 2 decimal places.
- **Depreciation final month absorbs rounding remainder** so total depreciation exactly equals
  cost basis. This must be explicitly tested.

### TUI-Specific
- **Terminal cleanup on panic.** The app must restore the terminal (leave raw mode, disable
  alternate screen) even on panic. Use a drop guard in `App::run()`.
- **No mouse.** All navigation is keyboard-driven. Do not implement mouse event handling.
- **500ms tick rate.** The event loop redraws on this interval even without user input.
  This keeps the status bar clock and message timeout responsive.

### Accounting Rules
- **Posted entries are immutable.** Corrections happen via reversing entries, never via editing.
- **Audit log is append-only.** No UPDATE, no DELETE. Ever.
- **Fiscal period lock is absolute.** No JE mutations (create, post, reverse, reconcile state)
  in a closed period. The period must be reopened first.
- **Envelope budgeting is separate from the GL.** Fills and transfers modify the envelope
  ledger only. They do not create journal entries. Year-end close does not affect envelope balances.
- **Owner's Draw does not trigger envelope fills.** Owner's Capital contributions do.

---

## Getting Started

1. Ensure Rust stable toolchain is installed (`rustup`).
2. Place `CLAUDE.md` at `~/coding-projects/accounting/CLAUDE.md`.
3. Place the `specs/` directory at `~/coding-projects/accounting/specs/`.
4. Initialize git: `git init && git add -A && git commit -m "Initial spec files"`.
5. Read `specs/implementation-protocols.md` and `specs/phase-1.md`.
6. Begin Phase 1, Task 1.
