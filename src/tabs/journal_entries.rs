use std::collections::HashMap;

use chrono::NaiveDate;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
};

use crate::db::{
    EntityDb,
    account_repo::Account,
    journal_repo::{JournalEntry, JournalEntryLine, JournalFilter, NewJournalEntry},
};
use crate::services::journal::{post_journal_entry, reverse_journal_entry};
use crate::tabs::{RecordId, Tab, TabAction, TabId};
use crate::types::{
    AccountId, EntryFrequency, JournalEntryId, JournalEntryStatus, Money, ReconcileState,
};
use crate::widgets::{
    JeForm, centered_rect,
    confirmation::{ConfirmAction, Confirmation},
    je_form::JeFormAction,
};

// ── Status filter cycle ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum StatusFilter {
    All,
    Draft,
    Posted,
}

impl StatusFilter {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Draft,
            Self::Draft => Self::Posted,
            Self::Posted => Self::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Draft => "Draft",
            Self::Posted => "Posted",
        }
    }

    fn to_filter(self) -> JournalFilter {
        JournalFilter {
            status: match self {
                Self::All => None,
                Self::Draft => Some(JournalEntryStatus::Draft),
                Self::Posted => Some(JournalEntryStatus::Posted),
            },
            from_date: None,
            to_date: None,
        }
    }
}

// ── Detail panel ─────────────────────────────────────────────────────────────

struct DetailState {
    lines: Vec<JournalEntryLine>,
    /// Index of the focused line (for reconcile toggle).
    focused_line: usize,
}

// ── Modal state machine ───────────────────────────────────────────────────────

enum Modal {
    /// JE form for creating new draft entries.
    NewEntry(JeForm),
    /// Confirmation to post a Draft entry.
    ConfirmPost {
        confirm: Confirmation,
        je_id: JournalEntryId,
    },
    /// Date input for the reversal entry date.
    ReverseDate {
        date_input: String,
        je_id: JournalEntryId,
        je_number: String,
        error: Option<String>,
    },
    /// Confirmation to proceed with reversal.
    ConfirmReverse {
        confirm: Confirmation,
        je_id: JournalEntryId,
        reversal_date: NaiveDate,
    },
    /// Form to set up a recurring template from a posted JE.
    /// Field 0 = start_date, Field 1 = frequency.
    RecurringSetup {
        je_id: JournalEntryId,
        je_number: String,
        start_date_str: String,
        frequency: EntryFrequency,
        /// 0 = start_date field active, 1 = frequency field active.
        focused_field: usize,
        error: Option<String>,
    },
}

// ── Tab ───────────────────────────────────────────────────────────────────────

pub struct JournalEntriesTab {
    entries: Vec<JournalEntry>,
    table_state: TableState,
    status_filter: StatusFilter,
    detail: Option<DetailState>,
    /// Full account list (including inactive) for name resolution in detail view.
    accounts: Vec<Account>,
    modal: Option<Modal>,
    entity_name: String,
    /// Envelope available balance per account (Earmarked − GL Balance for current FY).
    /// Accounts without allocations are absent from the map.
    envelope_avail: HashMap<AccountId, Money>,
}

impl Default for JournalEntriesTab {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            table_state: TableState::default(),
            status_filter: StatusFilter::All,
            detail: None,
            accounts: Vec::new(),
            modal: None,
            entity_name: String::new(),
            envelope_avail: HashMap::new(),
        }
    }
}

impl JournalEntriesTab {
    pub fn new() -> Self {
        Self::default()
    }

    /// Called from `EntityContext::new` to give this tab the entity name for audit logging.
    pub fn set_entity_name(&mut self, name: &str) {
        self.entity_name = name.to_string();
    }

    /// Computes available envelope balance (Earmarked − GL Balance) for each allocated
    /// account in the current fiscal year. Used as read-only context in the JE form.
    fn reload_envelope_avail(&mut self, db: &EntityDb) {
        let mut avail = HashMap::new();
        let allocations = match db.envelopes().get_all_allocations() {
            Ok(a) => a,
            Err(e) => {
                tracing::error!("Failed to load envelope allocations: {e}");
                self.envelope_avail = avail;
                return;
            }
        };

        // Find current fiscal year for date-range filtering.
        let today = chrono::Local::now().date_naive();
        let fy = db.fiscal().list_fiscal_years().ok().and_then(|years| {
            years
                .into_iter()
                .find(|y| today >= y.start_date && today <= y.end_date)
        });

        for alloc in &allocations {
            let earmarked = match &fy {
                Some(fy) => db
                    .envelopes()
                    .get_balance_for_date_range(alloc.account_id, fy.start_date, fy.end_date)
                    .unwrap_or(Money(0)),
                None => db
                    .envelopes()
                    .get_balance(alloc.account_id)
                    .unwrap_or(Money(0)),
            };
            let gl_balance = match &fy {
                Some(fy) => db
                    .accounts()
                    .get_balance_for_date_range(alloc.account_id, fy.start_date, fy.end_date)
                    .unwrap_or(Money(0)),
                None => db
                    .accounts()
                    .get_balance(alloc.account_id)
                    .unwrap_or(Money(0)),
            };
            avail.insert(alloc.account_id, Money(earmarked.0 - gl_balance.0));
        }
        self.envelope_avail = avail;
    }

