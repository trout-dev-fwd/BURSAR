//! TUI startup screen: entity picker shown after the splash, before any entity DB is loaded.

use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::{Event, KeyCode};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::config::WorkspaceConfig;
use crate::widgets::{
    ConfirmAction, Confirmation, ExistingDbAction, ExistingDbModal, TextInputAction, TextInputModal,
};

const BANNER: &str = r" _____  __ __  _____  _____ _____  _____
 /  _  \/  |  \/  _  \/  ___>  _  \/  _  \
 |  _  <|  |  ||  _  <|___  |  _  ||  _  <
 \_____/\_____/\__|\_/<_____|__|__/\__|\_/";

/// A single entity entry shown in the startup picker.
pub struct EntityEntry {
    pub name: String,
    pub db_path: String,
    pub config_path: Option<String>,
}

/// Actions returned by [`StartupScreen::handle_event`].
pub enum StartupAction {
    /// Open the entity with the given name and resolved database path.
    OpenEntity { name: String, db_path: PathBuf },
    /// Quit the application cleanly.
    Quit,
    /// No state change required.
    None,
}

/// Which entity management operation is awaiting text-input confirmation.
enum PendingEntityAction {
    Add,
    Edit(usize),
}

/// State saved when an add-entity operation is deferred pending the existing-db modal.
struct PendingAdd {
    name: String,
    db_filename: String,
    config_filename: String,
    entity_dir: PathBuf,
}

/// TUI screen displayed after the splash screen.
/// Shows all configured entities and lets the user select one to open.
pub struct StartupScreen {
    pub entities: Vec<EntityEntry>,
    pub selected_index: usize,
    /// Reserved for Task 4 (update check). Always `None` for now.
    pub update_notice: Option<String>,
    /// Path to `workspace.toml` — used by entity management writes.
    pub workspace_path: PathBuf,
    /// Active text-input modal (add / edit).
    text_input: Option<TextInputModal>,
    /// What to do when the text-input modal confirms.
    pending_action: Option<PendingEntityAction>,
    /// Active delete-confirmation modal.
    confirm_delete: Option<Confirmation>,
    /// Active existing-database modal (shown during add when db file already exists).
    existing_db_modal: Option<ExistingDbModal>,
    /// Deferred add state waiting for the existing-db modal decision.
    pending_add: Option<PendingAdd>,
    /// Status or error message shown below the entity list.
    status_message: Option<String>,
}

impl StartupScreen {
    /// Creates a new `StartupScreen` from the workspace config.
    /// Pre-selects the last-opened entity if recorded in `config.last_opened_entity`.
    /// `update_notice` is shown in yellow above the entity list when `Some`.
    pub fn new(
        config: &WorkspaceConfig,
        workspace_path: PathBuf,
        update_notice: Option<String>,
    ) -> Self {
        let entities = Self::entities_from_config(config);

        let selected_index = config
            .last_opened_entity
            .as_deref()
            .and_then(|last| entities.iter().position(|e| e.name == last))
            .unwrap_or(0);

        Self {
            entities,
            selected_index,
            update_notice,
            workspace_path,
            text_input: None,
            pending_action: None,
            confirm_delete: None,
            existing_db_modal: None,
            pending_add: None,
            status_message: None,
        }
    }

    // ── Rendering ─────────────────────────────────────────────────────────────

