use chrono::NaiveDate;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
};

use crate::db::{
    EntityDb,
    account_repo::Account,
    journal_repo::{DateRange, LedgerRow},
};
use crate::tabs::{RecordId, Tab, TabAction, TabId};
use crate::types::{AccountId, AccountType, BalanceDirection, Money, ReconcileState};
use crate::widgets::account_picker::{AccountPicker, PickerAction};
use crate::widgets::centered_rect;

// ── Modal state ───────────────────────────────────────────────────────────────

struct DateFilterState {
    from_str: String,
    to_str: String,
    /// 0 = from-field, 1 = to-field.
    focused: usize,
    error: Option<String>,
}

enum GlModal {
    PickAccount(AccountPicker),
    SetDateRange(DateFilterState),
}

// ── Tab struct ────────────────────────────────────────────────────────────────

pub struct GeneralLedgerTab {
    /// All accounts — kept for the AccountPicker.
    all_accounts: Vec<Account>,
    /// Currently displayed account (None = no account selected yet).
    account: Option<Account>,
    /// Loaded ledger rows for the current account and date range.
    rows: Vec<LedgerRow>,
    table_state: TableState,
    /// Active date range filter (both None = show all).
    date_range: DateRange,
    modal: Option<GlModal>,
}

impl Default for GeneralLedgerTab {
    fn default() -> Self {
        Self::new()
    }
}

impl GeneralLedgerTab {
    pub fn new() -> Self {
        Self {
            all_accounts: Vec::new(),
            account: None,
            rows: Vec::new(),
            table_state: TableState::default(),
            date_range: DateRange::default(),
            modal: None,
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// (Re-)loads ledger rows for the current account and date_range.
    fn load_rows(&mut self, db: &EntityDb) {
        let account_id = match &self.account {
            Some(a) => a.id,
            None => return,
        };
        let dr = self.date_range;
        match db.journals().list_lines_for_account(account_id, Some(dr)) {
            Ok(rows) => {
                self.rows = rows;
                if self.rows.is_empty() {
                    self.table_state.select(None);
                } else {
                    let sel = self
                        .table_state
                        .selected()
                        .unwrap_or(0)
                        .min(self.rows.len() - 1);
                    self.table_state.select(Some(sel));
                }
            }
            Err(e) => {
                tracing::error!(
                    "GL tab: failed to load rows for account {}: {e}",
                    i64::from(account_id)
                );
                self.rows.clear();
                self.table_state.select(None);
            }
        }
    }

    fn scroll_up(&mut self) {
        let cur = self.table_state.selected().unwrap_or(0);
        if cur > 0 {
            self.table_state.select(Some(cur - 1));
        }
    }

    fn scroll_down(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let cur = self.table_state.selected().unwrap_or(0);
        if cur + 1 < self.rows.len() {
            self.table_state.select(Some(cur + 1));
        }
    }

    fn select_account_by_id(&mut self, id: AccountId, db: &EntityDb) {
        match db.accounts().get_by_id(id) {
            Ok(account) => {
                self.account = Some(account);
                self.date_range = DateRange::default();
                self.load_rows(db);
            }
            Err(e) => {
                tracing::error!("GL tab: failed to load account {}: {e}", i64::from(id));
            }
        }
    }

    // ── Modal key handlers ────────────────────────────────────────────────────

    fn handle_picker_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        let picker = match &mut self.modal {
            Some(GlModal::PickAccount(p)) => p,
            _ => return TabAction::None,
        };

        match picker.handle_key(key, &self.all_accounts) {
            PickerAction::Selected(id) => {
                self.modal = None;
                self.select_account_by_id(id, db);
            }
            PickerAction::Cancelled => {
                self.modal = None;
            }
            PickerAction::Pending => {}
        }
        TabAction::None
    }

    fn handle_date_filter_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        match key.code {
            KeyCode::Esc => {
                self.modal = None;
                return TabAction::None;
            }
            KeyCode::Tab | KeyCode::Down => {
                if let Some(GlModal::SetDateRange(s)) = &mut self.modal {
                    s.focused = (s.focused + 1) % 2;
                }
                return TabAction::None;
            }
            KeyCode::BackTab | KeyCode::Up => {
                if let Some(GlModal::SetDateRange(s)) = &mut self.modal {
                    s.focused = (s.focused + 1) % 2;
                }
                return TabAction::None;
            }
            KeyCode::Backspace => {
                if let Some(GlModal::SetDateRange(s)) = &mut self.modal {
                    if s.focused == 0 {
                        s.from_str.pop();
                    } else {
                        s.to_str.pop();
                    }
                }
                return TabAction::None;
            }
            KeyCode::Char(c) => {
                if let Some(GlModal::SetDateRange(s)) = &mut self.modal {
                    if s.focused == 0 {
                        s.from_str.push(c);
                    } else {
                        s.to_str.push(c);
                    }
                }
                return TabAction::None;
            }
            KeyCode::Enter => {
                // On from-field: advance to to-field.
                let on_from =
                    matches!(&self.modal, Some(GlModal::SetDateRange(s)) if s.focused == 0);
                if on_from {
                    if let Some(GlModal::SetDateRange(s)) = &mut self.modal {
                        s.focused = 1;
                    }
                    return TabAction::None;
                }
                // On to-field: fall through to submit logic below.
            }
            _ => return TabAction::None,
        }

        // Submit — clone strings first to release the borrow on self.modal.
        let (from_str, to_str) = match &self.modal {
            Some(GlModal::SetDateRange(s)) => (s.from_str.clone(), s.to_str.clone()),
            _ => return TabAction::None,
        };

        let from = if from_str.is_empty() {
            None
        } else {
            match NaiveDate::parse_from_str(&from_str, "%Y-%m-%d") {
                Ok(d) => Some(d),
                Err(_) => {
                    if let Some(GlModal::SetDateRange(s)) = &mut self.modal {
                        s.error = Some(format!("Invalid date '{}' — use YYYY-MM-DD", from_str));
                        s.focused = 0;
                    }
                    return TabAction::None;
                }
            }
        };

        let to = if to_str.is_empty() {
            None
        } else {
            match NaiveDate::parse_from_str(&to_str, "%Y-%m-%d") {
                Ok(d) => Some(d),
                Err(_) => {
                    if let Some(GlModal::SetDateRange(s)) = &mut self.modal {
                        s.error = Some(format!("Invalid date '{}' — use YYYY-MM-DD", to_str));
                        s.focused = 1;
                    }
                    return TabAction::None;
                }
            }
        };

        self.date_range = DateRange { from, to };
        self.modal = None;
        self.load_rows(db);
        TabAction::None
    }

