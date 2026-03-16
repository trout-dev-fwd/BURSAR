//! Audit Log tab — read-only chronological view of all audit events.
//!
//! Supports filtering by date range (from / to) and by action type.
//! No mutations are possible; the audit log is strictly append-only.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
};

use crate::db::{
    EntityDb,
    audit_repo::{AuditEntry, AuditFilter},
};
use crate::tabs::{RecordId, Tab, TabAction};
use crate::types::AuditAction;
use crate::widgets::centered_rect;

// ── Action-type filter cycle ──────────────────────────────────────────────────

/// All audit action variants in display order, for cycling.
const ALL_ACTIONS: &[AuditAction] = &[
    AuditAction::JournalEntryCreated,
    AuditAction::JournalEntryPosted,
    AuditAction::JournalEntryReversed,
    AuditAction::AccountCreated,
    AuditAction::AccountModified,
    AuditAction::AccountDeactivated,
    AuditAction::AccountReactivated,
    AuditAction::AccountDeleted,
    AuditAction::PeriodClosed,
    AuditAction::PeriodReopened,
    AuditAction::YearEndClose,
    AuditAction::EnvelopeAllocationChanged,
    AuditAction::EnvelopeTransfer,
    AuditAction::PlaceInService,
    AuditAction::InterEntityEntryPosted,
    AuditAction::ArItemCreated,
    AuditAction::ArPaymentRecorded,
    AuditAction::ApItemCreated,
    AuditAction::ApPaymentRecorded,
];

// ── Date filter modal ─────────────────────────────────────────────────────────

/// Which field is focused in the date filter modal.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DateField {
    From,
    To,
}

struct DateFilterModal {
    from_str: String,
    to_str: String,
    focused: DateField,
    error: Option<String>,
}

// ── Tab struct ────────────────────────────────────────────────────────────────

pub struct AuditLogTab {
    entries: Vec<AuditEntry>,
    table_state: TableState,
    /// Active filter.
    filter: AuditFilter,
    /// Index into ALL_ACTIONS (None = show all action types).
    action_idx: Option<usize>,
    date_modal: Option<DateFilterModal>,
}

