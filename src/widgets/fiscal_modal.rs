//! Fiscal period management overlay modal.
//!
//! Access via global hotkey `f`. Shows all fiscal years and their periods.
//!
//! Key bindings:
//! - `↑`/`↓` or `j`/`k`: navigate the list
//! - `c`: close the selected period (with confirmation)
//! - `o`: reopen the selected period (with confirmation)
//! - `y`: initiate year-end close on the selected fiscal year (review + post)
//! - `Esc`: dismiss the modal

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::{
    db::EntityDb,
    services::fiscal::{execute_year_end_close, generate_closing_entries},
    types::{FiscalPeriodId, FiscalYearId},
};

use super::centered_rect;
use super::confirmation::{ConfirmAction, Confirmation};

// ── Public types ──────────────────────────────────────────────────────────────

/// Action returned by `FiscalModal::handle_key` to the owning app.
#[derive(Debug)]
pub enum FiscalModalAction {
    /// Nothing happened.
    None,
    /// Modal should be dismissed (user pressed Esc on the list view).
    Close,
    /// A mutation succeeded; refresh all tab data and show the message.
    Mutated(String),
}

// ── Internal types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum RowKind {
    Year(FiscalYearId),
    Period(FiscalPeriodId),
}

#[derive(Debug)]
struct ListRow {
    display: String,
    kind: RowKind,
    is_closed: bool,
}

enum ModalState {
    Browsing,
    AddYear {
        input: String,
    },
    ConfirmClose {
        id: FiscalPeriodId,
        confirm: Confirmation,
    },
    ConfirmReopen {
        id: FiscalPeriodId,
        confirm: Confirmation,
    },
    YearEndReview {
        fy_id: FiscalYearId,
        preview: Vec<String>,
        scroll: usize,
    },
}

// ── FiscalModal ───────────────────────────────────────────────────────────────

/// A modal overlay for managing fiscal periods and triggering year-end close.
pub struct FiscalModal {
    rows: Vec<ListRow>,
    list_state: ListState,
    state: ModalState,
    entity_name: String,
    error: Option<String>,
}

impl FiscalModal {
    /// Creates a new modal and immediately loads all fiscal year / period data.
    pub fn new(entity_name: String, db: &EntityDb) -> Self {
        let mut m = Self {
            rows: Vec::new(),
            list_state: ListState::default(),
            state: ModalState::Browsing,
            entity_name,
            error: None,
        };
        m.reload(db);
        m
    }

    /// Reloads fiscal year / period data from the database and rebuilds the flat row list.
    fn reload(&mut self, db: &EntityDb) {
        self.rows.clear();
        let fiscal = db.fiscal();
        let years = match fiscal.list_fiscal_years() {
            Ok(y) => y,
            Err(e) => {
                self.error = Some(format!("Failed to load fiscal data: {e}"));
                return;
            }
        };
        for year in &years {
            self.rows.push(ListRow {
                display: format!(
                    " FY {}  –  {}    {}",
                    year.start_date,
                    year.end_date,
                    if year.is_closed { "[CLOSED]" } else { "[OPEN]" }
                ),
                kind: RowKind::Year(year.id),
                is_closed: year.is_closed,
            });
            let periods = match fiscal.list_periods(year.id) {
                Ok(p) => p,
                Err(_) => continue,
            };
            for period in &periods {
                self.rows.push(ListRow {
                    display: format!(
                        "     P{:02}  {}  –  {}    {}",
                        period.period_number,
                        period.start_date,
                        period.end_date,
                        if period.is_closed {
                            "[CLOSED]"
                        } else {
                            "[OPEN]"
                        }
                    ),
                    kind: RowKind::Period(period.id),
                    is_closed: period.is_closed,
                });
            }
        }
        // Clamp selection to valid range.
        if self.rows.is_empty() {
            self.list_state.select(None);
        } else {
            let sel = self
                .list_state
                .selected()
                .unwrap_or(0)
                .min(self.rows.len() - 1);
            self.list_state.select(Some(sel));
        }
    }

    // ── Key handling ──────────────────────────────────────────────────────────

