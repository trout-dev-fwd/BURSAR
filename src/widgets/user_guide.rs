//! In-app user guide widget — three-level drill-down navigation.
//!
//! Content is embedded at compile time from `specs/guide/user-guide.md` and
//! parsed into a section → topic → text hierarchy.
//!
//! Navigation:
//! - Level 1 (ToC): list of `##` sections
//! - Level 2 (Topics): list of `###` headings within a section
//! - Level 3 (Content): scrollable body text
//!
//! `Esc` goes back one level; `Esc` at Level 1 closes the overlay.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::widgets::centered_rect;

// Embed the guide at compile time so the binary is self-contained.
static GUIDE_MD: &str = include_str!("../../specs/guide/user-guide.md");

// ── Internal data model ───────────────────────────────────────────────────────

struct GuideSection {
    title: String,
    /// Body lines that appear before the first `###` topic header.
    /// Only displayed for sections that have no `###` topics.
    intro_lines: Vec<String>,
    topics: Vec<GuideTopic>,
}

struct GuideTopic {
    title: String,
    lines: Vec<String>,
}

// ── Navigation level ──────────────────────────────────────────────────────────

enum GuideLevel {
    /// Table of contents — shows all section titles.
    Toc { selected: usize },
    /// Topic list — shows `###` headings within one section.
    Topics { section_idx: usize, selected: usize },
    /// Scrollable text content for one section or one topic.
    Content {
        section_idx: usize,
        /// `None` = section intro_lines; `Some(i)` = topic i's lines.
        topic_idx: Option<usize>,
        scroll: usize,
    },
}

// ── Public surface ────────────────────────────────────────────────────────────

/// Action returned by [`UserGuide::handle_key`].
pub enum UserGuideAction {
    /// Caller should close (remove) the guide overlay.
    Close,
    /// Key consumed; no state change visible to the caller.
    Pending,
}

/// Full-screen overlay user guide widget.
pub struct UserGuide {
    sections: Vec<GuideSection>,
    level: GuideLevel,
}

impl Default for UserGuide {
    fn default() -> Self {
        Self::new()
    }
}

impl UserGuide {
    /// Creates and parses the guide from the embedded markdown.
    pub fn new() -> Self {
        Self {
            sections: parse_guide(GUIDE_MD),
            level: GuideLevel::Toc { selected: 0 },
        }
    }

