use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::ai::{ApiContent, ApiMessage, ApiRole, ChatMessage, TypewriterState};
use crate::types::ChatRole;

// ── Action types ──────────────────────────────────────────────────────────────

/// Actions that the chat panel requests from `App`.
#[derive(Debug)]
pub enum ChatAction {
    None,
    SendMessage(Vec<ApiMessage>),
    SlashCommand(SlashCommand),
    Close,
    SkipTypewriter,
}

/// Parsed slash commands entered in the chat panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    Clear,
    Context,
    Compact,
    Persona(Option<String>), // Some(text) or None if no arg
    Match,
    Unknown(String),
}

impl SlashCommand {
    /// Parse a `/command [args]` string.
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim_start_matches('/').trim();
        let (cmd, rest) = trimmed
            .split_once(char::is_whitespace)
            .map(|(c, r)| (c, Some(r.trim())))
            .unwrap_or((trimmed, None));
        match cmd {
            "clear" => SlashCommand::Clear,
            "context" => SlashCommand::Context,
            "compact" => SlashCommand::Compact,
            "persona" => {
                SlashCommand::Persona(rest.filter(|s| !s.is_empty()).map(|s| s.to_string()))
            }
            "match" => SlashCommand::Match,
            other => SlashCommand::Unknown(other.to_string()),
        }
    }
}

// ── ChatPanel struct ───────────────────────────────────────────────────────────

/// AI chat panel widget.  Does not own an `AiClient` and makes no API calls.
/// Returns `ChatAction` values for `App` to handle.
pub struct ChatPanel {
    pub messages: Vec<ChatMessage>,
    pub input_buffer: String,
    pub cursor_pos: usize,
    pub scroll_offset: usize,
    pub system_prompt: String,
    pub is_visible: bool,
    pub typewriter: Option<TypewriterState>,
    pub entity_name: String,
    pub current_persona: String,
}

impl ChatPanel {
    pub fn new(entity_name: &str, persona: &str) -> Self {
        Self {
            messages: Vec::new(),
            input_buffer: String::new(),
            cursor_pos: 0,
            scroll_offset: 0,
            system_prompt: String::new(),
            is_visible: false,
            typewriter: None,
            entity_name: entity_name.to_string(),
            current_persona: persona.to_string(),
        }
    }

    /// Populate the welcome / help message shown on first open.
    pub fn build_welcome(&mut self) {
        self.messages.push(ChatMessage {
            role: ChatRole::System,
            content: format!(
                "AI Accountant — {}\n\
                 Persona: {}\n\n\
                 Ask any accounting question. Available commands:\n\
                 /clear   Reset conversation\n\
                 /context Refresh entity context\n\
                 /compact Summarise history\n\
                 /persona [text] View or update persona\n\
                 /match   Re-match selected draft",
                self.entity_name, self.current_persona
            ),
            is_fully_rendered: true,
        });
    }

    /// Toggle panel visibility. Returns new visibility state.
    pub fn toggle_visible(&mut self) -> bool {
        self.is_visible = !self.is_visible;
        if self.is_visible && self.messages.is_empty() {
            self.build_welcome();
        }
        self.is_visible
    }

    pub fn is_visible(&self) -> bool {
        self.is_visible
    }

    /// Advance the typewriter animation by 20 characters (char-boundary aligned).
    pub fn tick(&mut self) {
        let Some(tw) = self.typewriter.as_mut() else {
            return;
        };
        let full_len = tw.full_text.len();
        if tw.display_position >= full_len {
            // Animation complete — mark the message fully rendered.
            if let Some(msg) = self.messages.get_mut(tw.message_index) {
                msg.is_fully_rendered = true;
            }
            self.typewriter = None;
            return;
        }
        // Advance by up to 20 chars, staying on a char boundary.
        let target = (tw.display_position + 20).min(full_len);
        let mut pos = target;
        while pos > 0 && !tw.full_text.is_char_boundary(pos) {
            pos -= 1;
        }
        tw.display_position = pos;
        // If we've reached the end, finalize immediately.
        if pos >= full_len {
            let idx = tw.message_index;
            self.typewriter = None;
            if let Some(msg) = self.messages.get_mut(idx) {
                msg.is_fully_rendered = true;
            }
        }
    }

    // ── Rendering ─────────────────────────────────────────────────────────────

