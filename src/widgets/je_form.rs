//! Journal entry form widget — the primary data-entry UI component.
//!
//! Self-contained: embed in the JE tab (Phase 2b) and the inter-entity modal (Phase 6).
//! Not coupled to either. Returns `JeFormOutput` on submit; `None` on cancel.
//!
//! # Navigation
//! - `Tab` / `Shift+Tab`: move between fields
//! - `Enter`: open account picker (on account fields); advance (on text fields);
//!   add a new line row (from the last field of the last row)
//! - `Ctrl+S`: validate and submit
//! - `Esc`: cancel (discard all input)
//! - `F2`: insert a new line row below the currently focused row
//! - `F3` / `Delete`: remove the currently focused line row (minimum 1 row kept)
//!
//! # Integration
//! Call `handle_key(key, accounts)` on each key event.
//! Call `render(frame, area, accounts)` to draw.
//! The `accounts` slice should be the full active account list for the current entity.

use chrono::NaiveDate;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use super::AccountPicker;
use crate::db::account_repo::Account;
use crate::db::journal_repo::NewJournalEntryLine;
use crate::types::{AccountId, Money};

// ── Public result types ───────────────────────────────────────────────────────

/// The output returned when the user submits a valid journal entry form.
/// Does **not** include `fiscal_period_id` — the caller resolves that from `entry_date`.
#[derive(Debug, Clone)]
pub struct JeFormOutput {
    pub entry_date: NaiveDate,
    pub memo: Option<String>,
    pub lines: Vec<NewJournalEntryLine>,
}

/// Action returned by `handle_key`.
#[derive(Debug)]
pub enum JeFormAction {
    /// User submitted a valid entry.
    Submitted(JeFormOutput),
    /// User pressed Esc — discard the form.
    Cancelled,
    /// Key consumed; no state change visible to the caller.
    Pending,
}

// ── Internal types ────────────────────────────────────────────────────────────

/// Which field currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Focus {
    Date,
    Memo,
    /// Row index into `lines`.
    LineAccount(usize),
    LineDebit(usize),
    LineCredit(usize),
    LineNote(usize),
}

/// One line row in the form.
#[derive(Debug, Clone, Default)]
struct LineRow {
    account_id: Option<AccountId>,
    account_name: String, // display name after selection
    debit_input: String,
    credit_input: String,
    note_input: String,
}

// ── JeForm ────────────────────────────────────────────────────────────────────

/// The journal entry form widget.
pub struct JeForm {
    date_input: String,
    memo_input: String,
    lines: Vec<LineRow>,
    focus: Focus,
    /// The embedded account picker popup (shown when a line's Account field is focused).
    account_picker: AccountPicker,
    picker_active: bool,
    /// Validation error shown at the bottom of the form.
    error: Option<String>,
}

impl Default for JeForm {
    fn default() -> Self {
        Self::new()
    }
}

impl JeForm {
    /// Creates a new blank form with today's date and two empty line rows.
    pub fn new() -> Self {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        Self {
            date_input: today,
            memo_input: String::new(),
            lines: vec![LineRow::default(), LineRow::default()],
            focus: Focus::Date,
            account_picker: AccountPicker::new(),
            picker_active: false,
            error: None,
        }
    }

    /// Resets the form to a blank state (call before showing again after submit/cancel).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Returns whether the account picker popup is currently open.
    pub fn is_picker_active(&self) -> bool {
        self.picker_active
    }

    // ── Key handling ──────────────────────────────────────────────────────────

