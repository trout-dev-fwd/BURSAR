use std::time::{Duration, Instant};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// Displayed at the bottom of the screen.
/// Left: entity name. Center: current fiscal period. Right: transient message.
pub struct StatusBar {
    entity_name: String,
    fiscal_period: String,
    message: Option<String>,
    message_set_at: Option<Instant>,
    message_timeout: Duration,
}

impl StatusBar {
    pub fn new(entity_name: String, fiscal_period: String) -> Self {
        Self {
            entity_name,
            fiscal_period,
            message: None,
            message_set_at: None,
            message_timeout: Duration::from_secs(5),
        }
    }

    /// Sets the transient message displayed on the right side.
    pub fn set_message(&mut self, msg: String) {
        self.message = Some(msg);
        self.message_set_at = Some(Instant::now());
    }

    /// Returns the current message if one is set (before it has expired).
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Update the entity name (e.g., after loading a different entity).
    pub fn set_entity_name(&mut self, name: String) {
        self.entity_name = name;
    }

    /// Update the fiscal period display string.
    pub fn set_fiscal_period(&mut self, period: String) {
        self.fiscal_period = period;
    }

    /// Called every tick (500ms). Clears the message after the 5-second timeout.
    pub fn tick(&mut self) {
        if let (Some(msg_time), Some(_msg)) = (self.message_set_at, &self.message)
            && msg_time.elapsed() >= self.message_timeout
        {
            self.message = None;
            self.message_set_at = None;
        }
    }

    /// Renders the status bar into the given area.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(33),
                Constraint::Percentage(34),
                Constraint::Percentage(33),
            ])
            .split(area);

        let style = Style::default().bg(Color::DarkGray).fg(Color::White);

        // Left: entity name
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::raw(format!(
                " {}",
                self.entity_name
            ))]))
            .style(style),
            chunks[0],
        );

        // Center: fiscal period
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::raw(self.fiscal_period.clone())]))
                .style(style)
                .centered(),
            chunks[1],
        );

        // Right: message or empty
        let msg_text = self.message.clone().unwrap_or_default();
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::raw(format!("{msg_text} "))]))
                .style(style)
                .right_aligned(),
            chunks[2],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_is_stored_after_set() {
        let mut bar = StatusBar::new("Test Entity".to_owned(), "Period 1".to_owned());
        bar.set_message("Hello!".to_owned());
        assert_eq!(bar.message(), Some("Hello!"));
    }

    #[test]
    fn message_is_none_initially() {
        let bar = StatusBar::new("Test".to_owned(), "P1".to_owned());
        assert!(bar.message().is_none());
    }

    #[test]
    fn tick_clears_message_after_timeout() {
        let mut bar = StatusBar::new("Test".to_owned(), "P1".to_owned());
        bar.set_message("Temporary".to_owned());

        // Simulate enough ticks (override timeout to 0 via direct field manipulation).
        // We can't easily fast-forward Instant, so set timeout to near-zero.
        bar.message_timeout = Duration::from_nanos(1);
        std::thread::sleep(Duration::from_millis(1));
        bar.tick();

        assert!(
            bar.message().is_none(),
            "Message should be cleared after timeout"
        );
    }

    #[test]
    fn tick_preserves_message_before_timeout() {
        let mut bar = StatusBar::new("Test".to_owned(), "P1".to_owned());
        bar.set_message("Still here".to_owned());
        // Default timeout is 5 seconds; a single tick should not clear it.
        bar.tick();
        assert_eq!(bar.message(), Some("Still here"));
    }
}
