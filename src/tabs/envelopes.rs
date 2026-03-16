use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState,
    },
};

use crate::db::EntityDb;
use crate::db::account_repo::Account;
use crate::tabs::{RecordId, Tab, TabAction};
use crate::types::{AccountId, AuditAction, Money, Percentage};
use crate::widgets::confirmation::ConfirmAction;
use crate::widgets::je_form::parse_money;
use crate::widgets::{Confirmation, centered_rect};

// ── Sub-view selector ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Allocations,
    Balances,
}

// ── Allocation edit modal ─────────────────────────────────────────────────────

struct EditPercentState {
    account_id: AccountId,
    account_name: String,
    input: String,
    error: Option<String>,
}

// ── Transfer modal ────────────────────────────────────────────────────────────

enum TransferStep {
    /// Choosing which envelope to transfer from.
    SelectSource,
    /// Chose source; now choosing destination.
    SelectDest { source_id: AccountId },
    /// Both accounts chosen; enter amount.
    EnterAmount {
        source_id: AccountId,
        dest_id: AccountId,
        input: String,
        error: Option<String>,
    },
    /// Amount parsed; ask for confirmation.
    Confirm {
        source_id: AccountId,
        dest_id: AccountId,
        amount: Money,
        confirm: Confirmation,
    },
}

struct TransferModal {
    step: TransferStep,
    /// Navigable list of allocated accounts (same ordering as Balances view).
    list_state: ListState,
}

impl TransferModal {
    fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            step: TransferStep::SelectSource,
            list_state,
        }
    }
}

// ── Tab struct ────────────────────────────────────────────────────────────────

pub struct EnvelopesTab {
    entity_name: String,
    view: View,
    /// All non-placeholder, active accounts (for the allocation config view).
    accounts: Vec<Account>,
    /// Current allocations keyed by account_id.
    allocations: HashMap<AccountId, Percentage>,
    /// Envelope ledger balances (earmarked amounts) — refreshed from DB.
    envelope_balances: HashMap<AccountId, Money>,
    /// GL account balances — refreshed from DB.
    gl_balances: HashMap<AccountId, Money>,
    table_state: TableState,
    modal: Option<EditPercentState>,
    transfer: Option<TransferModal>,
}

impl Default for EnvelopesTab {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvelopesTab {
    pub fn new() -> Self {
        Self {
            entity_name: String::new(),
            view: View::Allocations,
            accounts: Vec::new(),
            allocations: HashMap::new(),
            envelope_balances: HashMap::new(),
            gl_balances: HashMap::new(),
            table_state: TableState::default(),
            modal: None,
            transfer: None,
        }
    }

    pub fn set_entity_name(&mut self, name: &str) {
        self.entity_name = name.to_string();
    }

    fn reload_accounts_and_allocations(&mut self, db: &EntityDb) {
        match db.accounts().list_active() {
            Ok(all) => {
                self.accounts = all.into_iter().filter(|a| !a.is_placeholder).collect();
            }
            Err(e) => {
                tracing::error!("Failed to load accounts: {e}");
                self.accounts.clear();
            }
        }

        match db.envelopes().get_all_allocations() {
            Ok(allocs) => {
                self.allocations = allocs
                    .into_iter()
                    .map(|a| (a.account_id, a.percentage))
                    .collect();
            }
            Err(e) => {
                tracing::error!("Failed to load allocations: {e}");
                self.allocations.clear();
            }
        }
    }

    fn reload_balances(&mut self, db: &EntityDb) {
        let mut env_bals = HashMap::new();
        let mut gl_bals = HashMap::new();
        for &account_id in self.allocations.keys() {
            if let Ok(b) = db.envelopes().get_balance(account_id) {
                env_bals.insert(account_id, b);
            }
            if let Ok(b) = db.accounts().get_balance(account_id) {
                gl_bals.insert(account_id, b);
            }
        }
        self.envelope_balances = env_bals;
        self.gl_balances = gl_bals;
    }

    fn clamp_selection(&mut self) {
        let len = self.visible_row_count();
        if len == 0 {
            self.table_state.select(None);
        } else if self.table_state.selected().is_none() {
            self.table_state.select(Some(0));
        } else if let Some(sel) = self.table_state.selected()
            && sel >= len
        {
            self.table_state.select(Some(len - 1));
        }
    }