    // ── Render helpers ────────────────────────────────────────────────────────

    fn render_no_account(&self, frame: &mut Frame, area: Rect) {
        let msg = Line::from(vec![
            Span::styled(
                "No account selected — press ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("p", Style::default().fg(Color::Yellow)),
            Span::styled(" to pick an account.", Style::default().fg(Color::DarkGray)),
        ]);
        frame.render_widget(
            Paragraph::new(msg)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" General Ledger "),
                )
                .alignment(Alignment::Center),
            area,
        );
    }

    fn render_table(&self, frame: &mut Frame, area: Rect) {
        let account_type = self.account.as_ref().map(|a| a.account_type);

        let header = Row::new(vec![
            Cell::from("Date").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("JE#").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Memo").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Debit").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Credit").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("R").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Balance").style(Style::default().add_modifier(Modifier::BOLD)),
        ])
        .style(Style::default().bg(Color::DarkGray));

        let table_rows: Vec<Row> = self
            .rows
            .iter()
            .map(|r| {
                let debit_str = if r.debit.0 > 0 {
                    r.debit.to_string()
                } else {
                    String::new()
                };
                let credit_str = if r.credit.0 > 0 {
                    r.credit.to_string()
                } else {
                    String::new()
                };
                let reconcile = match r.reconcile_state {
                    ReconcileState::Uncleared => " ",
                    ReconcileState::Cleared => "✓",
                    ReconcileState::Reconciled => "✓✓",
                };
                let balance_str = match account_type {
                    Some(at) => natural_balance(r.running_balance, at).to_string(),
                    None => r.running_balance.to_string(),
                };
                let memo = r.memo.as_deref().unwrap_or("");
                Row::new(vec![
                    Cell::from(r.entry_date.to_string()),
                    Cell::from(r.je_number.clone()),
                    Cell::from(memo.to_string()),
                    Cell::from(debit_str),
                    Cell::from(credit_str),
                    Cell::from(reconcile),
                    Cell::from(balance_str),
                ])
            })
            .collect();

        let title = match &self.account {
            Some(a) => format!(" General Ledger: {} {} ", a.number, a.name),
            None => " General Ledger ".to_string(),
        };

        let table = Table::new(
            table_rows,
            [
                Constraint::Length(10), // Date
                Constraint::Length(8),  // JE#
                Constraint::Min(20),    // Memo
                Constraint::Length(12), // Debit
                Constraint::Length(12), // Credit
                Constraint::Length(2),  // R
                Constraint::Length(12), // Balance
            ],
        )
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .row_highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("» ");

        let mut state = self.table_state.clone();
        frame.render_stateful_widget(table, area, &mut state);
    }

    fn render_date_filter_modal(&self, frame: &mut Frame, area: Rect, state: &DateFilterState) {
        let modal_area = centered_rect(52, 40, area);
        frame.render_widget(Clear, modal_area);

        let labels = ["From (YYYY-MM-DD)", "To   (YYYY-MM-DD)"];
        let values = [state.from_str.as_str(), state.to_str.as_str()];

        let mut lines = vec![Line::from(Span::raw(""))];
        for (i, (label, value)) in labels.iter().zip(values.iter()).enumerate() {
            let cursor = if i == state.focused { "█" } else { "" };
            let style = if i == state.focused {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<20} ", label), style),
                Span::raw(*value),
                Span::styled(cursor, Style::default().fg(Color::Yellow)),
            ]));
        }
        if let Some(err) = &state.error {
            lines.push(Line::from(Span::raw("")));
            lines.push(Line::from(Span::styled(
                format!("  {err}"),
                Style::default().fg(Color::Red),
            )));
        }
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "  Tab: next field  Enter: apply  Esc: cancel  (empty = no limit)",
            Style::default().fg(Color::DarkGray),
        )));

        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Filter by Date Range ")
                    .style(Style::default().fg(Color::Cyan)),
            ),
            modal_area,
        );
    }
}

