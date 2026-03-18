# Task 1: Rename Crate to "bursar"

Read `specs/handoff.md` for full codebase orientation, then read `specs/v2.1-build-spec.md` for the complete V2.1 plan. This prompt covers Task 1 only.

## What to Do

Rename the application from "accounting" to "bursar" throughout the codebase. This is a mechanical find-and-replace task but it touches many files.

### Step-by-step

1. **`Cargo.toml`**: Change `name = "accounting"` to `name = "bursar"`

2. **All `use` statements in `src/`**: Find every `use accounting::` and replace with `use bursar::`. Check for `extern crate accounting` too. Pay special attention to `src/integration_tests.rs` (478 lines) — it likely has crate-level imports.

3. **`CLAUDE.md`**: Update any references to the old binary name or crate name.

4. **`specs/guide/user-guide.md`**: Update references to the application name.

5. **User-facing strings in `src/`**: Search for string literals containing "accounting" (case-insensitive) that are displayed to users — status bar, help overlay, title rendering, etc. Replace with "Bursar" or "bursar" as appropriate for context.

6. **Do NOT rename**: The git repo directory, database files, config directories, or the `specs/` folder itself.

### Verification Checklist

Run all of these before committing:

```bash
cargo fmt
cargo clippy -D warnings
cargo test
```

Then confirm:
- `cargo build` produces a `bursar` binary in `target/debug/`
- `grep -rn "use accounting::" src/` returns zero results
- `grep -rn '"accounting"' src/` returns zero results (user-facing strings only — domain references like "accounting period" are fine)

### Commit

```
V2.1, Task 1: Rename crate from accounting to bursar
```
