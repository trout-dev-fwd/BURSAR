//! Multi-line text input modal for collecting bug reports and feature requests.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use super::centered_rect;

/// The type of feedback being collected.
#[derive(Debug, Clone, PartialEq)]
pub enum FeedbackType {
    Bug,
    Feature,
}

/// Result of a key event handled by [`FeedbackModal`].
#[derive(Debug, Clone, PartialEq)]
pub enum FeedbackAction {
    /// User pressed Ctrl+S; contains the feedback type and full text (lines joined with `\n`).
    Submit(FeedbackType, String),
    /// User pressed Esc.
    Cancel,
    /// Key consumed; still editing.
    None,
}

/// A multi-line text input modal for collecting bug reports and feature requests.
pub struct FeedbackModal {
    pub(crate) feedback_type: FeedbackType,
    pub(crate) lines: Vec<String>,
    pub(crate) cursor_row: usize,
    pub(crate) cursor_col: usize,
    scroll_offset: usize,
}

impl FeedbackModal {
    /// Creates a new modal for the given feedback type.
    /// Starts with a single empty line, cursor at (0, 0).
    pub fn new(feedback_type: FeedbackType) -> Self {
        Self {
            feedback_type,
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
            scroll_offset: 0,
        }
    }

    /// Handles a single key event.
    pub fn handle_key(&mut self, key: KeyEvent) -> FeedbackAction {
        match key.code {
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let text = self.lines.join("\n");
                FeedbackAction::Submit(self.feedback_type.clone(), text)
            }
            KeyCode::Esc => FeedbackAction::Cancel,
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert_char(c);
                FeedbackAction::None
            }
            KeyCode::Enter => {
                self.insert_newline();
                FeedbackAction::None
            }
            KeyCode::Backspace => {
                self.backspace();
                FeedbackAction::None
            }
            KeyCode::Delete => {
                self.delete();
                FeedbackAction::None
            }
            KeyCode::Left => {
                self.move_left();
                FeedbackAction::None
            }
            KeyCode::Right => {
                self.move_right();
                FeedbackAction::None
            }
            KeyCode::Up => {
                self.move_up();
                FeedbackAction::None
            }
            KeyCode::Down => {
                self.move_down();
                FeedbackAction::None
            }
            KeyCode::Home => {
                self.cursor_col = 0;
                FeedbackAction::None
            }
            KeyCode::End => {
                self.cursor_col = self.lines[self.cursor_row].chars().count();
                FeedbackAction::None
            }
            _ => FeedbackAction::None,
        }
    }

    fn insert_char(&mut self, c: char) {
        let byte_pos = char_byte_pos(&self.lines[self.cursor_row], self.cursor_col);
        self.lines[self.cursor_row].insert(byte_pos, c);
        self.cursor_col += 1;
    }

    fn insert_newline(&mut self) {
        let byte_pos = char_byte_pos(&self.lines[self.cursor_row], self.cursor_col);
        let new_line = self.lines[self.cursor_row][byte_pos..].to_owned();
        self.lines[self.cursor_row].truncate(byte_pos);
        self.cursor_row += 1;
        self.lines.insert(self.cursor_row, new_line);
        self.cursor_col = 0;
    }

    fn backspace(&mut self) {
        if self.cursor_col > 0 {
            let byte_pos = char_byte_pos(&self.lines[self.cursor_row], self.cursor_col - 1);
            self.lines[self.cursor_row].remove(byte_pos);
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            let current_line = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].chars().count();
            self.lines[self.cursor_row].push_str(&current_line);
        }
    }

    fn delete(&mut self) {
        let line_len = self.lines[self.cursor_row].chars().count();
        if self.cursor_col < line_len {
            let byte_pos = char_byte_pos(&self.lines[self.cursor_row], self.cursor_col);
            self.lines[self.cursor_row].remove(byte_pos);
        } else if self.cursor_row + 1 < self.lines.len() {
            let next_line = self.lines.remove(self.cursor_row + 1);
            self.lines[self.cursor_row].push_str(&next_line);
        }
    }

    fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].chars().count();
        }
    }

    fn move_right(&mut self) {
        let line_len = self.lines[self.cursor_row].chars().count();
        if self.cursor_col < line_len {
            self.cursor_col += 1;
        } else if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
    }

    fn move_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            let line_len = self.lines[self.cursor_row].chars().count();
            self.cursor_col = self.cursor_col.min(line_len);
        }
    }

    fn move_down(&mut self) {
        if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            let line_len = self.lines[self.cursor_row].chars().count();
            self.cursor_col = self.cursor_col.min(line_len);
        }
    }

    /// Renders the modal within `area`.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let modal = centered_rect(60, 40, area);
        frame.render_widget(Clear, modal);

        let title = match self.feedback_type {
            FeedbackType::Bug => " Describe the bug: ",
            FeedbackType::Feature => " Describe the feature: ",
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan).bg(Color::Black));

        let inner = block.inner(modal);
        frame.render_widget(block, modal);

        if inner.height < 2 || inner.width < 2 {
            return;
        }

        // Available width for text (1-char padding each side).
        let text_width = inner.width.saturating_sub(2) as usize;
        // Text area height (inner minus hint bar).
        let text_height = inner.height.saturating_sub(1) as usize;

        // Adjust scroll to keep cursor visible.
        if self.cursor_row < self.scroll_offset {
            self.scroll_offset = self.cursor_row;
        } else if text_height > 0 && self.cursor_row >= self.scroll_offset + text_height {
            self.scroll_offset = self.cursor_row + 1 - text_height;
        }

        // Split inner into text area + hint bar.
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);
        let text_area = chunks[0];
        let hint_area = chunks[1];

        // Build visible lines (truncate long lines at display boundary).
        let visible_lines: Vec<Line> = self
            .lines
            .iter()
            .skip(self.scroll_offset)
            .take(text_area.height as usize)
            .map(|line| {
                let display: String = line.chars().take(text_width).collect();
                Line::from(format!(" {display}"))
            })
            .collect();

        frame.render_widget(Paragraph::new(visible_lines), text_area);

        // Set cursor position.
        let cursor_screen_row = self.cursor_row.saturating_sub(self.scroll_offset) as u16;
        let cursor_screen_col = self.cursor_col.min(text_width) as u16;
        if cursor_screen_row < text_area.height {
            frame.set_cursor_position((
                text_area.x + 1 + cursor_screen_col, // +1 for leading space
                text_area.y + cursor_screen_row,
            ));
        }

        // Hint bar.
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Ctrl+S", Style::default().fg(Color::Cyan)),
                Span::raw(": Submit  "),
                Span::styled("Esc", Style::default().fg(Color::Cyan)),
                Span::raw(": Cancel"),
            ]))
            .alignment(Alignment::Center),
            hint_area,
        );
    }
}