    /// Routes a key event through the current navigation level.
    pub fn handle_key(&mut self, key: KeyEvent) -> UserGuideAction {
        // Take ownership of the level so we can freely borrow `self.sections`
        // inside the match arms without fighting the borrow checker.
        let level = std::mem::replace(&mut self.level, GuideLevel::Toc { selected: 0 });

        let new_level = match level {
            // ── Table of contents ─────────────────────────────────────────────
            GuideLevel::Toc { selected } => match key.code {
                KeyCode::Esc => return UserGuideAction::Close,
                KeyCode::Up | KeyCode::Char('k') => GuideLevel::Toc {
                    selected: selected.saturating_sub(1),
                },
                KeyCode::Down | KeyCode::Char('j') => {
                    let max = self.sections.len().saturating_sub(1);
                    GuideLevel::Toc {
                        selected: (selected + 1).min(max),
                    }
                }
                KeyCode::Enter => {
                    if self.sections[selected].topics.is_empty() {
                        GuideLevel::Content {
                            section_idx: selected,
                            topic_idx: None,
                            scroll: 0,
                        }
                    } else {
                        GuideLevel::Topics {
                            section_idx: selected,
                            selected: 0,
                        }
                    }
                }
                _ => GuideLevel::Toc { selected },
            },

            // ── Topic list ────────────────────────────────────────────────────
            GuideLevel::Topics {
                section_idx,
                selected,
            } => match key.code {
                KeyCode::Esc => GuideLevel::Toc {
                    selected: section_idx,
                },
                KeyCode::Up | KeyCode::Char('k') => GuideLevel::Topics {
                    section_idx,
                    selected: selected.saturating_sub(1),
                },
                KeyCode::Down | KeyCode::Char('j') => {
                    let max = self.sections[section_idx].topics.len().saturating_sub(1);
                    GuideLevel::Topics {
                        section_idx,
                        selected: (selected + 1).min(max),
                    }
                }
                KeyCode::Enter => GuideLevel::Content {
                    section_idx,
                    topic_idx: Some(selected),
                    scroll: 0,
                },
                _ => GuideLevel::Topics {
                    section_idx,
                    selected,
                },
            },

            // ── Scrollable content ────────────────────────────────────────────
            GuideLevel::Content {
                section_idx,
                topic_idx,
                scroll,
            } => match key.code {
                KeyCode::Esc => {
                    if self.sections[section_idx].topics.is_empty() {
                        GuideLevel::Toc {
                            selected: section_idx,
                        }
                    } else {
                        GuideLevel::Topics {
                            section_idx,
                            selected: topic_idx.unwrap_or(0),
                        }
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => GuideLevel::Content {
                    section_idx,
                    topic_idx,
                    scroll: scroll.saturating_sub(1),
                },
                KeyCode::Down | KeyCode::Char('j') => GuideLevel::Content {
                    section_idx,
                    topic_idx,
                    scroll: scroll + 1,
                },
                KeyCode::PageUp => GuideLevel::Content {
                    section_idx,
                    topic_idx,
                    scroll: scroll.saturating_sub(10),
                },
                KeyCode::PageDown => GuideLevel::Content {
                    section_idx,
                    topic_idx,
                    scroll: scroll + 10,
                },
                _ => GuideLevel::Content {
                    section_idx,
                    topic_idx,
                    scroll,
                },
            },
        };

        self.level = new_level;
        UserGuideAction::Pending
    }

    /// Renders the guide as a 90 × 90 % overlay centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let popup = centered_rect(90, 90, area);
        frame.render_widget(Clear, popup);
        match &self.level {
            GuideLevel::Toc { selected } => self.render_toc(frame, popup, *selected),
            GuideLevel::Topics {
                section_idx,
                selected,
            } => {
                self.render_topics(frame, popup, *section_idx, *selected);
            }
            GuideLevel::Content {
                section_idx,
                topic_idx,
                scroll,
            } => {
                self.render_content(frame, popup, *section_idx, *topic_idx, *scroll);
            }
        }
    }

    // ── Render helpers ────────────────────────────────────────────────────────

    fn render_toc(&self, frame: &mut Frame, area: Rect, selected: usize) {
        let block = Block::default()
            .title(
                " User Guide  \
                 ↑↓: navigate  Enter: open section  Esc: close ",
            )
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan).bg(Color::Black));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let items: Vec<ListItem> = self
            .sections
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let indicator = if s.topics.is_empty() { " " } else { "▶" };
                let style = if i == selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(format!("  {} {}", indicator, s.title)).style(style)
            })
            .collect();

        let mut state = ListState::default();
        state.select(Some(selected));
        frame.render_stateful_widget(
            List::new(items).style(Style::default().bg(Color::Black)),
            inner,
            &mut state,
        );
    }

    fn render_topics(&self, frame: &mut Frame, area: Rect, section_idx: usize, selected: usize) {
        let section = &self.sections[section_idx];
        let block = Block::default()
            .title(format!(
                " {} ▶ Topics  ↑↓: navigate  Enter: open  Esc: back ",
                section.title
            ))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan).bg(Color::Black));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let items: Vec<ListItem> = section
            .topics
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let style = if i == selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(format!("    {}", t.title)).style(style)
            })
            .collect();

        let mut state = ListState::default();
        state.select(Some(selected));
        frame.render_stateful_widget(
            List::new(items).style(Style::default().bg(Color::Black)),
            inner,
            &mut state,
        );
    }

    fn render_content(
        &self,
        frame: &mut Frame,
        area: Rect,
        section_idx: usize,
        topic_idx: Option<usize>,
        scroll: usize,
    ) {
        let section = &self.sections[section_idx];
        let title = match topic_idx {
            None => format!(" {}  ↑↓/PgUp/PgDn: scroll  Esc: back ", section.title),
            Some(i) => format!(
                " {} ▶ {}  ↑↓/PgUp/PgDn: scroll  Esc: back ",
                section.title, section.topics[i].title
            ),
        };
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan).bg(Color::Black));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let raw_lines: &[String] = match topic_idx {
            None => &section.intro_lines,
            Some(i) => &section.topics[i].lines,
        };

        let lines: Vec<Line<'static>> = raw_lines.iter().map(|l| format_guide_line(l)).collect();

        // Clamp scroll so we don't show an all-blank screen past the content.
        let available = inner.height as usize;
        let max_scroll = lines.len().saturating_sub(available);
        let clamped = scroll.min(max_scroll) as u16;

        frame.render_widget(
            Paragraph::new(lines)
                .scroll((clamped, 0))
                .style(Style::default().bg(Color::Black)),
            inner,
        );
    }
}

