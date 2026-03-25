//! A reusable single-line text input modal widget.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use super::centered_rect;

/// Result of a key event handled by [`TextInputModal`].
#[derive(Debug, Clone, PartialEq)]
pub enum TextInputAction {
    /// User pressed Enter; contains the current buffer contents.
    Confirm(String),
    /// User pressed Esc.
    Cancel,
    /// Key consumed; still editing.
    None,
}

/// A centered single-line text input modal.
///
/// Instantiate with [`TextInputModal::new`], dispatch keys via [`handle_key`],
/// and render via [`render`] until a non-[`None`] action is returned.
pub struct TextInputModal {
    title: String,
    buffer: String,
    cursor_pos: usize,
}

impl TextInputModal {
    /// Creates a new modal with the given title and pre-filled text.
    pub fn new(title: impl Into<String>, prefill: impl Into<String>) -> Self {
        let buffer = prefill.into();
        let cursor_pos = buffer.chars().count();
        Self {
            title: title.into(),
            buffer,
            cursor_pos,
        }
    }

    /// Handles a single key event.
    pub fn handle_key(&mut self, key: KeyEvent) -> TextInputAction {
        match key.code {
            KeyCode::Enter => TextInputAction::Confirm(self.buffer.clone()),
            KeyCode::Esc => TextInputAction::Cancel,

            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                }
                TextInputAction::None
            }
            KeyCode::Right => {
                if self.cursor_pos < self.buffer.chars().count() {
                    self.cursor_pos += 1;
                }
                TextInputAction::None
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
                TextInputAction::None
            }
            KeyCode::End => {
                self.cursor_pos = self.buffer.chars().count();
                TextInputAction::None
            }

            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    // Remove the character just before the cursor.
                    let byte_pos = self
                        .buffer
                        .char_indices()
                        .nth(self.cursor_pos - 1)
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.buffer.remove(byte_pos);
                    self.cursor_pos -= 1;
                }
                TextInputAction::None
            }
            KeyCode::Delete => {
                if self.cursor_pos < self.buffer.chars().count() {
                    let byte_pos = self
                        .buffer
                        .char_indices()
                        .nth(self.cursor_pos)
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.buffer.remove(byte_pos);
                }
                TextInputAction::None
            }

            KeyCode::Char(c) => {
                // Insert printable character at cursor position.
                let byte_pos = self
                    .buffer
                    .char_indices()
                    .nth(self.cursor_pos)
                    .map(|(i, _)| i)
                    .unwrap_or(self.buffer.len());
                self.buffer.insert(byte_pos, c);
                self.cursor_pos += 1;
                TextInputAction::None
            }

            _ => TextInputAction::None,
        }
    }

    /// Renders the modal centered within `area`.
    ///
    /// Uses horizontal scrolling so that long text always shows the region around
    /// the cursor, keeping the cursor visible regardless of buffer length.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let modal = centered_rect(80, 30, area);
        frame.render_widget(Clear, modal);

        // Inner content width: modal minus 2 border cols minus 2 padding spaces.
        let inner_width = modal.width.saturating_sub(4) as usize;

        // Calculate scroll offset so the cursor stays within the visible window.
        let scroll = if inner_width > 0 && self.cursor_pos >= inner_width {
            self.cursor_pos - inner_width + 1
        } else {
            0
        };

        // Build visible char slices.
        let chars: Vec<char> = self.buffer.chars().collect();
        let char_count = chars.len();

        let visible_before: String = chars
            .get(scroll..self.cursor_pos.min(char_count))
            .unwrap_or(&[])
            .iter()
            .collect();
        let cursor_char: String = chars
            .get(self.cursor_pos)
            .map(|c| c.to_string())
            .unwrap_or_else(|| " ".to_owned());
        let after_start = self.cursor_pos + 1;
        let after_end = (scroll + inner_width).min(char_count);
        let visible_after: String = if after_start <= after_end {
            chars[after_start..after_end].iter().collect()
        } else {
            String::new()
        };

        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}:", self.title),
                Style::default().fg(Color::White),
            )),
            Line::from(""),
            Line::from(vec![
                Span::raw("  "),
                Span::raw(visible_before),
                Span::styled(
                    cursor_char,
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(visible_after),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  Enter: confirm  Esc: cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", self.title))
                    .style(Style::default().fg(Color::Cyan)),
            ),
            modal,
        );
    }

    /// Returns the current buffer contents (for testing).
    #[cfg(test)]
    pub(crate) fn buffer(&self) -> &str {
        &self.buffer
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
    fn enter_confirms_buffer() {
        let mut m = TextInputModal::new("Title", "hello");
        assert_eq!(
            m.handle_key(key(KeyCode::Enter)),
            TextInputAction::Confirm("hello".to_string())
        );
    }

    #[test]
    fn esc_cancels() {
        let mut m = TextInputModal::new("Title", "hello");
        assert_eq!(m.handle_key(key(KeyCode::Esc)), TextInputAction::Cancel);
    }

    #[test]
    fn typing_appends_to_buffer() {
        let mut m = TextInputModal::new("Title", "");
        m.handle_key(key(KeyCode::Char('a')));
        m.handle_key(key(KeyCode::Char('b')));
        m.handle_key(key(KeyCode::Char('c')));
        assert_eq!(
            m.handle_key(key(KeyCode::Enter)),
            TextInputAction::Confirm("abc".to_string())
        );
    }

    #[test]
    fn backspace_removes_last_char() {
        let mut m = TextInputModal::new("Title", "hello");
        m.handle_key(key(KeyCode::Backspace));
        assert_eq!(
            m.handle_key(key(KeyCode::Enter)),
            TextInputAction::Confirm("hell".to_string())
        );
    }

    #[test]
    fn left_right_move_cursor() {
        let mut m = TextInputModal::new("Title", "ab");
        // Cursor starts at end (pos 2).
        m.handle_key(key(KeyCode::Left));
        // Now at pos 1; insert 'X' between a and b.
        m.handle_key(key(KeyCode::Char('X')));
        assert_eq!(
            m.handle_key(key(KeyCode::Enter)),
            TextInputAction::Confirm("aXb".to_string())
        );
    }

    #[test]
    fn home_moves_cursor_to_start() {
        let mut m = TextInputModal::new("Title", "abc");
        m.handle_key(key(KeyCode::Home));
        m.handle_key(key(KeyCode::Char('Z')));
        assert_eq!(
            m.handle_key(key(KeyCode::Enter)),
            TextInputAction::Confirm("Zabc".to_string())
        );
    }

    #[test]
    fn delete_removes_char_at_cursor() {
        let mut m = TextInputModal::new("Title", "abc");
        m.handle_key(key(KeyCode::Home));
        m.handle_key(key(KeyCode::Delete));
        assert_eq!(
            m.handle_key(key(KeyCode::Enter)),
            TextInputAction::Confirm("bc".to_string())
        );
    }

    #[test]
    fn prefill_cursor_at_end() {
        let mut m = TextInputModal::new("Title", "hello");
        // Cursor is at end; backspace should remove 'o'.
        m.handle_key(key(KeyCode::Backspace));
        assert_eq!(
            m.handle_key(key(KeyCode::Enter)),
            TextInputAction::Confirm("hell".to_string())
        );
    }

    #[test]
    fn unicode_backspace_removes_last_char() {
        let mut m = TextInputModal::new("Test", "José");
        // Cursor at char pos 4. Backspace should remove 'é', not 'J'.
        m.handle_key(key(KeyCode::Backspace));
        assert_eq!(m.buffer(), "Jos");
    }

    #[test]
    fn unicode_insert_mid_string() {
        let mut m = TextInputModal::new("Test", "café");
        // Move left twice: cursor at pos 2 (between 'a' and 'f').
        m.handle_key(key(KeyCode::Left));
        m.handle_key(key(KeyCode::Left));
        m.handle_key(key(KeyCode::Char('X')));
        assert_eq!(m.buffer(), "caXfé");
    }

    #[test]
    fn unicode_navigation_end_and_right() {
        let mut m = TextInputModal::new("Test", "aé");
        // Home, then Right twice should land at end (2 chars).
        m.handle_key(key(KeyCode::Home));
        m.handle_key(key(KeyCode::Right));
        m.handle_key(key(KeyCode::Right));
        // Right again should be a no-op (already at end).
        m.handle_key(key(KeyCode::Right));
        // Backspace should remove 'é', not 'a'.
        m.handle_key(key(KeyCode::Backspace));
        assert_eq!(m.buffer(), "a");
    }

    #[test]
    fn unicode_delete_at_cursor() {
        let mut m = TextInputModal::new("Test", "café");
        m.handle_key(key(KeyCode::Home));
        // Delete 'c'.
        m.handle_key(key(KeyCode::Delete));
        assert_eq!(m.buffer(), "afé");
    }

    #[test]
    fn unicode_end_key() {
        let mut m = TextInputModal::new("Test", "José");
        m.handle_key(key(KeyCode::Home));
        m.handle_key(key(KeyCode::End));
        // Now at end; insert should append.
        m.handle_key(key(KeyCode::Char('!')));
        assert_eq!(m.buffer(), "José!");
    }
}
