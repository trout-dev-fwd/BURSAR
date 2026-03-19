# Regenerate HANDOFF.md from Actual Code

The file `specs/handoff.md` is the living orientation document for this codebase. Regenerate it from the actual code — not from memory, not from specs, not from the existing handoff. Every fact must come from reading the current source files.

## Process

This is a large codebase (~43K lines, 71 files). To avoid losing context, delegate the information gathering to focused sub-tasks using subagents, then synthesize their reports into the final document.

### Step 1: Gather Codebase Stats

Run these commands directly (no subagent needed):

```bash
# File count
find src/ -name "*.rs" | wc -l

# Total line count
find src/ -name "*.rs" -exec cat {} + | wc -l

# Test count
cargo test 2>&1 | tail -5

# Per-file line counts for the file tree
find src/ -name "*.rs" -exec wc -l {} + | sort -rn

# Crate name and version
grep -E "^name|^version" Cargo.toml | head -2

# Dependencies
sed -n '/\[dependencies\]/,/^\[/p' Cargo.toml
```

### Step 2: Delegate Codebase Review to Subagents

Spawn each of these as a separate subagent task. Each subagent should read the specified files and return a structured summary.

**Subagent A — Architecture & App Core:**
Read these files and report:
- `src/main.rs` — What is the `AppState` enum? What does the wrapper loop do? What are the state transitions?
- `src/app/mod.rs` — What is the `App` struct (list all fields)? What are the extracted public methods (signatures)? What is `EntityContext`? What is `AppMode`? What enums/types are defined here?
- `src/app/key_dispatch.rs` — What is the key dispatch priority order? List every priority level from highest to lowest with what it routes to.
- `src/app/ai_handler.rs` — What methods are here? What is the AI request flow (step by step)?
- `src/app/import_handler.rs` — What is the CSV import pipeline (step by step)? What methods are here?
- `src/startup_screen.rs` — What is `StartupScreen` (all fields)? What is `StartupAction`? What does add/edit/delete do?
- `src/startup.rs` — What does `run_startup_checks` do?
- `src/update.rs` — What functions exist? What is the update check flow?
- `src/config.rs` — What is `WorkspaceConfig` (all fields with serde attributes)? What is `UpdatesConfig`? What is `SecretsConfig`? Where is secrets.toml loaded from? What does `expand_config_paths` do?

**Subagent B — Data Model & Database:**
Read these files and report:
- `src/db/schema.rs` — List every CREATE TABLE statement with table name and key columns
- `src/db/mod.rs` — What is `EntityDb`? What accessor methods does it have? What migrations exist?
- `src/types/enums.rs` — List every enum with variant count and whether it's persisted or in-memory
- `src/types/ids.rs` — List every ID newtype
- `src/types/money.rs` — How is money represented? What is the scale factor?
- `src/types/percentage.rs` — How are percentages represented?
- All `src/db/*_repo.rs` files — For each repo, list the key public methods (just names, not full signatures)

**Subagent C — Tabs & Widgets:**
Read these files and report:
- `src/tabs/mod.rs` — What is the `Tab` trait (all methods with signatures)? What is `TabAction` (all variants)? What is `TabId`?
- Each `src/tabs/*.rs` file — For each tab: what is the struct name, what are the key features, what hotkeys does `hotkey_help()` return?
- `src/widgets/mod.rs` — What's exported?
- Each `src/widgets/*.rs` file — For each widget: what is the struct, what action enum does it return, key functionality in one line

**Subagent D — Services, Reports, Inter-Entity, AI:**
Read these files and report:
- `src/services/journal.rs` — Key public functions (posting, reversal, depreciation, year-end close)
- `src/services/fiscal.rs` — Key public functions (period management, close/reopen)
- `src/reports/mod.rs` — What is the Report trait? What shared rendering exists?
- All `src/reports/*.rs` — List each report type with one-line description
- `src/inter_entity/mod.rs` — How does inter-entity mode work?
- `src/inter_entity/form.rs` — What does the form do?
- `src/inter_entity/write_protocol.rs` — What is the atomic write strategy?
- `src/inter_entity/recovery.rs` — What does orphan recovery do?
- `src/ai/mod.rs` — What wire types are defined (ApiMessage, ToolCall, RoundResult, etc.)?
- `src/ai/client.rs` — What is AiClient? What is the timeout? What headers are sent?
- `src/ai/tools.rs` — List all tool names and one-line descriptions
- `src/ai/context.rs` — What does context loading do?
- `src/ai/csv_import.rs` — What is the 3-pass matching pipeline?

