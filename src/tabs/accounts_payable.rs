use chrono::NaiveDate;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
};

use crate::db::{
    EntityDb,
    ap_repo::{ApFilter, ApItem, ApPayment, NewApItem},
};
use crate::tabs::{RecordId, Tab, TabAction, TabId};
use crate::types::{ApItemId, ArApStatus, AuditAction, JournalEntryId, Money};
use crate::widgets::centered_rect;
use crate::widgets::je_form::parse_money;

// ── Status filter cycle ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum StatusFilter {
    All,
    Open,
    Partial,
    Paid,
}

impl StatusFilter {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Open,
            Self::Open => Self::Partial,
            Self::Partial => Self::Paid,
            Self::Paid => Self::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Open => "Open",
            Self::Partial => "Partial",
            Self::Paid => "Paid",
        }
    }

    fn to_filter(self) -> ApFilter {
        ApFilter {
            status: match self {
                Self::All => None,
                Self::Open => Some(ArApStatus::Open),
                Self::Partial => Some(ArApStatus::Partial),
                Self::Paid => Some(ArApStatus::Paid),
            },
        }
    }
}

// ── Modal state ───────────────────────────────────────────────────────────────

struct NewItemForm {
    vendor_name: String,
    description: String,
    amount_str: String,
    due_date_str: String,
    je_id_str: String,
    focused: usize,
    error: Option<String>,
}

impl NewItemForm {
    const FIELD_COUNT: usize = 5;
    fn new() -> Self {
        Self {
            vendor_name: String::new(),
            description: String::new(),
            amount_str: String::new(),
            due_date_str: String::new(),
            je_id_str: String::new(),
            focused: 0,
            error: None,
        }
    }
}

struct PaymentForm {
    item_id: ApItemId,
    amount_str: String,
    date_str: String,
    je_id_str: String,
    focused: usize,
    error: Option<String>,
}

impl PaymentForm {
    const FIELD_COUNT: usize = 3;
    fn new(item_id: ApItemId) -> Self {
        Self {
            item_id,
            amount_str: String::new(),
            date_str: String::new(),
            je_id_str: String::new(),
            focused: 0,
            error: None,
        }
    }
}

struct PaymentHistoryView {
    item: ApItem,
    payments: Vec<ApPayment>,
    scroll: usize,
}

enum ApModal {
    NewItem(NewItemForm),
    Payment(PaymentForm),
    PaymentHistory(PaymentHistoryView),
}

// ── Tab struct ────────────────────────────────────────────────────────────────

pub struct AccountsPayableTab {
    items: Vec<ApItem>,
    table_state: TableState,
    status_filter: StatusFilter,
    entity_name: String,
    modal: Option<ApModal>,
}

impl Default for AccountsPayableTab {
    fn default() -> Self {
        Self::new()
    }
}