    /// Renders the complete startup screen.
    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        let notice_height = if self.update_notice.is_some() { 1 } else { 0 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),             // banner (4 lines) + version line
                Constraint::Length(notice_height), // update notice (0 or 1 line)
                Constraint::Min(3),                // entity list
                Constraint::Length(1),             // status message
                Constraint::Length(1),             // hotkey bar
            ])
            .split(area);

        render_banner_area(frame, chunks[0]);
        if let Some(notice) = &self.update_notice {
            frame.render_widget(
                Paragraph::new(notice.as_str())
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::Yellow)),
                chunks[1],
            );
        }
        self.render_entity_list(frame, chunks[2]);
        self.render_status_bar(frame, chunks[3]);
        self.render_hotkey_bar(frame, chunks[4]);

        // Overlay modals last so they appear on top.
        if let Some(modal) = &self.text_input {
            modal.render(frame, area);
        } else if let Some(modal) = &self.existing_db_modal {
            modal.render(frame, area);
        } else if let Some(confirm) = &self.confirm_delete {
            confirm.render(frame, area);
        }
    }

    fn render_entity_list(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(" Entities ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::DarkGray));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.entities.is_empty() {
            let msg = Paragraph::new("No entities configured. Press 'a' to add one.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(msg, inner);
            return;
        }

        let lines: Vec<Line> = self
            .entities
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let text = format!("{} \u{2014} {}", entry.name, entry.db_path);
                if i == self.selected_index {
                    Line::from(Span::styled(
                        format!(">> {text}"),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(Span::styled(
                        format!("   {text}"),
                        Style::default().fg(Color::White),
                    ))
                }
            })
            .collect();

        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        if let Some(msg) = &self.status_message {
            let color = if msg.starts_with("Error") || msg.starts_with("error") {
                Color::Red
            } else {
                Color::Green
            };
            let p = Paragraph::new(msg.as_str())
                .alignment(Alignment::Center)
                .style(Style::default().fg(color));
            frame.render_widget(p, area);
        }
    }

    fn render_hotkey_bar(&self, frame: &mut Frame, area: Rect) {
        let bar = Paragraph::new("[Enter] Open  [a] Add  [e] Edit  [d] Delete  [q] Quit")
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(bar, area);
    }

    // ── Event handling ────────────────────────────────────────────────────────

    /// Handles one input event. Returns a [`StartupAction`] describing any state change.
    pub fn handle_event(&mut self, event: &Event) -> StartupAction {
        let Event::Key(key) = event else {
            return StartupAction::None;
        };

        // ── Text-input modal is active ─────────────────────────────────────
        if let Some(mut modal) = self.text_input.take() {
            match modal.handle_key(*key) {
                TextInputAction::Confirm(text) => {
                    let text = text.trim().to_string();
                    if text.is_empty() {
                        self.status_message = Some("Name cannot be empty.".to_string());
                    } else if let Some(action) = self.pending_action.take()
                        && let Err(e) = self.apply_text_action(action, text)
                    {
                        self.status_message = Some(format!("Error: {e}"));
                    }
                    self.pending_action = None;
                }
                TextInputAction::Cancel => {
                    self.pending_action = None;
                }
                TextInputAction::None => {
                    // Put modal back — still editing.
                    self.text_input = Some(modal);
                }
            }
            return StartupAction::None;
        }

        // ── Existing-database modal is active ─────────────────────────────
        if let Some(mut modal) = self.existing_db_modal.take() {
            match modal.handle_key(*key) {
                ExistingDbAction::Restore => {
                    if let Some(pending) = self.pending_add.take()
                        && let Err(e) = self.finish_add_entity(&pending, false)
                    {
                        self.status_message = Some(format!("Error: {e}"));
                    }
                }
                ExistingDbAction::Fresh => {
                    if let Some(pending) = self.pending_add.take()
                        && let Err(e) = self.finish_add_entity(&pending, true)
                    {
                        self.status_message = Some(format!("Error: {e}"));
                    }
                }
                ExistingDbAction::Cancel => {
                    self.pending_add = None;
                }
                ExistingDbAction::Pending => {
                    self.existing_db_modal = Some(modal);
                }
            }
            return StartupAction::None;
        }

        // ── Delete confirmation modal is active ────────────────────────────
        if let Some(mut confirm) = self.confirm_delete.take() {
            match confirm.handle_key(*key) {
                ConfirmAction::Confirmed => {
                    let idx = self.selected_index;
                    if let Err(e) = self.delete_entity(idx) {
                        self.status_message = Some(format!("Error: {e}"));
                    }
                }
                ConfirmAction::Cancelled => {}
                ConfirmAction::Pending => {
                    self.confirm_delete = Some(confirm);
                }
            }
            return StartupAction::None;
        }

        // ── Normal navigation ──────────────────────────────────────────────
        // Any key clears the previous status message.
        self.status_message = None;

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected_index = self.selected_index.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected_index + 1 < self.entities.len() {
                    self.selected_index += 1;
                }
            }
            KeyCode::Enter => {
                if let Some(entry) = self.entities.get(self.selected_index) {
                    return StartupAction::OpenEntity {
                        name: entry.name.clone(),
                        db_path: Path::new(&entry.db_path).to_path_buf(),
                    };
                }
            }
            KeyCode::Char('q') => return StartupAction::Quit,

            KeyCode::Char('a') => {
                self.text_input = Some(TextInputModal::new("Entity name", ""));
                self.pending_action = Some(PendingEntityAction::Add);
            }
            KeyCode::Char('e') => {
                if !self.entities.is_empty() {
                    let current_name = self.entities[self.selected_index].name.clone();
                    self.text_input = Some(TextInputModal::new("Entity name", current_name));
                    self.pending_action = Some(PendingEntityAction::Edit(self.selected_index));
                }
            }
            KeyCode::Char('d') => {
                if !self.entities.is_empty() {
                    let name = &self.entities[self.selected_index].name;
                    self.confirm_delete = Some(Confirmation::new(format!(
                        "Remove '{name}' from workspace? (files are preserved)"
                    )));
                }
            }
            _ => {}
        }

        StartupAction::None
    }

    // ── Entity management helpers ─────────────────────────────────────────────

    /// Dispatches a confirmed text action to the appropriate add/edit handler.
    fn apply_text_action(&mut self, action: PendingEntityAction, name: String) -> Result<()> {
        match action {
            PendingEntityAction::Add => self.add_entity(name),
            PendingEntityAction::Edit(idx) => self.edit_entity(idx, name),
        }
    }

    /// Validates and begins the add-entity flow. If the database file already exists
    /// on disk, shows the existing-db modal instead of completing immediately.
    fn add_entity(&mut self, name: String) -> Result<()> {
        // Check for duplicate name (case-insensitive).
        let name_lower = name.to_lowercase();
        if let Some(existing) = self
            .entities
            .iter()
            .find(|e| e.name.to_lowercase() == name_lower)
        {
            anyhow::bail!("An entity named '{}' already exists.", existing.name);
        }

        let workspace_dir = self
            .workspace_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();

        // Derive filenames from the name.
        let stem = slugify(&name);
        if stem.is_empty() {
            anyhow::bail!(
                "Could not derive a valid filename from '{name}'. Use only letters, numbers, and spaces."
            );
        }
        let db_filename = format!("{stem}.sqlite");
        let config_filename = format!("{stem}.toml");

        // Check for db_path collision with an active entity.
        if let Some(existing) = self.entities.iter().find(|e| {
            let existing_file = std::path::Path::new(&e.db_path)
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("");
            existing_file == db_filename
        }) {
            anyhow::bail!(
                "Database path '{db_filename}' is already in use by '{}'.",
                existing.name
            );
        }

        // Resolve directory: sibling of first existing entity db, or workspace dir.
        let entity_dir = self
            .entities
            .first()
            .and_then(|e| {
                let p = std::path::Path::new(&e.db_path);
                if p.is_absolute() {
                    p.parent().map(|d| d.to_path_buf())
                } else {
                    workspace_dir.join(p).parent().map(|d| d.to_path_buf())
                }
            })
            .unwrap_or_else(|| workspace_dir.clone());

        // Check if the database file already exists on disk.
        let db_path = entity_dir.join(&db_filename);
        if db_path.exists() {
            self.pending_add = Some(PendingAdd {
                name,
                db_filename: db_filename.clone(),
                config_filename,
                entity_dir,
            });
            self.existing_db_modal = Some(ExistingDbModal::new(&db_filename));
            return Ok(());
        }

        let pending = PendingAdd {
            name,
            db_filename,
            config_filename,
            entity_dir,
        };
        self.finish_add_entity(&pending, false)
    }

    /// Completes the add-entity flow: writes to workspace.toml, optionally deletes
    /// old files (when `delete_existing` is true), and creates the entity config stub.
    fn finish_add_entity(&mut self, pending: &PendingAdd, delete_existing: bool) -> Result<()> {
        let db_path = pending.entity_dir.join(&pending.db_filename);
        let config_path = pending.entity_dir.join(&pending.config_filename);

        if delete_existing {
            // Remove old database and config files.
            if db_path.exists() {
                std::fs::remove_file(&db_path)?;
            }
            if config_path.exists() {
                std::fs::remove_file(&config_path)?;
            }
        }

        // Write to workspace.toml using toml_edit.
        let content = std::fs::read_to_string(&self.workspace_path)?;
        let mut doc = content.parse::<toml_edit::DocumentMut>()?;

        let mut entity = toml_edit::Table::new();
        entity["name"] = toml_edit::value(&pending.name);
        entity["db_path"] = toml_edit::value(&pending.db_filename);
        entity["config_path"] = toml_edit::value(&pending.config_filename);

        if doc.get("entities").is_none() {
            doc["entities"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
        }
        doc["entities"]
            .as_array_of_tables_mut()
            .ok_or_else(|| anyhow::anyhow!("entities is not an array of tables"))?
            .push(entity);

        std::fs::write(&self.workspace_path, doc.to_string())?;

        // Create a minimal entity .toml file (does not overwrite if it exists).
        if !config_path.exists() {
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(
                &config_path,
                format!("# Entity configuration for {}\n", pending.name),
            )?;
        }

        // Refresh.
        self.reload_entities()?;
        if let Some(idx) = self.entities.iter().position(|e| e.name == pending.name) {
            self.selected_index = idx;
        }
        let action = if delete_existing {
            "Added (fresh)"
        } else {
            "Added"
        };
        self.status_message = Some(format!("{action} '{}'.", pending.name));
        Ok(())
    }

    /// Renames the display name of entity at `index` in workspace.toml.
    fn edit_entity(&mut self, index: usize, new_name: String) -> Result<()> {
        let content = std::fs::read_to_string(&self.workspace_path)?;
        let mut doc = content.parse::<toml_edit::DocumentMut>()?;
        doc["entities"]
            .as_array_of_tables_mut()
            .ok_or_else(|| anyhow::anyhow!("entities is not an array of tables"))?
            .get_mut(index)
            .ok_or_else(|| anyhow::anyhow!("entity index out of bounds"))?["name"] =
            toml_edit::value(&new_name);
        std::fs::write(&self.workspace_path, doc.to_string())?;

        self.reload_entities()?;
        self.status_message = Some(format!("Renamed to '{new_name}'."));
        Ok(())
    }

    /// Removes entity at `index` from workspace.toml (files are preserved).
    fn delete_entity(&mut self, index: usize) -> Result<()> {
        let db_path = self.entities[index].db_path.clone();

        let content = std::fs::read_to_string(&self.workspace_path)?;
        let mut doc = content.parse::<toml_edit::DocumentMut>()?;
        doc["entities"]
            .as_array_of_tables_mut()
            .ok_or_else(|| anyhow::anyhow!("entities is not an array of tables"))?
            .remove(index);
        std::fs::write(&self.workspace_path, doc.to_string())?;

        self.reload_entities()?;
        // Keep selected_index in bounds.
        if self.selected_index >= self.entities.len() {
            self.selected_index = self.entities.len().saturating_sub(1);
        }
        self.status_message = Some(format!("Removed. Database preserved at {db_path}"));
        Ok(())
    }

    /// Re-reads workspace.toml and refreshes `self.entities`.
    fn reload_entities(&mut self) -> Result<()> {
        let config = crate::config::load_config(&self.workspace_path)?;
        self.entities = Self::entities_from_config(&config);
        Ok(())
    }

    /// Converts a [`WorkspaceConfig`] into a flat list of [`EntityEntry`] values.
    fn entities_from_config(config: &WorkspaceConfig) -> Vec<EntityEntry> {
        config
            .entities
            .iter()
            .map(|e| EntityEntry {
                name: e.name.clone(),
                db_path: e.db_path.to_string_lossy().to_string(),
                config_path: e.config_path.clone(),
            })
            .collect()
    }
}

