# Task 2: Startup Screen + Splash

Read `specs/handoff.md` for full codebase orientation, then read `specs/v2.1-build-spec.md` for the complete V2.1 plan. This prompt covers Task 2 only. Task 1 (crate rename to "bursar") has already been completed.

## What to Do

Add a three-state application wrapper that shows a splash screen, then an entity picker, before loading any entity database. This is the biggest architectural change — it extracts the event loop from `App` and introduces a wrapper state machine.

## Part A: Refactor `App::run`

Currently `App::run` in `src/app.rs` owns the terminal and the entire event loop. Extract the loop body so that an external wrapper can drive it:

1. Add public methods to `App`:
   - `render(&mut self, terminal: &mut Terminal<impl Backend>)` — draw one frame
   - `handle_event(&mut self, event: &crossterm::event::Event)` — process one key/resize event
   - `tick(&mut self)` — periodic work: chat typewriter advance, status bar expiry, unsaved-changes check
   - `should_quit(&self) -> bool`
   - Keep (or add) `process_pending(&mut self, terminal: &mut Terminal<impl Backend>)` for the pending AI/import flags that need forced renders

2. The existing `App::run` can either be removed or rewritten to call these methods in a loop — but the wrapper in `main.rs` will be the actual caller.

3. `App::new` should NOT require a terminal — it receives an already-loaded `EntityContext` and config.

## Part B: Three-State Wrapper in `main.rs`

Create an `AppState` enum and wrapper loop:

```rust
enum AppState {
    Splash,
    Startup(StartupScreen),
    Running(App),
}
```

The wrapper loop:
```
initialize terminal
parse workspace config
state = Splash

loop:
    match &mut state:
        Splash =>
            render splash screen (logo + version number)
            sleep 1 second  // update check is Task 4, just sleep for now
            state = Startup(StartupScreen::new(config, None))

        Startup(screen) =>
            terminal.draw(|f| screen.render(f))
            poll for event
            match screen.handle_event(event):
                StartupAction::OpenEntity(index) =>
                    write last_opened_entity to workspace.toml via toml_edit
                    load EntityContext for selected entity
                    construct App
                    state = Running(app)
                StartupAction::Quit => break
                _ => continue

        Running(app) =>
            app.render(terminal)
            poll for event → app.handle_event(event)
            app.process_pending(terminal)
            app.tick()
            if app.should_quit() => break

restore terminal
```

## Part C: StartupScreen Struct

**IMPORTANT:** `src/startup.rs` already exists (601 lines) and handles DB/config initialization. Check what's in it. Create a new `src/startup_screen.rs` for the TUI screen struct to avoid conflating concerns. Keep the existing `startup.rs` for its initialization logic.

```rust
pub struct StartupScreen {
    entities: Vec<EntityEntry>,
    selected_index: usize,
    update_notice: Option<String>,   // always None for now, Task 4 populates this
    workspace_path: PathBuf,         // path to workspace.toml, for writes
}

pub struct EntityEntry {
    pub name: String,
    pub db_path: String,
    pub config_path: Option<String>,
}

pub enum StartupAction {
    OpenEntity(usize),
    Quit,
    None,
    // AddEntity, EditEntity, DeleteEntity — added in Task 3
}
```

### Rendering Layout (top to bottom)

1. **ASCII banner** — centered horizontally, accent/highlight color:
```
  _____  __ __  _____  _____ _____  _____
 /  _  \/  |  \/  _  \/  ___>  _  \/  _  \
 |  _  <|  |  ||  _  <|___  |  _  ||  _  <
 \_____/\_____/\__|\_/<_____|__|__/\__|\_/
```

2. **Version** — right-aligned below banner: `v{env!("CARGO_PKG_VERSION")}`

3. **Update notice area** — placeholder, renders nothing for now

4. **Entity list** — centered block, each entry shows name and db_path on one line
   - Selected entity highlighted (inverse colors or `>> ` prefix)
   - Pre-select the entity matching `last_opened_entity` from workspace.toml, or index 0
   - Empty list: "No entities configured. Press 'a' to add one."

5. **Hotkey bar** at bottom: `[Enter] Open  [a] Add  [e] Edit  [d] Delete  [q] Quit`

### Key Handling

- `Up/k` / `Down/j`: Navigate entity list
- `Enter`: `StartupAction::OpenEntity(selected_index)` (only if entities exist)
- `q`: `StartupAction::Quit`
- `a`, `e`, `d`: No-op for now (Task 3 adds these)

### `last_opened_entity`

- On `StartupScreen::new`, read `last_opened_entity` from the parsed workspace config
- Match by name to set `selected_index`
- When the wrapper processes `OpenEntity`, use `toml_edit` to update `last_opened_entity` in workspace.toml:

```rust
let content = std::fs::read_to_string(&workspace_path)?;
let mut doc = content.parse::<toml_edit::DocumentMut>()?;
doc["last_opened_entity"] = toml_edit::value(&entity_name);
std::fs::write(&workspace_path, doc.to_string())?;
```

### db_path Resolution

Entity `db_path` values may be relative paths. They must be resolved relative to the directory containing workspace.toml, NOT the CWD. Check how the existing `src/startup.rs` resolves paths and follow the same pattern.

## Dependency Addition

Add to `Cargo.toml`:
```toml
toml_edit = "0.22"
```

Check the latest version on crates.io and use that.

## Verification Checklist

```bash
cargo fmt
cargo clippy -D warnings
cargo test
```

Then manually verify:
- App launches, shows splash screen with logo + version for ~1 second
- Transitions to entity picker showing all entities from workspace.toml
- Arrow keys navigate the entity list
- Enter opens the selected entity → normal app view with all tabs working
- `q` on the startup screen quits cleanly
- Previously opened entity is pre-selected on next launch
- Empty entity list shows helpful message
- All existing app functionality works after entity selection

## Commit

```
V2.1, Task 2: Add splash screen and startup entity picker
```