    fn selected_entry(&self) -> Option<&JournalEntry> {
        self.table_state
            .selected()
            .and_then(|i| self.entries.get(i))
    }

    fn account_display(&self, id: AccountId) -> String {
        self.accounts
            .iter()
            .find(|a| a.id == id)
            .map(|a| format!("{} {}", a.number, a.name))
            .unwrap_or_else(|| format!("Account #{}", i64::from(id)))
    }

    fn scroll_up(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let i = self
            .table_state
            .selected()
            .map(|i| i.saturating_sub(1))
            .unwrap_or(0);
        self.table_state.select(Some(i));
    }

    fn scroll_down(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() - 1;
        let i = self
            .table_state
            .selected()
            .map(|i| (i + 1).min(last))
            .unwrap_or(0);
        self.table_state.select(Some(i));
    }

    fn open_detail(&mut self, db: &EntityDb) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let id = entry.id;
        match db.journals().get_with_lines(id) {
            Ok((_, lines)) => {
                self.detail = Some(DetailState {
                    lines,
                    focused_line: 0,
                })
            }
            Err(e) => tracing::error!("Failed to load JE lines for {}: {e}", i64::from(id)),
        }
    }

    fn close_detail(&mut self) {
        self.detail = None;
    }

    fn scroll_to(&mut self, id: JournalEntryId) {
        if let Some(pos) = self.entries.iter().position(|e| e.id == id) {
            self.table_state.select(Some(pos));
            self.detail = None;
        }
    }

    fn detail_line_up(&mut self) {
        if let Some(ref mut d) = self.detail
            && d.focused_line > 0
        {
            d.focused_line -= 1;
        }
    }

    fn detail_line_down(&mut self) {
        if let Some(ref mut d) = self.detail {
            let max = d.lines.len().saturating_sub(1);
            if d.focused_line < max {
                d.focused_line += 1;
            }
        }
    }

    fn toggle_reconcile(&mut self, db: &EntityDb) -> TabAction {
        let Some(d) = &self.detail else {
            return TabAction::None;
        };
        let Some(line) = d.lines.get(d.focused_line) else {
            return TabAction::None;
        };
        let Some(entry) = self.selected_entry() else {
            return TabAction::None;
        };

        // Only allow on Posted entries.
        if entry.status != JournalEntryStatus::Posted {
            return TabAction::ShowMessage(
                "Reconcile state can only be changed on Posted entries.".to_string(),
            );
        }

        // Block changes to Reconciled lines.
        if line.reconcile_state == ReconcileState::Reconciled {
            return TabAction::ShowMessage("Cannot modify reconciled entries.".to_string());
        }

        // Block changes if fiscal period is closed.
        match db.fiscal().get_period_by_id(entry.fiscal_period_id) {
            Ok(period) if period.is_closed => {
                return TabAction::ShowMessage(
                    "Cannot modify entries in a closed fiscal period.".to_string(),
                );
            }
            Err(e) => {
                return TabAction::ShowMessage(format!("Failed to check fiscal period: {e}"));
            }
            Ok(_) => {}
        }

        let new_state = match line.reconcile_state {
            ReconcileState::Uncleared => ReconcileState::Cleared,
            ReconcileState::Cleared => ReconcileState::Uncleared,
            ReconcileState::Reconciled => unreachable!("already checked above"),
        };
        let line_id = line.id;

        match db.journals().update_reconcile_state(line_id, new_state) {
            Ok(()) => TabAction::RefreshData,
            Err(e) => TabAction::ShowMessage(format!("Failed to update reconcile state: {e}")),
        }
    }

    // ── Modal key handlers ────────────────────────────────────────────────────

    fn handle_new_entry_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        // Take the modal out so we can borrow self.accounts freely.
        let Some(Modal::NewEntry(mut form)) = self.modal.take() else {
            return TabAction::None;
        };
        let action = form.handle_key(key, &self.accounts);
        match action {
            JeFormAction::Cancelled => {
                // modal stays None (already taken)
            }
            JeFormAction::Submitted(output) => {
                match db.fiscal().get_period_for_date(output.entry_date) {
                    Err(e) => {
                        return TabAction::ShowMessage(format!("No fiscal period for date: {e}"));
                    }
                    Ok(period) => {
                        let new_je = NewJournalEntry {
                            entry_date: output.entry_date,
                            memo: output.memo,
                            fiscal_period_id: period.id,
                            reversal_of_je_id: None,
                            lines: output.lines,
                        };
                        return match db.journals().create_draft(&new_je) {
                            Ok(_) => TabAction::RefreshData,
                            Err(e) => TabAction::ShowMessage(format!("Failed to save draft: {e}")),
                        };
                    }
                }
            }
            JeFormAction::Pending => {
                // Restore modal.
                self.modal = Some(Modal::NewEntry(form));
            }
        }
        TabAction::None
    }

    fn handle_confirm_post_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        let Some(Modal::ConfirmPost { mut confirm, je_id }) = self.modal.take() else {
            return TabAction::None;
        };
        match confirm.handle_key(key) {
            ConfirmAction::Confirmed => {
                let entity_name = self.entity_name.clone();
                match post_journal_entry(db, je_id, &entity_name) {
                    Ok(()) => TabAction::RefreshData,
                    Err(e) => TabAction::ShowMessage(format!("Post failed: {e}")),
                }
            }
            ConfirmAction::Cancelled => TabAction::None,
            ConfirmAction::Pending => {
                self.modal = Some(Modal::ConfirmPost { confirm, je_id });
                TabAction::None
            }
        }
    }

    fn handle_reverse_date_key(&mut self, key: KeyEvent) -> TabAction {
        let Some(Modal::ReverseDate {
            mut date_input,
            je_id,
            je_number,
            mut error,
        }) = self.modal.take()
        else {
            return TabAction::None;
        };

        match key.code {
            KeyCode::Esc => {
                // modal stays None
            }
            KeyCode::Backspace => {
                date_input.pop();
                error = None;
                self.modal = Some(Modal::ReverseDate {
                    date_input,
                    je_id,
                    je_number,
                    error,
                });
            }
            KeyCode::Char(c) if date_input.len() < 10 => {
                date_input.push(c);
                error = None;
                self.modal = Some(Modal::ReverseDate {
                    date_input,
                    je_id,
                    je_number,
                    error,
                });
            }
            KeyCode::Enter => match NaiveDate::parse_from_str(&date_input, "%Y-%m-%d") {
                Err(_) => {
                    error = Some(format!("Invalid date '{}'. Use YYYY-MM-DD.", date_input));
                    self.modal = Some(Modal::ReverseDate {
                        date_input,
                        je_id,
                        je_number,
                        error,
                    });
                }
                Ok(reversal_date) => {
                    let msg = format!("Reverse {} on {}?", je_number, reversal_date);
                    self.modal = Some(Modal::ConfirmReverse {
                        confirm: Confirmation::new(msg),
                        je_id,
                        reversal_date,
                    });
                }
            },
            _ => {
                self.modal = Some(Modal::ReverseDate {
                    date_input,
                    je_id,
                    je_number,
                    error,
                });
            }
        }
        TabAction::None
    }

    fn handle_confirm_reverse_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        let Some(Modal::ConfirmReverse {
            mut confirm,
            je_id,
            reversal_date,
        }) = self.modal.take()
        else {
            return TabAction::None;
        };
        match confirm.handle_key(key) {
            ConfirmAction::Confirmed => {
                let entity_name = self.entity_name.clone();
                match reverse_journal_entry(db, je_id, reversal_date, &entity_name) {
                    Ok(_new_id) => TabAction::RefreshData,
                    Err(e) => TabAction::ShowMessage(format!("Reverse failed: {e}")),
                }
            }
            ConfirmAction::Cancelled => TabAction::None,
            ConfirmAction::Pending => {
                self.modal = Some(Modal::ConfirmReverse {
                    confirm,
                    je_id,
                    reversal_date,
                });
                TabAction::None
            }
        }
    }

    fn handle_recurring_setup_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        let Some(Modal::RecurringSetup {
            je_id,
            start_date_str,
            frequency,
            focused_field,
            error,
            ..
        }) = self.modal.take()
        else {
            return TabAction::None;
        };

        match key.code {
            KeyCode::Esc => {
                // modal stays None (already taken)
                TabAction::None
            }
            KeyCode::Tab => {
                let next_field = if focused_field == 0 { 1 } else { 0 };
                self.modal = Some(Modal::RecurringSetup {
                    je_id,
                    je_number: String::new(),
                    start_date_str,
                    frequency,
                    focused_field: next_field,
                    error,
                });
                TabAction::None
            }
            KeyCode::Left | KeyCode::Right if focused_field == 1 => {
                let next_freq = match frequency {
                    EntryFrequency::Monthly => EntryFrequency::Quarterly,
                    EntryFrequency::Quarterly => EntryFrequency::Annually,
                    EntryFrequency::Annually => EntryFrequency::Monthly,
                };
                self.modal = Some(Modal::RecurringSetup {
                    je_id,
                    je_number: String::new(),
                    start_date_str,
                    frequency: next_freq,
                    focused_field,
                    error,
                });
                TabAction::None
            }
            KeyCode::Backspace if focused_field == 0 => {
                let mut s = start_date_str;
                s.pop();
                self.modal = Some(Modal::RecurringSetup {
                    je_id,
                    je_number: String::new(),
                    start_date_str: s,
                    frequency,
                    focused_field,
                    error: None,
                });
                TabAction::None
            }
            KeyCode::Char(c) if focused_field == 0 => {
                let mut s = start_date_str;
                s.push(c);
                self.modal = Some(Modal::RecurringSetup {
                    je_id,
                    je_number: String::new(),
                    start_date_str: s,
                    frequency,
                    focused_field,
                    error: None,
                });
                TabAction::None
            }
            KeyCode::Enter => match NaiveDate::parse_from_str(&start_date_str, "%Y-%m-%d") {
                Err(_) => {
                    let err_msg = format!("Invalid date: '{start_date_str}'");
                    self.modal = Some(Modal::RecurringSetup {
                        je_id,
                        je_number: String::new(),
                        start_date_str,
                        frequency,
                        focused_field,
                        error: Some(err_msg),
                    });
                    TabAction::None
                }
                Ok(start_date) => {
                    match db.recurring().create_template(je_id, frequency, start_date) {
                        Ok(template_id) => TabAction::ShowMessage(format!(
                            "Recurring template #{} created ({} starting {})",
                            i64::from(template_id),
                            frequency,
                            start_date
                        )),
                        Err(e) => {
                            self.modal = Some(Modal::RecurringSetup {
                                je_id,
                                je_number: String::new(),
                                start_date_str,
                                frequency,
                                focused_field,
                                error: Some(format!("{e}")),
                            });
                            TabAction::None
                        }
                    }
                }
            },
            _ => {
                self.modal = Some(Modal::RecurringSetup {
                    je_id,
                    je_number: String::new(),
                    start_date_str,
                    frequency,
                    focused_field,
                    error,
                });
                TabAction::None
            }
        }
    }

    // ── Render helpers ────────────────────────────────────────────────────────

    fn render_list(&self, frame: &mut Frame, area: Rect) {
        let title = format!(
            " Journal Entries  [n] new  [p] post  [r] reverse  [f] filter: {}  ↑↓: scroll  Enter: detail ",
            self.status_filter.label()
        );

        let header = Row::new(vec![
            Cell::from("JE Number").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Date").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Memo").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Status").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Rev").style(Style::default().add_modifier(Modifier::BOLD)),
        ]);

        let rows: Vec<Row> = self
            .entries
            .iter()
            .map(|e| {
                let status_style = match e.status {
                    JournalEntryStatus::Draft => Style::default().fg(Color::Yellow),
                    JournalEntryStatus::Posted => Style::default().fg(Color::Green),
                };
                let rev_str = if e.is_reversed { "✓" } else { "" };
                let memo = e.memo.as_deref().unwrap_or("").to_string();

                Row::new(vec![
                    Cell::from(e.je_number.clone()),
                    Cell::from(e.entry_date.to_string()),
                    Cell::from(memo),
                    Cell::from(e.status.to_string()).style(status_style),
                    Cell::from(rev_str),
                ])
            })
            .collect();

        let widths = [
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Min(20),
            Constraint::Length(8),
            Constraint::Length(4),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().title(title).borders(Borders::ALL))
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        let mut state = self.table_state.clone();
        frame.render_stateful_widget(table, area, &mut state);
    }

    fn render_detail(&self, frame: &mut Frame, area: Rect) {
        let Some(d) = &self.detail else {
            return;
        };
        let Some(entry) = self.selected_entry() else {
            return;
        };

        let title = format!(
            " {} — {} line(s)  ↑↓: line  [c] Cleared  [g] GL  Esc: close ",
            entry.je_number,
            d.lines.len()
        );

        if let Some(memo) = &entry.memo {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(3)])
                .split(area);

            frame.render_widget(
                Paragraph::new(format!("  Memo: {memo}"))
                    .style(Style::default().fg(Color::DarkGray)),
                chunks[0],
            );
            self.render_lines_table(frame, chunks[1], d, &title);
        } else {
            self.render_lines_table(frame, area, d, &title);
        }
    }

    fn render_lines_table(&self, frame: &mut Frame, area: Rect, d: &DetailState, title: &str) {
        let header = Row::new(vec![
            Cell::from("#").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Account").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Debit").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Credit").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Note").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Rec").style(Style::default().add_modifier(Modifier::BOLD)),
        ]);

        let rows: Vec<Row> = d
            .lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let acct = self.account_display(line.account_id);
                let rec = match line.reconcile_state {
                    ReconcileState::Uncleared => "",
                    ReconcileState::Cleared => "✓",
                    ReconcileState::Reconciled => "✓✓",
                };
                let debit = if line.debit_amount.is_zero() {
                    String::new()
                } else {
                    line.debit_amount.to_string()
                };
                let credit = if line.credit_amount.is_zero() {
                    String::new()
                } else {
                    line.credit_amount.to_string()
                };
                let note = line.line_memo.as_deref().unwrap_or("").to_string();
                let row_style = if i == d.focused_line {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };

                Row::new(vec![
                    Cell::from(format!("{}", i + 1)),
                    Cell::from(acct),
                    Cell::from(debit),
                    Cell::from(credit),
                    Cell::from(note),
                    Cell::from(rec),
                ])
                .style(row_style)
            })
            .collect();

        let widths = [
            Constraint::Length(3),
            Constraint::Percentage(35),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Min(10),
            Constraint::Length(5),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().title(title).borders(Borders::ALL));

        frame.render_widget(table, area);
    }

    fn render_modal(&self, frame: &mut Frame, area: Rect) {
        let Some(modal) = &self.modal else {
            return;
        };
        match modal {
            Modal::NewEntry(form) => {
                let popup = centered_rect(90, 80, area);
                frame.render_widget(Clear, popup);
                form.render(frame, popup, &self.accounts, &self.envelope_avail);
            }
            Modal::ConfirmPost { confirm, .. } => {
                confirm.render(frame, area);
            }
            Modal::ReverseDate {
                date_input,
                je_number,
                error,
                ..
            } => {
                let popup = centered_rect(44, 20, area);
                frame.render_widget(Clear, popup);
                let title = format!(" Reversal date for {} ", je_number);
                let error_line = error.as_deref().unwrap_or("");
                let content = format!(
                    "  Date (YYYY-MM-DD): {date_input}_\n\n  {error_line}\n  Enter: continue  Esc: cancel"
                );
                frame.render_widget(
                    Paragraph::new(content).block(
                        Block::default()
                            .title(title)
                            .borders(Borders::ALL)
                            .style(Style::default().fg(Color::White)),
                    ),
                    popup,
                );
            }
            Modal::ConfirmReverse { confirm, .. } => {
                confirm.render(frame, area);
            }
            Modal::RecurringSetup {
                je_number,
                start_date_str,
                frequency,
                focused_field,
                error,
                ..
            } => {
                let popup = centered_rect(50, 30, area);
                frame.render_widget(Clear, popup);
                let date_indicator = if *focused_field == 0 { ">" } else { " " };
                let freq_indicator = if *focused_field == 1 { ">" } else { " " };
                let error_line = error.as_deref().unwrap_or("");
                let content = format!(
                    "\n  {date_indicator} Start Date (YYYY-MM-DD): {start_date_str}_\n\n  {freq_indicator} Frequency: {frequency}  (←/→ to change)\n\n  {error_line}\n\n  Tab: switch field   Enter: create   Esc: cancel"
                );
                frame.render_widget(
                    Paragraph::new(content).block(
                        Block::default()
                            .title(format!(" Recurring template for {} ", je_number))
                            .borders(Borders::ALL)
                            .style(Style::default().fg(Color::Cyan)),
                    ),
                    popup,
                );
            }
        }
    }
}