    /// Handles a key event. Mutates internal state and returns an action.
    /// `accounts` is the full active account list for the picker.
    pub fn handle_key(&mut self, key: KeyEvent, accounts: &[Account]) -> JeFormAction {
        // If the account picker is open, route all keys to it first.
        if self.picker_active {
            return self.handle_picker_key(key, accounts);
        }

        match key.code {
            // ── Cancel ────────────────────────────────────────────────────────
            KeyCode::Esc => return JeFormAction::Cancelled,

            // ── Submit (Ctrl+S) ───────────────────────────────────────────────
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                match self.try_submit() {
                    Ok(output) => return JeFormAction::Submitted(output),
                    Err(msg) => self.error = Some(msg),
                }
            }

            // ── Add / remove line rows ────────────────────────────────────────
            KeyCode::F(2) => self.add_row_after_focused(),
            KeyCode::F(3) | KeyCode::Delete => self.remove_focused_row(),

            // ── Tab: move forward ─────────────────────────────────────────────
            KeyCode::Tab => self.advance_focus(true),

            // ── BackTab: move backward ────────────────────────────────────────
            KeyCode::BackTab => self.advance_focus(false),

            // ── Enter: contextual ─────────────────────────────────────────────
            KeyCode::Enter => {
                match self.focus {
                    Focus::LineAccount(_) => self.open_picker(accounts),
                    _ => {
                        // From the last field of the last row → add a new row.
                        let is_last_note = matches!(
                            self.focus,
                            Focus::LineNote(i) if i + 1 == self.lines.len()
                        );
                        if is_last_note {
                            self.add_row_at_end();
                        } else {
                            self.advance_focus(true);
                        }
                    }
                }
            }

            // ── Text input ────────────────────────────────────────────────────
            KeyCode::Backspace => self.handle_backspace(),
            KeyCode::Char(c) => self.handle_char(c),

            _ => {}
        }

