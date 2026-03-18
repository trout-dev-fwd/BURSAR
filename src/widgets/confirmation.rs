//! A reusable Yes/No confirmation modal widget.
//!
//! Displays a message with two buttons. Returns `bool` via `ConfirmAction`.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use super::centered_rect;

/// Result of a key event handled by the confirmation widget.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmAction {
    /// User confirmed (y / Enter on Yes).
    Confirmed,
    /// User cancelled (n / Esc).
    Cancelled,
    /// Key consumed; still waiting.
    Pending,
}

/// Focused button in the confirmation dialog.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Focus {
    Yes,
    No,
}

/// A Yes/No confirmation modal.
///
/// Instantiate, call `handle_key`, and render until a non-`Pending` action is returned.
pub struct Confirmation {
    message: String,
    focus: Focus,
}

impl Confirmation {
    /// Creates a new confirmation dialog with `message` and focus defaulting to **No**
    /// (safer default: the user must explicitly choose Yes).
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            focus: Focus::No,
        }
    }

    /// Returns the current message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Handles a key event.
    ///
    /// - `y` or `Enter` while Yes is focused → `Confirmed`
    /// - `n` or `Esc` → `Cancelled`
    /// - `←` / `→` or `Tab` → toggle focus between Yes and No
    /// - `Enter` while No is focused → `Cancelled`
    pub fn handle_key(&mut self, key: KeyEvent) -> ConfirmAction {
        match key.code {
            KeyCode::Char('y') => ConfirmAction::Confirmed,
            KeyCode::Char('n') | KeyCode::Esc => ConfirmAction::Cancelled,
            KeyCode::Enter => match self.focus {
                Focus::Yes => ConfirmAction::Confirmed,
                Focus::No => ConfirmAction::Cancelled,
            },
            KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Yes => Focus::No,
                    Focus::No => Focus::Yes,
                };
                ConfirmAction::Pending
            }
            _ => ConfirmAction::Pending,
        }
    }

    /// Renders the confirmation modal centered within `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let modal = centered_rect(60, 30, area);
        frame.render_widget(Clear, modal);

        let yes_style = if self.focus == Focus::Yes {
            Style::default()
                .fg(Color::White)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };
        let no_style = if self.focus == Focus::No {
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Red)
        };

        let lines = vec![
            Line::from(Span::raw("")),
            Line::from(Span::raw(format!("  {}", self.message))),
            Line::from(Span::raw("")),
            Line::from(vec![
                Span::raw("  "),
                Span::styled("[ Yes ]", yes_style),
                Span::raw("   "),
                Span::styled("[ No ]", no_style),
            ]),
            Line::from(Span::raw("")),
            Line::from(Span::styled(
                "  y: confirm  n/Esc: cancel  ←→/Tab: toggle",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        frame.render_widget(
            Paragraph::new(lines)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Confirm ")
                        .style(Style::default().fg(Color::Yellow)),
                )
                .wrap(Wrap { trim: true }),
            modal,
        );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn y_key_confirms() {
        let mut c = Confirmation::new("Delete this?");
        assert_eq!(
            c.handle_key(key(KeyCode::Char('y'))),
            ConfirmAction::Confirmed
        );
    }

    #[test]
    fn n_key_cancels() {
        let mut c = Confirmation::new("Delete this?");
        assert_eq!(
            c.handle_key(key(KeyCode::Char('n'))),
            ConfirmAction::Cancelled
        );
    }

    #[test]
    fn esc_cancels() {
        let mut c = Confirmation::new("Delete this?");
        assert_eq!(c.handle_key(key(KeyCode::Esc)), ConfirmAction::Cancelled);
    }

    #[test]
    fn enter_on_no_focus_cancels() {
        // Default focus is No.
        let mut c = Confirmation::new("Delete this?");
        assert_eq!(c.handle_key(key(KeyCode::Enter)), ConfirmAction::Cancelled);
    }

    #[test]
    fn enter_on_yes_focus_confirms() {
        let mut c = Confirmation::new("Delete this?");
        // Tab moves focus to Yes.
        c.handle_key(key(KeyCode::Tab));
        assert_eq!(c.handle_key(key(KeyCode::Enter)), ConfirmAction::Confirmed);
    }

    #[test]
    fn tab_toggles_focus() {
        let mut c = Confirmation::new("Are you sure?");
        assert_eq!(c.focus, Focus::No);
        c.handle_key(key(KeyCode::Tab));
        assert_eq!(c.focus, Focus::Yes);
        c.handle_key(key(KeyCode::Tab));
        assert_eq!(c.focus, Focus::No);
    }

    #[test]
    fn left_right_toggle_focus() {
        let mut c = Confirmation::new("Are you sure?");
        c.handle_key(key(KeyCode::Left));
        assert_eq!(c.focus, Focus::Yes);
        c.handle_key(key(KeyCode::Right));
        assert_eq!(c.focus, Focus::No);
    }

    #[test]
    fn unknown_key_returns_pending() {
        let mut c = Confirmation::new("?");
        assert_eq!(c.handle_key(key(KeyCode::F(1))), ConfirmAction::Pending);
    }

    #[test]
    fn message_is_stored() {
        let c = Confirmation::new("Test message");
        assert_eq!(c.message(), "Test message");
    }
}