    pub fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> FiscalModalAction {
        self.error = None;
        if matches!(self.state, ModalState::Browsing) {
            return self.handle_browsing(key, db);
        }
        if matches!(self.state, ModalState::AddYear { .. }) {
            return self.handle_add_year(key, db);
        }
        if matches!(self.state, ModalState::ConfirmClose { .. }) {
            return self.handle_confirm_close(key, db);
        }
        if matches!(self.state, ModalState::ConfirmReopen { .. }) {
            return self.handle_confirm_reopen(key, db);
        }
        if matches!(self.state, ModalState::YearEndReview { .. }) {
            return self.handle_year_end_review(key, db);
        }
        FiscalModalAction::None
    }

    fn handle_browsing(&mut self, key: KeyEvent, _db: &EntityDb) -> FiscalModalAction {
        match key.code {
            KeyCode::Esc => return FiscalModalAction::Close,

            KeyCode::Up | KeyCode::Char('k') if key.modifiers == KeyModifiers::NONE => {
                let sel = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(sel.saturating_sub(1)));
            }

            KeyCode::Down | KeyCode::Char('j') if key.modifiers == KeyModifiers::NONE => {
                let sel = self.list_state.selected().unwrap_or(0);
                let new_sel = (sel + 1).min(self.rows.len().saturating_sub(1));
                self.list_state.select(Some(new_sel));
            }

            KeyCode::Char('a') if key.modifiers == KeyModifiers::NONE => {
                self.state = ModalState::AddYear {
                    input: String::new(),
                };
            }

            KeyCode::Char('c') if key.modifiers == KeyModifiers::NONE => {
                // Clone selected row data out first to release the borrow on self.rows.
                let data = self
                    .list_state
                    .selected()
                    .and_then(|i| self.rows.get(i))
                    .map(|r| (r.kind.clone(), r.is_closed, r.display.clone()));
                match data {
                    Some((RowKind::Period(pid), false, display)) => {
                        self.state = ModalState::ConfirmClose {
                            id: pid,
                            confirm: Confirmation::new(format!(
                                "Close {}?\nThis locks all journal entries in the period.",
                                display.trim()
                            )),
                        };
                    }
                    Some((RowKind::Period(_), true, _)) => {
                        self.error = Some("Period is already closed.".to_owned());
                    }
                    Some((RowKind::Year(_), _, _)) => {
                        self.error = Some("Select a period row (P##) to close it.".to_owned());
                    }
                    None => {}
                }
            }

            KeyCode::Char('o') if key.modifiers == KeyModifiers::NONE => {
                let data = self
                    .list_state
                    .selected()
                    .and_then(|i| self.rows.get(i))
                    .map(|r| (r.kind.clone(), r.is_closed, r.display.clone()));
                match data {
                    Some((RowKind::Period(pid), true, display)) => {
                        self.state = ModalState::ConfirmReopen {
                            id: pid,
                            confirm: Confirmation::new(format!(
                                "Reopen {}?\nAllows modifications to entries in this period.",
                                display.trim()
                            )),
                        };
                    }
                    Some((RowKind::Period(_), false, _)) => {
                        self.error = Some("Period is already open.".to_owned());
                    }
                    Some((RowKind::Year(_), _, _)) => {
                        self.error = Some("Select a period row (P##) to reopen it.".to_owned());
                    }
                    None => {}
                }
            }

            KeyCode::Char('y') if key.modifiers == KeyModifiers::NONE => {
                let data = self
                    .list_state
                    .selected()
                    .and_then(|i| self.rows.get(i))
                    .map(|r| (r.kind.clone(), r.is_closed));
                match data {
                    Some((RowKind::Year(fy_id), false)) => {
                        return self.start_year_end_review(fy_id, _db);
                    }
                    Some((RowKind::Year(_), true)) => {
                        self.error = Some("Fiscal year is already closed.".to_owned());
                    }
                    Some((RowKind::Period(_), _)) => {
                        self.error = Some(
                            "Select a fiscal year row (FY ...) for year-end close.".to_owned(),
                        );
                    }
                    None => {}
                }
            }

            _ => {}
        }
        FiscalModalAction::None
    }

    fn handle_add_year(&mut self, key: KeyEvent, db: &EntityDb) -> FiscalModalAction {
        match key.code {
            KeyCode::Esc => {
                self.state = ModalState::Browsing;
            }
            KeyCode::Backspace => {
                if let ModalState::AddYear { input } = &mut self.state {
                    input.pop();
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                if let ModalState::AddYear { input } = &mut self.state
                    && input.len() < 4
                {
                    input.push(c);
                }
            }
            KeyCode::Enter => {
                let year_str = if let ModalState::AddYear { input } = &self.state {
                    input.clone()
                } else {
                    return FiscalModalAction::None;
                };
                let year: i32 = match year_str.parse() {
                    Ok(y) if (1000..=9999).contains(&y) => y,
                    _ => {
                        self.error = Some("Enter a 4-digit year (e.g., 2026).".to_owned());
                        self.state = ModalState::Browsing;
                        return FiscalModalAction::None;
                    }
                };
                // Check for duplicate: compare start_date year.
                let existing = db.fiscal().list_fiscal_years().unwrap_or_default();
                if existing
                    .iter()
                    .any(|fy| fy.start_date.format("%Y").to_string() == year_str)
                {
                    self.error = Some(format!("Fiscal year {year} already exists."));
                    self.state = ModalState::Browsing;
                    return FiscalModalAction::None;
                }
                match db.fiscal().create_fiscal_year(1, year) {
                    Ok(_) => {
                        self.state = ModalState::Browsing;
                        self.reload(db);
                        return FiscalModalAction::Mutated(format!(
                            "Fiscal year {year} created with 12 monthly periods."
                        ));
                    }
                    Err(e) => {
                        self.error = Some(format!("Failed to create fiscal year: {e}"));
                        self.state = ModalState::Browsing;
                    }
                }
            }
            _ => {}
        }
        FiscalModalAction::None
    }

    fn start_year_end_review(&mut self, fy_id: FiscalYearId, db: &EntityDb) -> FiscalModalAction {
        match generate_closing_entries(db, fy_id) {
            Err(e) => {
                self.error = Some(format!("Cannot generate closing entries: {e}"));
                FiscalModalAction::None
            }
            Ok(entries) if entries.is_empty() => {
                self.error =
                    Some("No Revenue/Expense activity found — nothing to close.".to_owned());
                FiscalModalAction::None
            }
            Ok(entries) => {
                let preview = build_closing_entry_preview(&entries, db);
                self.state = ModalState::YearEndReview {
                    fy_id,
                    preview,
                    scroll: 0,
                };
                FiscalModalAction::None
            }
        }
    }

    fn handle_confirm_close(&mut self, key: KeyEvent, db: &EntityDb) -> FiscalModalAction {
        let outcome = if let ModalState::ConfirmClose { id, confirm } = &mut self.state {
            match confirm.handle_key(key) {
                ConfirmAction::Confirmed => Some(Ok(*id)),
                ConfirmAction::Cancelled => Some(Err(())),
                ConfirmAction::Pending => None,
            }
        } else {
            None
        };
        match outcome {
            Some(Ok(id)) => {
                self.state = ModalState::Browsing;
                match db.fiscal().close_period(id, &self.entity_name) {
                    Ok(()) => {
                        self.reload(db);
                        FiscalModalAction::Mutated("Period closed.".to_owned())
                    }
                    Err(e) => {
                        self.error = Some(format!("Close failed: {e}"));
                        FiscalModalAction::None
                    }
                }
            }
            Some(Err(())) => {
                self.state = ModalState::Browsing;
                FiscalModalAction::None
            }
            None => FiscalModalAction::None,
        }
    }

    fn handle_confirm_reopen(&mut self, key: KeyEvent, db: &EntityDb) -> FiscalModalAction {
        let outcome = if let ModalState::ConfirmReopen { id, confirm } = &mut self.state {
            match confirm.handle_key(key) {
                ConfirmAction::Confirmed => Some(Ok(*id)),
                ConfirmAction::Cancelled => Some(Err(())),
                ConfirmAction::Pending => None,
            }
        } else {
            None
        };
        match outcome {
            Some(Ok(id)) => {
                self.state = ModalState::Browsing;
                match db.fiscal().reopen_period(id, &self.entity_name) {
                    Ok(()) => {
                        self.reload(db);
                        FiscalModalAction::Mutated("Period reopened.".to_owned())
                    }
                    Err(e) => {
                        self.error = Some(format!("Reopen failed: {e}"));
                        FiscalModalAction::None
                    }
                }
            }
            Some(Err(())) => {
                self.state = ModalState::Browsing;
                FiscalModalAction::None
            }
            None => FiscalModalAction::None,
        }
    }

    fn handle_year_end_review(&mut self, key: KeyEvent, db: &EntityDb) -> FiscalModalAction {
        match key.code {
            KeyCode::Esc => {
                self.state = ModalState::Browsing;
                FiscalModalAction::None
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers == KeyModifiers::NONE => {
                if let ModalState::YearEndReview { scroll, .. } = &mut self.state {
                    *scroll = scroll.saturating_sub(1);
                }
                FiscalModalAction::None
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers == KeyModifiers::NONE => {
                if let ModalState::YearEndReview {
                    scroll, preview, ..
                } = &mut self.state
                {
                    let max = preview.len().saturating_sub(1);
                    if *scroll < max {
                        *scroll += 1;
                    }
                }
                FiscalModalAction::None
            }
            KeyCode::Enter => {
                let fy_id = if let ModalState::YearEndReview { fy_id, .. } = &self.state {
                    *fy_id
                } else {
                    return FiscalModalAction::None;
                };
                self.execute_year_end(fy_id, db)
            }
            _ => FiscalModalAction::None,
        }
    }

    fn execute_year_end(&mut self, fy_id: FiscalYearId, db: &EntityDb) -> FiscalModalAction {
        let entries = match generate_closing_entries(db, fy_id) {
            Ok(e) => e,
            Err(e) => {
                self.error = Some(format!("Failed to generate closing entries: {e}"));
                self.state = ModalState::Browsing;
                return FiscalModalAction::None;
            }
        };

        // Create drafts.
        let mut draft_ids = Vec::new();
        for entry in &entries {
            match db.journals().create_draft(entry) {
                Ok(id) => draft_ids.push(id),
                Err(e) => {
                    self.error = Some(format!("Failed to create closing JE: {e}"));
                    self.state = ModalState::Browsing;
                    return FiscalModalAction::None;
                }
            }
        }

        // Post and mark year closed.
        match execute_year_end_close(db, fy_id, &draft_ids, &self.entity_name) {
            Ok(()) => {
                self.state = ModalState::Browsing;
                self.reload(db);
                FiscalModalAction::Mutated("Year-end close completed.".to_owned())
            }
            Err(e) => {
                self.error = Some(format!("Year-end close failed: {e}"));
                self.state = ModalState::Browsing;
                FiscalModalAction::None
            }
        }
    }

    // ── Rendering ─────────────────────────────────────────────────────────────

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let modal_area = centered_rect(82, 88, area);
        frame.render_widget(Clear, modal_area);

        // Non-list states render and return early; borrow of self.state ends at each arm.
        match &self.state {
            ModalState::AddYear { input } => {
                render_add_year_prompt(frame, modal_area, input);
                return;
            }
            ModalState::ConfirmClose { confirm, .. } => {
                confirm.render(frame, area);
                return;
            }
            ModalState::ConfirmReopen { confirm, .. } => {
                confirm.render(frame, area);
                return;
            }
            ModalState::YearEndReview {
                preview, scroll, ..
            } => {
                render_year_end_review(frame, modal_area, preview, *scroll, self.error.as_deref());
                return;
            }
            ModalState::Browsing => {}
        }
        // Browsing: borrow of self.state released; render_list may take &mut self.
        self.render_list(frame, modal_area);
    }

    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(" Fiscal Period Management ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);

        let items: Vec<ListItem> = self
            .rows
            .iter()
            .map(|row| {
                let style = match (&row.kind, row.is_closed) {
                    (RowKind::Year(_), false) => Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                    (RowKind::Year(_), true) => Style::default().fg(Color::DarkGray),
                    (RowKind::Period(_), false) => Style::default().fg(Color::White),
                    (RowKind::Period(_), true) => Style::default().fg(Color::DarkGray),
                };
                ListItem::new(row.display.as_str()).style(style)
            })
            .collect();

        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, chunks[0], &mut self.list_state);

        let status = if let Some(err) = &self.error {
            Line::from(Span::styled(err.as_str(), Style::default().fg(Color::Red)))
        } else {
            Line::from(Span::styled(
                " a: add year  c: close  o: reopen  y: year-end  Esc: close",
                Style::default().fg(Color::DarkGray),
            ))
        };
        frame.render_widget(Paragraph::new(status), chunks[1]);
    }
}

