//! Modal shown when adding an entity whose database file already exists on disk.
//!
//! Offers three choices: restore the existing data, start fresh (deletes old file),
//! or cancel.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use super::centered_rect;

/// Result of a key event handled by the existing-database modal.
#[derive(Debug, Clone, PartialEq)]
pub enum ExistingDbAction {
    /// Restore the existing database and config files.
    Restore,
    /// Delete old files and start fresh.
    Fresh,
    /// Cancel — return to entity list.
    Cancel,
    /// Key consumed; still waiting for a decision.
    Pending,
}

/// A modal that asks the user how to handle an existing database file
/// when adding a new entity with a colliding filename.
pub struct ExistingDbModal {
    db_filename: String,
}

impl ExistingDbModal {
    /// Creates a new modal for the given database filename (just the filename, not full path).
    pub fn new(db_filename: impl Into<String>) -> Self {
        Self {
            db_filename: db_filename.into(),
        }
    }

    /// Handles a key event.
    ///
    /// - `r` → Restore existing data
    /// - `n` → Start fresh (destructive)
    /// - `Esc` → Cancel
    pub fn handle_key(&mut self, key: KeyEvent) -> ExistingDbAction {
        match key.code {
            KeyCode::Char('r') => ExistingDbAction::Restore,
            KeyCode::Char('n') => ExistingDbAction::Fresh,
            KeyCode::Esc => ExistingDbAction::Cancel,
            _ => ExistingDbAction::Pending,
        }
    }

    /// Renders the modal centered within `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let modal = centered_rect(60, 40, area);
        frame.render_widget(Clear, modal);

        let lines = vec![
            Line::from(Span::raw("")),
            Line::from(Span::styled(
                format!("  '{}' already exists on disk.", self.db_filename),
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                "  This may be from a previously removed entity.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::raw("")),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "[r]",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Restore existing data"),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "[n]",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Start fresh (deletes old data)"),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "[Esc]",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Cancel"),
            ]),
        ];

        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Database Already Exists ")
                    .style(Style::default().fg(Color::Yellow)),
            ),
            modal,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn r_key_restores() {
        let mut m = ExistingDbModal::new("test.sqlite");
        assert_eq!(
            m.handle_key(key(KeyCode::Char('r'))),
            ExistingDbAction::Restore
        );
    }

    #[test]
    fn n_key_starts_fresh() {
        let mut m = ExistingDbModal::new("test.sqlite");
        assert_eq!(
            m.handle_key(key(KeyCode::Char('n'))),
            ExistingDbAction::Fresh
        );
    }

    #[test]
    fn esc_cancels() {
        let mut m = ExistingDbModal::new("test.sqlite");
        assert_eq!(m.handle_key(key(KeyCode::Esc)), ExistingDbAction::Cancel);
    }

    #[test]
    fn unknown_key_returns_pending() {
        let mut m = ExistingDbModal::new("test.sqlite");
        assert_eq!(
            m.handle_key(key(KeyCode::Char('x'))),
            ExistingDbAction::Pending
        );
    }
}
