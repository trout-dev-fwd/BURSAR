# V2 SPEC — AI Accounting Assistant

## Overview

Add an AI-powered accounting assistant to the existing double-entry bookkeeping TUI. The assistant answers natural-language questions about the entity's books using Claude API tool use, and automates bank statement import with intelligent transaction-to-account matching. Three features built across three phases: infrastructure, chat panel, and CSV import.

## Success Criteria

- [ ] Ctrl+K opens a 30% width right-side chat panel with a working AI accountant
- [ ] The accountant answers questions about accounts, balances, transactions, and AR/AP using tool use
- [ ] Running conversation context maintained across messages within a session
- [ ] `/clear`, `/context`, `/compact`, `/persona`, `/match` slash commands all functional
- [ ] `U` in the Journal Entries tab imports a bank statement CSV
- [ ] New bank formats auto-detected by Claude from CSV headers
- [ ] Three-pass matching pipeline: local deterministic → AI batch → user clarification
- [ ] All imports create Draft journal entries (never Posted)
- [ ] Learned transaction mappings persist in SQLite for future imports
- [ ] Shift+U re-matches incomplete import drafts
- [ ] All AI interactions logged to audit trail
- [ ] All V1 tests pass (372+), new tests added for all V2 functionality
- [ ] Zero clippy warnings, formatted code, all tests green

## Tech Stack Additions

| Component | Choice | Purpose |
|-----------|--------|---------|
| HTTP client | `ureq` (synchronous) | Claude API calls |
| JSON | `serde_json` | API request/response serialization |
| CSV parsing | `csv` crate | Bank statement file parsing |
| AI model | Claude Sonnet (`claude-sonnet-4-20250514`) | Default, configurable in workspace.toml |

No async runtime. No threading. Blocking HTTP calls with forced render flush for loading states.

## Document Index

| File | Contents | When to Read |
|------|----------|-------------|
| `CLAUDE.md` | Project config, coding conventions, V1 + V2 | **Every session** |
| `specs/v2/v2-boundaries.md` | Always Do / Ask First / Never Do for V2 | **Every session** |
| `specs/v2/v2-progress.md` | Current state, completed tasks, next task | **Every session** |
| `specs/v2/v2-data-model.md` | import_mappings table, import_ref column, config schemas, integrity invariants | When touching database or config |
| `specs/v2/v2-type-system.md` | Enums, state machines, algorithms, tool schemas, tab key conflicts | When touching types, state, or AI logic |
| `specs/v2/v2-architecture.md` | Module tree, key structs, data flows, focus model, render layout, error handling | When touching app structure or adding modules |
| `specs/v2/v2-phase-1.md` | Phase 1 tasks: foundation (types, config, repos, migrations) | During Phase 1 |
| `specs/v2/v2-phase-2.md` | Phase 2 tasks: AI client, chat panel, tool use, slash commands | During Phase 2 |
| `specs/v2/v2-phase-3.md` | Phase 3 tasks: CSV import pipeline | During Phase 3 |
| `specs/v1/implementation-protocols.md` | Session management, commit rules, rollback protocol | **Every session** (V1 doc, still applies) |

## Architecture Summary

**New module:** `src/ai/` with four files — `client.rs` (API calls), `tools.rs` (tool definitions + fulfillment), `context.rs` (entity context files), `csv_import.rs` (parsing + matching pipeline).

**New widget:** `src/widgets/chat_panel.rs` — right-side panel with conversation history, input line, typewriter animation.

**New repo:** `src/db/import_mapping_repo.rs` — CRUD for learned transaction-to-account mappings.

**Communication pattern:** ChatPanel returns `ChatAction` → App handles I/O → App feeds results back. Same pattern as Tab returning `TabAction`. The widget never makes API calls directly.

**Tool use:** Claude requests data via tool calls → App fulfills locally from SQLite repos (read-only) → App sends results back → Claude synthesizes answer. Maximum 5 tool use rounds per query.

**Focus model:** When chat panel is open, `FocusTarget` determines which side receives keyboard input. Tab switches focus. Panel focused = all keys go to chat input. Main tab focused = all normal hotkeys work.

## Data Model Summary

**One new table:** `import_mappings` — stores learned transaction description → account mappings with match type (exact/substring), source (confirmed/AI-suggested), bank name scope, and usage tracking.

**One new column:** `journal_entries.import_ref TEXT` — nullable composite reference for tracing imported drafts back to their source bank statement line.

