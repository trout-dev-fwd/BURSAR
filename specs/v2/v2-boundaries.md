# V2 Boundaries — AI Accounting Assistant

This document defines the three-tier guardrail system for V2 development. It supplements the V1 boundaries — all V1 rules remain in effect. New rules here address AI integration, configuration management, and the expanded scope of V2.

---

## Always Do

### Project Management

1. **Run verification after every change:** `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test`. All three must pass before committing. No exceptions.
2. **One task, one commit.** Commit message format: `V2 Phase N, Task M: [short description]`. Never bundle multiple tasks in one commit.
3. **Update `specs/v2/v2-progress.md` after each task.** Mark task complete, note any decisions or discoveries, update "next task" pointer.
4. **Read context files listed in each task before starting.** The task spec lists which files to read — read all of them. Don't guess from memory.
5. **Follow existing patterns.** When adding a new repo, match the style of existing repos. When adding a new widget, match existing widgets. When adding a new enum, match existing enums. Consistency matters more than "better" alternatives.
6. **Test foundational code first.** Tasks marked [TEST-FIRST] must have tests written before or alongside the implementation. Types, repos, and algorithms all need unit tests.

### Code Quality

7. **No `.unwrap()` in production code.** Use `?` for propagation, `.expect("reason")` only in init code with a clear invariant.
8. **Iterators over loops** for transformation and aggregation.
9. **Immutability by default.** `mut` only when genuinely needed.
10. **Borrow before own.** Prefer `&T`/`&mut T` over taking ownership.
11. **No `unsafe`** without a `// SAFETY:` comment documenting every invariant.
12. **No `async` or `tokio`.** This remains a synchronous application. `ureq` is the HTTP client, blocking calls are by design.
13. **No `println!` in library code.** Use `tracing` for logging.
14. **Parameterized SQL only.** `params![]` / `named_params!{}`. Never string interpolation in SQL.
15. **Money is always `Money(i64)`.** Never raw `i64` or `f64` in function signatures for monetary values.
16. **Enums for all state values.** Never raw strings for states, types, or categories. All enums have `FromStr`/`Display`.

### AI-Specific

17. **Tools are read-only.** Claude's tool fulfillment functions must never write to the database. All mutations happen in application code after Claude returns its response.
18. **Drafts only.** The CSV import pipeline must never create Posted journal entries. Everything is a Draft for user review.
19. **Log all AI interactions to the audit log.** Every user question (AiPrompt), every Claude response summary (AiResponse), and every tool call (AiToolUse) gets an audit entry. No silent AI operations.
20. **10-second timeout on every API call.** No API call should block the UI for more than 10 seconds. If it times out, fail gracefully with "The Call Dropped ☹".
21. **Strip the SUMMARY line from display.** The user never sees the `SUMMARY:` line — it's parsed out and stored in the audit log only.
22. **API key never in version control.** The key lives in `~/.config/bookkeeper/secrets.toml` and nowhere else. Never log the API key, never include it in error messages, never serialize it to any output.

---

## Ask First

These actions require developer permission before proceeding. If working autonomously (e.g., in a Claude Code session), stop and ask.

### Architecture

23. **Adding new dependencies beyond the spec.** The spec allows `ureq`, `serde_json`, and `csv`. Any additional crate requires approval.
24. **Changing existing module structure.** Moving, renaming, or splitting existing V1 files requires approval.
25. **Modifying the event loop dispatch order.** The dispatch order (modal → chat panel → focus switch → inter-entity → global → tab) is specified. Changes to this order require approval.
26. **Changing the ChatPanel ↔ App communication pattern.** ChatPanel returns ChatAction, App handles I/O. If this needs to change, ask first.
27. **Introducing threading or async.** If the blocking `ureq` approach proves problematic, discuss before adding `std::thread` or any async runtime.

### Data

28. **Adding new database tables or columns beyond the spec.** The spec defines `import_mappings` (new table) and `import_ref` (new column). Any other schema change requires approval.
29. **Modifying existing table schemas.** Never alter existing V1 tables beyond the specified `import_ref` addition.
30. **Changing the workspace.toml structure beyond the spec.** The spec adds `context_dir`, `[ai]`, and `config_path`. Any other workspace.toml changes require approval.
31. **Writing to files outside the defined paths.** The application writes to: entity toml files, entity context `.md` files, and the `~/.config/bookkeeper/` directory. Writing to any other location requires approval.

