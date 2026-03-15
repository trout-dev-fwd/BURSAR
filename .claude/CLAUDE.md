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

_(Updated as the project evolves — add discoveries here)_