    fn visible_row_count(&self) -> usize {
        match self.view {
            View::Allocations => self.accounts.len(),
            View::Balances => self
                .accounts
                .iter()
                .filter(|a| self.allocations.contains_key(&a.id))
                .count(),
        }
    }

    /// Returns a sorted list of accounts that have allocations (used for transfer pickers).
    fn allocated_accounts(&self) -> Vec<&Account> {
        self.accounts
            .iter()
            .filter(|a| self.allocations.contains_key(&a.id))
            .collect()
    }

    fn scroll_down(&mut self) {
        let len = self.visible_row_count();
        if len == 0 {
            return;
        }
        let next = self
            .table_state
            .selected()
            .map(|i| (i + 1).min(len - 1))
            .unwrap_or(0);
        self.table_state.select(Some(next));
    }

    fn scroll_up(&mut self) {
        let next = self
            .table_state
            .selected()
            .map(|i| i.saturating_sub(1))
            .unwrap_or(0);
        self.table_state.select(Some(next));
    }

    fn open_edit_for_selected(&mut self) {
        let idx = match self.table_state.selected() {
            Some(i) => i,
            None => return,
        };
        let acct = match self.accounts.get(idx) {
            Some(a) => a,
            None => return,
        };
        let current = self
            .allocations
            .get(&acct.id)
            .map(|p| format!("{:.2}", p.0 as f64 / 1_000_000.0))
            .unwrap_or_default();
        self.modal = Some(EditPercentState {
            account_id: acct.id,
            account_name: acct.name.clone(),
            input: current,
            error: None,
        });
    }

    fn remove_allocation_for_selected(&mut self, db: &EntityDb) {
        let idx = match self.table_state.selected() {
            Some(i) => i,
            None => return,
        };
        let acct = match self.accounts.get(idx) {
            Some(a) => a,
            None => return,
        };
        if !self.allocations.contains_key(&acct.id) {
            return;
        }
        let acct_id = acct.id;
        let acct_name = acct.name.clone();
        match db.envelopes().remove_allocation(acct_id) {
            Ok(()) => {
                self.allocations.remove(&acct_id);
                let _ = db.audit().append(
                    AuditAction::EnvelopeAllocationChanged,
                    &self.entity_name,
                    Some("Account"),
                    Some(i64::from(acct_id)),
                    &format!("Removed envelope allocation for {acct_name}"),
                );
            }
            Err(e) => tracing::error!("Failed to remove allocation: {e}"),
        }
    }

    fn save_allocation(
        &mut self,
        account_id: AccountId,
        account_name: &str,
        pct_display: f64,
        db: &EntityDb,
    ) {
        if pct_display == 0.0 {
            match db.envelopes().remove_allocation(account_id) {
                Ok(()) => {
                    self.allocations.remove(&account_id);
                    let _ = db.audit().append(
                        AuditAction::EnvelopeAllocationChanged,
                        &self.entity_name,
                        Some("Account"),
                        Some(i64::from(account_id)),
                        &format!("Removed envelope allocation for {account_name}"),
                    );
                }
                Err(e) => tracing::error!("Failed to remove allocation: {e}"),
            }
        } else {
            let pct = Percentage::from_display(pct_display);
            match db.envelopes().set_allocation(account_id, pct) {
                Ok(()) => {
                    self.allocations.insert(account_id, pct);
                    let _ = db.audit().append(
                        AuditAction::EnvelopeAllocationChanged,
                        &self.entity_name,
                        Some("Account"),
                        Some(i64::from(account_id)),
                        &format!("Set envelope allocation for {account_name} to {pct}"),
                    );
                }
                Err(e) => tracing::error!("Failed to set allocation: {e}"),
            }
        }
    }

