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

### V1 Specs

| File | Contents |
|------|----------|
| `specs/v1/implementation-protocols.md` | **Read every session.** Session management, commit rules, rollback protocol, progress tracking. |
| `specs/v1/boundaries.md` | **Read every session.** Always Do / Ask First / Never Do guardrails. |
| `specs/v1/progress.md` | **Read every session.** Current state, completed tasks, next task, decisions log. |
| `specs/v1/data-model.md` | SQLite schema — all 14 tables, design decisions, integrity invariants. |
| `specs/v1/type-system.md` | Rust newtypes, enums, state machines, transition rules, algorithms. |
| `specs/v1/architecture.md` | Module structure, Tab trait, EntityDb, repos, event loop, data flow. |
| `specs/v1/phase-*.md` | Task-by-task implementation plans with context files, verification, and constraints. |

### V2 Specs

| File | Contents |
|------|----------|
| `specs/v2/v2-boundaries.md` | **Read every session.** V2-specific Always Do / Ask First / Never Do guardrails. |
| `specs/v2/v2-data-model.md` | New table (import_mappings), new column (import_ref), config file schemas. |
| `specs/v2/v2-type-system.md` | New enums, state machines, algorithms, tool schemas, tab key conflicts. |
| `specs/v2/v2-architecture.md` | New modules (src/ai/), key structs, data flows, focus model, render layout. |
| `specs/v2/v2-phase-1.md` | Foundation: types, config, repos, migrations, key remapping. |
| `specs/v2/v2-phase-2.md` | AI client, chat panel, tool use, slash commands, focus model. |
| `specs/v2/v2-phase-3.md` | CSV import pipeline, bank detection, matching, review, draft creation. |
| `specs/v2/v2-SPEC.md` | Master entry point: success criteria, architecture summary, kickoff prompts. |
| `specs/v2/v2-progress.md` | V2 task tracking, decisions log, known issues. |

### V2.2 Specs

| File | Contents |
|------|----------|
| `specs/v2.2/v2.2-SPEC.md` | Master entry point: success criteria, feature specs, architecture impact, constraints. |
| `specs/v2.2/v2.2-progress.md` | V2.2 task tracking, decisions log, known issues. |
| `specs/v2.2/v2.2-phase-1.md` | Foundation: repo rename, splash polish, CI pipeline. |
| `specs/v2.2/v2.2-phase-2.md` | Auto-update: download, verify, replace, restart. |
| `specs/v2.2/v2.2-phase-3.md` | In-app feedback: bug reports, feature requests, help overlay. |

**Do not duplicate spec content here.** This file stays lean. Specs are the source of truth.

## Key Decisions

### V1
- **Money**: 8 decimal places internally (1 dollar = 100,000,000 units). Display rounds to 2.
- **Percentages**: 6 decimal places (1% = 1,000,000 units).
- **Enums in SQLite**: stored as TEXT for human readability.
- **Event loop**: synchronous crossterm polling, 500ms tick rate. No tokio.
- **Tabs**: each implements a `Tab` trait, one file per tab under `src/tabs/`.
- **Repos**: one per domain under `src/db/`, borrowing `&Connection` from `EntityDb`.
- **Single entity active**: second entity opens only in inter-entity modal.

### V2
- **AI client:** `ureq` (synchronous, blocking). No async, no threading.
- **Tools:** Read-only access to existing repos. Never write through tools.
- **Imports:** Always create Drafts, never Posted entries.
- **Config:** workspace.toml (global) + per-entity .toml (entity-specific) + ~/.config/bookkeeper/secrets.toml (API key).
- **Focus model:** Tab switches between chat panel and main tab when panel is open. Panel intercepts all keys when focused except Tab/Esc/Ctrl+K.
- **Envelopes view toggle:** `v` key (replaced Tab).
- **Audit logging:** All AI interactions logged as AiPrompt/AiResponse/AiToolUse. Responses logged as single-line summaries only.
- **CSV import:** Three-pass pipeline (local → AI → clarification). Learned mappings in SQLite (import_mappings table).

### V2.2
- **Versioning:** semver `0.x.y` (pre-1.0). `Cargo.toml` is source of truth. Git tags match exactly (`vX.Y.Z`).
- **Auto-update:** Forced on launch. Downloads from GitHub Releases API, verifies SHA256, replaces binary, restarts. Falls through gracefully on any failure.
- **GitHub API:** All requests require `User-Agent: bursar/{version}` header (403 without it).
- **Binary replacement:** Rename current → `.old`, new → current. Old binary cleaned up on next launch. Never deleted during update.
- **Platform restart:** Linux uses `exec()` (in-place process replacement). Windows restores terminal first, then `spawn()` + `process::exit(0)`.
- **Feedback:** `b`/`f` keys in `?` overlay only (not global). Pre-filled GitHub issue URLs via `xdg-open`/`cmd /c start`. No GitHub PAT required.
- **CI/CD:** GitHub Actions triggered by `v*` tags. Builds Linux + Windows x86_64. Runs fmt/clippy/test before release build. `checksums.txt` with SHA256 hashes.
- **New dependencies:** `semver` (version comparison), `sha2` (checksum verification), `urlencoding` (percent-encoding for issue URLs).

## User Guide Maintenance