### Scope

32. **Building features ahead of the current phase.** Phase 1 builds foundation, Phase 2 builds the chat panel, Phase 3 builds CSV import. Don't implement Phase 3 code during Phase 1, even if it seems convenient.
33. **Refactoring existing V1 code.** If existing code needs changes to support V2 features, make the minimum necessary change. Don't refactor "while we're in there" without approval.
34. **Adding UI elements not in the spec.** The spec defines the chat panel layout, import modals, review screen, and help overlay changes. Any new UI elements require approval.
35. **Changing the persona or system prompt wording.** The system prompt structure is specified. Changes to persona instructions, the SUMMARY instruction, or the 3-paragraph limit require approval.

### Reversibility Heuristic

36. **General rule: if it's hard to undo, ask first.** Database migrations, file format changes, public API changes, dependency additions — anything that creates a commitment that's expensive to reverse requires approval.

---

## Never Do

### Data Safety

37. **Never create Posted journal entries from imports.** All imported entries are Drafts. The user posts them manually after review. No exceptions.
38. **Never delete or modify existing journal entries through AI operations.** AI can create drafts and update draft lines (via re-match). It cannot touch Posted entries, cannot reverse entries, cannot delete entries.
39. **Never store the API key anywhere except `~/.config/bookkeeper/secrets.toml`.** Not in workspace.toml, not in entity toml, not in the database, not in any log output, not in error messages.
40. **Never send the full database to Claude.** Use tools for selective data access. The system prompt contains the context file and persona only — never bulk account/transaction data.
41. **Never auto-write to the entity context `.md` file without user action.** The file is auto-created with a skeleton on first use, but content is only added when the user explicitly confirms something (e.g., `/persona` update). Claude does not silently modify the context file.

### Code Safety

42. **Never use floating point for money.** All monetary values use `Money(i64)` with 10^8 scale. This applies to CSV import parsing — parse amounts as strings, convert to Money via the established `Money::from_str` or equivalent.
43. **Never use `async` or `tokio`.** The application is synchronous. `ureq` is the HTTP client.
44. **Never use `println!` in library code.** Use `tracing`.
45. **Never use string interpolation in SQL.** Parameterized queries only.
46. **Never call `.unwrap()` in production code.** Use `?`, `expect("reason")`, or explicit error handling.

### Process Safety

47. **Never skip verification.** Every commit must pass `cargo fmt && cargo clippy -D warnings && cargo test`. No "I'll fix it later" commits.
48. **Never combine multiple tasks in one commit.** One task = one commit. This enables clean rollback.
49. **Never modify `specs/` files other than `specs/v2/v2-progress.md` without approval.** The specs are the source of truth. If the spec is wrong, ask the developer to update it rather than silently changing it.
50. **Never build features not in the current phase spec.** Don't anticipate future needs. Build what the task says, verify, commit, move on.

### Scope Exclusions (Not in V2 Phase 1)

51. **Never add mouse input support.**
52. **Never add network features beyond the Claude API.** No HTTP servers, no websockets, no bank feed APIs.
53. **Never add multi-user or authentication features.**
54. **Never add PDF report output.**
55. **Never modify the inter-entity modal to support more than 2 entities.**
56. **Never add invoice management (line items, invoice numbers, customer/vendor records).**

---

## Ambiguity Resolution

When the boundaries above don't clearly cover a situation:

1. **Check the spec files.** Read the relevant spec (data-model, type-system, architecture, phase task) for guidance.
2. **Check progress.md.** Previous decisions and discoveries may clarify.
3. **Make the conservative choice.** Prefer the simpler approach, the one that changes less existing code, the one that's easiest to undo.
4. **State your assumption.** In the commit message or progress.md, note: "Assumed X because Y." This makes the decision visible for review.
5. **When truly stuck, ask.** Stop the task, document what's unclear in progress.md, and wait for developer input.

---

## CLAUDE.md Updates for V2

The project CLAUDE.md has been updated to include V2 specs, key decisions, user guide
maintenance requirements, and gotchas. See the merged CLAUDE.md — V2 content is integrated
into the existing structure, not appended as a separate section.

Key additions:
- V2 Specs table under `specs/v2/v2-*.md`
- V2 Key Decisions section
- V2 Guide Updates checklist under User Guide Maintenance
- V2 Gotchas under the existing Gotchas section