    fn handle_modal_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        match key.code {
            KeyCode::Esc => {
                self.modal = None;
            }
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                if let Some(s) = self.modal.as_mut() {
                    s.input.push(c);
                    s.error = None;
                }
            }
            KeyCode::Backspace => {
                if let Some(s) = self.modal.as_mut() {
                    s.input.pop();
                    s.error = None;
                }
            }
            KeyCode::Enter => {
                let (account_id, account_name, input) = match &self.modal {
                    Some(s) => (
                        s.account_id,
                        s.account_name.clone(),
                        s.input.trim().to_string(),
                    ),
                    None => return TabAction::None,
                };

                let pct_display: f64 = match input.parse() {
                    Ok(v) => v,
                    Err(_) => {
                        if let Some(s) = self.modal.as_mut() {
                            s.error = Some("Enter a number (e.g., 15.5)".to_string());
                        }
                        return TabAction::None;
                    }
                };

                if pct_display < 0.0 {
                    if let Some(s) = self.modal.as_mut() {
                        s.error = Some("Percentage cannot be negative".to_string());
                    }
                    return TabAction::None;
                }

                self.modal = None;
                self.save_allocation(account_id, &account_name, pct_display, db);
            }
            _ => {}
        }
        TabAction::None
    }

    // ── Transfer modal key handling ────────────────────────────────────────────

    fn open_transfer_modal(&mut self) {
        if self.allocated_accounts().is_empty() {
            return;
        }
        let mut tm = TransferModal::new();
        // Clamp list_state to available accounts.
        let count = self.allocated_accounts().len();
        if count == 0 {
            tm.list_state.select(None);
        }
        self.transfer = Some(tm);
    }

    fn handle_transfer_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        let tm = match self.transfer.as_mut() {
            Some(t) => t,
            None => return TabAction::None,
        };

        let allocated: Vec<AccountId> = self
            .accounts
            .iter()
            .filter(|a| self.allocations.contains_key(&a.id))
            .map(|a| a.id)
            .collect();
        let count = allocated.len();

        match &mut tm.step {
            TransferStep::SelectSource => match key.code {
                KeyCode::Esc => {
                    self.transfer = None;
                }
                KeyCode::Up => {
                    let sel = tm.list_state.selected().unwrap_or(0);
                    tm.list_state.select(Some(sel.saturating_sub(1)));
                }
                KeyCode::Down => {
                    let sel = tm.list_state.selected().unwrap_or(0);
                    tm.list_state
                        .select(Some((sel + 1).min(count.saturating_sub(1))));
                }
                KeyCode::Enter => {
                    let idx = tm.list_state.selected().unwrap_or(0);
                    if let Some(&source_id) = allocated.get(idx) {
                        tm.step = TransferStep::SelectDest { source_id };
                        tm.list_state.select(Some(0));
                    }
                }
                _ => {}
            },
            TransferStep::SelectDest { source_id } => {
                let source_id = *source_id;
                // Dest list excludes the source account.
                let dest_ids: Vec<AccountId> = allocated
                    .iter()
                    .copied()
                    .filter(|&id| id != source_id)
                    .collect();
                let dest_count = dest_ids.len();
                match key.code {
                    KeyCode::Esc => {
                        tm.step = TransferStep::SelectSource;
                        tm.list_state.select(Some(0));
                    }
                    KeyCode::Up => {
                        let sel = tm.list_state.selected().unwrap_or(0);
                        tm.list_state.select(Some(sel.saturating_sub(1)));
                    }
                    KeyCode::Down => {
                        let sel = tm.list_state.selected().unwrap_or(0);
                        tm.list_state
                            .select(Some((sel + 1).min(dest_count.saturating_sub(1))));
                    }
                    KeyCode::Enter => {
                        let idx = tm.list_state.selected().unwrap_or(0);
                        if let Some(&dest_id) = dest_ids.get(idx) {
                            tm.step = TransferStep::EnterAmount {
                                source_id,
                                dest_id,
                                input: String::new(),
                                error: None,
                            };
                        }
                    }
                    _ => {}
                }
            }
            TransferStep::EnterAmount {
                source_id,
                dest_id,
                input,
                error,
            } => {
                let source_id = *source_id;
                let dest_id = *dest_id;
                match key.code {
                    KeyCode::Esc => {
                        // Go back to dest selection.
                        tm.step = TransferStep::SelectDest { source_id };
                        tm.list_state.select(Some(0));
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                        input.push(c);
                        *error = None;
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        *error = None;
                    }
                    KeyCode::Enter => {
                        let trimmed = input.trim().to_string();
                        match parse_money(&trimmed) {
                            Err(msg) => {
                                *error = Some(msg);
                            }
                            Ok(amount) if amount.0 <= 0 => {
                                *error = Some("Amount must be positive".to_string());
                            }
                            Ok(amount) => {
                                // Validate source balance.
                                let src_balance = self
                                    .envelope_balances
                                    .get(&source_id)
                                    .copied()
                                    .unwrap_or(Money(0));
                                if amount.0 > src_balance.0 {
                                    *error = Some(format!(
                                        "Insufficient balance: envelope has {src_balance}"
                                    ));
                                } else {
                                    // Build confirm message.
                                    let src_name = self
                                        .accounts
                                        .iter()
                                        .find(|a| a.id == source_id)
                                        .map(|a| a.name.as_str())
                                        .unwrap_or("?");
                                    let dst_name = self
                                        .accounts
                                        .iter()
                                        .find(|a| a.id == dest_id)
                                        .map(|a| a.name.as_str())
                                        .unwrap_or("?");
                                    let msg = format!(
                                        "Transfer {amount} from [{src_name}] to [{dst_name}]?"
                                    );
                                    tm.step = TransferStep::Confirm {
                                        source_id,
                                        dest_id,
                                        amount,
                                        confirm: Confirmation::new(msg),
                                    };
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            TransferStep::Confirm {
                source_id,
                dest_id,
                amount,
                confirm,
            } => {
                let source_id = *source_id;
                let dest_id = *dest_id;
                let amount = *amount;
                match confirm.handle_key(key) {
                    ConfirmAction::Cancelled => {
                        self.transfer = None;
                    }
                    ConfirmAction::Confirmed => {
                        self.transfer = None;
                        self.execute_transfer(source_id, dest_id, amount, db);
                    }
                    ConfirmAction::Pending => {}
                }
            }
        }

        TabAction::None
    }

    fn execute_transfer(
        &mut self,
        source_id: AccountId,
        dest_id: AccountId,
        amount: Money,
        db: &EntityDb,
    ) {
        match db.envelopes().record_transfer(source_id, dest_id, amount) {
            Ok(_transfer_group_id) => {
                // Update local balance cache immediately.
                if let Some(bal) = self.envelope_balances.get_mut(&source_id) {
                    bal.0 -= amount.0;
                }
                let dest_bal = self.envelope_balances.entry(dest_id).or_insert(Money(0));
                dest_bal.0 += amount.0;

                let src_name = self
                    .accounts
                    .iter()
                    .find(|a| a.id == source_id)
                    .map(|a| a.name.clone())
                    .unwrap_or_default();
                let dst_name = self
                    .accounts
                    .iter()
                    .find(|a| a.id == dest_id)
                    .map(|a| a.name.clone())
                    .unwrap_or_default();

                let _ = db.audit().append(
                    AuditAction::EnvelopeTransfer,
                    &self.entity_name,
                    Some("Account"),
                    Some(i64::from(source_id)),
                    &format!("Envelope transfer {amount} from {src_name} to {dst_name}"),
                );
            }
            Err(e) => tracing::error!("Failed to record transfer: {e}"),
        }
    }

    // ── Rendering ─────────────────────────────────────────────────────────────

    fn render_allocations(&self, frame: &mut Frame, area: Rect) {
        let rows: Vec<Row> = self
            .accounts
            .iter()
            .map(|acct| {
                let pct_str = self
                    .allocations
                    .get(&acct.id)
                    .map(|p| format!("{p}"))
                    .unwrap_or_else(|| "—".to_string());

                let style = if self.allocations.contains_key(&acct.id) {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                };

                Row::new(vec![
                    Cell::from(acct.number.as_str()),
                    Cell::from(acct.name.as_str()),
                    Cell::from(pct_str),
                ])
                .style(style)
            })
            .collect();

        let widths = [
            Constraint::Length(8),
            Constraint::Min(30),
            Constraint::Length(12),
        ];

        let table = Table::new(rows, widths)
            .header(
                Row::new(vec!["#", "Account Name", "Allocation %"])
                    .style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Allocation Config"),
            )
            .row_highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            );

        let mut ts = self.table_state.clone();
        frame.render_stateful_widget(table, area, &mut ts);
    }

    fn render_balances(&self, frame: &mut Frame, area: Rect) {
        let rows: Vec<Row> = self
            .accounts
            .iter()
            .filter(|a| self.allocations.contains_key(&a.id))
            .map(|acct| {
                let pct = self.allocations[&acct.id];
                let earmarked = self
                    .envelope_balances
                    .get(&acct.id)
                    .copied()
                    .unwrap_or(Money(0));
                let gl_balance = self.gl_balances.get(&acct.id).copied().unwrap_or(Money(0));
                let available = Money(gl_balance.0 - earmarked.0);

                Row::new(vec![
                    Cell::from(acct.name.as_str()),
                    Cell::from(format!("{pct}")),
                    Cell::from(format!("{gl_balance}")),
                    Cell::from(format!("{earmarked}")),
                    Cell::from(format!("{available}")),
                ])
            })
            .collect();

        let widths = [
            Constraint::Min(28),
            Constraint::Length(12),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Length(14),
        ];

        let table = Table::new(rows, widths)
            .header(
                Row::new(vec![
                    "Account Name",
                    "Allocation %",
                    "GL Balance",
                    "Earmarked",
                    "Available",
                ])
                .style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Envelope Balances"),
            )
            .row_highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            );

        let mut ts = self.table_state.clone();
        frame.render_stateful_widget(table, area, &mut ts);
    }

    fn render_edit_modal(&self, frame: &mut Frame, area: Rect) {
        let state = match &self.modal {
            Some(s) => s,
            None => return,
        };

        let popup = centered_rect(50, 10, area);
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Set Allocation: {}", state.account_name));

        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let lines = vec![
            Line::from(vec![
                Span::raw("Percentage: "),
                Span::styled(
                    format!("{}█", state.input),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                state
                    .error
                    .as_deref()
                    .unwrap_or("Enter % (0 = remove). Enter to save, Esc to cancel."),
                if state.error.is_some() {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::DarkGray)
                },
            )),
        ];

        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn render_transfer_modal(&self, frame: &mut Frame, area: Rect) {
        let tm = match &self.transfer {
            Some(t) => t,
            None => return,
        };

        let allocated: Vec<&Account> = self.allocated_accounts();

        match &tm.step {
            TransferStep::SelectSource => {
                let popup = centered_rect(60, 60, area);
                frame.render_widget(Clear, popup);

                let items: Vec<ListItem> = allocated
                    .iter()
                    .map(|a| {
                        let bal = self
                            .envelope_balances
                            .get(&a.id)
                            .copied()
                            .unwrap_or(Money(0));
                        ListItem::new(Line::from(vec![
                            Span::raw(format!("{:<30}", a.name)),
                            Span::styled(format!("  {bal}"), Style::default().fg(Color::Cyan)),
                        ]))
                    })
                    .collect();

                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" Transfer — Select Source ")
                            .style(Style::default().fg(Color::Yellow)),
                    )
                    .highlight_style(
                        Style::default()
                            .bg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD),
                    );

                let mut ls = tm.list_state.clone();
                frame.render_stateful_widget(list, popup, &mut ls);

                // Hint line at bottom of popup.
                let hint_area = Rect {
                    y: popup.y + popup.height.saturating_sub(1),
                    height: 1,
                    ..popup
                };
                frame.render_widget(
                    Paragraph::new("↑↓ Navigate  Enter Select  Esc Cancel")
                        .style(Style::default().fg(Color::DarkGray)),
                    hint_area,
                );
            }
            TransferStep::SelectDest { source_id } => {
                let popup = centered_rect(60, 60, area);
                frame.render_widget(Clear, popup);

                let items: Vec<ListItem> = allocated
                    .iter()
                    .filter(|a| a.id != *source_id)
                    .map(|a| {
                        let bal = self
                            .envelope_balances
                            .get(&a.id)
                            .copied()
                            .unwrap_or(Money(0));
                        ListItem::new(Line::from(vec![
                            Span::raw(format!("{:<30}", a.name)),
                            Span::styled(format!("  {bal}"), Style::default().fg(Color::Cyan)),
                        ]))
                    })
                    .collect();

                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" Transfer — Select Destination ")
                            .style(Style::default().fg(Color::Yellow)),
                    )
                    .highlight_style(
                        Style::default()
                            .bg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD),
                    );

                let mut ls = tm.list_state.clone();
                frame.render_stateful_widget(list, popup, &mut ls);

                let hint_area = Rect {
                    y: popup.y + popup.height.saturating_sub(1),
                    height: 1,
                    ..popup
                };
                frame.render_widget(
                    Paragraph::new("↑↓ Navigate  Enter Select  Esc Back")
                        .style(Style::default().fg(Color::DarkGray)),
                    hint_area,
                );
            }
            TransferStep::EnterAmount {
                source_id,
                dest_id,
                input,
                error,
            } => {
                let popup = centered_rect(55, 12, area);
                frame.render_widget(Clear, popup);

                let src_name = self
                    .accounts
                    .iter()
                    .find(|a| a.id == *source_id)
                    .map(|a| a.name.as_str())
                    .unwrap_or("?");
                let dst_name = self
                    .accounts
                    .iter()
                    .find(|a| a.id == *dest_id)
                    .map(|a| a.name.as_str())
                    .unwrap_or("?");

                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(" Transfer — Enter Amount ")
                    .style(Style::default().fg(Color::Yellow));
                let inner = block.inner(popup);
                frame.render_widget(block, popup);

                let src_bal = self
                    .envelope_balances
                    .get(source_id)
                    .copied()
                    .unwrap_or(Money(0));

                let lines = vec![
                    Line::from(vec![
                        Span::raw("From: "),
                        Span::styled(src_name, Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("  (available: {src_bal})"),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]),
                    Line::from(vec![
                        Span::raw("  To: "),
                        Span::styled(dst_name, Style::default().fg(Color::Cyan)),
                    ]),
                    Line::from(""),
                    Line::from(vec![
                        Span::raw("Amount: "),
                        Span::styled(format!("{input}█"), Style::default().fg(Color::Yellow)),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled(
                        error
                            .as_deref()
                            .unwrap_or("Enter amount. Enter to continue, Esc to go back."),
                        if error.is_some() {
                            Style::default().fg(Color::Red)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        },
                    )),
                ];

                frame.render_widget(Paragraph::new(lines), inner);
            }
            TransferStep::Confirm { confirm, .. } => {
                confirm.render(frame, area);
            }
        }
    }

    fn hint_text(&self) -> &'static str {
        match self.view {
            View::Allocations => "↑↓ Navigate  Enter Edit%  d Remove  Tab→Balances",
            View::Balances => "↑↓ Navigate  t Transfer  Tab→Allocations",
        }
    }
}