impl AccountsPayableTab {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            table_state: TableState::default(),
            status_filter: StatusFilter::All,
            entity_name: String::new(),
            modal: None,
        }
    }

    pub fn set_entity_name(&mut self, name: &str) {
        self.entity_name = name.to_string();
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn reload(&mut self, db: &EntityDb) {
        match db.ap().list(&self.status_filter.to_filter()) {
            Ok(items) => {
                self.items = items;
                if self.items.is_empty() {
                    self.table_state.select(None);
                } else {
                    let sel = self
                        .table_state
                        .selected()
                        .unwrap_or(0)
                        .min(self.items.len() - 1);
                    self.table_state.select(Some(sel));
                }
            }
            Err(e) => {
                tracing::error!("AP tab: failed to load items: {e}");
                self.items.clear();
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
        if self.items.is_empty() {
            return;
        }
        let cur = self.table_state.selected().unwrap_or(0);
        if cur + 1 < self.items.len() {
            self.table_state.select(Some(cur + 1));
        }
    }

    fn selected_item(&self) -> Option<&ApItem> {
        self.table_state.selected().and_then(|i| self.items.get(i))
    }

    // ── Modal key handlers ────────────────────────────────────────────────────

    fn handle_new_item_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        let form = match &mut self.modal {
            Some(ApModal::NewItem(f)) => f,
            _ => return TabAction::None,
        };

        match key.code {
            KeyCode::Esc => {
                self.modal = None;
                return TabAction::None;
            }
            KeyCode::Tab | KeyCode::Down => {
                form.focused = (form.focused + 1) % NewItemForm::FIELD_COUNT;
                return TabAction::None;
            }
            KeyCode::BackTab | KeyCode::Up => {
                form.focused =
                    (form.focused + NewItemForm::FIELD_COUNT - 1) % NewItemForm::FIELD_COUNT;
                return TabAction::None;
            }
            KeyCode::Backspace => {
                match form.focused {
                    0 => {
                        form.vendor_name.pop();
                    }
                    1 => {
                        form.description.pop();
                    }
                    2 => {
                        form.amount_str.pop();
                    }
                    3 => {
                        form.due_date_str.pop();
                    }
                    4 => {
                        form.je_id_str.pop();
                    }
                    _ => {}
                }
                return TabAction::None;
            }
            KeyCode::Char(c) => {
                match form.focused {
                    0 => form.vendor_name.push(c),
                    1 => form.description.push(c),
                    2 => form.amount_str.push(c),
                    3 => form.due_date_str.push(c),
                    4 => form.je_id_str.push(c),
                    _ => {}
                }
                return TabAction::None;
            }
            KeyCode::Enter => {
                if form.focused < NewItemForm::FIELD_COUNT - 1 {
                    form.focused += 1;
                    return TabAction::None;
                }
                // Submit — fall through below.
            }
            _ => return TabAction::None,
        }

        // Collect values before submitting (avoids borrow conflict).
        let (vendor_name, description, amount_str, due_date_str, je_id_str) = match &self.modal {
            Some(ApModal::NewItem(f)) => (
                f.vendor_name.trim().to_string(),
                f.description.trim().to_string(),
                f.amount_str.clone(),
                f.due_date_str.clone(),
                f.je_id_str.clone(),
            ),
            _ => return TabAction::None,
        };

        if vendor_name.is_empty() {
            if let Some(ApModal::NewItem(f)) = &mut self.modal {
                f.error = Some("Vendor name is required".to_string());
                f.focused = 0;
            }
            return TabAction::None;
        }

        let amount = match parse_money(&amount_str) {
            Ok(amt) if amt.0 > 0 => amt,
            _ => {
                if let Some(ApModal::NewItem(f)) = &mut self.modal {
                    f.error = Some("Amount must be a positive value (e.g. 1234.56)".to_string());
                    f.focused = 2;
                }
                return TabAction::None;
            }
        };

        let due_date = match NaiveDate::parse_from_str(&due_date_str, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => {
                if let Some(ApModal::NewItem(f)) = &mut self.modal {
                    f.error = Some(format!(
                        "Invalid due date '{}' — use YYYY-MM-DD",
                        due_date_str
                    ));
                    f.focused = 3;
                }
                return TabAction::None;
            }
        };

        let originating_je_id = match je_id_str.trim().parse::<i64>() {
            Ok(n) if n > 0 => JournalEntryId::from(n),
            _ => {
                if let Some(ApModal::NewItem(f)) = &mut self.modal {
                    f.error = Some("Enter the originating JE ID (positive integer)".to_string());
                    f.focused = 4;
                }
                return TabAction::None;
            }
        };

        // AP account 2100 — Accounts Payable.
        let ap_account = match db.accounts().list_active().ok().and_then(|accounts| {
            accounts
                .into_iter()
                .find(|a| a.number == "2100" && !a.is_placeholder)
        }) {
            Some(a) => a.id,
            None => {
                if let Some(ApModal::NewItem(f)) = &mut self.modal {
                    f.error = Some(
                        "AP account 2100 not found — create it in Chart of Accounts first"
                            .to_string(),
                    );
                }
                return TabAction::None;
            }
        };

        let new_item = NewApItem {
            account_id: ap_account,
            vendor_name: vendor_name.clone(),
            description: if description.is_empty() {
                None
            } else {
                Some(description)
            },
            amount,
            due_date,
            originating_je_id,
        };

        match db.ap().create_item(&new_item) {
            Ok(item_id) => {
                let entity_name = self.entity_name.clone();
                if let Err(e) = db.audit().append(
                    AuditAction::ApItemCreated,
                    &entity_name,
                    Some("ApItem"),
                    Some(i64::from(item_id)),
                    &format!("AP item created: {} — {}", vendor_name, amount),
                ) {
                    tracing::warn!("AP tab: audit log write failed: {e}");
                }
                self.modal = None;
                self.reload(db);
                TabAction::ShowMessage(format!("AP item created for {}", vendor_name))
            }
            Err(e) => {
                if let Some(ApModal::NewItem(f)) = &mut self.modal {
                    f.error = Some(format!("Failed to create item: {e}"));
                }
                TabAction::None
            }
        }
    }

    fn handle_payment_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        let form = match &mut self.modal {
            Some(ApModal::Payment(f)) => f,
            _ => return TabAction::None,
        };

        match key.code {
            KeyCode::Esc => {
                self.modal = None;
                return TabAction::None;
            }
            KeyCode::Tab | KeyCode::Down => {
                form.focused = (form.focused + 1) % PaymentForm::FIELD_COUNT;
                return TabAction::None;
            }
            KeyCode::BackTab | KeyCode::Up => {
                form.focused =
                    (form.focused + PaymentForm::FIELD_COUNT - 1) % PaymentForm::FIELD_COUNT;
                return TabAction::None;
            }
            KeyCode::Backspace => {
                match form.focused {
                    0 => {
                        form.amount_str.pop();
                    }
                    1 => {
                        form.date_str.pop();
                    }
                    2 => {
                        form.je_id_str.pop();
                    }
                    _ => {}
                }
                return TabAction::None;
            }
            KeyCode::Char(c) => {
                match form.focused {
                    0 => form.amount_str.push(c),
                    1 => form.date_str.push(c),
                    2 => form.je_id_str.push(c),
                    _ => {}
                }
                return TabAction::None;
            }
            KeyCode::Enter => {
                if form.focused < PaymentForm::FIELD_COUNT - 1 {
                    form.focused += 1;
                    return TabAction::None;
                }
                // Submit — fall through below.
            }
            _ => return TabAction::None,
        }

        // Collect values before submitting.
        let (item_id, amount_str, date_str, je_id_str) = match &self.modal {
            Some(ApModal::Payment(f)) => (
                f.item_id,
                f.amount_str.clone(),
                f.date_str.clone(),
                f.je_id_str.clone(),
            ),
            _ => return TabAction::None,
        };

        let amount = match parse_money(&amount_str) {
            Ok(amt) if amt.0 > 0 => amt,
            _ => {
                if let Some(ApModal::Payment(f)) = &mut self.modal {
                    f.error = Some("Amount must be a positive value (e.g. 1234.56)".to_string());
                    f.focused = 0;
                }
                return TabAction::None;
            }
        };

        let payment_date = match NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => {
                if let Some(ApModal::Payment(f)) = &mut self.modal {
                    f.error = Some(format!("Invalid date '{}' — use YYYY-MM-DD", date_str));
                    f.focused = 1;
                }
                return TabAction::None;
            }
        };

        let je_id = match je_id_str.trim().parse::<i64>() {
            Ok(n) if n > 0 => JournalEntryId::from(n),
            _ => {
                if let Some(ApModal::Payment(f)) = &mut self.modal {
                    f.error = Some("Enter the payment JE ID (positive integer)".to_string());
                    f.focused = 2;
                }
                return TabAction::None;
            }
        };

        match db.ap().record_payment(item_id, je_id, amount, payment_date) {
            Ok(()) => {
                let entity_name = self.entity_name.clone();
                if let Err(e) = db.audit().append(
                    AuditAction::ApPaymentRecorded,
                    &entity_name,
                    Some("ApItem"),
                    Some(i64::from(item_id)),
                    &format!("Payment of {} recorded on {}", amount, payment_date),
                ) {
                    tracing::warn!("AP tab: audit log write failed: {e}");
                }
                self.modal = None;
                self.reload(db);
                TabAction::ShowMessage(format!("Payment of {} recorded", amount))
            }
            Err(e) => {
                if let Some(ApModal::Payment(f)) = &mut self.modal {
                    f.error = Some(format!("{e}"));
                }
                TabAction::None
            }
        }
    }

    fn handle_history_key(&mut self, key: KeyEvent) -> TabAction {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                self.modal = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(ApModal::PaymentHistory(h)) = &mut self.modal
                    && h.scroll > 0
                {
                    h.scroll -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(ApModal::PaymentHistory(h)) = &mut self.modal
                    && !h.payments.is_empty()
                    && h.scroll + 1 < h.payments.len()
                {
                    h.scroll += 1;
                }
            }
            _ => {}
        }
        TabAction::None
    }

    // ── Render helpers ────────────────────────────────────────────────────────

    fn render_table(&self, frame: &mut Frame, area: Rect) {
        let today = chrono::Local::now().date_naive();

        let header = Row::new(vec![
            Cell::from("Vendor").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Description").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Amount").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Paid").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Remaining").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Due Date").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Status").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Days").style(Style::default().add_modifier(Modifier::BOLD)),
        ])
        .style(Style::default().bg(Color::DarkGray));

        let table_rows: Vec<Row> = self
            .items
            .iter()
            .map(|item| {
                let paid = infer_paid_amount(item);
                let remaining = Money(item.amount.0 - paid.0);
                let days_outstanding = (today - item.due_date).num_days();
                let is_overdue = item.due_date < today && item.status != ArApStatus::Paid;

                let days_str = if item.status == ArApStatus::Paid {
                    "Paid".to_string()
                } else if days_outstanding > 0 {
                    format!("{}d over", days_outstanding)
                } else if days_outstanding == 0 {
                    "Due today".to_string()
                } else {
                    format!("{}d left", -days_outstanding)
                };

                let row_style = if is_overdue {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default()
                };

                Row::new(vec![
                    Cell::from(item.vendor_name.clone()),
                    Cell::from(item.description.as_deref().unwrap_or("").to_string()),
                    Cell::from(item.amount.to_string()),
                    Cell::from(paid.to_string()),
                    Cell::from(remaining.to_string()),
                    Cell::from(item.due_date.to_string()),
                    Cell::from(item.status.to_string()),
                    Cell::from(days_str),
                ])
                .style(row_style)
            })
            .collect();

        let filter_label = self.status_filter.label();
        let title = format!(" Accounts Payable  [Filter: {filter_label}] ");

        let table = Table::new(
            table_rows,
            [
                Constraint::Min(15),    // Vendor
                Constraint::Min(15),    // Description
                Constraint::Length(12), // Amount
                Constraint::Length(12), // Paid
                Constraint::Length(12), // Remaining
                Constraint::Length(10), // Due Date
                Constraint::Length(8),  // Status
                Constraint::Length(10), // Days
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

    fn render_new_item_modal(&self, frame: &mut Frame, area: Rect, form: &NewItemForm) {
        let modal_area = centered_rect(64, 60, area);
        frame.render_widget(Clear, modal_area);

        let labels = [
            "Vendor Name *  ",
            "Description    ",
            "Amount *       ",
            "Due Date *     ",
            "Orig. JE ID *  ",
        ];
        let values = [
            form.vendor_name.as_str(),
            form.description.as_str(),
            form.amount_str.as_str(),
            form.due_date_str.as_str(),
            form.je_id_str.as_str(),
        ];

        let mut lines = vec![Line::from(Span::raw(""))];
        for (i, (label, value)) in labels.iter().zip(values.iter()).enumerate() {
            let cursor = if i == form.focused { "█" } else { "" };
            let style = if i == form.focused {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {label} "), style),
                Span::raw(*value),
                Span::styled(cursor, Style::default().fg(Color::Yellow)),
            ]));
        }
        if let Some(err) = &form.error {
            lines.push(Line::from(Span::raw("")));
            lines.push(Line::from(Span::styled(
                format!("  {err}"),
                Style::default().fg(Color::Red),
            )));
        }
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "  Tab/↑↓: next field  Enter: advance/submit  Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )));

        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" New AP Item ")
                    .style(Style::default().fg(Color::Cyan)),
            ),
            modal_area,
        );
    }

    fn render_payment_modal(&self, frame: &mut Frame, area: Rect, form: &PaymentForm) {
        let modal_area = centered_rect(56, 50, area);
        frame.render_widget(Clear, modal_area);

        let labels = ["Amount *       ", "Payment Date * ", "Payment JE ID *"];
        let values = [
            form.amount_str.as_str(),
            form.date_str.as_str(),
            form.je_id_str.as_str(),
        ];

        let mut lines = vec![Line::from(Span::raw(""))];
        for (i, (label, value)) in labels.iter().zip(values.iter()).enumerate() {
            let cursor = if i == form.focused { "█" } else { "" };
            let style = if i == form.focused {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {label} "), style),
                Span::raw(*value),
                Span::styled(cursor, Style::default().fg(Color::Yellow)),
            ]));
        }
        if let Some(err) = &form.error {
            lines.push(Line::from(Span::raw("")));
            lines.push(Line::from(Span::styled(
                format!("  {err}"),
                Style::default().fg(Color::Red),
            )));
        }
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "  Tab/↑↓: next field  Enter: advance/submit  Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )));

        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Record Payment ")
                    .style(Style::default().fg(Color::Green)),
            ),
            modal_area,
        );
    }

    fn render_payment_history(&self, frame: &mut Frame, area: Rect, hist: &PaymentHistoryView) {
        let modal_area = centered_rect(70, 70, area);
        frame.render_widget(Clear, modal_area);

        let mut lines = vec![
            Line::from(Span::raw("")),
            Line::from(vec![
                Span::styled("  Vendor:   ", Style::default().fg(Color::DarkGray)),
                Span::raw(hist.item.vendor_name.clone()),
            ]),
            Line::from(vec![
                Span::styled("  Amount:   ", Style::default().fg(Color::DarkGray)),
                Span::raw(hist.item.amount.to_string()),
            ]),
            Line::from(vec![
                Span::styled("  Status:   ", Style::default().fg(Color::DarkGray)),
                Span::raw(hist.item.status.to_string()),
            ]),
            Line::from(Span::raw("")),
        ];

        if hist.payments.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No payments recorded.",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "  Date        Amount         JE ID",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for p in hist.payments.iter().skip(hist.scroll) {
                lines.push(Line::from(format!(
                    "  {}  {:>12}  #{}",
                    p.payment_date,
                    p.amount.to_string(),
                    i64::from(p.je_id)
                )));
            }
        }

        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "  Esc/Enter: close",
            Style::default().fg(Color::DarkGray),
        )));

        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Payment History ")
                    .style(Style::default().fg(Color::Cyan)),
            ),
            modal_area,
        );
    }
}