// ── Free functions ────────────────────────────────────────────────────────────

/// Renders the "add fiscal year" text-input prompt.
fn render_add_year_prompt(frame: &mut Frame, area: Rect, input: &str) {
    use ratatui::{
        style::{Color, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Paragraph},
    };

    let modal = centered_rect(50, 25, area);
    frame.render_widget(Clear, modal);

    let lines = vec![
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  Enter fiscal year (e.g., 2026):",
            Style::default().fg(Color::Gray),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            format!("  > {input}_"),
            Style::default().fg(Color::White),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  Enter: confirm   Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Add Fiscal Year ")
                .style(Style::default().fg(Color::Cyan)),
        ),
        modal,
    );
}

/// Renders the year-end review screen (free function to avoid borrow conflicts).
fn render_year_end_review(
    frame: &mut Frame,
    area: Rect,
    preview: &[String],
    scroll: usize,
    error: Option<&str>,
) {
    let block = Block::default()
        .title(" Year-End Close — Review  (↑↓: scroll   Enter: post entries   Esc: cancel) ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    let lines: Vec<Line> = preview
        .iter()
        .skip(scroll)
        .map(|s| Line::from(s.as_str()))
        .collect();
    frame.render_widget(Paragraph::new(lines), chunks[0]);

    let status_line = if let Some(err) = error {
        Line::from(Span::styled(err, Style::default().fg(Color::Red)))
    } else {
        Line::from(Span::styled(
            " Enter: post closing entries and close year   Esc: cancel",
            Style::default().fg(Color::DarkGray),
        ))
    };
    frame.render_widget(Paragraph::new(status_line), chunks[1]);
}

/// Formats `NewJournalEntry` structs into human-readable preview lines.
fn build_closing_entry_preview(
    entries: &[crate::db::journal_repo::NewJournalEntry],
    db: &EntityDb,
) -> Vec<String> {
    let account_map: HashMap<crate::types::AccountId, (String, String)> = db
        .accounts()
        .list_active()
        .unwrap_or_default()
        .into_iter()
        .map(|a| (a.id, (a.number, a.name)))
        .collect();

    let mut lines = Vec::new();
    for entry in entries {
        lines.push(format!(
            "Date: {}    Memo: {}",
            entry.entry_date,
            entry.memo.as_deref().unwrap_or("(none)")
        ));
        lines.push(format!(
            "  {:<44} {:>12}  {:>12}",
            "Account", "Debit", "Credit"
        ));
        lines.push(format!("  {}", "─".repeat(70)));
        for line in &entry.lines {
            let (num, name) = account_map
                .get(&line.account_id)
                .map(|(n, nm)| (n.as_str(), nm.as_str()))
                .unwrap_or(("?", "Unknown"));
            let debit_str = if line.debit_amount.0 > 0 {
                format!("{}", line.debit_amount)
            } else {
                String::new()
            };
            let credit_str = if line.credit_amount.0 > 0 {
                format!("{}", line.credit_amount)
            } else {
                String::new()
            };
            lines.push(format!(
                "  {:<44} {:>12}  {:>12}",
                format!("{} {}", num, name),
                debit_str,
                credit_str
            ));
        }
        lines.push(String::new());
    }
    lines
}