/// Renders the ASCII banner + version line into `area`.
/// Shared between the splash screen and the startup screen.
pub fn render_banner_area(frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Length(1)])
        .split(area);

    let banner_lines: Vec<Line> = BANNER
        .lines()
        .map(|l| {
            Line::from(Span::styled(
                l,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
        })
        .collect();

    frame.render_widget(
        Paragraph::new(banner_lines).alignment(Alignment::Center),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(format!("v{}", env!("CARGO_PKG_VERSION")))
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray)),
        chunks[1],
    );
}

/// Converts a display name into a filesystem-safe slug.
///
/// Lowercases, replaces non-ASCII-alphanumeric characters with hyphens,
/// collapses consecutive hyphens, and trims leading/trailing hyphens.
fn slugify(name: &str) -> String {
    let mut result = String::new();
    let mut prev_hyphen = true; // treat start as hyphen to trim leading
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            prev_hyphen = false;
        } else if !prev_hyphen {
            result.push('-');
            prev_hyphen = true;
        }
    }
    // Trim trailing hyphen.
    if result.ends_with('-') {
        result.pop();
    }
    result
}

/// Progress state for a binary download on the splash screen.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateProgress {
    /// Known total size; shows `▰▰▰▱▱▱▱▱▱▱ [30%]`.
    Determinate { percent: u8 },
    /// Unknown total size (no Content-Length header); shows `▰▱▰▱▰▱▰▱▰▱`.
    Indeterminate,
    /// Download complete; shows `▰▰▰▰▰▰▰▰▰▰ [100%]`.
    Complete,
}

