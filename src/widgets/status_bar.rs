use std::time::{Duration, Instant};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// Classification of a transient status-bar message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    /// Displayed in red, auto-clears after 5 seconds.
    Error,
    /// Displayed in green, auto-clears after 3 seconds.
    Success,
}

impl MessageKind {
    fn timeout(self) -> Duration {
        match self {
            MessageKind::Error => Duration::from_secs(5),
            MessageKind::Success => Duration::from_secs(3),
        }
    }

    fn color(self) -> Color {
        match self {
            MessageKind::Error => Color::Red,
            MessageKind::Success => Color::Green,
        }
    }
}

/// Displayed at the bottom of the screen.
/// Left: entity name + unsaved indicator. Center: current fiscal period. Right: transient message.
pub struct StatusBar {
    entity_name: String,
    fiscal_period: String,
    message: Option<String>,
    message_kind: MessageKind,
    message_set_at: Option<Instant>,
    /// Whether the active tab has unsaved in-progress changes.
    unsaved: bool,
    /// While an AI request is in flight, shows a loading indicator that takes priority
    /// over normal status messages. Cleared by `set_ai_status(None)`.
    ai_status: Option<String>,
}

impl StatusBar {
    pub fn new(entity_name: String, fiscal_period: String) -> Self {
        Self {
            entity_name,
            fiscal_period,
            message: None,
            message_kind: MessageKind::Success,
            message_set_at: None,
            unsaved: false,
            ai_status: None,
        }
    }

    /// Sets the AI loading message shown while an AI request is in flight.
    /// Pass `None` to clear (restores normal message display).
    pub fn set_ai_status(&mut self, status: Option<String>) {
        self.ai_status = status;
    }

    /// Sets a success message (green, auto-clears after 3 seconds).
    pub fn set_success(&mut self, msg: String) {
        self.message = Some(msg);
        self.message_kind = MessageKind::Success;
        self.message_set_at = Some(Instant::now());
    }

    /// Sets an error message (red, auto-clears after 5 seconds).
    pub fn set_error(&mut self, msg: String) {
        self.message = Some(msg);
        self.message_kind = MessageKind::Error;
        self.message_set_at = Some(Instant::now());
    }

    /// Backwards-compatible helper: displays as a success message.
    pub fn set_message(&mut self, msg: String) {
        self.set_success(msg);
    }

    /// Returns the current message if one is set (before it has expired).
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Returns the kind of the current message.
    pub fn message_kind(&self) -> MessageKind {
        self.message_kind
    }

    /// Update the entity name (e.g., after loading a different entity).
    pub fn set_entity_name(&mut self, name: String) {
        self.entity_name = name;
    }

    /// Update the fiscal period display string.
    pub fn set_fiscal_period(&mut self, period: String) {
        self.fiscal_period = period;
    }

    /// Update the unsaved-changes indicator.
    /// Pass `true` when the active tab has in-progress unsaved content.
    pub fn set_unsaved(&mut self, unsaved: bool) {
        self.unsaved = unsaved;
    }

    /// Called every tick (500ms). Clears the message after its kind-specific timeout.
    pub fn tick(&mut self) {
        if let (Some(msg_time), Some(_msg)) = (self.message_set_at, &self.message)
            && msg_time.elapsed() >= self.message_kind.timeout()
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

        let bg_style = Style::default().bg(Color::DarkGray).fg(Color::White);

        // Left: entity name + unsaved indicator + help hint.
        let unsaved_marker = if self.unsaved { " [*]" } else { "" };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(format!(" {}{}", self.entity_name, unsaved_marker)),
                Span::styled("  │  ? help", Style::default().fg(Color::Gray)),
            ]))
            .style(bg_style),
            chunks[0],
        );

        // Center: fiscal period.
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::raw(self.fiscal_period.clone())]))
                .style(bg_style)
                .centered(),
            chunks[1],
        );

        // Right: AI loading status takes priority; falls back to normal transient message.
        let (msg_text, msg_style) = if let Some(ai_msg) = &self.ai_status {
            (
                ai_msg.clone(),
                Style::default().bg(Color::DarkGray).fg(Color::Green),
            )
        } else {
            let text = self.message.clone().unwrap_or_default();
            let style = if self.message.is_some() {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(self.message_kind.color())
            } else {
                bg_style
            };
            (text, style)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("{msg_text} "),
                msg_style,
            )]))
            .style(bg_style)
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
        // Force expiry by backdating the message timestamp.
        bar.message_set_at = Some(
            Instant::now()
                .checked_sub(Duration::from_secs(10))
                .unwrap_or_else(Instant::now),
        );
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
        // Default timeout is 3 seconds (success); a single tick should not clear it.
        bar.tick();
        assert_eq!(bar.message(), Some("Still here"));
    }

    #[test]
    fn error_message_uses_red_kind() {
        let mut bar = StatusBar::new("Test".to_owned(), "P1".to_owned());
        bar.set_error("Bad thing happened".to_owned());
        assert_eq!(bar.message(), Some("Bad thing happened"));
        assert_eq!(bar.message_kind(), MessageKind::Error);
    }

    #[test]
    fn success_message_uses_green_kind() {
        let mut bar = StatusBar::new("Test".to_owned(), "P1".to_owned());
        bar.set_success("Done!".to_owned());
        assert_eq!(bar.message_kind(), MessageKind::Success);
    }

    #[test]
    fn unsaved_indicator_toggled() {
        let mut bar = StatusBar::new("Entity".to_owned(), "P1".to_owned());
        assert!(!bar.unsaved);
        bar.set_unsaved(true);
        assert!(bar.unsaved);
        bar.set_unsaved(false);
        assert!(!bar.unsaved);
    }
}