The in-app user guide lives at `specs/guide/user-guide.md` and is embedded into the
binary at compile time (`include_str!`). **It must be kept in sync with the code.**

- Any task that adds, changes, or removes a user-visible feature (key binding, workflow,
  tab behavior, column, color coding, etc.) **must** update the guide in the same commit.
- The guide is organized by tab. Find the relevant section and update it.
- Do not add or remove sections without checking whether other parts of the guide reference them.

### V2 Guide Updates

V2 introduces several user-visible features that require guide updates:

- **Ctrl+K:** AI Accountant panel — new section needed
- **Tab key:** Focus switching when panel is open — document in the AI panel section
- **`v` key in Envelopes:** Replaces Tab for view toggle — update Envelopes section
- **Arrow key + Enter in JE form:** Alternative to Tab for field navigation — update JE section
- **Ctrl+Left/Right:** Tab cycling — update global hotkeys section
- **`e` key in JE tab:** Edit draft entries — update JE section
- **`U` / Shift+U in JE tab:** CSV import and re-match — new section needed
- **Slash commands:** /clear, /context, /compact, /persona, /match — document in AI panel section
- **`?` help overlay:** New Chat Panel section — update help section

### V2.2 Guide Updates

V2.2 introduces user-visible features that require guide updates:

- **`?` help overlay:** New "Feedback" section with `b` (report bug) and `f` (request feature)
- **Splash screen:** Centered version number, update progress bar during auto-update
- **Auto-update behavior:** Document that updates are forced on launch with graceful fallthrough

## Commit Messages

```
V2.2 Phase N, Task M: [short description]
```

One commit per task. See `specs/v1/implementation-protocols.md` for full protocol.

## Release Protocol

When the developer requests a release (e.g., "ship it", "cut a release", "bump and tag"):

1. Determine the version bump type:
   - **Patch** (0.2.0 → 0.2.1): bug fixes, minor polish, no new features
   - **Minor** (0.2.0 → 0.3.0): new user-visible features
   - **Major**: reserved for post-1.0 breaking changes
2. Update the version in `Cargo.toml`
3. Run: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
4. Commit: `chore: bump version to vX.Y.Z`
5. Push to master: `git push origin master`
6. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`
7. Update the current version's progress file with a release entry

**Never bump the version or push a tag without an explicit request from the developer.**
The tag push triggers GitHub Actions to build and publish release binaries automatically.

## File Size Limit

No single .rs file should exceed 1,500 lines. If a file approaches this limit, split it
into a directory module (mod.rs + submodules) before adding new features.

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

### Confirmation widget
- **Confirmation widget handles its own centering** via `centered_rect()`. Never call `centered_rect()` on the area before passing it to `Confirmation::render()` — this causes double-centering that makes the content area too small to display anything.

### V2 — AI & Import
- **Tab key conflict:** Tab is intercepted at App level when chat panel is open. JE form uses arrow keys + Enter as fallback navigation. Envelopes uses `v` for view toggle always.
- **Forced render before blocking calls:** Must call `terminal.draw()` before any `ureq` call so the user sees the loading state before the UI freezes.
- **SUMMARY parsing:** System prompt instructs Claude to end responses with `SUMMARY: [one sentence]`. Client strips this line from display text and logs it to audit. Fallback if missing: truncate first sentence to 100 chars.
- **import_ref format:** `"{bank_name}|{date}|{description}|{amount}"` — if descriptions contain pipe characters, parse from the ends (bank_name is first segment, amount is last, date is second, description is everything in between).
- **Money from CSV:** Parse amount strings → Money via established conversion. Never use f64 as intermediate. Handle both `"-1234.56"` and `"(1234.56)"` negative formats if encountered.
- **Entity toml location:** Same directory as workspace.toml, referenced via `config_path` on each entity entry.
- **API key loaded lazily:** Not at startup. First Ctrl+K or U import triggers the load. Missing key shows a specific error directing the user to the secrets file path.
- **Tool use loop:** `handle_ai_request` in `app.rs` drives the tool use loop round by round. Between rounds it logs `AiToolUse` to audit, updates `ai_state` to `FulfillingTools`, and calls `terminal.draw()` to show "Checking the books". Each round is a separate blocking `ureq` call.

### V2.2 — Update & Feedback
- **Symlinks break binary replacement.** `std::env::current_exe()` resolves through symlinks. Renaming at the resolved path leaves the symlink pointing at `.old`. Detect and fall through with warning.
- **Write permissions on binary directory.** If installed to `/usr/local/bin/` or similar, rename fails. Pre-flight check tests write access before downloading.
- **Windows terminal cleanup before restart.** `std::process::exit(0)` may bypass drop guards. Explicitly restore terminal (disable raw mode, leave alternate screen) before spawning new process and exiting.
- **Windows tests with `HOME` env var.** Windows uses `USERPROFILE` not `HOME`. Tests that depend on tilde expansion are gated with `#[cfg(not(target_os = "windows"))]`.
- **Progress bar during download.** Same forced-render pattern as AI calls: `terminal.draw()` between chunk reads. Blocks event loop, acceptable on splash screen.
- **`b`/`f` feedback keys scoped to `?` overlay.** Not global hotkeys. No conflicts with per-tab bindings. Feedback only available in Running state, not startup screen.
