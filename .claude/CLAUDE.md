# Rust Development Guidelines

## MCP Tools — Use These First

Two MCP servers are available. Use them proactively:

- **rust-mcp-server**: Run `cargo check`, `cargo build`, `cargo test`, `cargo clippy`,
  `cargo fmt`, and `cargo add` directly. After every code change, run
  `cargo clippy --all-targets -- -D warnings` and fix all warnings before
  presenting output. Never hand off code with clippy warnings.
- **mcp-rust-docs** (or crates-mcp): Before using any external crate, look up its
  current API via docs.rs. Do not rely on training-data memory for crate APIs —
  they change. Fetch docs first, then write code.

## Verification Loop (Always Follow This Order)

```
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings   # zero warnings required
cargo test
```

If `rust-mcp-server` is available, run these commands through it rather than
assuming correctness.

## Code Philosophy

- **Clarity over cleverness.** Prefer the simplest solution that correctly solves
  the problem. Idiomatic Rust is the goal, not impressive Rust.
- **Let the type system work.** Encode invariants in types, not comments or runtime
  checks. Use newtypes to distinguish semantically different values.
- **Immutability by default.** Only introduce `mut` when genuinely needed.
- **Borrow before own.** Prefer `&T` and `&mut T` over taking ownership unless the
  function semantically consumes the value.

## Error Handling

- **Never** use `.unwrap()` in production code. Use `.expect("reason")` only for
  invariants that are genuinely impossible to violate, with a message explaining why.
- Use `thiserror` for library/module error types; use `anyhow` for application-level
  or CLI error handling.
- Propagate with `?`. Add context with `.context("what was happening")` from `anyhow`.
- Return `Result<T, E>` for all fallible operations; never panic for recoverable errors.

## Iterators and Data Transformation

- Prefer iterator combinators (`.map()`, `.filter()`, `.flat_map()`, `.fold()`,
  `.collect()`) over manual `for` loops when the intent is transformation or
  aggregation without side effects.
- Use `for` loops when the body has meaningful side effects or early-return logic
  that would obscure a chain.
- Avoid collecting into a `Vec` just to iterate again — chain adapters lazily.

## Types and Traits

- Derive `Debug`, `Clone`, and `PartialEq` when appropriate. Do not derive what
  you will not use.
- Use the builder pattern for structs with more than 4 fields or complex
  initialization.
- Prefer `impl Trait` in function arguments for flexibility; use generics when
  you need the concrete type for bounds or associated types.
- `unsafe` is forbidden except in justified, documented blocks with a
  `// SAFETY:` comment explaining every invariant being upheld.

## Concurrency

- All async code uses `tokio`. Offload blocking work with
  `tokio::task::spawn_blocking`.
- Shared state: `Arc<RwLock<T>>` for read-heavy data; `Arc<Mutex<T>>` for
  write-heavy. Prefer message-passing (channels) over shared state.
- Never hold a lock across an `.await` point.

## Documentation

- All public functions, structs, enums, and traits need `///` doc comments with
  a description, `# Errors` (if `Result`), and a `# Examples` block.
- Private code does not need comments if the code is clear. Do not add comments
  that restate what the code already says.

## Dependencies

- Check `Cargo.toml` before adding a crate — it may already be present.
- Use `mcp-rust-docs` to verify the correct current API before writing code
  against a dependency.
- Use `thiserror`, `anyhow`, `tokio`, `serde`/`serde_json`, `tracing` as the
  standard stack. Justify deviations.

## What Not to Do

- No `println!` in library code — use `tracing::debug!` / `tracing::error!`.
- No `unwrap()` or unhandled `expect()` outside of tests.
- No wildcard imports (`use module::*`) except in `#[cfg(test)]` modules
  (`use super::*` is fine).
- No commented-out code, no debug `dbg!` macros in commits.
- No secrets or `.env` values in source — use `dotenvy` + `.gitignore`.
