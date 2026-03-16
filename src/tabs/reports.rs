//! Reports tab — menu of all 8 reports with parameter entry and file generation.

use std::path::PathBuf;

use chrono::{Local, NaiveDate};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::db::{EntityDb, account_repo::Account};
use crate::reports::{
    Report, ReportParams, account_detail::AccountDetail, ap_aging::ApAging, ar_aging::ArAging,
    balance_sheet::BalanceSheet, cash_flow::CashFlow, fixed_asset_schedule::FixedAssetSchedule,
    income_statement::IncomeStatement, trial_balance::TrialBalance, write_report,
};
use crate::tabs::{RecordId, Tab, TabAction};
use crate::types::AccountId;
use crate::widgets::account_picker::{AccountPicker, PickerAction};
use crate::widgets::centered_rect;

// ── Report descriptors ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum ParamKind {
    /// Single as-of date.
    AsOf,
    /// Date range (start + end).
    DateRange,
    /// Date range + account picker.
    DateRangeWithAccount,
}

struct ReportDescriptor {
    label: &'static str,
    kind: ParamKind,
}

const REPORTS: &[ReportDescriptor] = &[
    ReportDescriptor {
        label: "Trial Balance",
        kind: ParamKind::AsOf,
    },
    ReportDescriptor {
        label: "Balance Sheet",
        kind: ParamKind::AsOf,
    },
    ReportDescriptor {
        label: "Income Statement",
        kind: ParamKind::DateRange,
    },
    ReportDescriptor {
        label: "Cash Flow Statement",
        kind: ParamKind::DateRange,
    },
    ReportDescriptor {
        label: "Account Detail",
        kind: ParamKind::DateRangeWithAccount,
    },
    ReportDescriptor {
        label: "AR Aging",
        kind: ParamKind::AsOf,
    },
    ReportDescriptor {
        label: "AP Aging",
        kind: ParamKind::AsOf,
    },
    ReportDescriptor {
        label: "Fixed Asset Schedule",
        kind: ParamKind::AsOf,
    },
];

// ── Phase / field focus ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    SelectReport,
    ConfigParams,
}

/// Which parameter field is focused in the params form.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ParamField {
    Date1,
    Date2,
    Account,
}

// ── Tab struct ────────────────────────────────────────────────────────────────

pub struct ReportsTab {
    entity_name: String,
    output_dir: PathBuf,
    all_accounts: Vec<Account>,

    // Menu state.
    selected: usize,
    list_state: ListState,
    phase: Phase,

    // Param form state.
    date1_str: String,
    date2_str: String,
    account_id: Option<AccountId>,
    account_label: String,
    focused_field: ParamField,
    account_picker: Option<AccountPicker>,

    // Error message shown below the form.
    error: Option<String>,
}