**New audit actions:** `AiPrompt`, `AiResponse`, `AiToolUse`, `CsvImport`, `MappingLearned` — logged to existing `audit_log` table.

**Configuration:** Three-tier — `workspace.toml` (global), per-entity `.toml` (entity-specific bank accounts, persona), `~/.config/bookkeeper/secrets.toml` (API key).

## Implementation Summary

**37 tasks across 3 phases:**

- **Phase 1 (12 tasks):** Types, enums, config parsing, database schema, repos, context files, key remapping (Envelopes `v`, JE form arrow keys), CSV parser. No API calls. All testable in isolation.
- **Phase 2 (12 tasks):** ureq dependency, API client, tool definitions + fulfillment, chat panel widget, focus model, AI request orchestration, slash commands, help overlay, status bar messages. Produces a working AI accountant.
- **Phase 3 (13 tasks):** Import flow state machine, wizard modals, bank format detection, duplicate detection, three-pass matching pipeline, review screen, draft creation, re-match capabilities, final polish. Produces complete CSV import.

**Protocols:** One task = one commit. Fresh session per phase. Progress tracked in `specs/v2/v2-progress.md`. Developer review gate at each phase boundary.

## Out of Scope

These are explicitly NOT part of V2 Phase 1:

- Threaded/async API calls (blocking ureq is the design choice)
- Conversation persistence across sessions (in-memory only, audit log for trail)
- Import mapping management UI (mappings stored in SQLite, editable via future feature)
- Bank format editing after creation (edit toml manually)
- Multiple simultaneous imports
- CSV export
- Mouse input
- PDF report output
- Multi-user / authentication
- Network features beyond Claude API
- More than 2 entities in inter-entity modal
- Invoice management
- Accelerated depreciation methods
- Consolidated multi-entity reports
- Automated backup
- Bank feed / OFX / QFX import
- Budget vs. actuals variance

## Constraints & Gotchas

- **Tab key:** Intercepted at App level when chat panel is open. Envelopes uses `v` always. JE form has arrow key + Enter fallback.
- **Blocking calls:** `terminal.draw()` MUST be called before every `ureq` call to show loading state.
- **Money from CSV:** Parse string → Money. Never f64 intermediate.
- **import_ref pipe delimiter:** Parse from ends if description contains pipes.
- **API key:** Lazy-loaded on first AI use. Never logged or displayed.
- **SUMMARY line:** Stripped from display, logged to audit. Fallback if Claude omits it.
- **Debit/credit rules:** Depend on linked account type (Asset vs Liability). Thoroughly tested.
- **Entity toml writes:** `last_import_dir`, new `[[bank_accounts]]`, `ai_persona` — all write to entity toml, never workspace.toml.

## Getting Started

1. Ensure V1 is complete and all 372 tests pass
2. Ensure the draft editing feature is merged (V2 prerequisite)
3. Place all V2 spec files in `specs/v2/`
4. Replace `CLAUDE.md` with the merged version (V1 + V2 content)
5. Commit spec files: `V2: Add Phase 1 specifications`
6. Begin with the kickoff prompt (see below)

## Kickoff Prompt

```
Read the following files in order before doing anything:
1. CLAUDE.md
2. specs/v2/v2-boundaries.md
3. specs/v1/implementation-protocols.md
4. specs/v2/v2-progress.md
5. specs/v2/v2-phase-1.md

Then begin V2 Phase 1, Task 1. Work one task at a time. After each task,
run verification (cargo fmt, cargo clippy -D warnings, cargo test),
commit with the format "V2 Phase N, Task M: [description]",
update specs/v2/v2-progress.md, then proceed to the next task.
Stop at the end of Phase 1 for my review.
```

## Resume Prompt

```
Read CLAUDE.md, specs/v2/v2-boundaries.md, specs/v1/implementation-protocols.md,
and specs/v2/v2-progress.md. Pick up where we left off. Work one task at a time
with verification and commits after each. Stop at the end of the phase
for my review.
```

## Phase Gate Review Prompt

```
Review the git history for V2 Phase N (all commits since the previous phase).
For each task's commit, check:
1. Does the code follow the conventions in CLAUDE.md?
2. Does it satisfy the verification criteria in specs/v2/v2-phase-N.md?
3. Are there patterns established in early tasks that need correction?
4. Are there boundary violations from specs/v2/v2-boundaries.md?
5. For Phase 2+: are AI interactions properly logged to audit?
6. For Phase 3: are debit/credit rules correct for both Asset and Liability accounts?
Flag issues by severity: must-fix before next phase vs. nice-to-have.
```