// ── Infer paid amount from status (list-view approximation) ──────────────────
fn infer_paid_amount(item: &ApItem) -> Money {
    match item.status {
        ArApStatus::Paid => item.amount,
        ArApStatus::Open | ArApStatus::Partial => Money(0),
    }
}

// ── Tab trait ─────────────────────────────────────────────────────────────────

impl Tab for AccountsPayableTab {
    fn title(&self) -> &str {
        "Accounts Payable"
    }

    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        // Modal dispatch first.
        match &self.modal {
            Some(ApModal::NewItem(_)) => return self.handle_new_item_key(key, db),
            Some(ApModal::Payment(_)) => return self.handle_payment_key(key, db),
            Some(ApModal::PaymentHistory(_)) => return self.handle_history_key(key),
            None => {}
        }

        if key.modifiers != KeyModifiers::NONE && key.modifiers != KeyModifiers::SHIFT {
            return TabAction::None;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(),
            KeyCode::Char('n') => {
                self.modal = Some(ApModal::NewItem(NewItemForm::new()));
            }
            KeyCode::Char('p') => {
                if let Some(item) = self.selected_item() {
                    if item.status == ArApStatus::Paid {
                        return TabAction::ShowMessage(
                            "This item is already fully paid".to_string(),
                        );
                    }
                    let item_id = item.id;
                    self.modal = Some(ApModal::Payment(PaymentForm::new(item_id)));
                }
            }
            KeyCode::Enter => {
                if let Some(item) = self.selected_item() {
                    match db.ap().get_with_payments(item.id) {
                        Ok((item, payments)) => {
                            self.modal = Some(ApModal::PaymentHistory(PaymentHistoryView {
                                item,
                                payments,
                                scroll: 0,
                            }));
                        }
                        Err(e) => {
                            return TabAction::ShowMessage(format!(
                                "Failed to load payment history: {e}"
                            ));
                        }
                    }
                }
            }
            KeyCode::Char('o') => {
                if let Some(item) = self.selected_item() {
                    return TabAction::NavigateTo(
                        TabId::JournalEntries,
                        RecordId::JournalEntry(item.originating_je_id),
                    );
                }
            }
            KeyCode::Char('s') | KeyCode::Char('f') => {
                self.status_filter = self.status_filter.next();
                self.reload(db);
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

        self.render_table(frame, chunks[0]);

        let count = self.items.len();
        let selected = self.table_state.selected().map(|i| i + 1).unwrap_or(0);
        let hint = Line::from(vec![
            Span::styled(
                " n: new  p: payment  Enter: history  o: open JE  s: cycle filter  ↑↓/jk: navigate",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("  [{}/{}]", selected, count),
                Style::default().fg(Color::Gray),
            ),
        ]);
        frame.render_widget(Paragraph::new(hint), chunks[1]);

        // Render modal overlay on top.
        if let Some(ref modal) = self.modal {
            match modal {
                ApModal::NewItem(form) => self.render_new_item_modal(frame, area, form),
                ApModal::Payment(form) => self.render_payment_modal(frame, area, form),
                ApModal::PaymentHistory(hist) => self.render_payment_history(frame, area, hist),
            }
        }
    }

    fn wants_input(&self) -> bool {
        self.modal.is_some()
    }

    fn refresh(&mut self, db: &EntityDb) {
        self.reload(db);
    }

    fn navigate_to(&mut self, record_id: RecordId, _db: &EntityDb) {
        let _ = record_id;
    }
}
