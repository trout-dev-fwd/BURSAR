# Implementation Protocols

## Overview

This document defines the rules and workflows that apply across ALL phases of implementation.
The agent MUST read this file at the start of every session before beginning any work.

Seven phases, ordered by dependency. Each phase is independently testable and results in a
working (if incomplete) application.

```
Phase 1:  Foundation             → types, schema, TUI shell, entity management
Phase 2a: Chart of Accounts      → account repo, CoA tab, audit log
Phase 2b: Journal Entries         → JE repo, form widgets, JE tab, reconciliation
Phase 3:  GL, AR/AP, Fiscal      → ledger view, receivables/payables, period management
Phase 4:  Envelopes + Assets     → budgeting fills, depreciation generation
Phase 5:  Reports + Recurring    → all 8 reports, templates, startup checks
Phase 6:  Inter-Entity + Polish  → split-pane modal, recovery protocol, edge cases
```

---

## Session Management

### Fresh Sessions Are Mandatory

- Start a **fresh Claude Code session** for each phase.
- Within a phase, start a **fresh session** for any task that the agent estimates will consume
  significant context (complex UI, multi-file changes, or more than ~15 minutes of work).
- At the start of every session, the agent reads:
  1. `CLAUDE.md` (project style guide)
  2. `specs/implementation-protocols.md` (this file)
  3. `specs/progress.md` (current state)
  4. The relevant phase file (e.g., `specs/phase-1.md`)
  5. Any files listed in the task's "Context" section
- Do NOT rely on conversation history from previous sessions. The spec files and codebase
  are the source of truth.

### Context Discipline

- The agent reads ONLY the files listed in each task's "Context" section plus any files
  it needs to discover via the codebase. Do not preemptively read the entire project.
- If a task touches 1–3 files, keep context tight. If it touches 5+, consider splitting
  the work across multiple commits.

---

## Commit Protocol

### One Task = One Commit

Every task produces exactly one git commit (or a small, focused series if the task naturally
decomposes). Commits happen AFTER verification passes.

### Commit Message Format

```
Phase N[x], Task M: [short description]

[optional body explaining key decisions]
```

Examples:
```
Phase 1, Task 2: Create Money(i64) newtype with arithmetic and display

Phase 2b, Task 4: Implement journal entry form widget

  - Dynamic line rows with add/remove
  - Running debit/credit totals
  - Account picker integration via AccountPicker widget
```

### Pre-Commit Verification

Before every commit, the agent runs:

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

ALL THREE must pass. If any fail, fix before committing. Never commit code that fails
any of these checks.

**Recommendation for the developer**: Set up a git pre-commit hook that enforces this automatically:

```bash
#!/bin/sh
# .git/hooks/pre-commit
cargo fmt -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test
```

---

## Rollback Protocol

If a task fails verification after **2 correction attempts** within the same session:

1. Do NOT continue trying to fix it in the same session.
2. Run `git reset --hard HEAD` to return to the last good commit.
3. Start a **fresh session**.
4. Re-read the task requirements and the relevant source files.
5. Attempt the task again from scratch with fresh context.

This prevents context poisoning — the failure-correction loop accumulates confused state
that makes subsequent attempts worse, not better.

---

## Progress Tracking

### `progress.md` Is the Source of Truth

The file `specs/progress.md` is maintained by the agent and reviewed by the developer.
After every completed task:

1. Mark the task as done in `progress.md`.
2. Note any decisions made, patterns discovered, or gotchas encountered.
3. Commit `progress.md` alongside the task's code changes.

When starting a fresh session, `progress.md` tells the agent exactly where things stand
without needing prior conversation history.

### Format

```markdown
## Current State
- **Active Phase**: Phase 2a
- **Last Completed Task**: Phase 2a, Task 3
- **Next Task**: Phase 2a, Task 4
- **Blockers**: None

## Completed Phases
- [x] Phase 1: Foundation (completed YYYY-MM-DD)

## Current Phase Progress
- [x] Task 1: description
- [x] Task 2: description
- [x] Task 3: description
- [ ] Task 4: description
- [ ] Task 5: description

## Decisions & Discoveries
- [Phase 2a, Task 2]: Discovered that rusqlite's `params!` macro requires...
- [Phase 2a, Task 3]: Account search uses LIKE '%query%' on both name and number columns...

## Known Issues
- None currently
```

---

## End-of-Phase Review Gate

At the end of each phase:

1. The agent runs the full verification suite (`fmt`, `clippy`, `test`).
2. The agent updates `progress.md` to mark the phase complete.
3. The agent commits with message: `Phase N[x] complete: [phase name]`.
4. **The developer reviews all code** produced during the phase.
5. The developer signs off before the agent begins the next phase.

The developer may request changes. If so, the agent applies them in the current phase
(fresh session if needed) before moving on.

---

## Test-First Tasks

Tasks marked with **[TEST-FIRST]** must be implemented in this order:

1. Write the test(s) that verify the acceptance criteria.
2. Run the tests — they should FAIL (they test functionality that doesn't exist yet).
3. Implement the feature.
4. Run the tests — they should PASS.

This applies to foundational types, repository methods, and state machine transitions
where the acceptance criteria are concrete and testable.

---

## Scope Discipline

### Per-Task "Do NOT" Rules

Every task has a "Do NOT" section listing what the agent must not build in that task.
These exist because the most common agent failure is scope creep — building ahead into
future tasks or phases. Respect the boundaries.

### Phase-Level "Does NOT Cover"

Each phase file lists features that are explicitly deferred. If the agent encounters a
situation where a deferred feature would be helpful, it should:

1. Leave a `// TODO(Phase N): [description]` comment in the code.
2. Note it in the "Decisions & Discoveries" section of `progress.md`.
3. Move on without implementing it.

---

## Reference Implementation

For Ratatui application structure, the agent should consult:
- The `ratatui` crate examples on docs.rs (especially the `demo2` example)
- The event loop pattern from crossterm's documentation

When implementing a pattern for the first time (e.g., the first tab, the first repo,
the first report), the agent should establish the pattern carefully. Subsequent instances
of the same pattern should follow the established convention. If the agent finds the
established pattern needs improvement, it should note this in `progress.md` and discuss
with the developer before refactoring.

---

## File Organization Reminder

All spec files live in the project under `specs/`:

```
specs/
├── implementation-protocols.md   ← this file (read every session)
├── progress.md                   ← maintained by agent (read every session)
├── data-model.md                 ← SQLite schema reference
├── type-system.md                ← Rust types and state machines
├── architecture.md               ← module structure and component design
├── phase-1.md                    ← Foundation
├── phase-2a.md                   ← Chart of Accounts
├── phase-2b.md                   ← Journal Entries
├── phase-3.md                    ← GL, AR/AP, Fiscal
├── phase-4.md                    ← Envelopes, Fixed Assets
├── phase-5.md                    ← Reports, Recurring, Startup
├── phase-6.md                    ← Inter-Entity, Polish
└── boundaries.md                 ← Always Do / Ask First / Never Do
```