impl Default for AuditLogTab {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditLogTab {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            table_state: TableState::default(),
            filter: AuditFilter::default(),
            action_idx: None,
            date_modal: None,
        }
    }

    fn load(&mut self, db: &EntityDb) {
        match db.audit().list(&self.filter) {
            Ok(entries) => self.entries = entries,
            Err(_) => self.entries.clear(),
        }
        // Keep selection in bounds.
        if self.entries.is_empty() {
            self.table_state.select(None);
        } else {
            let sel = self.table_state.selected().unwrap_or(0);
            self.table_state
                .select(Some(sel.min(self.entries.len() - 1)));
        }
    }

    fn scroll_up(&mut self) {
        if let Some(sel) = self.table_state.selected() {
            if sel > 0 {
                self.table_state.select(Some(sel - 1));
            }
        } else if !self.entries.is_empty() {
            self.table_state.select(Some(0));
        }
    }

    fn scroll_down(&mut self) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        let next = self
            .table_state
            .selected()
            .map_or(0, |s| s + 1)
            .min(len - 1);
        self.table_state.select(Some(next));
    }

    /// Cycle action type filter: None → each variant → None.
    fn cycle_action_forward(&mut self, db: &EntityDb) {
        self.action_idx = match self.action_idx {
            None => Some(0),
            Some(i) if i + 1 < ALL_ACTIONS.len() => Some(i + 1),
            Some(_) => None,
        };
        self.filter.action_type = self.action_idx.map(|i| ALL_ACTIONS[i]);
        self.load(db);
    }

    fn cycle_action_backward(&mut self, db: &EntityDb) {
        self.action_idx = match self.action_idx {
            None => Some(ALL_ACTIONS.len() - 1),
            Some(0) => None,
            Some(i) => Some(i - 1),
        };
        self.filter.action_type = self.action_idx.map(|i| ALL_ACTIONS[i]);
        self.load(db);
    }

    fn action_label(&self) -> String {
        match self.action_idx {
            None => "All".to_owned(),
            Some(i) => ALL_ACTIONS[i].to_string(),
        }
    }

    // ── Date modal ────────────────────────────────────────────────────────────

    fn open_date_modal(&mut self) {
        self.date_modal = Some(DateFilterModal {
            from_str: self.filter.from.clone().unwrap_or_default(),
            to_str: self.filter.to.clone().unwrap_or_default(),
            focused: DateField::From,
            error: None,
        });
    }

    fn handle_date_modal_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        let Some(ref mut modal) = self.date_modal else {
            return TabAction::None;
        };

        match key.code {
            KeyCode::Esc => {
                self.date_modal = None;
            }
            KeyCode::Tab => {
                modal.focused = if modal.focused == DateField::From {
                    DateField::To
                } else {
                    DateField::From
                };
            }
            KeyCode::Backspace => {
                modal.error = None;
                match modal.focused {
                    DateField::From => {
                        modal.from_str.pop();
                    }
                    DateField::To => {
                        modal.to_str.pop();
                    }
                }
            }
            KeyCode::Char(c) => {
                modal.error = None;
                match modal.focused {
                    DateField::From => modal.from_str.push(c),
                    DateField::To => modal.to_str.push(c),
                }
            }
            KeyCode::Enter => {
                // Validate: if non-empty must be YYYY-MM-DD.
                let from_ok = modal.from_str.is_empty()
                    || chrono::NaiveDate::parse_from_str(&modal.from_str, "%Y-%m-%d").is_ok();
                let to_ok = modal.to_str.is_empty()
                    || chrono::NaiveDate::parse_from_str(&modal.to_str, "%Y-%m-%d").is_ok();
                if !from_ok || !to_ok {
                    modal.error = Some("Dates must be YYYY-MM-DD (or blank to clear)".to_owned());
                } else {
                    let from = if modal.from_str.is_empty() {
                        None
                    } else {
                        // Use as timestamp prefix for filtering.
                        Some(format!("{} 00:00:00", modal.from_str))
                    };
                    let to = if modal.to_str.is_empty() {
                        None
                    } else {
                        Some(format!("{} 23:59:59", modal.to_str))
                    };
                    self.filter.from = from;
                    self.filter.to = to;
                    self.date_modal = None;
                    self.load(db);
                }
            }
            _ => {}
        }
        TabAction::None
    }

    // ── Rendering ─────────────────────────────────────────────────────────────

    fn render_table(&self, frame: &mut Frame, area: Rect) {
        let date_range_label = match (&self.filter.from, &self.filter.to) {
            (None, None) => "all dates".to_owned(),
            (Some(f), None) => format!("from {}", &f[..10]),
            (None, Some(t)) => format!("to {}", &t[..10]),
            (Some(f), Some(t)) => format!("{} – {}", &f[..10], &t[..10]),
        };
        let title = format!(
            " Audit Log  [←/→] action: {}  [d] date filter: {}  [c] clear  ↑↓: scroll ",
            self.action_label(),
            date_range_label,
        );

        let col_widths = [
            Constraint::Length(19), // Timestamp
            Constraint::Length(28), // Action Type
            Constraint::Min(0),     // Description
        ];

        let header = Row::new([
            Cell::from("Timestamp").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Action Type").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Description").style(Style::default().add_modifier(Modifier::BOLD)),
        ])
        .style(Style::default().fg(Color::Cyan));

        let rows: Vec<Row> = self
            .entries
            .iter()
            .map(|e| {
                Row::new([
                    Cell::from(e.created_at.clone()),
                    Cell::from(e.action_type.to_string()),
                    Cell::from(e.description.clone()),
                ])
            })
            .collect();

        let table = Table::new(rows, col_widths)
            .header(header)
            .block(Block::default().title(title).borders(Borders::ALL))
            .row_highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_stateful_widget(table, area, &mut self.table_state.clone());
    }

    fn render_date_modal(&self, frame: &mut Frame, area: Rect) {
        let Some(ref modal) = self.date_modal else {
            return;
        };
        let popup = centered_rect(55, 30, area);
        frame.render_widget(Clear, popup);

        let from_ind = if modal.focused == DateField::From {
            ">"
        } else {
            " "
        };
        let to_ind = if modal.focused == DateField::To {
            ">"
        } else {
            " "
        };
        let err_line = modal.error.as_deref().unwrap_or("");
        let content = format!(
            "\n  {from_ind} From (YYYY-MM-DD): {}_\n\n  {to_ind} To   (YYYY-MM-DD): {}_\n\n  {err_line}\n\n  Tab: switch   Enter: apply   Esc: cancel",
            modal.from_str, modal.to_str,
        );
        frame.render_widget(
            Paragraph::new(content).block(
                Block::default()
                    .title(" Date Range Filter ")
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Cyan)),
            ),
            popup,
        );
    }
}

// ── Tab impl ──────────────────────────────────────────────────────────────────

impl Tab for AuditLogTab {
    fn title(&self) -> &str {
        "Audit Log"
    }

    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        if self.date_modal.is_some() {
            return self.handle_date_modal_key(key, db);
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(),
            KeyCode::Right => self.cycle_action_forward(db),
            KeyCode::Left => self.cycle_action_backward(db),
            KeyCode::Char('d') | KeyCode::Char('D') => self.open_date_modal(),
            KeyCode::Char('c') | KeyCode::Char('C') => {
                // Clear all filters.
                self.filter = AuditFilter::default();
                self.action_idx = None;
                self.load(db);
            }
            _ => {}
        }
        TabAction::None
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        // Split into table + status hint.
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        self.render_table(frame, chunks[0]);