**Subagent E — Gotchas & Patterns:**
Read the full codebase looking specifically for:
- Any `// HACK`, `// TODO`, `// NOTE`, `// IMPORTANT`, `// WARNING` comments
- Any `unwrap_or`, `expect` calls in production code (not tests) — are they safe?
- How does borrow splitting work for the AI client?
- How does the forced render before blocking calls pattern work?
- How is money parsed from CSV (verify no f64 intermediary)?
- What is the import_ref format?
- What is the SUMMARY line convention in AI responses?
- How do in-memory test DBs differ from file-based DBs?
- How does `toml_edit` array-of-tables work for entity management?
- What is the entity path resolution strategy?
- Where is the secrets file and why is it at a different path than the config dir?

Also read `CLAUDE.md` and report any coding rules or conventions defined there.

### Step 3: Synthesize into HANDOFF.md

Using all subagent reports plus the stats from Step 1, write `specs/handoff.md` following this exact structure:

1. **Header** — Title, "Living orientation document" disclaimer, last updated date (use today's date)
2. **What This Is** — One paragraph summary
3. **Tech Stack** — Table of layer/technology pairs, crate name, hard constraints
4. **Codebase Overview** — File count, line count, test count, full file tree with line counts per file
5. **Architecture** — Three-state wrapper loop, extracted App methods table, key dispatch order (numbered priority list), App struct (all fields), StartupScreen struct (all fields), Tab trait, EntityDb pattern, ChatPanel→App communication, AI request flow, CSV import pipeline
6. **Data Model** — Table list with columns, money representation, ID types, enums list
7. **Feature Summary** — Startup screen features (verify: edit only changes display name, delete preserves files), 9 tabs table, AI Accountant, CSV Import, Reports list
8. **All Hotkeys** — Startup screen, Global, each tab, chat panel — use tables with Key and Action columns
9. **Key Design Patterns** — How to add: tab, repo, AI tool, slash command, widget. Verify all file paths point to the correct post-refactor locations (e.g., `src/app/mod.rs` not `src/app.rs`, `src/app/ai_handler.rs` for slash command execution)
10. **Gotchas** — Every non-obvious behavior, edge case, or trap. This section is critical — include everything the subagents found.
11. **Configuration Reference** — workspace.toml example with ALL current fields, per-entity toml, secrets.toml
12. **Dependencies** — Full `[dependencies]` block from Cargo.toml
13. **Out of Scope** — Explicitly excluded features

### Rules for Writing

- Every fact must come from a subagent report or a direct command. Do NOT use information from the existing handoff.md or from memory.
- If a subagent reports something that contradicts another subagent, re-read the source file yourself to resolve it.
- Use the exact struct field names, method signatures, and enum variants from the code.
- For "How to Add" patterns, verify the file paths are correct post-refactor (e.g., `src/app/mod.rs` not `src/app.rs`, `src/app/ai_handler.rs` for slash commands).
- Keep it concise — facts, not opinions. Tables for structured data, code blocks for types.
- The Gotchas section should include EVERY non-obvious behavior. When in doubt, include it.
- Feature descriptions must match what the code actually does, not what the spec said it would do. For example: entity edit only changes the display name in workspace.toml (does NOT rename files). Entity delete removes the entry from workspace.toml (does NOT delete .sqlite or .toml files). Verify each claim by checking what the code does.

## Verification

After writing the handoff:

1. Spot-check 10 random facts against the actual code:
   - Pick 3 struct fields and verify they exist with the stated types
   - Pick 2 hotkeys and verify they do what the handoff says
   - Pick 2 file paths and verify the line counts are within 5% of actual
   - Pick 1 gotcha and verify it's accurate
   - Pick 1 "How to Add" recipe and trace each step against the actual files
   - Pick 1 config field and verify its serde attribute

2. Verify no stale references:
   - `grep "src/app.rs" specs/handoff.md` should return zero hits (should be `src/app/mod.rs` or submodules)
   - All "How to Add" file paths should point to files that actually exist

3. Verify the Feature Summary section matches what the code does, not what was specced

## Commit

```
docs: Regenerate handoff.md from codebase review
```