// ── Markdown parser ───────────────────────────────────────────────────────────

/// Parses the embedded markdown into a flat list of `GuideSection`s.
/// Rules:
/// - `# ` lines (H1) are skipped.
/// - `## ` lines start a new section.
/// - `### ` lines start a new topic inside the current section.
/// - `---` horizontal rules are discarded.
/// - All other lines are appended to the current topic's line list,
///   or to the section's `intro_lines` if no topic has started yet.
fn parse_guide(markdown: &str) -> Vec<GuideSection> {
    let mut sections: Vec<GuideSection> = Vec::new();

    for line in markdown.lines() {
        if line.starts_with("# ") {
            // H1 — skip (document title)
        } else if let Some(title) = line.strip_prefix("## ") {
            sections.push(GuideSection {
                title: title.to_string(),
                intro_lines: Vec::new(),
                topics: Vec::new(),
            });
        } else if let Some(title) = line.strip_prefix("### ") {
            if let Some(section) = sections.last_mut() {
                section.topics.push(GuideTopic {
                    title: title.to_string(),
                    lines: Vec::new(),
                });
            }
        } else if line.starts_with("---") {
            // Horizontal rule — discard
        } else if let Some(section) = sections.last_mut() {
            if let Some(topic) = section.topics.last_mut() {
                topic.lines.push(line.to_string());
            } else {
                section.intro_lines.push(line.to_string());
            }
        }
        // Lines before the first ## are silently ignored.
    }

    sections
}

// ── Inline markdown renderer ──────────────────────────────────────────────────

