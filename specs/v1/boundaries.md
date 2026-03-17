# Boundaries — Agent Guardrails

## Overview

Three-tier rules governing what the agent does autonomously, what requires developer permission,
and what is strictly prohibited. These rules prevent drift, scope creep, and irreversible mistakes.

The agent MUST read this file at the start of every session alongside `implementation-protocols.md`.

**Design principle**: This file covers *project management* guardrails — what to build, when to ask,
and what's off-limits. Coding *style* rules (iterators over loops, borrow before own, etc.) live in
`CLAUDE.md` and are not duplicated here. Boundaries.md focuses on decisions and actions, not syntax.

---

## Always Do (agent acts without asking)

These are non-negotiable habits. The agent performs them automatically on every relevant action.

### Verification
- Run `cargo fmt` → `cargo clippy --all-targets --all-features -- -D warnings` → `cargo test`
  after every code change, in that order. All three must pass before committing.
- Run the full verification before every git commit. Never commit a broken state.

### Tracking & Communication
- Update `specs/progress.md` after every completed task.
- Write descriptive commit messages following the format in `implementation-protocols.md`.
- Add `// TODO(Phase N): description` comments when encountering work that belongs to a later phase.
- Log all database mutations to the audit log (from Phase 2a onward, once audit_repo exists).
- **State assumptions explicitly.** If the task is ambiguous and the agent makes a judgment call,
  document it in a code comment or in `progress.md` so the developer can validate it.

### Testing
- Write tests for every repo method, covering both the happy path and expected error cases.
- Write tests for every state transition.
- For tasks marked **[TEST-FIRST]**, write failing tests before implementation.

### Git Discipline
- One commit per completed task. Not mid-task, not after multiple tasks.
- Every commit must be a working state (compiles, lints, tests pass). The git history must
  be bisectable.

### Pattern Consistency
- Follow the established pattern when implementing the Nth instance of something.
  The first tab sets the convention for all tabs. The first repo for all repos. The first report
  for all reports. Do not invent new patterns without asking.
- Use the `Tab` trait for all tabs, the `Report` trait for all reports. No exceptions.
- Return `TabAction` from tab key handlers. Never mutate App state directly from a tab.

### Simplicity
- **Do not over-engineer.** Only build what the spec asks for. Do not add extra traits, generics,
  abstraction layers, configuration options, or "flexibility" that wasn't requested.
  If a simple function works, do not wrap it in a trait. If a concrete type works, do not
  make it generic. The spec is the ceiling, not the floor.
- Keep solutions minimal and focused. One goal per task. Do not make "improvements" or
  refactors beyond what the task requires.

---

## Ask First (agent requests developer permission)

The agent MUST pause and ask the developer before doing any of the following.
Present the proposal clearly, explain the reasoning, and wait for approval.

### General Principle: Reversibility
Any action that is **hard to reverse, affects shared state, or could be destructive** requires
permission — even if it isn't explicitly listed below. When in doubt, ask. Examples: deleting
files, force-pushing branches, altering committed database schemas, modifying files outside the
project directory.

### Dependencies
- **Adding a new crate dependency** to `Cargo.toml`.
  Exception: crates already listed in `specs/architecture.md` Tech Stack are pre-approved.
- **Updating an existing crate version** beyond a patch release (e.g., 0.x → 0.y).

### Architecture
- **Changing the module structure** — adding, removing, or renaming files/directories beyond
  what's specified in `specs/architecture.md`.
- **Creating a `services/` module** — the spec says to start with free functions and extract
  to a services module only if complexity warrants it. Ask before creating it.
- **Changing any trait signature** (`Tab`, `Report`, `EntityDb` accessors).
- **Introducing a new shared widget** beyond those specified in the architecture spec.
- **Introducing new abstractions** — new traits, new generic type parameters, new builder
  patterns, or new design patterns not in the spec. The spec defines the abstractions;
  the agent implements them.

### Data Model
- **Modifying any table schema** beyond what's defined in `specs/data-model.md`.
  If the agent discovers the schema needs a change, it must propose the change and get approval
  before modifying `schema.rs`.
- **Adding indexes** — performance decisions are made deliberately by the developer.
- **Changing the Money or Percentage scale** (10^8 and 10^6 respectively).

### Scope
- **Implementing anything from a future phase** — even if it seems trivial or "while we're here."
  Leave a `// TODO(Phase N)` comment and move on.
- **Refactoring existing working code** — if the agent sees an improvement opportunity in code
  from a previous phase, note it in `progress.md` and ask. Do not refactor without permission.
- **Changing the behavior of a verified, committed feature** — unless the current task explicitly
  requires it.