/// State passed to [`render_splash`] to control what is shown below the banner.
#[derive(Default)]
pub struct SplashState {
    /// Status line text (e.g. `"Updating to v0.2.2. . ."`). `None` = show nothing.
    pub update_status: Option<String>,
    /// Progress bar state. `None` = no bar rendered.
    pub progress: Option<UpdateProgress>,
}

/// Renders the progress bar string for the given `UpdateProgress`.
///
/// Returns a string like `"▰▰▰▰▱▱▱▱▱▱ [40%]"` (determinate),
/// `"▰▱▰▱▰▱▰▱▰▱"` (indeterminate), or `"▰▰▰▰▰▰▰▰▰▰ [100%]"` (complete).
pub fn render_progress_bar(progress: &UpdateProgress) -> String {
    const FILLED: char = '▰';
    const EMPTY: char = '▱';
    const WIDTH: usize = 10;

    match progress {
        UpdateProgress::Indeterminate => (0..WIDTH)
            .map(|i| if i % 2 == 0 { FILLED } else { EMPTY })
            .collect(),
        UpdateProgress::Complete => {
            format!(
                "{} [100%]",
                std::iter::repeat_n(FILLED, WIDTH).collect::<String>()
            )
        }
        UpdateProgress::Determinate { percent } => {
            let filled = ((*percent as usize) / WIDTH).min(WIDTH);
            let bar: String = (0..WIDTH)
                .map(|i| if i < filled { FILLED } else { EMPTY })
                .collect();
            format!("{bar} [{}%]", percent)
        }
    }
}