        let count = self.entries.len();
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!(" {count} entries "),
                Style::default().fg(Color::DarkGray),
            )])),
            chunks[1],
        );

        self.render_date_modal(frame, area);
    }

    fn refresh(&mut self, db: &EntityDb) {
        self.load(db);
    }

    fn wants_input(&self) -> bool {
        self.date_modal.is_some()
    }

    fn navigate_to(&mut self, _record_id: RecordId, _db: &EntityDb) {}
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entity_db_from_conn;
    use crate::db::schema::initialize_schema;
    use rusqlite::Connection;

    fn make_db() -> EntityDb {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema");
        entity_db_from_conn(conn)
    }

    #[test]
    fn audit_log_tab_title() {
        let tab = AuditLogTab::new();
        assert_eq!(tab.title(), "Audit Log");
    }

    #[test]
    fn audit_log_tab_starts_empty() {
        let tab = AuditLogTab::new();
        assert!(tab.entries.is_empty());
    }

    #[test]
    fn audit_log_tab_refresh_loads_entries() {
        let db = make_db();
        // Append a test audit entry directly.
        db.audit()
            .append(
                AuditAction::JournalEntryCreated,
                "Test",
                None,
                None,
                "test entry",
            )
            .expect("append");
        let mut tab = AuditLogTab::new();
        tab.refresh(&db);
        assert_eq!(tab.entries.len(), 1);
    }

    #[test]
    fn audit_log_tab_action_filter_cycle() {
        let db = make_db();
        // Append entries with two different action types.
        db.audit()
            .append(
                AuditAction::JournalEntryPosted,
                "Test",
                None,
                None,
                "posted",
            )
            .expect("a1");
        db.audit()
            .append(
                AuditAction::AccountCreated,
                "Test",
                None,
                None,
                "created account",
            )
            .expect("a2");

        let mut tab = AuditLogTab::new();
        tab.refresh(&db);
        assert_eq!(tab.entries.len(), 2);

        // Cycle forward until JournalEntryPosted is selected.
        for _ in 0..10 {
            tab.cycle_action_forward(&db);
            if tab.filter.action_type == Some(AuditAction::JournalEntryPosted) {
                break;
            }
        }
        assert_eq!(
            tab.filter.action_type,
            Some(AuditAction::JournalEntryPosted)
        );
        assert_eq!(tab.entries.len(), 1);
        assert_eq!(tab.entries[0].action_type, AuditAction::JournalEntryPosted);
    }

    #[test]
    fn audit_log_tab_clear_resets_filter() {
        let db = make_db();
        db.audit()
            .append(
                AuditAction::JournalEntryPosted,
                "Test",
                None,
                None,
                "posted",
            )
            .expect("a1");

        let mut tab = AuditLogTab::new();
        tab.refresh(&db);
        tab.cycle_action_forward(&db); // set filter to first action
        // Clear.
        let action = tab.handle_key(
            KeyEvent::new(KeyCode::Char('c'), crossterm::event::KeyModifiers::NONE),
            &db,
        );
        assert!(matches!(action, TabAction::None));
        assert!(tab.filter.action_type.is_none());
        assert_eq!(tab.entries.len(), 1);
    }

    #[test]
    fn audit_log_tab_scroll_up_down() {
        let db = make_db();
        for i in 0..3u32 {
            db.audit()
                .append(
                    AuditAction::AccountCreated,
                    "Test",
                    None,
                    None,
                    &format!("entry {i}"),
                )
                .expect("append");
        }
        let mut tab = AuditLogTab::new();
        tab.refresh(&db);

        tab.handle_key(
            KeyEvent::new(KeyCode::Down, crossterm::event::KeyModifiers::NONE),
            &db,
        );
        tab.handle_key(
            KeyEvent::new(KeyCode::Down, crossterm::event::KeyModifiers::NONE),
            &db,
        );
        assert_eq!(tab.table_state.selected(), Some(2));

        tab.handle_key(
            KeyEvent::new(KeyCode::Up, crossterm::event::KeyModifiers::NONE),
            &db,
        );
        assert_eq!(tab.table_state.selected(), Some(1));
    }

    #[test]
    fn audit_log_tab_wants_input_when_modal_open() {
        let mut tab = AuditLogTab::new();
        assert!(!tab.wants_input());
        tab.open_date_modal();
        assert!(tab.wants_input());
    }
}