    pub fn render(&self, frame: &mut Frame, area: Rect, is_focused: bool) {
        let border_style = if is_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Span::styled(
                format!(" AI Accountant — {} ", self.entity_name),
                if is_focused {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                },
            ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height < 3 {
            // Too small to render content.
            return;
        }

        // Reserve 1 line for input at the bottom.
        let msg_height = inner.height.saturating_sub(1) as usize;
        let msg_area = Rect {
            y: inner.y,
            height: inner.height.saturating_sub(1),
            ..inner
        };
        let input_area = Rect {
            y: inner.y + inner.height.saturating_sub(1),
            height: 1,
            ..inner
        };

        // Build lines from messages.
        let mut all_lines: Vec<Line<'static>> = Vec::new();
        for (idx, msg) in self.messages.iter().enumerate() {
            let content: &str = if let Some(tw) = &self.typewriter {
                if tw.message_index == idx {
                    &tw.full_text[..tw.display_position]
                } else {
                    &msg.content
                }
            } else {
                &msg.content
            };

            match msg.role {
                ChatRole::User => {
                    all_lines.push(Line::from(Span::styled(
                        format!("You: {content}"),
                        Style::default().fg(Color::Yellow),
                    )));
                }
                ChatRole::Assistant => {
                    all_lines.push(Line::from(Span::styled(
                        "Accountant:".to_string(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )));
                    for line in content.lines() {
                        all_lines.push(Line::from(Span::styled(
                            line.to_string(),
                            Style::default().fg(Color::White),
                        )));
                    }
                }
                ChatRole::System => {
                    for line in content.lines() {
                        all_lines.push(Line::from(Span::styled(
                            line.to_string(),
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::ITALIC),
                        )));
                    }
                }
            }
            // Blank separator between messages.
            all_lines.push(Line::default());
        }

        // Apply scroll offset.
        let total_lines = all_lines.len();
        let skip = self
            .scroll_offset
            .min(total_lines.saturating_sub(msg_height));
        let visible: Vec<Line<'static>> = all_lines.into_iter().skip(skip).collect();

        let msg_para = Paragraph::new(visible)
            .wrap(Wrap { trim: false })
            .alignment(Alignment::Left);
        frame.render_widget(msg_para, msg_area);

        // Input line.
        let cursor_display = if self.cursor_pos <= self.input_buffer.len() {
            self.cursor_pos
        } else {
            self.input_buffer.len()
        };
        let (before, after) = self.input_buffer.split_at(cursor_display);
        let input_spans = vec![
            Span::styled("> ", Style::default().fg(Color::DarkGray)),
            Span::raw(before.to_string()),
            Span::styled("█", Style::default().fg(Color::White)),
            Span::raw(after.to_string()),
        ];
        let input_para = Paragraph::new(Line::from(input_spans));
        frame.render_widget(input_para, input_area);
    }

    // ── Stub interaction methods (implemented fully in Task 7) ────────────────

    /// Submit the current input buffer as a user message.
    /// Returns the full API message history, or None if the buffer is empty.
    pub fn submit_input(&mut self) -> Option<Vec<ApiMessage>> {
        let text = self.input_buffer.trim().to_string();
        if text.is_empty() {
            return None;
        }
        self.messages.push(ChatMessage {
            role: ChatRole::User,
            content: text,
            is_fully_rendered: true,
        });
        self.input_buffer.clear();
        self.cursor_pos = 0;
        Some(self.api_messages())
    }

    /// Add an assistant response. Starts typewriter animation.
    pub fn add_response(&mut self, content: String) {
        let msg_index = self.messages.len();
        self.messages.push(ChatMessage {
            role: ChatRole::Assistant,
            content: content.clone(),
            is_fully_rendered: false,
        });
        self.typewriter = Some(TypewriterState {
            full_text: content,
            display_position: 0,
            message_index: msg_index,
        });
    }

    /// Add a system notification (no typewriter animation).
    pub fn add_system_note(&mut self, note: &str) {
        self.messages.push(ChatMessage {
            role: ChatRole::System,
            content: note.to_string(),
            is_fully_rendered: true,
        });
    }

    /// Replace conversation with a compacted summary.
    pub fn replace_with_summary(&mut self, summary: String, original_count: usize) {
        self.messages.clear();
        self.typewriter = None;
        self.messages.push(ChatMessage {
            role: ChatRole::System,
            content: format!("[Compacted from {original_count} messages]\n\n{summary}"),
            is_fully_rendered: true,
        });
    }

    /// Rebuild the system prompt from fresh config/context.
    pub fn rebuild_system_prompt(&mut self, persona: &str, entity_name: &str, context: &str) {
        use crate::ai::client::AiClient;
        self.system_prompt = AiClient::build_system_prompt(persona, entity_name, context);
        self.current_persona = persona.to_string();
    }

    /// Get the API message history (User + Assistant only; System notes excluded).
    pub fn api_messages(&self) -> Vec<ApiMessage> {
        self.messages
            .iter()
            .filter_map(|msg| match msg.role {
                ChatRole::User => Some(ApiMessage {
                    role: ApiRole::User,
                    content: ApiContent::Text(msg.content.clone()),
                }),
                ChatRole::Assistant => Some(ApiMessage {
                    role: ApiRole::Assistant,
                    content: ApiContent::Text(msg.content.clone()),
                }),
                ChatRole::System => None,
            })
            .collect()
    }

    /// Skip typewriter animation — reveal full text immediately.
    pub fn skip_typewriter(&mut self) {
        if let Some(tw) = self.typewriter.take()
            && let Some(msg) = self.messages.get_mut(tw.message_index)
        {
            msg.is_fully_rendered = true;
        }
    }

    /// True when a typewriter animation is in progress.
    pub fn typewriter_active(&self) -> bool {
        self.typewriter.is_some()
    }

    /// Handle key events. Returns a `ChatAction` for `App` to process.
    /// Tab is NOT handled here — the App intercepts it for focus switching.
    pub fn handle_key(&mut self, key: KeyEvent) -> ChatAction {
        match key.code {
            // Close panel.
            KeyCode::Esc => ChatAction::Close,
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                ChatAction::Close
            }

            // Confirm / submit.
            KeyCode::Enter => {
                if self.typewriter_active() {
                    return ChatAction::SkipTypewriter;
                }
                let trimmed = self.input_buffer.trim().to_string();
                if trimmed.is_empty() {
                    return ChatAction::None;
                }
                if trimmed.starts_with('/') {
                    let cmd = SlashCommand::parse(&trimmed);
                    self.input_buffer.clear();
                    self.cursor_pos = 0;
                    return ChatAction::SlashCommand(cmd);
                }
                match self.submit_input() {
                    Some(messages) => ChatAction::SendMessage(messages),
                    None => ChatAction::None,
                }
            }

            // Backspace — delete char before cursor.
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    let mut prev = self.cursor_pos - 1;
                    while prev > 0 && !self.input_buffer.is_char_boundary(prev) {
                        prev -= 1;
                    }
                    self.input_buffer.drain(prev..self.cursor_pos);
                    self.cursor_pos = prev;
                }
                ChatAction::None
            }