/// Renders the splash screen: banner centered vertically with optional update status.
pub fn render_splash(frame: &mut Frame, state: &SplashState) {
    let area = frame.area();

    let extra_lines: u16 = match (&state.update_status, &state.progress) {
        (None, None) => 0,
        (Some(_), None) | (None, Some(_)) => 1,
        (Some(_), Some(_)) => 2,
    };
    let banner_height: u16 = 5 + extra_lines;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(banner_height),
            Constraint::Fill(1),
        ])
        .split(area);

    let banner_area = chunks[1];
    let inner_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // banner (4 lines) + version
            Constraint::Length(if state.update_status.is_some() { 1 } else { 0 }),
            Constraint::Length(if state.progress.is_some() { 1 } else { 0 }),
        ])
        .split(banner_area);

    render_banner_area(frame, inner_chunks[0]);

    if let Some(status) = &state.update_status {
        frame.render_widget(
            Paragraph::new(status.as_str())
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray)),
            inner_chunks[1],
        );
    }

    if let Some(progress) = &state.progress {
        frame.render_widget(
            Paragraph::new(render_progress_bar(progress))
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Cyan)),
            inner_chunks[2],
        );
    }
}

#[cfg(test)]
mod progress_bar_tests {
    use super::*;

    #[test]
    fn progress_bar_zero_percent() {
        let bar = render_progress_bar(&UpdateProgress::Determinate { percent: 0 });
        assert_eq!(bar, "▱▱▱▱▱▱▱▱▱▱ [0%]");
    }

    #[test]
    fn progress_bar_fifty_percent() {
        let bar = render_progress_bar(&UpdateProgress::Determinate { percent: 50 });
        assert_eq!(bar, "▰▰▰▰▰▱▱▱▱▱ [50%]");
    }

    #[test]
    fn progress_bar_one_hundred_percent() {
        let bar = render_progress_bar(&UpdateProgress::Complete);
        assert_eq!(bar, "▰▰▰▰▰▰▰▰▰▰ [100%]");
    }

    #[test]
    fn progress_bar_indeterminate() {
        let bar = render_progress_bar(&UpdateProgress::Indeterminate);
        assert_eq!(bar, "▰▱▰▱▰▱▰▱▰▱");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_simple_name() {
        assert_eq!(slugify("My Farm LLC"), "my-farm-llc");
    }

    #[test]
    fn slugify_special_characters() {
        assert_eq!(slugify("O'Brien & Sons"), "o-brien-sons");
    }

    #[test]
    fn slugify_extra_whitespace() {
        assert_eq!(slugify("  Weird  --  Name  "), "weird-name");
    }

    #[test]
    fn slugify_already_simple() {
        assert_eq!(slugify("simple"), "simple");
    }

    #[test]
    fn slugify_accented_chars() {
        assert_eq!(slugify("José's Café"), "jos-s-caf");
    }

    #[test]
    fn slugify_empty() {
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn slugify_only_special_chars() {
        assert_eq!(slugify("!!!"), "");
    }
}
