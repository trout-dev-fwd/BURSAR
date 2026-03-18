# Task 3: Entity Management (Add / Edit / Delete)

Read `specs/handoff.md` for full codebase orientation, then read `specs/v2.1-build-spec.md` for the complete V2.1 plan. This prompt covers Task 3 only. Tasks 1-2 have been completed — the crate is named "bursar", the startup screen and splash exist, and `toml_edit` is already a dependency.

## What to Do

Implement the `a` (add), `e` (edit), and `d` (delete) hotkeys on the startup screen for managing entities in workspace.toml.

## Part A: TextInputModal Widget

Create `src/widgets/text_input_modal.rs` — a reusable single-line text input dialog.

```rust
pub struct TextInputModal {
    title: String,
    buffer: String,
    cursor_pos: usize,
}

pub enum TextInputAction {
    Confirm(String),
    Cancel,
    None,
}

impl TextInputModal {
    pub fn new(title: impl Into<String>, prefill: impl Into<String>) -> Self { ... }
    pub fn handle_key(&mut self, key: KeyEvent) -> TextInputAction { ... }
    pub fn render(&self, frame: &mut Frame, area: Rect) { ... }
}
```

Behavior:
- Renders as a centered modal box, similar in style to the existing `src/widgets/confirmation.rs`
- Title at top of modal
- Single-line text input with visible cursor (underscore or block)
- **Keys**: Left/Right/Home/End for cursor movement, Backspace/Delete for editing, Enter → `Confirm(buffer)`, Esc → `Cancel`
- All printable characters insert at cursor position

Register in `src/widgets/mod.rs`.

## Part B: Wire Up StartupScreen

Add to `StartupScreen`:

```rust
pub struct StartupScreen {
    // ... existing fields from Task 2 ...
    text_input: Option<TextInputModal>,
    pending_action: Option<PendingEntityAction>,
    status_message: Option<String>,
}

enum PendingEntityAction {
    Add,
    Edit(usize),  // index being edited
}
```

### Key dispatch in StartupScreen

When `text_input.is_some()`, ALL keys go to the text input modal first. Otherwise, keys go to the normal startup screen handler.

```
handle_event:
    if let Some(modal) = &mut self.text_input:
        match modal.handle_key(key):
            Confirm(text) => process based on pending_action
            Cancel => clear text_input and pending_action
            None => ()
        return StartupAction::None

    // For delete: if a confirmation modal is active, handle it similarly
    // (reuse the existing confirmation widget from src/widgets/confirmation.rs)

    match key:
        'a' => open TextInputModal with title "Entity name:", set pending_action = Add
        'e' => if entities not empty, open TextInputModal pre-filled with current name, set pending_action = Edit(index)
        'd' => if entities not empty, open confirmation dialog
        // ... existing Enter, Up/Down, q handling ...
```

## Part C: Add Entity

When `TextInputModal` confirms with `PendingEntityAction::Add`:

1. **Derive filenames** from the entered name:
   - Database: lowercase, replace spaces with hyphens, append `.sqlite` → e.g. "My Farm LLC" → `my-farm-llc.sqlite`
   - Entity config: same stem, `.toml` extension → `my-farm-llc.toml`

2. **Resolve directory**: Use the parent directory of the first existing entity's `db_path` (resolved relative to workspace.toml's directory). If no entities exist, use the workspace.toml directory itself.

3. **Write to workspace.toml** using `toml_edit`:
   ```rust
   let content = std::fs::read_to_string(&workspace_path)?;
   let mut doc = content.parse::<toml_edit::DocumentMut>()?;

   let mut entity = toml_edit::Table::new();
   entity["name"] = toml_edit::value(&name);
   entity["db_path"] = toml_edit::value(&db_filename);
   entity["config_path"] = toml_edit::value(&config_filename);

   // Get or create the entities array of tables
   if doc.get("entities").is_none() {
       doc["entities"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
   }
   doc["entities"]
       .as_array_of_tables_mut()
       .ok_or_else(|| anyhow::anyhow!("entities is not an array of tables"))?
       .push(entity);

   std::fs::write(&workspace_path, doc.to_string())?;
   ```

4. **Create the entity `.toml` file** with minimal content (e.g., a comment header). Do NOT create the `.sqlite` file — `EntityDb::open` handles that on first use.

5. **Refresh** the entity list by re-reading workspace.toml.

## Part D: Edit Entity

When `TextInputModal` confirms with `PendingEntityAction::Edit(index)`:

1. **Update workspace.toml** using `toml_edit`:
   ```rust
   let mut doc = content.parse::<toml_edit::DocumentMut>()?;
   doc["entities"]
       .as_array_of_tables_mut()
       .ok_or_else(|| anyhow::anyhow!("entities is not an array of tables"))?
       .get_mut(index)
       .ok_or_else(|| anyhow::anyhow!("entity index out of bounds"))?
       ["name"] = toml_edit::value(&new_name);
   std::fs::write(&workspace_path, doc.to_string())?;
   ```

2. Do NOT rename any files — just the display name.

3. **Refresh** the entity list.

## Part E: Delete Entity

When the user presses `d` and the confirmation modal returns "yes":

1. **Remove from workspace.toml** using `toml_edit`:
   ```rust
   doc["entities"]
       .as_array_of_tables_mut()
       .ok_or_else(|| anyhow::anyhow!("entities is not an array of tables"))?
       .remove(index);
   std::fs::write(&workspace_path, doc.to_string())?;
   ```

2. Do NOT delete the `.sqlite` or `.toml` files.

3. Set `status_message = Some(format!("Removed. Database preserved at {}", db_path))`. Render this message on the startup screen (it can fade on next keypress or persist until dismissed).

4. **Adjust `selected_index`**: if it's now >= the entity count, set it to `entities.len().saturating_sub(1)`.

5. **Refresh** the entity list.

## Error Handling

All `toml_edit` operations and file I/O should use `?` propagation. If an operation fails, show the error as a status message on the startup screen rather than panicking. The startup screen should have a way to display error messages (same area as the status message).

## Verification Checklist

```bash
cargo fmt
cargo clippy -D warnings
cargo test
```

Then manually verify:
- `a` → type name → Enter: new entity appears in list and in workspace.toml, `.toml` config file created, no `.sqlite` created
- `e` → edit name → Enter: name updated in workspace.toml, files untouched
- `d` → confirmation → Enter: entity removed from workspace.toml, files preserved, status message shown
- Operations work correctly with 1 entity, multiple entities, and 0 entities
- Formatting of untouched sections in workspace.toml is preserved (comments, key ordering)
- After adding an entity and selecting it with Enter, the database is created by `EntityDb::open` and the app works normally
- `selected_index` stays in bounds after delete
- Esc cancels add/edit without changes

## Commit

```
V2.1, Task 3: Add entity management (add/edit/delete) on startup screen
```