            // Delete — delete char at cursor.
            KeyCode::Delete => {
                if self.cursor_pos < self.input_buffer.len() {
                    let mut next = self.cursor_pos + 1;
                    while next < self.input_buffer.len()
                        && !self.input_buffer.is_char_boundary(next)
                    {
                        next += 1;
                    }
                    self.input_buffer.drain(self.cursor_pos..next);
                }
                ChatAction::None
            }

            // Cursor movement.
            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    let mut prev = self.cursor_pos - 1;
                    while prev > 0 && !self.input_buffer.is_char_boundary(prev) {
                        prev -= 1;
                    }
                    self.cursor_pos = prev;
                }
                ChatAction::None
            }
            KeyCode::Right => {
                if self.cursor_pos < self.input_buffer.len() {
                    let mut next = self.cursor_pos + 1;
                    while next < self.input_buffer.len()
                        && !self.input_buffer.is_char_boundary(next)
                    {
                        next += 1;
                    }
                    self.cursor_pos = next;
                }
                ChatAction::None
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
                ChatAction::None
            }
            KeyCode::End => {
                self.cursor_pos = self.input_buffer.len();
                ChatAction::None
            }

            // Scroll message history (only when input is empty).
            KeyCode::Up => {
                if self.input_buffer.is_empty() {
                    self.scroll_offset = self.scroll_offset.saturating_add(1);
                }
                ChatAction::None
            }
            KeyCode::Down => {
                if self.input_buffer.is_empty() {
                    self.scroll_offset = self.scroll_offset.saturating_sub(1);
                }
                ChatAction::None
            }

            // Printable characters — insert at cursor position.
            KeyCode::Char(c) => {
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf);
                self.input_buffer.insert_str(self.cursor_pos, encoded);
                self.cursor_pos += encoded.len();
                ChatAction::None
            }

            _ => ChatAction::None,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_panel() -> ChatPanel {
        ChatPanel::new("Acme Corp", "Professional Accountant")
    }

    // ── SlashCommand::parse ────────────────────────────────────────────────

    #[test]
    fn slash_parse_clear() {
        assert_eq!(SlashCommand::parse("/clear"), SlashCommand::Clear);
    }

    #[test]
    fn slash_parse_context() {
        assert_eq!(SlashCommand::parse("/context"), SlashCommand::Context);
    }

    #[test]
    fn slash_parse_compact() {
        assert_eq!(SlashCommand::parse("/compact"), SlashCommand::Compact);
    }

    #[test]
    fn slash_parse_match() {
        assert_eq!(SlashCommand::parse("/match"), SlashCommand::Match);
    }

    #[test]
    fn slash_parse_persona_no_args() {
        assert_eq!(SlashCommand::parse("/persona"), SlashCommand::Persona(None));
    }

    #[test]
    fn slash_parse_persona_with_args() {
        assert_eq!(
            SlashCommand::parse("/persona Expert CPA"),
            SlashCommand::Persona(Some("Expert CPA".to_string()))
        );
    }

    #[test]
    fn slash_parse_unknown() {
        assert_eq!(
            SlashCommand::parse("/unknown"),
            SlashCommand::Unknown("unknown".to_string())
        );
    }

    // ── visibility ────────────────────────────────────────────────────────

    #[test]
    fn new_panel_is_not_visible() {
        let panel = make_panel();
        assert!(!panel.is_visible());
    }

    #[test]
    fn toggle_visible_returns_new_state() {
        let mut panel = make_panel();
        assert!(panel.toggle_visible()); // now visible
        assert!(!panel.toggle_visible()); // now hidden
    }

    #[test]
    fn toggle_visible_first_open_builds_welcome() {
        let mut panel = make_panel();
        panel.toggle_visible();
        assert!(
            !panel.messages.is_empty(),
            "Welcome message should be added"
        );
        // Welcome message has System role.
        assert!(panel.messages.iter().any(|m| m.role == ChatRole::System));
    }

    // ── typewriter ────────────────────────────────────────────────────────

    #[test]
    fn add_response_starts_typewriter() {
        let mut panel = make_panel();
        panel.add_response("Hello, world!".to_string());
        assert!(panel.typewriter.is_some());
        let tw = panel.typewriter.as_ref().unwrap();
        assert_eq!(tw.display_position, 0);
        assert!(!panel.messages.last().unwrap().is_fully_rendered);
    }

    #[test]
    fn tick_advances_typewriter() {
        let mut panel = make_panel();
        panel.add_response("A".repeat(100));
        panel.tick();
        let tw = panel.typewriter.as_ref().unwrap();
        assert_eq!(tw.display_position, 20);
    }

    #[test]
    fn tick_completes_short_response() {
        let mut panel = make_panel();
        panel.add_response("Short.".to_string());
        panel.tick(); // 20 chars > 6, so completes
        assert!(panel.typewriter.is_none());
        assert!(panel.messages.last().unwrap().is_fully_rendered);
    }

    #[test]
    fn skip_typewriter_reveals_full_text() {
        let mut panel = make_panel();
        panel.add_response("Some long text here.".to_string());
        assert!(panel.typewriter.is_some());
        panel.skip_typewriter();
        assert!(panel.typewriter.is_none());
        assert!(panel.messages.last().unwrap().is_fully_rendered);
    }

    #[test]
    fn typewriter_active_returns_correct_state() {
        let mut panel = make_panel();
        assert!(!panel.typewriter_active());
        panel.add_response("Hello".to_string());
        assert!(panel.typewriter_active());
        panel.skip_typewriter();
        assert!(!panel.typewriter_active());
    }

    // ── message management ────────────────────────────────────────────────

    #[test]
    fn add_system_note_adds_system_message() {
        let mut panel = make_panel();
        panel.add_system_note("[Context refreshed]");
        assert_eq!(panel.messages.len(), 1);
        assert_eq!(panel.messages[0].role, ChatRole::System);
        assert_eq!(panel.messages[0].content, "[Context refreshed]");
        assert!(panel.messages[0].is_fully_rendered);
    }

    #[test]
    fn replace_with_summary_clears_messages() {
        let mut panel = make_panel();
        panel.add_system_note("Note 1");
        panel.add_system_note("Note 2");
        panel.replace_with_summary("Summary text.".to_string(), 5);
        assert_eq!(panel.messages.len(), 1);
        assert!(panel.messages[0].content.contains("Summary text."));
        assert!(panel.messages[0].content.contains("Compacted from 5"));
    }

    #[test]
    fn replace_with_summary_clears_typewriter() {
        let mut panel = make_panel();
        panel.add_response("Long response...".to_string());
        assert!(panel.typewriter.is_some());
        panel.replace_with_summary("Summary.".to_string(), 3);
        assert!(panel.typewriter.is_none());
    }

    // ── api_messages ──────────────────────────────────────────────────────

    #[test]
    fn api_messages_excludes_system_messages() {
        let mut panel = make_panel();
        panel.add_system_note("[Context refreshed]");
        panel.messages.push(ChatMessage {
            role: ChatRole::User,
            content: "Hello".to_string(),
            is_fully_rendered: true,
        });
        panel.messages.push(ChatMessage {
            role: ChatRole::Assistant,
            content: "Hi there".to_string(),
            is_fully_rendered: true,
        });

        let api_msgs = panel.api_messages();
        assert_eq!(api_msgs.len(), 2);
        assert!(matches!(api_msgs[0].role, ApiRole::User));
        assert!(matches!(api_msgs[1].role, ApiRole::Assistant));
    }

    #[test]
    fn api_messages_empty_when_only_system_notes() {
        let mut panel = make_panel();
        panel.add_system_note("Welcome");
        assert!(panel.api_messages().is_empty());
    }

    // ── submit_input ──────────────────────────────────────────────────────

    #[test]
    fn submit_input_returns_none_for_empty_buffer() {
        let mut panel = make_panel();
        assert!(panel.submit_input().is_none());
    }

    #[test]
    fn submit_input_adds_user_message_and_clears_buffer() {
        let mut panel = make_panel();
        panel.input_buffer = "What is the balance?".to_string();
        panel.cursor_pos = 20;
        let result = panel.submit_input();
        assert!(result.is_some());
        assert_eq!(panel.input_buffer, "");
        assert_eq!(panel.cursor_pos, 0);
        let last = panel.messages.last().unwrap();
        assert_eq!(last.role, ChatRole::User);
        assert_eq!(last.content, "What is the balance?");
    }

    // ── rebuild_system_prompt ─────────────────────────────────────────────

    #[test]
    fn rebuild_system_prompt_updates_persona() {
        let mut panel = make_panel();
        panel.rebuild_system_prompt("New Persona", "Corp", "context");
        assert_eq!(panel.current_persona, "New Persona");
        assert!(!panel.system_prompt.is_empty());
        assert!(panel.system_prompt.contains("New Persona"));
    }

    // ── handle_key ────────────────────────────────────────────────────────

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn key_char_inserts_into_buffer() {
        let mut panel = make_panel();
        panel.handle_key(key(KeyCode::Char('h')));
        panel.handle_key(key(KeyCode::Char('i')));
        assert_eq!(panel.input_buffer, "hi");
        assert_eq!(panel.cursor_pos, 2);
    }

    #[test]
    fn key_backspace_deletes_before_cursor() {
        let mut panel = make_panel();
        panel.input_buffer = "hello".to_string();
        panel.cursor_pos = 5;
        panel.handle_key(key(KeyCode::Backspace));
        assert_eq!(panel.input_buffer, "hell");
        assert_eq!(panel.cursor_pos, 4);
    }

    #[test]
    fn key_backspace_at_start_does_nothing() {
        let mut panel = make_panel();
        panel.input_buffer = "hi".to_string();
        panel.cursor_pos = 0;
        panel.handle_key(key(KeyCode::Backspace));
        assert_eq!(panel.input_buffer, "hi");
        assert_eq!(panel.cursor_pos, 0);
    }

    #[test]
    fn key_delete_removes_char_at_cursor() {
        let mut panel = make_panel();
        panel.input_buffer = "hello".to_string();
        panel.cursor_pos = 0;
        panel.handle_key(key(KeyCode::Delete));
        assert_eq!(panel.input_buffer, "ello");
        assert_eq!(panel.cursor_pos, 0);
    }

    #[test]
    fn key_left_moves_cursor() {
        let mut panel = make_panel();
        panel.input_buffer = "abc".to_string();
        panel.cursor_pos = 3;
        panel.handle_key(key(KeyCode::Left));
        assert_eq!(panel.cursor_pos, 2);
    }

    #[test]
    fn key_right_moves_cursor() {
        let mut panel = make_panel();
        panel.input_buffer = "abc".to_string();
        panel.cursor_pos = 0;
        panel.handle_key(key(KeyCode::Right));
        assert_eq!(panel.cursor_pos, 1);
    }

    #[test]
    fn key_home_moves_to_start() {
        let mut panel = make_panel();
        panel.input_buffer = "abc".to_string();
        panel.cursor_pos = 3;
        panel.handle_key(key(KeyCode::Home));
        assert_eq!(panel.cursor_pos, 0);
    }

    #[test]
    fn key_end_moves_to_end() {
        let mut panel = make_panel();
        panel.input_buffer = "abc".to_string();
        panel.cursor_pos = 0;
        panel.handle_key(key(KeyCode::End));
        assert_eq!(panel.cursor_pos, 3);
    }

    #[test]
    fn key_escape_returns_close() {
        let mut panel = make_panel();
        let action = panel.handle_key(key(KeyCode::Esc));
        assert!(matches!(action, ChatAction::Close));
    }

    #[test]
    fn key_ctrl_k_returns_close() {
        let mut panel = make_panel();
        let action = panel.handle_key(ctrl_key('k'));
        assert!(matches!(action, ChatAction::Close));
    }

    #[test]
    fn key_enter_empty_input_returns_none() {
        let mut panel = make_panel();
        let action = panel.handle_key(key(KeyCode::Enter));
        assert!(matches!(action, ChatAction::None));
    }

    #[test]
    fn key_enter_with_typewriter_returns_skip() {
        let mut panel = make_panel();
        panel.add_response("Long response text here.".to_string());
        let action = panel.handle_key(key(KeyCode::Enter));
        assert!(matches!(action, ChatAction::SkipTypewriter));
    }

    #[test]
    fn key_enter_slash_clear_returns_slash_command() {
        let mut panel = make_panel();
        panel.input_buffer = "/clear".to_string();
        panel.cursor_pos = 6;
        let action = panel.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            action,
            ChatAction::SlashCommand(SlashCommand::Clear)
        ));
        assert_eq!(panel.input_buffer, "");
    }

    #[test]
    fn key_enter_slash_persona_with_args() {
        let mut panel = make_panel();
        panel.input_buffer = "/persona Expert CPA".to_string();
        panel.cursor_pos = 19;
        let action = panel.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            action,
            ChatAction::SlashCommand(SlashCommand::Persona(Some(_)))
        ));
    }

    #[test]
    fn key_enter_slash_unknown_returns_slash_command() {
        let mut panel = make_panel();
        panel.input_buffer = "/unknown".to_string();
        panel.cursor_pos = 8;
        let action = panel.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            action,
            ChatAction::SlashCommand(SlashCommand::Unknown(_))
        ));
    }

    #[test]
    fn key_enter_with_text_returns_send_message() {
        let mut panel = make_panel();
        panel.input_buffer = "What is the balance?".to_string();
        panel.cursor_pos = 20;
        let action = panel.handle_key(key(KeyCode::Enter));
        assert!(matches!(action, ChatAction::SendMessage(_)));
        assert_eq!(panel.input_buffer, "");
    }

    #[test]
    fn key_up_scrolls_history_when_input_empty() {
        let mut panel = make_panel();
        assert_eq!(panel.scroll_offset, 0);
        panel.handle_key(key(KeyCode::Up));
        assert_eq!(panel.scroll_offset, 1);
    }

    #[test]
    fn key_up_does_not_scroll_when_input_nonempty() {
        let mut panel = make_panel();
        panel.input_buffer = "typing".to_string();
        panel.handle_key(key(KeyCode::Up));
        assert_eq!(panel.scroll_offset, 0);
    }

    #[test]
    fn key_down_scrolls_down_when_input_empty() {
        let mut panel = make_panel();
        panel.scroll_offset = 3;
        panel.handle_key(key(KeyCode::Down));
        assert_eq!(panel.scroll_offset, 2);
    }
}
