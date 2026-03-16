# CLAUDE.md — Double-Entry Bookkeeping TUI

## Project

Rust TUI accounting application using Ratatui + SQLite. Single-user, synchronous, no async/tokio.

## Verification (run after every change, in this order)

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings   # zero warnings required
cargo test
```

All three must pass before committing. Git hooks enforce this: `git config core.hooksPath .githooks`

## Rust Style

- **No `.unwrap()` in production code.** Use `?` for propagation. `thiserror` for domain errors, `anyhow` at the CLI boundary. `.expect("reason")` only in init code with a clear invariant.
- **Iterators over loops** for transformation and aggregation.
- **Immutability by default.** `mut` only when genuinely needed.
- **Borrow before own.** Prefer `&T`/`&mut T` over taking ownership.
- **No `unsafe`** without a `// SAFETY:` comment documenting every invariant.
- **No `async` or `tokio`.** This is a synchronous application.
- **Logging:** `tracing` crate. No `println!` in library code.
- **SQL:** parameterized queries only (`params![]` / `named_params!{}`). Never string interpolation.
- **Money:** always `Money(i64)` newtype. Never raw `i64` or `f64` in function signatures.
- **Enums:** all state values are Rust enums with `FromStr`/`Display`. Never raw strings.

## Specs (read before starting work)

Detailed specifications live in `specs/`. Read the relevant files for your current task:

| File | Contents |
|------|----------|
| `specs/implementation-protocols.md` | **Read every session.** Session management, commit rules, rollback protocol, progress tracking. |
| `specs/boundaries.md` | **Read every session.** Always Do / Ask First / Never Do guardrails. |
| `specs/progress.md` | **Read every session.** Current state, completed tasks, next task, decisions log. |
| `specs/data-model.md` | SQLite schema — all 14 tables, design decisions, integrity invariants. |
| `specs/type-system.md` | Rust newtypes, enums, state machines, transition rules, algorithms. |
| `specs/architecture.md` | Module structure, Tab trait, EntityDb, repos, event loop, data flow. |
| `specs/phase-*.md` | Task-by-task implementation plans with context files, verification, and constraints. |

**Do not duplicate spec content here.** This file stays lean. Specs are the source of truth.

## Key Decisions

- **Money**: 8 decimal places internally (1 dollar = 100,000,000 units). Display rounds to 2.
- **Percentages**: 6 decimal places (1% = 1,000,000 units).
- **Enums in SQLite**: stored as TEXT for human readability.
- **Event loop**: synchronous crossterm polling, 500ms tick rate. No tokio.
- **Tabs**: each implements a `Tab` trait, one file per tab under `src/tabs/`.
- **Repos**: one per domain under `src/db/`, borrowing `&Connection` from `EntityDb`.
- **Single entity active**: second entity opens only in inter-entity modal.

## Commit Messages

```
Phase N[x], Task M: [short description]
```

One commit per task. See `specs/implementation-protocols.md` for full protocol.

## Gotchas

_(Discoveries from implementation — update as the project evolves)_

### Money & Precision
- **$1 = 100,000,000 internal units** (8 decimal places). Test values: `$100 = 10_000_000_000`.
- **Percentages**: `1% = 1,000,000 units`, `10% = 10_000_000`.
- **Rounding**: final depreciation month absorbs remainder so `SUM(all months) == cost_basis` exactly.

### Architecture
- **`EntityDb` is a wrapper** that owns the `rusqlite::Connection` and hands out repo objects via
  accessor methods (`db.accounts()`, `db.journals()`, etc.). Repos borrow `&Connection`.
- **`InterEntityMode`** takes primary DB as `&EntityDb` parameter — does NOT store a reference.
  Secondary `EntityDb` is owned (drops when mode exits).
- **`Tab::handle_key`** returns `TabAction`; tabs never mutate `App` state directly.
- **`TabAction::ShowMessage`** routes to `StatusBar::set_success`. Use `App::set_error` callers
  directly for explicit error paths.

### Cash account detection (envelope fill)
- Cash = `account_type == Asset && !is_placeholder && name.to_lowercase().contains("cash|bank|checking|savings")`.
- Owner's Draw suppression: `account_type == Equity && is_contra` → skip fill.
- If JE has **multiple** cash debit lines, envelope fill amount is the **sum of all** cash debits.

### Fiscal periods
- `create_draft` rejects closed periods at creation time (avoids orphaned un-postable entries).
- `generate_pending_depreciation` returns `(Vec<NewJournalEntry>, Option<String>)`. The warning
  fires when a depreciation month has no fiscal period; generation stops for that asset (not error).
- Year-end close zeroes GL balances for revenue/expense; **does NOT** clear envelope earmarks.

### Cross-module test access
- Private struct fields in production code can't be set from cross-module tests. Add
  `#[cfg(test)] pub(crate) fn set_test_state(...)` helpers to widgets/structs that need it.

### CIP account detection
- `PlaceInService` form opens only when selected account name contains "construction"
  (case-insensitive). Tested via substring match, not account type.

### Status bar
- `set_message` → success (green, 3s). `set_error` → error (red, 5s).
- `[*]` unsaved indicator: driven by `Tab::has_unsaved_changes()`; App polls each tick.
- JournalEntriesTab overrides `has_unsaved_changes()` to reflect new-entry form content.