// ── Tab trait ─────────────────────────────────────────────────────────────────

impl Tab for GeneralLedgerTab {
    fn title(&self) -> &str {
        "General Ledger"
    }

    fn hotkey_help(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("↑/↓ or k/j", "Scroll entries"),
            ("p", "Pick account"),
            ("f", "Set date range filter"),
        ]
    }

    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        // Modal dispatch takes priority over normal navigation.
        match &self.modal {
            Some(GlModal::PickAccount(_)) => return self.handle_picker_key(key, db),
            Some(GlModal::SetDateRange(_)) => return self.handle_date_filter_key(key, db),
            None => {}
        }

        if key.modifiers != KeyModifiers::NONE && key.modifiers != KeyModifiers::SHIFT {
            return TabAction::None;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(),
            KeyCode::Enter => {
                if let Some(idx) = self.table_state.selected()
                    && let Some(row) = self.rows.get(idx)
                {
                    return TabAction::NavigateTo(
                        TabId::JournalEntries,
                        RecordId::JournalEntry(row.je_id),
                    );
                }
            }
            KeyCode::Char('p') => {
                let mut picker = AccountPicker::new();
                picker.refresh(&self.all_accounts);
                self.modal = Some(GlModal::PickAccount(picker));
            }
            KeyCode::Char('f') => {
                let from_str = self
                    .date_range
                    .from
                    .map(|d| d.to_string())
                    .unwrap_or_default();
                let to_str = self
                    .date_range
                    .to
                    .map(|d| d.to_string())
                    .unwrap_or_default();
                self.modal = Some(GlModal::SetDateRange(DateFilterState {
                    from_str,
                    to_str,
                    focused: 0,
                    error: None,
                }));
            }
            KeyCode::Esc => {
                // Clear date filter if one is active.
                if self.date_range.from.is_some() || self.date_range.to.is_some() {
                    self.date_range = DateRange::default();
                    self.load_rows(db);
                }
            }
            _ => {}
        }
        TabAction::None
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        // Main area: show table or no-account prompt.
        if self.account.is_none() {
            self.render_no_account(frame, chunks[0]);
        } else {
            self.render_table(frame, chunks[0]);
        }

        // Hint bar.
        let date_info = match (self.date_range.from, self.date_range.to) {
            (None, None) => String::new(),
            (Some(f), None) => format!("  from: {f}"),
            (None, Some(t)) => format!("  to: {t}"),
            (Some(f), Some(t)) => format!("  {f} → {t}"),
        };
        let count = self.rows.len();
        let selected = self.table_state.selected().map(|i| i + 1).unwrap_or(0);
        let hint = Line::from(vec![
            Span::styled(
                " p: pick account  f: filter dates  Esc: clear filter  Enter: open JE  ↑↓/jk: navigate",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(date_info, Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("  [{}/{}]", selected, count),
                Style::default().fg(Color::Gray),
            ),
        ]);
        frame.render_widget(Paragraph::new(hint), chunks[1]);

        // Modal overlay (rendered last so it appears on top).
        if let Some(ref modal) = self.modal {
            match modal {
                GlModal::PickAccount(picker) => {
                    picker.render(frame, area, &self.all_accounts);
                }
                GlModal::SetDateRange(state) => {
                    self.render_date_filter_modal(frame, area, state);
                }
            }
        }
    }

    fn wants_input(&self) -> bool {
        self.modal.is_some()
    }

    fn refresh(&mut self, db: &EntityDb) {
        match db.accounts().list_all() {
            Ok(accounts) => self.all_accounts = accounts,
            Err(e) => tracing::error!("GL tab: failed to load accounts: {e}"),
        }
        self.load_rows(db);
    }

    fn navigate_to(&mut self, record_id: RecordId, db: &EntityDb) {
        if let RecordId::Account(aid) = record_id {
            self.select_account_by_id(aid, db);
        }
    }
}

// ── Free functions ─────────────────────────────────────────────────────────────

/// Returns the running balance in the "natural" direction for the account type.
/// Debit-normal (Asset, Expense): positive = debit balance (unchanged).
/// Credit-normal (Liability, Equity, Revenue): positive = credit balance (negated).
fn natural_balance(balance: Money, account_type: AccountType) -> Money {
    match account_type.normal_balance() {
        BalanceDirection::Debit => balance,
        BalanceDirection::Credit => Money(-balance.0),
    }
}