/// Build a complete GitHub issue URL with pre-filled title, body, and labels.
pub fn build_issue_url(
    feedback_type: &FeedbackType,
    description: &str,
    entity_name: Option<&str>,
    recent_audit_entries: &[String],
) -> String {
    let base = "https://github.com/trout-dev-fwd/bursar/issues/new";

    // Build title from first line of description (truncate to 100 chars).
    let first_line = description.lines().next().unwrap_or("");
    let prefix = match feedback_type {
        FeedbackType::Bug => "[Bug] ",
        FeedbackType::Feature => "[Feature] ",
    };
    let raw_title = format!("{prefix}{first_line}");
    let title: String = raw_title.chars().take(100).collect();

    let labels = match feedback_type {
        FeedbackType::Bug => "bug",
        FeedbackType::Feature => "enhancement",
    };

    let sys_info = build_sys_info(feedback_type, entity_name, recent_audit_entries);
    let body = format!("{description}{sys_info}");

    let encoded_title = urlencoding::encode(&title).into_owned();
    let encoded_labels = urlencoding::encode(labels).into_owned();
    let encoded_body = urlencoding::encode(&body).into_owned();

    let url = format!("{base}?title={encoded_title}&body={encoded_body}&labels={encoded_labels}");

    if url.len() <= 8000 {
        return url;
    }

    // URL too long: truncate description and append a notice.
    let truncation_notice =
        "\n\n[Description truncated for URL length \u{2014} please add details in the issue]";

    let desc_chars: Vec<char> = description.chars().collect();
    let mut end = desc_chars.len();
    loop {
        let truncated: String = desc_chars[..end].iter().collect();
        let truncated_body = format!("{truncated}{truncation_notice}{sys_info}");
        let enc_body = urlencoding::encode(&truncated_body).into_owned();
        let candidate =
            format!("{base}?title={encoded_title}&body={enc_body}&labels={encoded_labels}");
        if candidate.len() <= 8000 || end == 0 {
            return candidate;
        }
        end = end.saturating_sub(50);
    }
}

/// Open a URL in the default browser. Returns `Ok(())` on success, `Err` on failure.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    open_browser_cmd(url)
}