/// Converts a single markdown body line into a styled `Line<'static>`.
///
/// Formatting applied:
/// - `` `code` `` → cyan
/// - `**bold**` → yellow + bold
/// - Table separator rows (`|---|----|`) → rendered as a dim separator
/// - Everything else → white
fn format_guide_line(line: &str) -> Line<'static> {
    // Table separator: line of `|`, `-`, and spaces only.
    let trimmed = line.trim();
    if trimmed.starts_with('|') && trimmed.chars().all(|c| matches!(c, '|' | '-' | ' ' | ':')) {
        return Line::from(Span::styled(
            "  ─────────────────────────────────────────────────",
            Style::default().fg(Color::DarkGray),
        ));
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut chars = line.chars().peekable();
    let mut current = String::new();

    while let Some(c) = chars.next() {
        match c {
            // Backtick: collect until closing backtick → cyan span.
            '`' => {
                if !current.is_empty() {
                    spans.push(Span::styled(
                        current.clone(),
                        Style::default().fg(Color::White),
                    ));
                    current.clear();
                }
                let mut code = String::new();
                for inner in chars.by_ref() {
                    if inner == '`' {
                        break;
                    }
                    code.push(inner);
                }
                spans.push(Span::styled(code, Style::default().fg(Color::Cyan)));
            }
            // Double-star: collect until closing ** → yellow bold span.
            '*' if chars.peek() == Some(&'*') => {
                chars.next(); // consume second `*`
                if !current.is_empty() {
                    spans.push(Span::styled(
                        current.clone(),
                        Style::default().fg(Color::White),
                    ));
                    current.clear();
                }
                let mut bold = String::new();
                while let Some(inner) = chars.next() {
                    if inner == '*' && chars.peek() == Some(&'*') {
                        chars.next(); // consume closing second `*`
                        break;
                    }
                    bold.push(inner);
                }
                spans.push(Span::styled(
                    bold,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            other => current.push(other),
        }
    }

    if !current.is_empty() {
        spans.push(Span::styled(current, Style::default().fg(Color::White)));
    }

    // Empty line → return an empty Line so Paragraph respects the blank line spacing.
    if spans.is_empty() {
        Line::from("")
    } else {
        Line::from(spans)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn parse_guide_produces_sections() {
        let sections = parse_guide(GUIDE_MD);
        // Must have at least the main content sections.
        assert!(
            sections.len() >= 5,
            "expected many sections, got {}",
            sections.len()
        );
    }

    #[test]
    fn parse_guide_tab_sections_have_topics() {
        let sections = parse_guide(GUIDE_MD);
        // "Tab 1: Chart of Accounts" section should have multiple topics.
        let coa = sections
            .iter()
            .find(|s| s.title.contains("Chart of Accounts"))
            .expect("CoA section not found");
        assert!(
            coa.topics.len() >= 3,
            "expected ≥3 topics in CoA section, got {}",
            coa.topics.len()
        );
    }

    #[test]
    fn parse_guide_sections_without_topics_have_intro_lines() {
        let sections = parse_guide(GUIDE_MD);
        // "Understanding Double-Entry Accounting" has no ### topics, only body text.
        let intro = sections
            .iter()
            .find(|s| s.title.contains("Double-Entry Accounting"))
            .expect("intro section not found");
        assert!(intro.topics.is_empty(), "expected no sub-topics");
        assert!(!intro.intro_lines.is_empty(), "expected intro body text");
    }

    #[test]
    fn toc_esc_closes_guide() {
        let mut guide = UserGuide::new();
        // At Level 1 Toc, Esc should return Close.
        let key = crossterm::event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert!(matches!(guide.handle_key(key), UserGuideAction::Close));
    }

    #[test]
    fn toc_enter_on_section_with_topics_goes_to_topics_level() {
        let mut guide = UserGuide::new();
        // Find the index of a section that has topics.
        let idx = guide
            .sections
            .iter()
            .position(|s| !s.topics.is_empty())
            .expect("at least one section with topics");

        // Move selection to that index.
        guide.level = GuideLevel::Toc { selected: idx };

        let key = crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        guide.handle_key(key);
        assert!(
            matches!(guide.level, GuideLevel::Topics { section_idx, .. } if section_idx == idx)
        );
    }

    #[test]
    fn toc_enter_on_section_without_topics_goes_to_content() {
        let mut guide = UserGuide::new();
        let idx = guide
            .sections
            .iter()
            .position(|s| s.topics.is_empty())
            .expect("at least one section without topics");

        guide.level = GuideLevel::Toc { selected: idx };
        let key = crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        guide.handle_key(key);
        assert!(matches!(
            guide.level,
            GuideLevel::Content {
                topic_idx: None,
                ..
            }
        ));
    }

    #[test]
    fn topics_esc_goes_back_to_toc() {
        let mut guide = UserGuide::new();
        guide.level = GuideLevel::Topics {
            section_idx: 3,
            selected: 1,
        };
        let key = crossterm::event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        guide.handle_key(key);
        assert!(matches!(guide.level, GuideLevel::Toc { selected: 3 }));
    }

    #[test]
    fn content_esc_with_topics_goes_back_to_topics() {
        let mut guide = UserGuide::new();
        let idx = guide
            .sections
            .iter()
            .position(|s| !s.topics.is_empty())
            .expect("section with topics");
        guide.level = GuideLevel::Content {
            section_idx: idx,
            topic_idx: Some(2),
            scroll: 5,
        };
        let key = crossterm::event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        guide.handle_key(key);
        assert!(matches!(
            guide.level,
            GuideLevel::Topics { section_idx, selected: 2 } if section_idx == idx
        ));
    }

    #[test]
    fn format_guide_line_plain_text() {
        let line = format_guide_line("Hello world");
        assert_eq!(line.spans.len(), 1);
    }

    #[test]
    fn format_guide_line_backtick_becomes_cyan() {
        let line = format_guide_line("Press `Ctrl+S` to submit");
        let cyan = line.spans.iter().any(|s| s.style.fg == Some(Color::Cyan));
        assert!(cyan, "expected a cyan span for backtick-wrapped text");
    }

    #[test]
    fn format_guide_line_bold_becomes_yellow() {
        let line = format_guide_line("**Important:** read this");
        let yellow = line.spans.iter().any(|s| s.style.fg == Some(Color::Yellow));
        assert!(yellow, "expected a yellow span for **bold** text");
    }
}