impl Tab for EnvelopesTab {
    fn title(&self) -> &str {
        "Envelopes"
    }

    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        if self.transfer.is_some() {
            return self.handle_transfer_key(key, db);
        }
        if self.modal.is_some() {
            return self.handle_modal_key(key, db);
        }

        match key.code {
            KeyCode::Tab => {
                self.view = match self.view {
                    View::Allocations => View::Balances,
                    View::Balances => View::Allocations,
                };
                self.table_state.select(Some(0));
                self.reload_balances(db);
            }
            KeyCode::Down => self.scroll_down(),
            KeyCode::Up => self.scroll_up(),
            KeyCode::Enter => {
                if self.view == View::Allocations {
                    self.open_edit_for_selected();
                }
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                if self.view == View::Allocations {
                    self.remove_allocation_for_selected(db);
                }
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                if self.view == View::Balances {
                    self.open_transfer_modal();
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

        match self.view {
            View::Allocations => self.render_allocations(frame, chunks[0]),
            View::Balances => self.render_balances(frame, chunks[0]),
        }

        frame.render_widget(
            Paragraph::new(self.hint_text()).style(Style::default().fg(Color::DarkGray)),
            chunks[1],
        );

        if self.modal.is_some() {
            self.render_edit_modal(frame, area);
        }
        if self.transfer.is_some() {
            self.render_transfer_modal(frame, area);
        }
    }

    fn refresh(&mut self, db: &EntityDb) {
        self.reload_accounts_and_allocations(db);
        self.reload_balances(db);
        self.clamp_selection();
    }

    fn navigate_to(&mut self, _record_id: RecordId, _db: &EntityDb) {}

    fn wants_input(&self) -> bool {
        self.modal.is_some() || self.transfer.is_some()
    }
}
