# Progress Tracker

## Current State
- **Active Phase**: Phase 2a (complete — review fixes applied)
- **Last Completed Task**: Phase 2a review fixes
- **Next Task**: Phase 2b, Task 1
- **Blockers**: None

## Completed Phases
- [x] Phase 1: Foundation (completed 2026-03-15)
- [x] Phase 2a: Chart of Accounts (completed 2026-03-15, review fixes applied 2026-03-15)

## Current Phase Progress

### Phase 2a: Chart of Accounts
- [x] Task 1: Create AccountRepo [TEST-FIRST]
- [x] Task 2: Create AuditRepo [TEST-FIRST]
- [x] Task 3: CoA tab — list view
- [x] Task 4: CoA tab — CRUD actions
- [x] Task 5: Account picker widget
- [x] Task 6: Confirmation widget

### Phase 1: Foundation (complete)
- [x] Task 1: Initialize Cargo project with dependencies
- [x] Task 2: Create Money(i64) newtype
- [x] Task 3: Create Percentage(i64) newtype
- [x] Task 4: Create ID newtypes via macro
- [x] Task 5: Create all enums with FromStr/Display
- [x] Task 6: Create types/mod.rs re-exports
- [x] Task 7: Create workspace config (config.rs)
- [x] Task 8: Create database schema (db/schema.rs)
- [x] Task 9: Create default account seeding
- [x] Task 10: Create EntityDb struct
- [x] Task 11: Create FiscalRepo with year/period creation
- [x] Task 12: Wire fiscal year into EntityDb::create
- [x] Task 13: Create Tab trait and enums
- [x] Task 14: Create stub tab implementations
- [x] Task 15: Create StatusBar widget
- [x] Task 16: Create App struct and event loop
- [x] Task 17: Create main.rs entry point
- [x] Task 18: Entity creation flow in TUI
- [x] Task 19: Entity open/picker flow
- [x] Task 20: Set up pre-commit hook

## Decisions & Discoveries

- **[Phase 2a, Task 4 + review fix]**: CRUD modals implemented as a `CoaModal` enum on the tab struct.
  After review, the Add form's parent field was wired to use the AccountPicker widget (opens as a
  sub-overlay popup), and the deactivate/activate confirmation was wired to use the Confirmation
  widget. This establishes the integration pattern for Phase 2b's JE form. Entity name is set on
  the tab via `set_entity_name()` called from `EntityContext::new`.

- **[Phase 2a, Task 3]**: `EntityContext::new` now calls `tab.refresh(&db)` on all tabs after
  construction so data shows immediately on first render. `Table::highlight_style` is deprecated
  in ratatui 0.29 — use `row_highlight_style` instead. `TableState` must be cloned for immutable
  `render()` since `render_stateful_widget` requires `&mut TableState`.

- **[Phase 2a, Task 2 + review fix]**: `AuditRepo::list` uses empty-string sentinels
  (`?1 = '' OR ...`) — acceptable for the small audit_log table but NOT to be propagated to
  high-volume repos like JournalRepo (use dynamic SQL building instead). After review,
  `append()` was changed to accept `Option<&str>` / `Option<i64>` for `record_type` /
  `record_id`, matching the nullable schema columns. Entity-level events (e.g., YearEndClose)
  can now pass None.

- **[Phase 2a, Task 1]**: `row_to_account` is a free function (not a method) to satisfy rusqlite's
  `FnMut(&Row) -> Result<T>` callback signature — closures borrowing `self` cause lifetime conflicts
  with `query_map`. Parent-existence check is done app-side with a COUNT query before INSERT
  (belt-and-suspenders, as SQLite FK constraints also enforce this with PRAGMA foreign_keys=ON).
  `get_balance` returns raw `SUM(debit_amount - credit_amount)` across posted JE lines; direction
  interpretation (normal vs. contra) is deferred to Phase 3 display logic.


- **[Phase 1, Task 2]**: Introduced `src/lib.rs` to avoid dead_code warnings in the binary crate.
  In a pure binary, all types are considered dead until reachable from `main()`. With a lib.rs,
  `pub` items are not flagged. This is standard Rust practice for binaries with substantial library code.
  All future modules are declared in lib.rs.

- **[Phase 1, Task 9]**: "feature spec Section 2.2 (Standard Built-in Account Categories)" referenced
  in phase-1.md was not found in any spec file. The account hierarchy was designed as a judgment call
  for a small real estate LLC. Hierarchy: Assets (1000–1521), Liabilities (2000–2400),
  Equity (3000–3300), Revenue (4000–4200), Expenses (5000–5800). Contra accounts: Accumulated
  Depreciation - Buildings (1521), Owner's Draw (3200). **Developer should validate this hierarchy
  and request additions/removals before Phase 2a.**

- **[Phase 1, Task 13]**: Tab trait defined with `navigate_to` as a default no-op method matching
  the architecture spec exactly.

- **[Phase 1, Task 16]**: App uses `&&let` pattern (Rust 2024 let-chains) for collapsing nested
  `if poll() { if let Event::Key ... }` — required by clippy::collapsible_if.

- **[Phase 1, Task 20]**: Pre-commit hook sources `~/.cargo/env` before running cargo commands,
  which is necessary because git hooks run in a minimal shell environment without the user's PATH.

## Phase 2a Review Fixes (2026-03-15)

Applied 9 fixes from the end-of-phase developer review:

1. **CoA tab wired to use AccountPicker + Confirmation widgets** (was inline reimplementations)
2. **AccountRepo::update() made atomic** — single COALESCE-based UPDATE instead of two statements
3. **AuditRepo::append() signature fixed** — record_type/record_id now Option to match nullable schema
4. **AuditRepo::list() sentinel pattern documented** — doc comment warns against propagation
5. **N+1 balance queries replaced** — new get_all_balances() bulk query, CoA refresh uses 1 query
6. **Duplicated now_str() consolidated** — shared helper in db/mod.rs
7. **Duplicated centered_rect() consolidated** — shared helper in widgets/mod.rs
8. **Edit form Enter behavior fixed** — now advances through fields before submit (consistent with Add)
9. **AccountReactivated audit action added** — reactivations no longer logged as AccountDeactivated

## Known Issues
- None currently.