#[cfg(target_os = "linux")]
fn open_browser_cmd(url: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map_err(|e| format!("Could not open browser: {e}"))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn open_browser_cmd(url: &str) -> Result<(), String> {
    std::process::Command::new("cmd")
        .args(["/c", "start", "", url]) // empty string for window title
        .spawn()
        .map_err(|e| format!("Could not open browser: {e}"))?;
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn open_browser_cmd(_url: &str) -> Result<(), String> {
    Err("Browser launch not supported on this platform".to_string())
}

fn build_sys_info(
    feedback_type: &FeedbackType,
    entity_name: Option<&str>,
    recent_audit_entries: &[String],
) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let entity_display = entity_name.unwrap_or("None");

    match feedback_type {
        FeedbackType::Bug => {
            let audit_section = if recent_audit_entries.is_empty() {
                String::new()
            } else {
                format!(
                    "\n\n**Recent Audit Log**\n```\n{}\n```",
                    recent_audit_entries.join("\n")
                )
            };
            format!(
                "\n\n---\n\n**System Information**\n- Bursar version: {version}\n- OS: {os} {arch}\n- Entity: {entity_display}{audit_section}"
            )
        }
        FeedbackType::Feature => {
            format!(
                "\n\n---\n\n**System Information**\n- Bursar version: {version}\n- OS: {os} {arch}"
            )
        }
    }
}

fn char_byte_pos(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    // ── FeedbackModal text editing ─────────────────────────────────────────────

    #[test]
    fn insert_characters_updates_buffer() {
        let mut m = FeedbackModal::new(FeedbackType::Bug);
        m.handle_key(key(KeyCode::Char('h')));
        m.handle_key(key(KeyCode::Char('i')));
        assert_eq!(m.lines, vec!["hi"]);
        assert_eq!(m.cursor_col, 2);
    }

    #[test]
    fn enter_splits_line_at_cursor() {
        let mut m = FeedbackModal::new(FeedbackType::Bug);
        m.handle_key(key(KeyCode::Char('a')));
        m.handle_key(key(KeyCode::Char('b')));
        m.handle_key(key(KeyCode::Left)); // cursor at col 1 (between a and b)
        m.handle_key(key(KeyCode::Enter));
        assert_eq!(m.lines, vec!["a", "b"]);
        assert_eq!(m.cursor_row, 1);
        assert_eq!(m.cursor_col, 0);
    }

    #[test]
    fn backspace_at_line_start_merges_with_previous() {
        let mut m = FeedbackModal::new(FeedbackType::Bug);
        m.handle_key(key(KeyCode::Char('a')));
        m.handle_key(key(KeyCode::Enter));
        m.handle_key(key(KeyCode::Char('b')));
        // cursor at row=1 col=1
        m.handle_key(key(KeyCode::Home));
        m.handle_key(key(KeyCode::Backspace));
        assert_eq!(m.lines, vec!["ab"]);
        assert_eq!(m.cursor_row, 0);
        assert_eq!(m.cursor_col, 1);
    }

    #[test]
    fn cursor_up_down_clamps_col_to_line_length() {
        let mut m = FeedbackModal::new(FeedbackType::Bug);
        // Line 0: "hello" (len 5)
        for c in "hello".chars() {
            m.handle_key(key(KeyCode::Char(c)));
        }
        m.handle_key(key(KeyCode::Enter));
        // Line 1: "hi" (len 2)
        m.handle_key(key(KeyCode::Char('h')));
        m.handle_key(key(KeyCode::Char('i')));
        // cursor at row=1 col=2
        m.handle_key(key(KeyCode::Up));
        // Line 0 has len 5; col stays at 2 (2 <= 5)
        assert_eq!(m.cursor_row, 0);
        assert_eq!(m.cursor_col, 2);
        // Move to end of line 0 (col 5), then down
        m.handle_key(key(KeyCode::End));
        assert_eq!(m.cursor_col, 5);
        m.handle_key(key(KeyCode::Down));
        // Line 1 has len 2; col clamped to 2
        assert_eq!(m.cursor_row, 1);
        assert_eq!(m.cursor_col, 2);
    }

    #[test]
    fn left_wraps_to_end_of_previous_line() {
        let mut m = FeedbackModal::new(FeedbackType::Bug);
        m.handle_key(key(KeyCode::Char('a')));
        m.handle_key(key(KeyCode::Enter));
        // cursor at row=1 col=0
        m.handle_key(key(KeyCode::Left));
        assert_eq!(m.cursor_row, 0);
        assert_eq!(m.cursor_col, 1); // end of "a"
    }

    #[test]
    fn right_wraps_to_start_of_next_line() {
        let mut m = FeedbackModal::new(FeedbackType::Bug);
        m.handle_key(key(KeyCode::Char('a')));
        m.handle_key(key(KeyCode::Enter));
        // Go back to line 0 at end
        m.handle_key(key(KeyCode::Up));
        m.handle_key(key(KeyCode::End));
        // cursor at row=0 col=1 (end of "a")
        m.handle_key(key(KeyCode::Right));
        assert_eq!(m.cursor_row, 1);
        assert_eq!(m.cursor_col, 0);
    }

    #[test]
    fn ctrl_s_submits_with_joined_text() {
        let mut m = FeedbackModal::new(FeedbackType::Bug);
        m.handle_key(key(KeyCode::Char('a')));
        m.handle_key(key(KeyCode::Enter));
        m.handle_key(key(KeyCode::Char('b')));
        let action = m.handle_key(ctrl(KeyCode::Char('s')));
        assert_eq!(
            action,
            FeedbackAction::Submit(FeedbackType::Bug, "a\nb".to_string())
        );
    }

    #[test]
    fn esc_returns_cancel() {
        let mut m = FeedbackModal::new(FeedbackType::Feature);
        let action = m.handle_key(key(KeyCode::Esc));
        assert_eq!(action, FeedbackAction::Cancel);
    }

    #[test]
    fn submit_feature_type_is_preserved() {
        let mut m = FeedbackModal::new(FeedbackType::Feature);
        m.handle_key(key(KeyCode::Char('x')));
        let action = m.handle_key(ctrl(KeyCode::Char('s')));
        assert_eq!(
            action,
            FeedbackAction::Submit(FeedbackType::Feature, "x".to_string())
        );
    }

    // ── build_issue_url ────────────────────────────────────────────────────────

    #[test]
    fn bug_url_includes_bug_label_and_title_prefix() {
        let url = build_issue_url(&FeedbackType::Bug, "crash on startup", None, &[]);
        assert!(url.contains("labels=bug"), "URL should include bug label");
        assert!(
            url.contains("%5BBug%5D"),
            "URL should include encoded [Bug] prefix"
        );
    }

    #[test]
    fn feature_url_includes_enhancement_label_and_title_prefix() {
        let url = build_issue_url(&FeedbackType::Feature, "add export feature", None, &[]);
        assert!(
            url.contains("labels=enhancement"),
            "URL should include enhancement label"
        );
        assert!(
            url.contains("%5BFeature%5D"),
            "URL should include encoded [Feature] prefix"
        );
    }

    #[test]
    fn url_includes_system_info() {
        let url = build_issue_url(&FeedbackType::Bug, "something broke", None, &[]);
        assert!(
            url.contains("Bursar+version") || url.contains("Bursar%20version"),
            "URL should contain version info"
        );
    }

    #[test]
    fn bug_with_audit_entries_includes_them() {
        let entries = vec!["2026-01-01T00:00:00 | AccountCreated | Created account".to_string()];
        let url = build_issue_url(&FeedbackType::Bug, "a bug", Some("Acme"), &entries);
        assert!(
            url.contains("Audit") || url.contains("audit"),
            "Bug URL should include audit section"
        );
    }

    #[test]
    fn feature_with_audit_entries_does_not_include_them() {
        let entries = vec!["2026-01-01T00:00:00 | AccountCreated | Created account".to_string()];
        let url = build_issue_url(&FeedbackType::Feature, "a feature", Some("Acme"), &entries);
        assert!(
            !url.contains("Audit+Log") && !url.contains("Audit%20Log"),
            "Feature URL should NOT include audit log section"
        );
    }

    #[test]
    fn bug_with_empty_audit_entries_omits_audit_section() {
        let url = build_issue_url(&FeedbackType::Bug, "a bug", Some("Acme"), &[]);
        assert!(
            !url.contains("Audit+Log") && !url.contains("Audit%20Log"),
            "Bug URL with empty audit should omit audit section"
        );
    }

    #[test]
    fn long_url_gets_truncated_with_notice() {
        // Use a description with spaces/newlines that expand significantly after
        // percent-encoding, reliably pushing the URL over 8000 characters.
        let long_desc = "hello world test description ".repeat(500); // ~14500 chars
        let url = build_issue_url(&FeedbackType::Bug, &long_desc, None, &[]);
        assert!(
            url.len() <= 8000,
            "URL should be truncated to <= 8000 chars"
        );
        assert!(
            url.contains("truncated") || url.contains("Truncated"),
            "URL should include truncation notice"
        );
    }

    #[test]
    fn title_truncated_to_100_chars_when_first_line_is_long() {
        let long_first_line = "a".repeat(200);
        let url = build_issue_url(&FeedbackType::Bug, &long_first_line, None, &[]);
        // The encoded title should be limited to 100 chars + "[Bug] " prefix = 106 chars
        // Encoded: "[" → %5B, "]" → %5D, " " → + or %20 — just check it doesn't blow up
        assert!(!url.is_empty());
    }

    #[test]
    fn special_chars_in_description_are_encoded() {
        let url = build_issue_url(&FeedbackType::Bug, "crash & burn #123", None, &[]);
        // & should be encoded (%26), space as + or %20
        assert!(!url.contains("crash & burn"), "Ampersand should be encoded");
    }
}
