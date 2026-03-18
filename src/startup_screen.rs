//! TUI startup screen: entity picker shown after the splash, before any entity DB is loaded.

use std::path::PathBuf;

use crossterm::event::{Event, KeyCode};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::config::WorkspaceConfig;

const BANNER: &str = r"  _____  __ __  _____  _____ _____  _____
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
    /// Open the entity at the given index in the entity list.
    OpenEntity(usize),
    /// Quit the application cleanly.
    Quit,
    /// No state change required.
    None,
}

/// TUI screen displayed after the splash screen.
/// Shows all configured entities and lets the user select one to open.
pub struct StartupScreen {
    pub entities: Vec<EntityEntry>,
    pub selected_index: usize,
    /// Reserved for Task 4 (update check). Always `None` for now.
    pub update_notice: Option<String>,
    /// Path to `workspace.toml` — used by Task 3 entity management writes.
    pub workspace_path: PathBuf,
}

impl StartupScreen {
    /// Creates a new `StartupScreen` from the workspace config.
    /// Pre-selects the last-opened entity if recorded in `config.last_opened_entity`.
    pub fn new(config: &WorkspaceConfig, workspace_path: PathBuf) -> Self {
        let entities: Vec<EntityEntry> = config
            .entities
            .iter()
            .map(|e| EntityEntry {
                name: e.name.clone(),
                db_path: e.db_path.to_string_lossy().to_string(),
                config_path: e.config_path.clone(),
            })
            .collect();

        let selected_index = config
            .last_opened_entity
            .as_deref()
            .and_then(|last| entities.iter().position(|e| e.name == last))
            .unwrap_or(0);

        Self {
            entities,
            selected_index,
            update_notice: None,
            workspace_path,
        }
    }

    /// Renders the complete startup screen.
    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5), // banner (4 lines) + version line
                Constraint::Min(3),    // entity list
                Constraint::Length(1), // hotkey bar
            ])
            .split(area);

        render_banner_area(frame, chunks[0]);
        self.render_entity_list(frame, chunks[1]);
        self.render_hotkey_bar(frame, chunks[2]);
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

    fn render_hotkey_bar(&self, frame: &mut Frame, area: Rect) {
        let bar = Paragraph::new("[Enter] Open  [a] Add  [e] Edit  [d] Delete  [q] Quit")
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(bar, area);
    }

    /// Handles one input event. Returns a [`StartupAction`] describing any state change.
    pub fn handle_event(&mut self, event: &Event) -> StartupAction {
        if let Event::Key(key) = event {
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
                    if !self.entities.is_empty() {
                        return StartupAction::OpenEntity(self.selected_index);
                    }
                }
                KeyCode::Char('q') => return StartupAction::Quit,
                _ => {}
            }
        }
        StartupAction::None
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
            .alignment(Alignment::Right)
            .style(Style::default().fg(Color::DarkGray)),
        chunks[1],
    );
}

/// Renders the splash screen: banner centered vertically with no other controls.
pub fn render_splash(frame: &mut Frame) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(5),
            Constraint::Fill(1),
        ])
        .split(area);
    render_banner_area(frame, chunks[1]);
}