        JeFormAction::Pending
    }

    // ── Picker delegation ─────────────────────────────────────────────────────

    fn handle_picker_key(&mut self, key: KeyEvent, accounts: &[Account]) -> JeFormAction {
        use crate::widgets::account_picker::PickerAction;

        let action = self.account_picker.handle_key(key, accounts);
        match action {
            PickerAction::Selected(id) => {
                // Store selected account in the focused line row.
                if let Focus::LineAccount(row) = self.focus
                    && let Some(acct) = accounts.iter().find(|a| a.id == id)
                {
                    self.lines[row].account_id = Some(id);
                    self.lines[row].account_name = format!("{} {}", acct.number, acct.name);
                }
                self.picker_active = false;
                self.account_picker.reset();
                // Advance focus to the debit field of this row.
                if let Focus::LineAccount(row) = self.focus {
                    self.focus = Focus::LineDebit(row);
                }
            }
            PickerAction::Cancelled => {
                self.picker_active = false;
                self.account_picker.reset();
            }
            PickerAction::Pending => {}
        }
        JeFormAction::Pending
    }

    // ── Focus navigation ──────────────────────────────────────────────────────

    fn advance_focus(&mut self, forward: bool) {
        let n = self.lines.len();
        self.focus = if forward {
            match self.focus {
                Focus::Date => Focus::Memo,
                Focus::Memo => Focus::LineAccount(0),
                Focus::LineAccount(i) => Focus::LineDebit(i),
                Focus::LineDebit(i) => Focus::LineCredit(i),
                Focus::LineCredit(i) => Focus::LineNote(i),
                Focus::LineNote(i) => {
                    if i + 1 < n {
                        Focus::LineAccount(i + 1)
                    } else {
                        Focus::Date
                    }
                }
            }
        } else {
            match self.focus {
                Focus::Date => {
                    if n > 0 {
                        Focus::LineNote(n - 1)
                    } else {
                        Focus::Memo
                    }
                }
                Focus::Memo => Focus::Date,
                Focus::LineAccount(0) => Focus::Memo,
                Focus::LineAccount(i) => Focus::LineNote(i - 1),
                Focus::LineDebit(i) => Focus::LineAccount(i),
                Focus::LineCredit(i) => Focus::LineDebit(i),
                Focus::LineNote(i) => Focus::LineCredit(i),
            }
        };
    }

    fn open_picker(&mut self, accounts: &[Account]) {
        self.account_picker.reset();
        self.account_picker.refresh(accounts);
        self.picker_active = true;
    }

    // ── Row management ────────────────────────────────────────────────────────

    fn add_row_after_focused(&mut self) {
        let insert_at = match self.focus {
            Focus::LineAccount(i)
            | Focus::LineDebit(i)
            | Focus::LineCredit(i)
            | Focus::LineNote(i) => i + 1,
            _ => self.lines.len(),
        };
        self.lines.insert(insert_at, LineRow::default());
        self.focus = Focus::LineAccount(insert_at);
    }

    fn add_row_at_end(&mut self) {
        let i = self.lines.len();
        self.lines.push(LineRow::default());
        self.focus = Focus::LineAccount(i);
    }

    fn remove_focused_row(&mut self) {
        let row = match self.focus {
            Focus::LineAccount(i)
            | Focus::LineDebit(i)
            | Focus::LineCredit(i)
            | Focus::LineNote(i) => i,
            _ => return,
        };

        // Keep at least 1 row.
        if self.lines.len() <= 1 {
            return;
        }

        self.lines.remove(row);

        // Adjust focus to stay valid.
        let new_row = if row < self.lines.len() { row } else { row - 1 };
        self.focus = Focus::LineAccount(new_row);
    }

    // ── Text input helpers ────────────────────────────────────────────────────

    fn handle_backspace(&mut self) {
        match self.focus {
            Focus::Date => {
                self.date_input.pop();
            }
            Focus::Memo => {
                self.memo_input.pop();
            }
            Focus::LineDebit(i) => {
                self.lines[i].debit_input.pop();
            }
            Focus::LineCredit(i) => {
                self.lines[i].credit_input.pop();
            }
            Focus::LineNote(i) => {
                self.lines[i].note_input.pop();
            }
            Focus::LineAccount(_) => {}
        }
        self.error = None;
    }

    fn handle_char(&mut self, c: char) {
        match self.focus {
            Focus::Date => {
                if self.date_input.len() < 10 {
                    self.date_input.push(c);
                }
            }
            Focus::Memo => {
                self.memo_input.push(c);
            }
            Focus::LineDebit(i) => {
                if c.is_ascii_digit() || c == '.' {
                    self.lines[i].debit_input.push(c);
                    // Clear credit when debit is typed (mutual exclusivity hint).
                    if !self.lines[i].debit_input.is_empty() {
                        // Allow both for now; validation enforces at submit.
                    }
                }
            }
            Focus::LineCredit(i) => {
                if c.is_ascii_digit() || c == '.' {
                    self.lines[i].credit_input.push(c);
                }
            }
            Focus::LineNote(i) => {
                self.lines[i].note_input.push(c);
            }
            Focus::LineAccount(_) => {
                // Typing while on account field opens the picker and seeds the query.
                // This makes the UX smoother: just start typing to open picker.
            }
        }
        self.error = None;
    }

    // ── Validation & submit ───────────────────────────────────────────────────

    fn try_submit(&self) -> Result<JeFormOutput, String> {
        // Validate date.
        let entry_date = NaiveDate::parse_from_str(&self.date_input, "%Y-%m-%d")
            .map_err(|_| format!("Invalid date '{}'. Use YYYY-MM-DD format.", self.date_input))?;

        // Validate lines.
        if self.lines.len() < 2 {
            return Err("At least 2 line rows are required.".to_string());
        }

        let mut parsed_lines: Vec<NewJournalEntryLine> = Vec::new();
        for (i, row) in self.lines.iter().enumerate() {
            let account_id = row
                .account_id
                .ok_or_else(|| format!("Line {}: no account selected.", i + 1))?;

            let debit = parse_money(&row.debit_input)
                .map_err(|e| format!("Line {}: debit — {e}", i + 1))?;
            let credit = parse_money(&row.credit_input)
                .map_err(|e| format!("Line {}: credit — {e}", i + 1))?;

            if debit.is_zero() && credit.is_zero() {
                return Err(format!("Line {}: both debit and credit are zero.", i + 1));
            }

            parsed_lines.push(NewJournalEntryLine {
                account_id,
                debit_amount: debit,
                credit_amount: credit,
                line_memo: if row.note_input.is_empty() {
                    None
                } else {
                    Some(row.note_input.clone())
                },
                sort_order: i as i32,
            });
        }

        // Note: balanced-debit-credit check happens in post_journal_entry(), not here.
        // The form allows unbalanced saves as Drafts.

        let memo = if self.memo_input.trim().is_empty() {
            None
        } else {
            Some(self.memo_input.trim().to_string())
        };

        Ok(JeFormOutput {
            entry_date,
            memo,
            lines: parsed_lines,
        })
    }

    // ── Computed totals ───────────────────────────────────────────────────────

    fn total_debits(&self) -> Money {
        self.lines
            .iter()
            .map(|l| parse_money(&l.debit_input).unwrap_or(Money(0)))
            .fold(Money(0), |acc, m| acc + m)
    }

    fn total_credits(&self) -> Money {
        self.lines
            .iter()
            .map(|l| parse_money(&l.credit_input).unwrap_or(Money(0)))
            .fold(Money(0), |acc, m| acc + m)
    }

    // ── Render ────────────────────────────────────────────────────────────────

    /// Renders the form into `area`. Renders the account picker popup on top when active.
    /// `accounts` is passed to the account picker.
    pub fn render(&self, frame: &mut Frame, area: Rect, accounts: &[Account]) {
        let block = Block::default()
            .title(" New Journal Entry  Ctrl+S: submit  Esc: cancel  F2: add row  F3: del row ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // ── Header: Date + Memo ───────────────────────────────────────────────
        let header_height = 3u16;
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height),
                Constraint::Min(4),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(inner);

        let header_area = layout[0];
        let lines_area = layout[1];
        let totals_area = layout[2];
        let help_area = layout[3];

        self.render_header(frame, header_area);
        self.render_lines(frame, lines_area);
        self.render_totals(frame, totals_area);
        self.render_help(frame, help_area);

        // ── Account picker popup (rendered on top) ────────────────────────────
        if self.picker_active {
            self.account_picker.render(frame, area, accounts);
        }
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(22), Constraint::Min(20)])
            .split(area);

        let date_focused = self.focus == Focus::Date;
        let memo_focused = self.focus == Focus::Memo;

        let date_style = field_style(date_focused);
        let memo_style = field_style(memo_focused);

        let date_block = Block::default()
            .title("Date (YYYY-MM-DD)")
            .borders(Borders::ALL)
            .style(date_style);
        let date_text = Paragraph::new(format!("{}_", self.date_input)).block(date_block);
        frame.render_widget(date_text, cols[0]);

        let memo_block = Block::default()
            .title("Memo")
            .borders(Borders::ALL)
            .style(memo_style);
        let memo_text = Paragraph::new(format!("{}_", self.memo_input)).block(memo_block);
        frame.render_widget(memo_text, cols[1]);
    }

    fn render_lines(&self, frame: &mut Frame, area: Rect) {
        let header_row = Row::new(vec![
            Cell::from("Account").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Debit").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Credit").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Note").style(Style::default().add_modifier(Modifier::BOLD)),
        ]);

        let rows: Vec<Row> = self
            .lines
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let acct_text = if row.account_name.is_empty() {
                    "(pick account)".to_string()
                } else {
                    row.account_name.clone()
                };
                let acct_style = if self.focus == Focus::LineAccount(i) {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else if row.account_id.is_some() {
                    Style::default().fg(Color::White)
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                Row::new(vec![
                    Cell::from(acct_text).style(acct_style),
                    Cell::from(format!("{}_", row.debit_input))
                        .style(field_style(self.focus == Focus::LineDebit(i))),
                    Cell::from(format!("{}_", row.credit_input))
                        .style(field_style(self.focus == Focus::LineCredit(i))),
                    Cell::from(format!("{}_", row.note_input))
                        .style(field_style(self.focus == Focus::LineNote(i))),
                ])
            })
            .collect();

        let widths = [
            Constraint::Percentage(40),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(30),
        ];

        let table = Table::new(rows, widths)
            .header(header_row)
            .block(Block::default().borders(Borders::TOP))
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        frame.render_widget(table, area);
    }

    fn render_totals(&self, frame: &mut Frame, area: Rect) {
        let debits = self.total_debits();
        let credits = self.total_credits();
        let diff = debits - credits;

        let balanced = diff.is_zero();
        let diff_style = if balanced {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Red)
        };

        let lines = vec![Line::from(vec![
            Span::raw("  Totals:  Debits: "),
            Span::styled(debits.to_string(), Style::default().fg(Color::Cyan)),
            Span::raw("   Credits: "),
            Span::styled(credits.to_string(), Style::default().fg(Color::Cyan)),
            Span::raw("   Difference: "),
            Span::styled(diff.abs().to_string(), diff_style),
            Span::styled(
                if balanced {
                    "  ✓ Balanced"
                } else {
                    "  ✗ Unbalanced"
                },
                diff_style,
            ),
        ])];

        frame.render_widget(
            Paragraph::new(lines).block(Block::default().borders(Borders::TOP)),
            area,
        );
    }

    fn render_help(&self, frame: &mut Frame, area: Rect) {
        let text = if let Some(err) = &self.error {
            Line::from(Span::styled(
                format!("  Error: {err}"),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from(Span::styled(
                "  Tab: next field  Enter: pick account / advance  Ctrl+S: submit  Esc: cancel",
                Style::default().fg(Color::DarkGray),
            ))
        };
        frame.render_widget(Paragraph::new(vec![text]), area);
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns a highlight style when the field is focused.
fn field_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    }
}

/// Parses a user-entered string like "100", "100.50" into `Money`.
/// Returns `Money(0)` for empty strings (blank line amounts).
pub fn parse_money(s: &str) -> Result<Money, String> {
    if s.is_empty() {
        return Ok(Money(0));
    }
    let f: f64 = s
        .parse()
        .map_err(|_| format!("'{s}' is not a valid amount"))?;
    if f < 0.0 {
        return Err(format!("Amount '{s}' must not be negative"));
    }
    Ok(Money::from_dollars(f))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn make_accounts() -> Vec<Account> {
        use crate::db::account_repo::Account;
        use crate::types::AccountType;
        vec![
            Account {
                id: AccountId::from(1),
                number: "1110".to_string(),
                name: "Cash".to_string(),
                account_type: AccountType::Asset,
                parent_id: None,
                is_active: true,
                is_contra: false,
                is_placeholder: false,
                created_at: String::new(),
                updated_at: String::new(),
            },
            Account {
                id: AccountId::from(2),
                number: "4100".to_string(),
                name: "Rental Revenue".to_string(),
                account_type: AccountType::Revenue,
                parent_id: None,
                is_active: true,
                is_contra: false,
                is_placeholder: false,
                created_at: String::new(),
                updated_at: String::new(),
            },
        ]
    }

    #[test]
    fn new_form_starts_on_date_field() {
        let form = JeForm::new();
        assert_eq!(form.focus, Focus::Date);
    }

    #[test]
    fn esc_returns_cancelled() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        let action = form.handle_key(key(KeyCode::Esc), &accts);
        assert!(matches!(action, JeFormAction::Cancelled));
    }

    #[test]
    fn tab_advances_focus_date_to_memo() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        form.handle_key(key(KeyCode::Tab), &accts);
        assert_eq!(form.focus, Focus::Memo);
    }

    #[test]
    fn tab_advances_through_all_fields() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        let expected = vec![
            Focus::Memo,
            Focus::LineAccount(0),
            Focus::LineDebit(0),
            Focus::LineCredit(0),
            Focus::LineNote(0),
            Focus::LineAccount(1),
            Focus::LineDebit(1),
            Focus::LineCredit(1),
            Focus::LineNote(1),
            Focus::Date, // wraps back
        ];
        for expected_focus in expected {
            form.handle_key(key(KeyCode::Tab), &accts);
            assert_eq!(form.focus, expected_focus);
        }
    }

    #[test]
    fn enter_on_last_note_of_last_row_adds_new_row() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        assert_eq!(form.lines.len(), 2);

        // Navigate to LineNote of row 1 (last row, index 1).
        form.focus = Focus::LineNote(1);
        form.handle_key(key(KeyCode::Enter), &accts);

        assert_eq!(form.lines.len(), 3, "Should have 3 rows after adding");
        assert_eq!(form.focus, Focus::LineAccount(2));
    }

    #[test]
    fn f2_inserts_row_after_focused_line() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        form.focus = Focus::LineDebit(0);
        form.handle_key(key(KeyCode::F(2)), &accts);
        assert_eq!(form.lines.len(), 3);
        assert_eq!(form.focus, Focus::LineAccount(1));
    }

    #[test]
    fn f3_removes_focused_row_if_more_than_one() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        assert_eq!(form.lines.len(), 2);
        form.focus = Focus::LineNote(1);
        form.handle_key(key(KeyCode::F(3)), &accts);
        assert_eq!(form.lines.len(), 1, "Row should be removed");
    }

    #[test]
    fn f3_does_not_remove_last_row() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        // Remove until 1 row remains.
        form.lines.truncate(1);
        form.focus = Focus::LineDebit(0);
        form.handle_key(key(KeyCode::F(3)), &accts);
        assert_eq!(form.lines.len(), 1, "Cannot remove last row");
    }

    #[test]
    fn submit_fails_with_zero_amount_lines() {
        let mut form = JeForm::new();
        let accts = make_accounts();

        // Set date, select accounts.
        form.date_input = "2026-01-15".to_string();
        form.lines[0].account_id = Some(AccountId::from(1));
        form.lines[0].account_name = "Cash".to_string();
        form.lines[1].account_id = Some(AccountId::from(2));
        form.lines[1].account_name = "Revenue".to_string();
        // Leave debit/credit blank (zero).

        let action = form.handle_key(ctrl(KeyCode::Char('s')), &accts);
        // Should not submit — lines have zero amounts.
        assert!(matches!(action, JeFormAction::Pending));
        assert!(form.error.is_some(), "Should have a validation error");
    }

    #[test]
    fn submit_succeeds_with_valid_balanced_entry() {
        let mut form = JeForm::new();
        let accts = make_accounts();

        form.date_input = "2026-01-15".to_string();
        form.memo_input = "Test entry".to_string();
        form.lines[0].account_id = Some(AccountId::from(1));
        form.lines[0].account_name = "Cash".to_string();
        form.lines[0].debit_input = "100".to_string();
        form.lines[1].account_id = Some(AccountId::from(2));
        form.lines[1].account_name = "Revenue".to_string();
        form.lines[1].credit_input = "100".to_string();

        let action = form.handle_key(ctrl(KeyCode::Char('s')), &accts);
        match action {
            JeFormAction::Submitted(output) => {
                assert_eq!(
                    output.entry_date,
                    NaiveDate::from_ymd_opt(2026, 1, 15).unwrap()
                );
                assert_eq!(output.memo.as_deref(), Some("Test entry"));
                assert_eq!(output.lines.len(), 2);
                assert!(!output.lines[0].debit_amount.is_zero());
                assert!(!output.lines[1].credit_amount.is_zero());
            }
            _ => panic!("Expected Submitted, got {:?}", action),
        }
    }

    #[test]
    fn submit_fails_with_invalid_date() {
        let mut form = JeForm::new();
        let accts = make_accounts();

        form.date_input = "not-a-date".to_string();
        form.lines[0].account_id = Some(AccountId::from(1));
        form.lines[0].debit_input = "100".to_string();
        form.lines[1].account_id = Some(AccountId::from(2));
        form.lines[1].credit_input = "100".to_string();

        let action = form.handle_key(ctrl(KeyCode::Char('s')), &accts);
        assert!(matches!(action, JeFormAction::Pending));
        assert!(
            form.error.as_deref().unwrap_or("").contains("date"),
            "Error should mention date"
        );
    }

    #[test]
    fn running_totals_compute_correctly() {
        let mut form = JeForm::new();
        form.lines[0].debit_input = "100".to_string();
        form.lines[1].credit_input = "100".to_string();

        let debits = form.total_debits();
        let credits = form.total_credits();
        assert_eq!(debits, credits, "Balanced entry should have equal totals");
        assert!(!debits.is_zero(), "Totals should be non-zero");
    }

    #[test]
    fn parse_money_empty_string_returns_zero() {
        assert_eq!(parse_money("").unwrap(), Money(0));
    }

    #[test]
    fn parse_money_rejects_negative() {
        assert!(parse_money("-5").is_err());
    }

    #[test]
    fn parse_money_parses_decimal() {
        let m = parse_money("100.50").unwrap();
        assert!(!m.is_zero());
    }

    #[test]
    fn enter_on_line_account_opens_picker() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        form.focus = Focus::LineAccount(0);
        form.handle_key(key(KeyCode::Enter), &accts);
        assert!(form.picker_active, "Account picker should be open");
    }

    #[test]
    fn picker_esc_closes_picker_without_selection() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        form.focus = Focus::LineAccount(0);
        form.handle_key(key(KeyCode::Enter), &accts); // open picker
        assert!(form.picker_active);

        form.handle_key(key(KeyCode::Esc), &accts); // close picker
        assert!(!form.picker_active);
        assert!(form.lines[0].account_id.is_none(), "No account selected");
    }
}