impl ReportsTab {
    pub fn new(output_dir: PathBuf) -> Self {
        let today = Local::now().date_naive().format("%Y-%m-%d").to_string();
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            entity_name: String::new(),
            output_dir,
            all_accounts: Vec::new(),
            selected: 0,
            list_state,
            phase: Phase::SelectReport,
            date1_str: today.clone(),
            date2_str: today,
            account_id: None,
            account_label: String::new(),
            focused_field: ParamField::Date1,
            account_picker: None,
            error: None,
        }
    }

    pub fn set_entity_name(&mut self, name: &str) {
        self.entity_name = name.to_owned();
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    fn current_kind(&self) -> ParamKind {
        REPORTS[self.selected].kind
    }

    /// Reset param form for the selected report.
    fn reset_params(&mut self) {
        let today = Local::now().date_naive().format("%Y-%m-%d").to_string();
        self.date1_str = today.clone();
        self.date2_str = today;
        self.account_id = None;
        self.account_label = String::new();
        self.focused_field = ParamField::Date1;
        self.account_picker = None;
        self.error = None;
    }

    /// Cycle to the next editable field based on report kind.
    fn advance_field(&mut self) {
        match (self.current_kind(), self.focused_field) {
            (ParamKind::AsOf, _) => {}
            (ParamKind::DateRange, ParamField::Date1) => {
                self.focused_field = ParamField::Date2;
            }
            (ParamKind::DateRange, _) => {}
            (ParamKind::DateRangeWithAccount, ParamField::Date1) => {
                self.focused_field = ParamField::Date2;
            }
            (ParamKind::DateRangeWithAccount, ParamField::Date2) => {
                self.focused_field = ParamField::Account;
            }
            (ParamKind::DateRangeWithAccount, ParamField::Account) => {}
        }
    }

    /// Attempt to generate the report. Returns a message to show, or an error.
    fn generate_report(&mut self, db: &EntityDb) -> Result<String, String> {
        let date1 = NaiveDate::parse_from_str(&self.date1_str, "%Y-%m-%d")
            .map_err(|_| format!("Invalid date: '{}'", self.date1_str))?;
        let date2 = if self.current_kind() != ParamKind::AsOf {
            Some(
                NaiveDate::parse_from_str(&self.date2_str, "%Y-%m-%d")
                    .map_err(|_| format!("Invalid end date: '{}'", self.date2_str))?,
            )
        } else {
            None
        };

        if let Some(end) = date2
            && end < date1
        {
            return Err("End date must be on or after start date.".to_owned());
        }

        if self.current_kind() == ParamKind::DateRangeWithAccount && self.account_id.is_none() {
            return Err("Please select an account.".to_owned());
        }

        let params = ReportParams {
            entity_name: self.entity_name.clone(),
            as_of_date: if self.current_kind() == ParamKind::AsOf {
                Some(date1)
            } else {
                None
            },
            date_range: date2.map(|end| (date1, end)),
            account_id: self.account_id,
        };

        let report: Box<dyn Report> = match self.selected {
            0 => Box::new(TrialBalance),
            1 => Box::new(BalanceSheet),
            2 => Box::new(IncomeStatement),
            3 => Box::new(CashFlow),
            4 => Box::new(AccountDetail),
            5 => Box::new(ArAging),
            6 => Box::new(ApAging),
            7 => Box::new(FixedAssetSchedule),
            _ => unreachable!(),
        };

        let content = report
            .generate(db, &params)
            .map_err(|e| format!("Report error: {e}"))?;

        let path = write_report(&content, report.name(), &self.output_dir)
            .map_err(|e| format!("Write error: {e}"))?;

        Ok(format!("Saved: {}", path.display()))
    }

    // ── Key handling helpers ──────────────────────────────────────────────────

    fn handle_select_phase(&mut self, key: KeyEvent) -> TabAction {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected > 0 {
                    self.selected -= 1;
                    self.list_state.select(Some(self.selected));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected + 1 < REPORTS.len() {
                    self.selected += 1;
                    self.list_state.select(Some(self.selected));
                }
            }
            KeyCode::Enter => {
                self.reset_params();
                self.phase = Phase::ConfigParams;
            }
            _ => {}
        }
        TabAction::None
    }

    fn handle_params_phase(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        // If account picker is open, route all keys to it.
        if let Some(ref mut picker) = self.account_picker {
            match picker.handle_key(key, &self.all_accounts) {
                PickerAction::Selected(id) => {
                    self.account_id = Some(id);
                    self.account_label = self
                        .all_accounts
                        .iter()
                        .find(|a| a.id == id)
                        .map(|a| format!("{} {}", a.number, a.name))
                        .unwrap_or_default();
                    self.account_picker = None;
                }
                PickerAction::Cancelled => {
                    self.account_picker = None;
                }
                PickerAction::Pending => {}
            }
            return TabAction::None;
        }

        match key.code {
            KeyCode::Esc => {
                self.phase = Phase::SelectReport;
                self.error = None;
            }

            KeyCode::Tab => {
                self.advance_field();
            }

            KeyCode::Backspace => {
                match self.focused_field {
                    ParamField::Date1 => {
                        self.date1_str.pop();
                    }
                    ParamField::Date2 => {
                        self.date2_str.pop();
                    }
                    ParamField::Account => {
                        // Clear account selection.
                        self.account_id = None;
                        self.account_label.clear();
                    }
                }
                self.error = None;
            }

            KeyCode::Char(c) => {
                self.error = None;
                match self.focused_field {
                    ParamField::Date1 => {
                        self.date1_str.push(c);
                    }
                    ParamField::Date2 => {
                        self.date2_str.push(c);
                    }
                    ParamField::Account => {
                        // Open picker on any character.
                        let mut picker = AccountPicker::new();
                        picker.handle_key(
                            crossterm::event::KeyEvent::new(
                                KeyCode::Char(c),
                                crossterm::event::KeyModifiers::NONE,
                            ),
                            &self.all_accounts,
                        );
                        self.account_picker = Some(picker);
                    }
                }
            }

            KeyCode::Enter => {
                if self.focused_field == ParamField::Account
                    && self.current_kind() == ParamKind::DateRangeWithAccount
                {
                    // Enter on account field opens the picker.
                    let mut picker = AccountPicker::new();
                    picker.refresh(&self.all_accounts);
                    self.account_picker = Some(picker);
                    return TabAction::None;
                }

                // Last field or generate key: run the report.
                match self.generate_report(db) {
                    Ok(msg) => {
                        self.phase = Phase::SelectReport;
                        self.error = None;
                        return TabAction::ShowMessage(msg);
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
            }

            KeyCode::F(9) => {
                // F9 = generate regardless of focused field.
                match self.generate_report(db) {
                    Ok(msg) => {
                        self.phase = Phase::SelectReport;
                        self.error = None;
                        return TabAction::ShowMessage(msg);
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
            }

            _ => {}
        }
        TabAction::None
    }

    // ── Rendering ─────────────────────────────────────────────────────────────

    fn render_menu(&self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = REPORTS
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let style = if i == self.selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };
                ListItem::new(Line::from(vec![Span::styled(
                    format!("  {:2}. {}", i + 1, r.label),
                    style,
                )]))
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Reports (↑↓ select, Enter configure) "),
            )
            .highlight_style(Style::default().fg(Color::Yellow).bg(Color::DarkGray));

        frame.render_stateful_widget(list, area, &mut self.list_state.clone());
    }

    fn render_params(&self, frame: &mut Frame, area: Rect) {
        let kind = self.current_kind();
        let report_label = REPORTS[self.selected].label;

        // Split into form area + generate hint.
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        let block = Block::default().borders(Borders::ALL).title(format!(
            " {} Parameters (Tab to advance, Enter/F9 to generate, Esc back) ",
            report_label
        ));

        let inner = block.inner(chunks[0]);
        frame.render_widget(block, chunks[0]);

        // Build rows: label + value for each field.
        let mut field_lines: Vec<Line> = Vec::new();

        let date1_label = if kind == ParamKind::AsOf {
            "As-of Date (YYYY-MM-DD)"
        } else {
            "Start Date (YYYY-MM-DD)"
        };
        field_lines.push(field_line(
            date1_label,
            &self.date1_str,
            self.focused_field == ParamField::Date1,
        ));

        if kind != ParamKind::AsOf {
            field_lines.push(field_line(
                "End Date   (YYYY-MM-DD)",
                &self.date2_str,
                self.focused_field == ParamField::Date2,
            ));
        }

        if kind == ParamKind::DateRangeWithAccount {
            let acct_val = if self.account_label.is_empty() {
                "(none — press Enter to pick)"
            } else {
                &self.account_label
            };
            field_lines.push(field_line(
                "Account",
                acct_val,
                self.focused_field == ParamField::Account,
            ));
        }

        // Error line.
        if let Some(ref err) = self.error {
            field_lines.push(Line::from(vec![Span::styled(
                format!("  Error: {err}"),
                Style::default().fg(Color::Red),
            )]));
        }

        let form_para = Paragraph::new(field_lines);

        // Center the form vertically in the inner block.
        let form_height = inner.height.min(10);
        let top_pad = inner.height.saturating_sub(form_height) / 2;
        let form_area = Rect {
            x: inner.x,
            y: inner.y + top_pad,
            width: inner.width,
            height: form_height,
        };
        frame.render_widget(form_para, form_area);

        // Hint at bottom.
        frame.render_widget(
            Paragraph::new(" F9 to generate report ")
                .alignment(Alignment::Right)
                .style(Style::default().fg(Color::DarkGray)),
            chunks[1],
        );

        // Account picker overlay.
        if let Some(ref picker) = self.account_picker {
            let popup_area = centered_rect(70, 60, area);
            frame.render_widget(Clear, popup_area);
            picker.render(frame, popup_area, &self.all_accounts);
        }
    }
}

/// Build a single labeled-field line.
fn field_line<'a>(label: &'a str, value: &'a str, focused: bool) -> Line<'a> {
    let label_style = if focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let value_style = if focused {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };
    Line::from(vec![
        Span::styled(format!("  {label}: "), label_style),
        Span::styled(value.to_owned(), value_style),
    ])
}

// ── Tab impl ──────────────────────────────────────────────────────────────────

impl Tab for ReportsTab {
    fn title(&self) -> &str {
        "Reports"
    }

    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        match self.phase {
            Phase::SelectReport => self.handle_select_phase(key),
            Phase::ConfigParams => self.handle_params_phase(key, db),
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        match self.phase {
            Phase::SelectReport => self.render_menu(frame, area),
            Phase::ConfigParams => self.render_params(frame, area),
        }
    }

    fn refresh(&mut self, db: &EntityDb) {
        match db.accounts().list_all() {
            Ok(accounts) => self.all_accounts = accounts,
            Err(_) => self.all_accounts.clear(),
        }
    }

    fn wants_input(&self) -> bool {
        self.phase == Phase::ConfigParams
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
        let db = entity_db_from_conn(conn);
        db.fiscal().create_fiscal_year(1, 2026).expect("fy");
        db
    }

    fn make_tab() -> ReportsTab {
        let mut tab = ReportsTab::new(std::path::PathBuf::from("/tmp/reports"));
        tab.set_entity_name("Test Entity");
        tab
    }

    #[test]
    fn reports_tab_title() {
        let tab = make_tab();
        assert_eq!(tab.title(), "Reports");
    }

    #[test]
    fn reports_tab_initial_phase_is_select() {
        let tab = make_tab();
        assert_eq!(tab.phase, Phase::SelectReport);
    }

    #[test]
    fn reports_tab_enter_moves_to_config_phase() {
        let db = make_db();
        let mut tab = make_tab();
        tab.handle_key(
            KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE),
            &db,
        );
        assert_eq!(tab.phase, Phase::ConfigParams);
    }

    #[test]
    fn reports_tab_esc_in_config_returns_to_select() {
        let db = make_db();
        let mut tab = make_tab();
        tab.phase = Phase::ConfigParams;
        tab.handle_key(
            KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE),
            &db,
        );
        assert_eq!(tab.phase, Phase::SelectReport);
    }

    #[test]
    fn reports_tab_down_advances_selection() {
        let db = make_db();
        let mut tab = make_tab();
        assert_eq!(tab.selected, 0);
        tab.handle_key(
            KeyEvent::new(KeyCode::Down, crossterm::event::KeyModifiers::NONE),
            &db,
        );
        assert_eq!(tab.selected, 1);
    }

    #[test]
    fn reports_tab_wants_input_in_config_phase() {
        let mut tab = make_tab();
        assert!(!tab.wants_input());
        tab.phase = Phase::ConfigParams;
        assert!(tab.wants_input());
    }

    #[test]
    fn reports_tab_tab_key_advances_field_in_date_range_mode() {
        let db = make_db();
        let mut tab = make_tab();
        // Select Income Statement (index 2) which is DateRange.
        tab.selected = 2;
        tab.phase = Phase::ConfigParams;
        tab.focused_field = ParamField::Date1;
        tab.handle_key(
            KeyEvent::new(KeyCode::Tab, crossterm::event::KeyModifiers::NONE),
            &db,
        );
        assert_eq!(tab.focused_field, ParamField::Date2);
    }

    #[test]
    fn reports_tab_generate_trial_balance() {
        let db = make_db();
        let mut tab = make_tab();
        tab.refresh(&db);
        // Select Trial Balance (index 0).
        tab.selected = 0;
        tab.phase = Phase::ConfigParams;
        tab.date1_str = "2026-03-31".to_owned();
        // F9 to generate.
        let action = tab.handle_key(
            KeyEvent::new(KeyCode::F(9), crossterm::event::KeyModifiers::NONE),
            &db,
        );
        // Should return ShowMessage with file path.
        assert!(
            matches!(action, TabAction::ShowMessage(ref msg) if msg.contains("Saved:")),
            "expected ShowMessage with Saved:, got {:?}",
            action
        );
        // Phase resets to SelectReport.
        assert_eq!(tab.phase, Phase::SelectReport);
    }

    #[test]
    fn reports_tab_generate_with_invalid_date_shows_error() {
        let db = make_db();
        let mut tab = make_tab();
        tab.selected = 0;
        tab.phase = Phase::ConfigParams;
        tab.date1_str = "not-a-date".to_owned();
        tab.handle_key(
            KeyEvent::new(KeyCode::F(9), crossterm::event::KeyModifiers::NONE),
            &db,
        );
        assert!(tab.error.is_some(), "should show an error for invalid date");
        assert_eq!(
            tab.phase,
            Phase::ConfigParams,
            "should stay in params phase"
        );
    }
}