impl Tab for JournalEntriesTab {
    fn title(&self) -> &str {
        "Journal Entries"
    }

    fn hotkey_help(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("↑/↓ or k/j", "Navigate"),
            ("n", "New journal entry"),
            ("p", "Post selected entry"),
            ("r", "Reverse posted entry"),
            ("c", "Create inter-entity entry"),
            ("g", "Go to General Ledger"),
            ("f", "Cycle fiscal period filter"),
            ("t", "Create recurring template"),
        ]
    }

    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        // Route all keys to the active modal first.
        if self.modal.is_some() {
            return match &self.modal {
                Some(Modal::NewEntry(_)) => self.handle_new_entry_key(key, db),
                Some(Modal::ConfirmPost { .. }) => self.handle_confirm_post_key(key, db),
                Some(Modal::ReverseDate { .. }) => self.handle_reverse_date_key(key),
                Some(Modal::ConfirmReverse { .. }) => self.handle_confirm_reverse_key(key, db),
                Some(Modal::RecurringSetup { .. }) => self.handle_recurring_setup_key(key, db),
                None => TabAction::None,
            };
        }

        match key.code {
            KeyCode::Up => {
                if self.detail.is_some() {
                    self.detail_line_up();
                } else {
                    self.scroll_up();
                }
            }
            KeyCode::Down => {
                if self.detail.is_some() {
                    self.detail_line_down();
                } else {
                    self.scroll_down();
                }
            }
            KeyCode::Enter => {
                if self.detail.is_some() {
                    self.close_detail();
                } else {
                    self.open_detail(db);
                }
            }
            KeyCode::Esc => self.close_detail(),
            KeyCode::Char('c') | KeyCode::Char('C') => {
                return self.toggle_reconcile(db);
            }
            // Navigate to the GL for the focused line's account.
            KeyCode::Char('g') | KeyCode::Char('G') => {
                if let Some(d) = &self.detail
                    && let Some(line) = d.lines.get(d.focused_line)
                {
                    let account_id = line.account_id;
                    return TabAction::NavigateTo(
                        TabId::GeneralLedger,
                        RecordId::Account(account_id),
                    );
                }
            }
            KeyCode::Char('f') | KeyCode::Char('F') => {
                self.status_filter = self.status_filter.next();
                self.close_detail();
                self.refresh(db);
            }

            // ── Actions ───────────────────────────────────────────────────────
            KeyCode::Char('n') | KeyCode::Char('N') => {
                self.modal = Some(Modal::NewEntry(JeForm::new()));
            }
            KeyCode::Char('p') | KeyCode::Char('P')
                if !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if let Some(entry) = self.selected_entry() {
                    if entry.status == JournalEntryStatus::Draft {
                        let je_id = entry.id;
                        let je_number = entry.je_number.clone();
                        self.modal = Some(Modal::ConfirmPost {
                            confirm: Confirmation::new(format!("Post {}?", je_number)),
                            je_id,
                        });
                    } else {
                        return TabAction::ShowMessage(
                            "Only Draft entries can be posted.".to_string(),
                        );
                    }
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if let Some(entry) = self.selected_entry() {
                    if entry.status == JournalEntryStatus::Posted && !entry.is_reversed {
                        let je_id = entry.id;
                        let je_number = entry.je_number.clone();
                        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                        self.modal = Some(Modal::ReverseDate {
                            date_input: today,
                            je_id,
                            je_number,
                            error: None,
                        });
                    } else if entry.is_reversed {
                        return TabAction::ShowMessage(
                            "This entry has already been reversed.".to_string(),
                        );
                    } else {
                        return TabAction::ShowMessage(
                            "Only Posted entries can be reversed.".to_string(),
                        );
                    }
                }
            }
            // [t] create recurring template from a posted JE.
            KeyCode::Char('t') | KeyCode::Char('T') => {
                if let Some(entry) = self.selected_entry() {
                    if entry.status == JournalEntryStatus::Posted {
                        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                        self.modal = Some(Modal::RecurringSetup {
                            je_id: entry.id,
                            je_number: entry.je_number.clone(),
                            start_date_str: today,
                            frequency: EntryFrequency::Monthly,
                            focused_field: 0,
                            error: None,
                        });
                    } else {
                        return TabAction::ShowMessage(
                            "Only Posted entries can be made recurring.".to_string(),
                        );
                    }
                }
            }
            _ => {}
        }
        TabAction::None
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        if self.detail.is_some() {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
                .split(area);
            self.render_list(frame, chunks[0]);
            self.render_detail(frame, chunks[1]);
        } else {
            self.render_list(frame, area);
        }
        self.render_modal(frame, area);
    }

    fn wants_input(&self) -> bool {
        self.modal.is_some()
    }

    fn refresh(&mut self, db: &EntityDb) {
        let filter = self.status_filter.to_filter();
        match db.journals().list(&filter) {
            Ok(entries) => self.entries = entries,
            Err(e) => tracing::error!("JE list refresh failed: {e}"),
        }
        match db.accounts().list_all() {
            Ok(accts) => self.accounts = accts,
            Err(e) => tracing::error!("Account list refresh failed: {e}"),
        }
        self.reload_envelope_avail(db);
        // Clamp selection to valid range.
        if self.entries.is_empty() {
            self.table_state.select(None);
        } else {
            let sel = self.table_state.selected().unwrap_or(0);
            self.table_state
                .select(Some(sel.min(self.entries.len() - 1)));
        }
        // Re-sync detail if open.
        if let Some(id) = self.selected_entry().map(|e| e.id)
            && self.detail.is_some()
        {
            match db.journals().get_with_lines(id) {
                Ok((_, lines)) => {
                    self.detail = Some(DetailState {
                        lines,
                        focused_line: 0,
                    })
                }
                Err(e) => {
                    tracing::error!("Detail refresh failed: {e}");
                    self.detail = None;
                }
            }
        }
    }

    fn navigate_to(&mut self, record_id: RecordId, db: &EntityDb) {
        let RecordId::JournalEntry(id) = record_id else {
            return;
        };
        // Try current list first.
        if let Some(pos) = self.entries.iter().position(|e| e.id == id) {
            self.table_state.select(Some(pos));
            self.detail = None;
            return;
        }
        // Entry may be filtered out — switch to All and retry.
        self.status_filter = StatusFilter::All;
        self.close_detail();
        self.refresh(db);
        self.scroll_to(id);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::{initialize_schema, seed_default_accounts};
    use crate::db::{entity_db_from_conn, fiscal_repo::FiscalRepo, journal_repo::NewJournalEntry};
    use crate::types::FiscalPeriodId;
    use crossterm::event::KeyModifiers;
    use rusqlite::Connection;

    fn make_db() -> EntityDb {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();
        seed_default_accounts(&conn).unwrap();
        FiscalRepo::new(&conn).create_fiscal_year(1, 2026).unwrap();
        entity_db_from_conn(conn)
    }

    fn non_placeholder_accounts(db: &EntityDb) -> Vec<AccountId> {
        db.accounts()
            .list_all()
            .unwrap()
            .into_iter()
            .filter(|a| !a.is_placeholder && a.is_active)
            .map(|a| a.id)
            .collect()
    }

    fn period_for_jan(db: &EntityDb) -> FiscalPeriodId {
        db.fiscal()
            .get_period_for_date(NaiveDate::from_ymd_opt(2026, 1, 15).unwrap())
            .unwrap()
            .id
    }

    fn create_draft(db: &EntityDb) -> JournalEntryId {
        let pid = period_for_jan(db);
        let accts = non_placeholder_accounts(db);
        let a1 = accts[0];
        let a2 = accts[1];
        db.journals()
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                memo: Some("Test JE".to_string()),
                fiscal_period_id: pid,
                reversal_of_je_id: None,
                lines: vec![
                    crate::db::journal_repo::NewJournalEntryLine {
                        account_id: a1,
                        debit_amount: crate::types::Money(10_000_000_000),
                        credit_amount: crate::types::Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    crate::db::journal_repo::NewJournalEntryLine {
                        account_id: a2,
                        debit_amount: crate::types::Money(0),
                        credit_amount: crate::types::Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .unwrap()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn refresh_loads_entries() {
        let db = make_db();
        create_draft(&db);
        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);
        assert_eq!(tab.entries.len(), 1);
    }

    #[test]
    fn scroll_down_and_up_within_bounds() {
        let db = make_db();
        create_draft(&db);
        create_draft(&db);
        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);

        tab.scroll_down();
        assert_eq!(tab.table_state.selected(), Some(1));
        tab.scroll_down(); // already at last
        assert_eq!(tab.table_state.selected(), Some(1));
        tab.scroll_up();
        assert_eq!(tab.table_state.selected(), Some(0));
    }

    #[test]
    fn filter_draft_excludes_posted() {
        let db = make_db();
        let id = create_draft(&db);
        crate::services::journal::post_journal_entry(&db, id, "Test Entity").unwrap();
        create_draft(&db);

        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);
        assert_eq!(tab.entries.len(), 2);

        tab.status_filter = StatusFilter::Draft;
        tab.refresh(&db);
        assert_eq!(tab.entries.len(), 1);
        assert_eq!(tab.entries[0].status, JournalEntryStatus::Draft);
    }

    #[test]
    fn navigate_to_selects_correct_entry() {
        let db = make_db();
        create_draft(&db);
        let id2 = create_draft(&db);

        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);

        tab.navigate_to(RecordId::JournalEntry(id2), &db);
        let sel = tab.table_state.selected().unwrap();
        assert_eq!(tab.entries[sel].id, id2);
    }

    #[test]
    fn navigate_to_filtered_out_entry_switches_to_all() {
        let db = make_db();
        let id = create_draft(&db);
        crate::services::journal::post_journal_entry(&db, id, "Test Entity").unwrap();

        let mut tab = JournalEntriesTab::new();
        tab.status_filter = StatusFilter::Draft;
        tab.refresh(&db);
        assert_eq!(tab.entries.len(), 0);

        tab.navigate_to(RecordId::JournalEntry(id), &db);
        assert_eq!(tab.status_filter, StatusFilter::All);
        let sel = tab.table_state.selected().unwrap();
        assert_eq!(tab.entries[sel].id, id);
    }

    #[test]
    fn open_detail_loads_lines() {
        let db = make_db();
        create_draft(&db);
        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);

        tab.open_detail(&db);
        assert!(tab.detail.is_some());
        assert_eq!(tab.detail.as_ref().unwrap().lines.len(), 2);
    }

    #[test]
    fn n_key_opens_new_entry_modal() {
        let db = make_db();
        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);

        tab.handle_key(key(KeyCode::Char('n')), &db);
        assert!(matches!(tab.modal, Some(Modal::NewEntry(_))));
    }

    #[test]
    fn p_on_draft_opens_confirm_post_modal() {
        let db = make_db();
        create_draft(&db);
        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);

        tab.handle_key(key(KeyCode::Char('p')), &db);
        assert!(matches!(tab.modal, Some(Modal::ConfirmPost { .. })));
    }

    #[test]
    fn p_on_posted_shows_message() {
        let db = make_db();
        let id = create_draft(&db);
        crate::services::journal::post_journal_entry(&db, id, "Test Entity").unwrap();

        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);

        let action = tab.handle_key(key(KeyCode::Char('p')), &db);
        assert!(matches!(action, TabAction::ShowMessage(_)));
        assert!(tab.modal.is_none());
    }

    #[test]
    fn confirm_post_triggers_refresh() {
        let db = make_db();
        create_draft(&db);
        let mut tab = JournalEntriesTab::new();
        tab.set_entity_name("Test Entity");
        tab.refresh(&db);

        // Open confirm post modal.
        tab.handle_key(key(KeyCode::Char('p')), &db);
        assert!(matches!(tab.modal, Some(Modal::ConfirmPost { .. })));

        // Confirm (y key).
        let action = tab.handle_key(key(KeyCode::Char('y')), &db);
        assert!(matches!(action, TabAction::RefreshData));
        assert!(tab.modal.is_none());

        // Verify posted.
        let (je, _) = db
            .journals()
            .get_with_lines(
                db.journals()
                    .list(&JournalFilter {
                        status: None,
                        from_date: None,
                        to_date: None,
                    })
                    .unwrap()[0]
                    .id,
            )
            .unwrap();
        assert_eq!(je.status, JournalEntryStatus::Posted);
    }

    #[test]
    fn r_on_posted_opens_reverse_date_modal() {
        let db = make_db();
        let id = create_draft(&db);
        crate::services::journal::post_journal_entry(&db, id, "Test Entity").unwrap();

        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);

        tab.handle_key(key(KeyCode::Char('r')), &db);
        assert!(matches!(tab.modal, Some(Modal::ReverseDate { .. })));
    }

    #[test]
    fn r_on_draft_shows_message() {
        let db = make_db();
        create_draft(&db);
        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);

        let action = tab.handle_key(key(KeyCode::Char('r')), &db);
        assert!(matches!(action, TabAction::ShowMessage(_)));
    }

    #[test]
    fn full_reverse_workflow_creates_reversal_entry() {
        let db = make_db();
        let id = create_draft(&db);
        crate::services::journal::post_journal_entry(&db, id, "Test Entity").unwrap();

        let mut tab = JournalEntriesTab::new();
        tab.set_entity_name("Test Entity");
        tab.refresh(&db);
        assert_eq!(tab.entries.len(), 1);

        // Open reverse date modal.
        tab.handle_key(key(KeyCode::Char('r')), &db);

        // Type a valid reversal date.
        for c in "2026-01-31".chars() {
            tab.handle_key(key(KeyCode::Char(c)), &db);
        }
        // Advance to ConfirmReverse.
        tab.handle_key(key(KeyCode::Enter), &db);
        assert!(matches!(tab.modal, Some(Modal::ConfirmReverse { .. })));

        // Confirm.
        let action = tab.handle_key(key(KeyCode::Char('y')), &db);
        assert!(matches!(action, TabAction::RefreshData));

        // After refresh, there should be 2 entries.
        tab.refresh(&db);
        assert_eq!(tab.entries.len(), 2);
    }

    #[test]
    fn c_key_toggles_uncleared_to_cleared_on_posted_line() {
        let db = make_db();
        let id = create_draft(&db);
        crate::services::journal::post_journal_entry(&db, id, "Test Entity").unwrap();

        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);
        // Open detail.
        tab.open_detail(&db);
        assert!(tab.detail.is_some());
        // First line should be Uncleared.
        assert_eq!(
            tab.detail.as_ref().unwrap().lines[0].reconcile_state,
            ReconcileState::Uncleared
        );

        let action = tab.handle_key(key(KeyCode::Char('c')), &db);
        assert!(matches!(action, TabAction::RefreshData));

        // Reload detail to see new state.
        tab.refresh(&db);
        tab.open_detail(&db);
        assert_eq!(
            tab.detail.as_ref().unwrap().lines[0].reconcile_state,
            ReconcileState::Cleared
        );
    }

    #[test]
    fn c_key_toggles_cleared_back_to_uncleared() {
        let db = make_db();
        let id = create_draft(&db);
        crate::services::journal::post_journal_entry(&db, id, "Test Entity").unwrap();
        // Set first line to Cleared directly in DB.
        let (_, lines) = db.journals().get_with_lines(id).unwrap();
        db.journals()
            .update_reconcile_state(lines[0].id, ReconcileState::Cleared)
            .unwrap();

        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);
        tab.open_detail(&db);

        let action = tab.handle_key(key(KeyCode::Char('c')), &db);
        assert!(matches!(action, TabAction::RefreshData));

        tab.refresh(&db);
        tab.open_detail(&db);
        assert_eq!(
            tab.detail.as_ref().unwrap().lines[0].reconcile_state,
            ReconcileState::Uncleared
        );
    }

    #[test]
    fn c_key_rejects_reconciled_line() {
        let db = make_db();
        let id = create_draft(&db);
        crate::services::journal::post_journal_entry(&db, id, "Test Entity").unwrap();
        let (_, lines) = db.journals().get_with_lines(id).unwrap();
        db.journals()
            .update_reconcile_state(lines[0].id, ReconcileState::Reconciled)
            .unwrap();

        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);
        tab.open_detail(&db);

        let action = tab.handle_key(key(KeyCode::Char('c')), &db);
        assert!(matches!(action, TabAction::ShowMessage(_)));
    }

    #[test]
    fn c_key_on_draft_line_shows_message() {
        let db = make_db();
        create_draft(&db);

        let mut tab = JournalEntriesTab::new();
        tab.refresh(&db);
        tab.open_detail(&db);

        let action = tab.handle_key(key(KeyCode::Char('c')), &db);
        assert!(matches!(action, TabAction::ShowMessage(_)));
    }
}