### External
- **Running commands that modify files outside `~/coding-projects/accounting/`**.
- **Modifying `.gitignore`** beyond standard Rust entries.
- **Changing the workspace.toml format** (structure is specified in architecture.md).

---

## Never Do (hard stops — non-negotiable)

The agent must NEVER do any of the following, regardless of reasoning or convenience.

### Data Safety
- **Never modify the SQLite schema outside of `db/schema.rs`.**
  All schema lives in `initialize_schema()`. No ad-hoc CREATE/ALTER TABLE elsewhere.
- **Never write SQL that modifies the audit_log** (UPDATE, DELETE, ALTER).
  Only INSERT and SELECT are permitted.
- **Never store Money as floating point** — not in the database, not in Rust structs.
  All money is `i64` / `Money` newtype / `INTEGER` in SQLite.
- **Never use `f64` for money arithmetic** except as a brief intermediate inside
  `Money::from_dollars()` and `Percentage::as_multiplier()`, immediately captured back
  into the integer representation.
- **Never delete user data.** Accounts are deactivated. Journal entries are reversed.
  The only deletions permitted are Draft entries during inter-entity rollback recovery (Phase 6).

### Code Safety
- **Never use `.unwrap()` in production code.** No exceptions.
  `.expect("reason")` is permitted ONLY in initialization with a clear invariant comment.
- **Never use `unsafe`** without a `// SAFETY:` comment. This project should require zero unsafe blocks.
- **Never use `println!` or `eprintln!` in library code.** Use `tracing` macros.
  `println!` only in `main.rs` for fatal startup errors before the TUI initializes.
- **Never use string interpolation for SQL.** Always parameterized queries (`params![]` / `named_params!{}`).
- **Never silence clippy warnings** with `#[allow(...)]` without a documented justification.
  Fix the code instead.
- **Never introduce `async` or `tokio`.** This is a synchronous application.
- **Never bypass safety checks.** Do not use `--no-verify` on git commits. Do not use
  `#[cfg(test)]` to disable production safety logic. Do not suppress errors to make tests pass.

### Process Safety
- **Never commit code that fails `cargo fmt`, `cargo clippy -D warnings`, or `cargo test`.**
- **Never push to main** without developer approval. Developer manages branch strategy.
- **Never skip the end-of-phase review gate.**
- **Never modify spec files** (`specs/*.md`) except `specs/progress.md`.
  If a spec is inaccurate, note it in `progress.md` and raise it with the developer.
- **Never one-shot an entire phase.** One task at a time, commit, proceed.
- **Never continue implementing after a test failure** without fixing it first.

### Scope
- **Never implement authentication, multi-user features, or network services.**
- **Never implement PDF generation.** Reports are `.txt` with box-drawing characters.
- **Never implement inventory, invoicing, or payroll.**
- **Never implement depreciation methods other than straight-line.**
- **Never implement bank feed import** (OFX, CSV, QFX).
- **Never implement features listed in the "Out of Scope" section of the feature spec.**

---

## CLAUDE.md Guidance

The project's `CLAUDE.md` file should be **lean** — a pointer document, not a copy of the specs.

**CLAUDE.md should contain:**
- Verification order (`fmt` → `clippy` → `test`)
- Rust coding style rules (no unwrap, iterators over loops, borrow before own, etc.)
- A list of spec files with brief descriptions, instructing the agent to read the relevant
  ones before starting work
- Git hooks path configuration (`git config core.hooksPath .githooks`)
- Project-specific gotchas discovered during implementation (updated as the project evolves)

**CLAUDE.md should NOT contain:**
- Copies of the spec content (point to the files instead)
- Lengthy architectural descriptions (that's `specs/architecture.md`)
- Task lists or progress tracking (that's `specs/progress.md`)
- Schema definitions (that's `specs/data-model.md`)

**Why**: CLAUDE.md is loaded into every session. Keeping it lean preserves context window space
for the actual task. Heavy reference material lives in spec files that the agent reads on demand.

---

## How to Handle Ambiguity

If the agent encounters a situation not clearly covered by these rules:

1. **Check the spec files** — the answer may be in `data-model.md`, `type-system.md`,
   `architecture.md`, or the relevant phase file.
2. **Check `progress.md`** — a previous decision may have addressed this.
3. **Default to the conservative choice** — do less, not more. Leave a TODO.
4. **State assumptions explicitly.** If proceeding requires a judgment call, document it
   in a code comment and in `progress.md` so the developer can validate it later.
5. **If still unclear, ask the developer.** Present the situation, the options, and your
   recommendation. Wait for a decision.

The cost of asking is low. The cost of building the wrong thing is high.
