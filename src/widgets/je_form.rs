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
//! - `Ctrl+Down`: insert a new line row below the currently focused row
//! - `Ctrl+Up` / `Delete`: remove the currently focused line row (minimum 1 row kept)
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

use std::collections::HashMap;

use super::AccountPicker;
use crate::db::account_repo::Account;
use crate::db::journal_repo::{JournalEntry, JournalEntryLine, NewJournalEntryLine};
use crate::types::{AccountId, JournalEntryId, Money};

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
    /// Title displayed in the form border.
    title: String,
    /// Set when editing an existing draft; `None` when creating a new entry.
    editing_id: Option<JournalEntryId>,
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
            title:
                " New Journal Entry  Ctrl+S: submit  Esc: cancel  Ctrl+↓: add row  Ctrl+↑: del row "
                    .to_string(),
            editing_id: None,
            date_input: today,
            memo_input: String::new(),
            lines: vec![LineRow::default(), LineRow::default()],
            focus: Focus::Date,
            account_picker: AccountPicker::new(),
            picker_active: false,
            error: None,
        }
    }

    /// Creates a form pre-populated with data from an existing draft journal entry.
    /// The caller is responsible for ensuring `entry` has Draft status.
    pub fn from_existing(
        entry: &JournalEntry,
        lines: &[JournalEntryLine],
        accounts: &[Account],
    ) -> Self {
        let mut form = Self::new();
        form.title = format!(
            " Edit Draft JE #{}  Ctrl+S: save  Esc: cancel  Ctrl+↓: add row  Ctrl+↑: del row ",
            entry.je_number
        );
        form.editing_id = Some(entry.id);
        form.date_input = entry.entry_date.format("%Y-%m-%d").to_string();
        form.memo_input = entry.memo.clone().unwrap_or_default();

        // Build line rows sorted by sort_order.
        let mut sorted = lines.to_vec();
        sorted.sort_by_key(|l| l.sort_order);

        form.lines = sorted
            .iter()
            .map(|l| {
                let account_name = accounts
                    .iter()
                    .find(|a| a.id == l.account_id)
                    .map(|a| format!("{} {}", a.number, a.name))
                    .unwrap_or_default();
                LineRow {
                    account_id: Some(l.account_id),
                    account_name,
                    debit_input: Self::money_to_input_str(l.debit_amount),
                    credit_input: Self::money_to_input_str(l.credit_amount),
                    note_input: l.line_memo.clone().unwrap_or_default(),
                }
            })
            .collect();

        // Ensure at least 2 rows.
        while form.lines.len() < 2 {
            form.lines.push(LineRow::default());
        }

        form.focus = Focus::Date;
        form
    }

    /// Returns the ID of the entry being edited, or `None` if this is a new entry form.
    pub fn editing_id(&self) -> Option<JournalEntryId> {
        self.editing_id
    }

    /// Converts a `Money` value to a display string suitable for the amount input fields.
    /// Returns an empty string for zero (leaves the field blank).
    fn money_to_input_str(m: Money) -> String {
        if m.is_zero() {
            return String::new();
        }
        let units = m.0;
        let dollars = units / 100_000_000;
        // Fractional part scaled to 2 decimal places (cents).
        let cents = (units % 100_000_000) / 1_000_000;
        if cents == 0 {
            format!("{dollars}")
        } else {
            format!("{dollars}.{cents:02}")
        }
    }

    /// Resets the form to a blank state (call before showing again after submit/cancel).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Test helper: set date and line data directly without driving key events.
    /// Available only in test builds so private fields stay private in production.
    #[cfg(test)]
    pub(crate) fn set_test_state(&mut self, date: &str, lines: &[(AccountId, &str, &str)]) {
        self.date_input = date.to_string();
        while self.lines.len() < lines.len() {
            self.lines.push(LineRow::default());
        }
        for (i, (id, debit, credit)) in lines.iter().enumerate() {
            self.lines[i].account_id = Some(*id);
            self.lines[i].debit_input = debit.to_string();
            self.lines[i].credit_input = credit.to_string();
        }
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
            KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.add_row_after_focused()
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.remove_focused_row()
            }
            KeyCode::Delete => self.remove_focused_row(),

            // ── Tab: move forward ─────────────────────────────────────────────
            KeyCode::Tab => self.advance_focus(true),

            // ── BackTab: move backward ────────────────────────────────────────
            KeyCode::BackTab => self.advance_focus(false),

            // ── Enter: contextual ─────────────────────────────────────────────
            KeyCode::Enter => {
                match self.focus {
                    Focus::LineAccount(_) => self.open_picker(accounts),
                    _ => {
                        // From the last credit field of the last row → add a new row.
                        let is_last_credit = matches!(
                            self.focus,
                            Focus::LineCredit(i) if i + 1 == self.lines.len()
                        );
                        if is_last_credit {
                            self.add_row_at_end();
                        } else {
                            self.advance_focus(true);
                        }
                    }
                }
            }

            // ── Arrow navigation ──────────────────────────────────────────────
            // Down/Up move between rows; Left/Right move between columns in a row.
            // These mirror Tab/BackTab as a fallback when Tab is intercepted by
            // the chat panel (V2 focus model).
            KeyCode::Down => self.move_focus_down(),
            KeyCode::Up => self.move_focus_up(),
            KeyCode::Right => self.move_focus_right(),
            KeyCode::Left => self.move_focus_left(),

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
                Focus::LineCredit(i) => {
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
                        Focus::LineCredit(n - 1)
                    } else {
                        Focus::Memo
                    }
                }
                Focus::Memo => Focus::Date,
                Focus::LineAccount(0) => Focus::Memo,
                Focus::LineAccount(i) => Focus::LineCredit(i - 1),
                Focus::LineDebit(i) => Focus::LineAccount(i),
                Focus::LineCredit(i) => Focus::LineDebit(i),
            }
        };
    }

    /// Move down one row. From the header (Date/Memo), jumps to LineAccount(0).
    /// From a line row, moves to the same column type on the next row.
    fn move_focus_down(&mut self) {
        match self.focus {
            Focus::Date | Focus::Memo => {
                if !self.lines.is_empty() {
                    self.focus = Focus::LineAccount(0);
                }
            }
            Focus::LineAccount(i) => {
                if i + 1 < self.lines.len() {
                    self.focus = Focus::LineAccount(i + 1);
                }
            }
            Focus::LineDebit(i) => {
                if i + 1 < self.lines.len() {
                    self.focus = Focus::LineDebit(i + 1);
                }
            }
            Focus::LineCredit(i) => {
                if i + 1 < self.lines.len() {
                    self.focus = Focus::LineCredit(i + 1);
                }
            }
        }
    }

    /// Move up one row. From LineX(0), jumps back to Date. From LineX(i), moves
    /// to the same column type on the previous row.
    fn move_focus_up(&mut self) {
        match self.focus {
            Focus::Date | Focus::Memo => {}
            Focus::LineAccount(0) | Focus::LineDebit(0) | Focus::LineCredit(0) => {
                self.focus = Focus::Date;
            }
            Focus::LineAccount(i) => self.focus = Focus::LineAccount(i - 1),
            Focus::LineDebit(i) => self.focus = Focus::LineDebit(i - 1),
            Focus::LineCredit(i) => self.focus = Focus::LineCredit(i - 1),
        }
    }

    /// Move right one column within the same line row.
    /// Column order: Account → Debit → Credit. Does nothing at Credit or on header fields.
    fn move_focus_right(&mut self) {
        self.focus = match self.focus {
            Focus::LineAccount(i) => Focus::LineDebit(i),
            Focus::LineDebit(i) => Focus::LineCredit(i),
            other => other,
        };
    }

    /// Move left one column within the same line row.
    /// Column order: Credit → Debit → Account. Does nothing at Account or on header fields.
    fn move_focus_left(&mut self) {
        self.focus = match self.focus {
            Focus::LineCredit(i) => Focus::LineDebit(i),
            Focus::LineDebit(i) => Focus::LineAccount(i),
            other => other,
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
            Focus::LineAccount(i) | Focus::LineDebit(i) | Focus::LineCredit(i) => i + 1,
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
            Focus::LineAccount(i) | Focus::LineDebit(i) | Focus::LineCredit(i) => i,
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
                    // Allow both for now; validation enforces at submit.
                }
            }
            Focus::LineCredit(i) => {
                if c.is_ascii_digit() || c == '.' {
                    self.lines[i].credit_input.push(c);
                }
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

    // ── Accessors for inter-entity form integration ───────────────────────────

    /// Returns `true` when focus is on the Date or Memo field (the "header" fields).
    /// Used by `InterEntityForm` to detect when Tab has wrapped back to the top,
    /// so it can switch the active section to the next entity.
    pub fn is_at_header(&self) -> bool {
        matches!(self.focus, Focus::Date | Focus::Memo)
    }

    /// Returns `true` if focus is on the last line row (any column).
    pub fn is_at_last_line_row(&self) -> bool {
        let last = self.lines.len().saturating_sub(1);
        matches!(
            self.focus,
            Focus::LineAccount(i) | Focus::LineDebit(i) | Focus::LineCredit(i)
            if i == last
        )
    }

    /// Returns `true` if focus is on the first line row (any column).
    pub fn is_at_first_line_row(&self) -> bool {
        matches!(
            self.focus,
            Focus::LineAccount(0) | Focus::LineDebit(0) | Focus::LineCredit(0)
        )
    }

    /// Advances focus to the first line-item field, bypassing Date and Memo.
    /// Called by `InterEntityForm` when switching into this form from another section,
    /// so the user lands directly on the line rows rather than the header.
    pub fn skip_to_lines(&mut self) {
        self.focus = Focus::LineAccount(0);
    }

    /// Advances focus to the last line-item field (Credit of the last row).
    /// Called by `InterEntityForm` when BackTab-ing into this form from the next section.
    pub fn skip_to_last_line_field(&mut self) {
        if self.lines.is_empty() {
            self.focus = Focus::Memo;
        } else {
            self.focus = Focus::LineCredit(self.lines.len() - 1);
        }
    }

    /// Advances focus to the Account column of the last line row.
    /// Called by `InterEntityForm` when ↑ arrow navigates from the next entity.
    pub fn skip_to_last_line_account(&mut self) {
        if self.lines.is_empty() {
            self.focus = Focus::Memo;
        } else {
            self.focus = Focus::LineAccount(self.lines.len() - 1);
        }
    }

    /// Returns `true` if the form has any user-entered content (account selected,
    /// any text typed in date, memo, debit, credit, or note fields).
    /// Used by `InterEntityForm` to decide whether to show an "unsaved changes" prompt.
    pub fn has_content(&self) -> bool {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        if self.date_input != today || !self.memo_input.is_empty() {
            return true;
        }
        self.lines.iter().any(|l| {
            l.account_id.is_some()
                || !l.debit_input.is_empty()
                || !l.credit_input.is_empty()
                || !l.note_input.is_empty()
        })
    }

    /// Validates line items only (no date/memo check). Returns the parsed lines on success
    /// or an error message on failure.
    /// Called by `InterEntityForm` during combined submission — the shared date is owned
    /// by the inter-entity form header, not by each embedded `JeForm`.
    pub fn validate_lines(&self) -> Result<Vec<NewJournalEntryLine>, String> {
        if self.lines.len() < 2 {
            return Err("At least 2 line rows are required.".to_string());
        }
        let mut parsed: Vec<NewJournalEntryLine> = Vec::new();
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
            parsed.push(NewJournalEntryLine {
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
        Ok(parsed)
    }

    /// Renders only the line-item table and running totals, without the Date/Memo header
    /// or the help bar. Used by `InterEntityForm` to embed this form as a sub-section.
    ///
    /// Does NOT render the account picker overlay — call `render_picker_overlay` separately
    /// after all other UI elements so the dropdown appears on top of everything.
    pub fn render_lines_only(
        &self,
        frame: &mut Frame,
        area: Rect,
        envelope_avail: &HashMap<AccountId, Money>,
    ) {
        // Lines area takes most of the space; totals row is fixed at 2 lines.
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(2)])
            .split(area);

        self.render_lines(frame, layout[0], envelope_avail);
        self.render_totals(frame, layout[1]);
    }

    /// Renders the account picker popup as a floating overlay centered within `area`.
    /// Call this AFTER all other UI elements so the dropdown renders on top.
    pub fn render_picker_overlay(&self, frame: &mut Frame, area: Rect, accounts: &[Account]) {
        if self.picker_active {
            self.account_picker.render(frame, area, accounts);
        }
    }

    // ── Render ────────────────────────────────────────────────────────────────

    /// Renders the form into `area`. Renders the account picker popup on top when active.
    /// `accounts` is passed to the account picker.
    /// `envelope_avail` maps account IDs to their available envelope balance
    /// (Earmarked − GL Balance for the current fiscal year). Accounts not in the map
    /// have no envelope allocation and show "—" in the Avail column.
    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        accounts: &[Account],
        envelope_avail: &HashMap<AccountId, Money>,
    ) {
        let block = Block::default()
            .title(self.title.as_str())
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 30)));

        let inner = block.inner(area);
        frame.render_widget(block, area);
        // Fill inner area with the same dark background.
        frame.render_widget(
            Paragraph::new("").style(Style::default().bg(Color::Rgb(30, 30, 30))),
            inner,
        );

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
        self.render_lines(frame, lines_area, envelope_avail);
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

        // Horizontal scroll: cursor is always at the end of memo_input.
        let cursor_pos = self.memo_input.chars().count();
        let visible_width = cols[1].width.saturating_sub(2) as usize;
        let scroll = if visible_width > 0 && cursor_pos >= visible_width {
            cursor_pos - visible_width + 1
        } else {
            0
        };
        let memo_chars: Vec<char> = self.memo_input.chars().collect();
        let visible_memo: String = memo_chars.get(scroll..).unwrap_or(&[]).iter().collect();
        let memo_text = Paragraph::new(format!("{visible_memo}_")).block(memo_block);
        frame.render_widget(memo_text, cols[1]);
    }

    fn render_lines(
        &self,
        frame: &mut Frame,
        area: Rect,
        envelope_avail: &HashMap<AccountId, Money>,
    ) {
        let header_row = Row::new(vec![
            Cell::from("Account").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Avail").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Debit").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Credit").style(Style::default().add_modifier(Modifier::BOLD)),
        ]);

        let rows: Vec<Row> =
            self.lines
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

                    let avail_cell = match row.account_id {
                        Some(id) => match envelope_avail.get(&id) {
                            Some(avail) => Cell::from(format!("{avail}"))
                                .style(Style::default().fg(Color::Cyan)),
                            None => Cell::from("—").style(Style::default().fg(Color::DarkGray)),
                        },
                        None => Cell::from(""),
                    };

                    Row::new(vec![
                        Cell::from(acct_text).style(acct_style),
                        avail_cell,
                        Cell::from(format!("{}_", row.debit_input))
                            .style(field_style(self.focus == Focus::LineDebit(i))),
                        Cell::from(format!("{}_", row.credit_input))
                            .style(field_style(self.focus == Focus::LineCredit(i))),
                    ])
                })
                .collect();

        let widths = [
            Constraint::Percentage(44),
            Constraint::Length(12),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
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
            Focus::LineAccount(1),
            Focus::LineDebit(1),
            Focus::LineCredit(1),
            Focus::Date, // wraps back
        ];
        for expected_focus in expected {
            form.handle_key(key(KeyCode::Tab), &accts);
            assert_eq!(form.focus, expected_focus);
        }
    }

    #[test]
    fn enter_on_last_credit_of_last_row_adds_new_row() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        assert_eq!(form.lines.len(), 2);

        // Navigate to LineCredit of row 1 (last row, index 1).
        form.focus = Focus::LineCredit(1);
        form.handle_key(key(KeyCode::Enter), &accts);

        assert_eq!(form.lines.len(), 3, "Should have 3 rows after adding");
        assert_eq!(form.focus, Focus::LineAccount(2));
    }

    #[test]
    fn ctrl_down_inserts_row_after_focused_line() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        form.focus = Focus::LineDebit(0);
        form.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL), &accts);
        assert_eq!(form.lines.len(), 3);
        assert_eq!(form.focus, Focus::LineAccount(1));
    }

    #[test]
    fn ctrl_up_removes_focused_row_if_more_than_one() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        assert_eq!(form.lines.len(), 2);
        form.focus = Focus::LineCredit(1);
        form.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL), &accts);
        assert_eq!(form.lines.len(), 1, "Row should be removed");
    }

    #[test]
    fn ctrl_up_does_not_remove_last_row() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        // Remove until 1 row remains.
        form.lines.truncate(1);
        form.focus = Focus::LineDebit(0);
        form.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL), &accts);
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
    fn submit_fails_with_fewer_than_two_lines() {
        let mut form = JeForm::new();
        let accts = make_accounts();

        form.date_input = "2026-01-15".to_string();
        form.lines.truncate(1);
        form.lines[0].account_id = Some(AccountId::from(1));
        form.lines[0].account_name = "Cash".to_string();
        form.lines[0].debit_input = "100".to_string();

        let action = form.handle_key(ctrl(KeyCode::Char('s')), &accts);
        assert!(matches!(action, JeFormAction::Pending));
        assert!(
            form.error.as_deref().unwrap_or("").contains("2 line"),
            "Error should mention 2-line requirement; got: {:?}",
            form.error
        );
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

    // ── Arrow key navigation ──────────────────────────────────────────────────

    #[test]
    fn down_from_date_moves_to_first_line_account() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        assert_eq!(form.focus, Focus::Date);
        form.handle_key(key(KeyCode::Down), &accts);
        assert_eq!(form.focus, Focus::LineAccount(0));
    }

    #[test]
    fn down_from_memo_moves_to_first_line_account() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        form.focus = Focus::Memo;
        form.handle_key(key(KeyCode::Down), &accts);
        assert_eq!(form.focus, Focus::LineAccount(0));
    }

    #[test]
    fn up_from_first_line_moves_to_date() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        form.focus = Focus::LineAccount(0);
        form.handle_key(key(KeyCode::Up), &accts);
        assert_eq!(form.focus, Focus::Date);
    }

    #[test]
    fn down_and_up_navigate_rows_preserving_column() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        // new() starts with 2 rows already
        assert_eq!(form.lines.len(), 2);

        form.focus = Focus::LineDebit(0);
        form.handle_key(key(KeyCode::Down), &accts);
        assert_eq!(
            form.focus,
            Focus::LineDebit(1),
            "Down preserves column type"
        );

        form.handle_key(key(KeyCode::Up), &accts);
        assert_eq!(form.focus, Focus::LineDebit(0), "Up preserves column type");
    }

    #[test]
    fn right_moves_through_columns_in_row() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        form.focus = Focus::LineAccount(0);
        form.handle_key(key(KeyCode::Right), &accts);
        assert_eq!(form.focus, Focus::LineDebit(0));
        form.handle_key(key(KeyCode::Right), &accts);
        assert_eq!(form.focus, Focus::LineCredit(0));
        // Right at Credit does nothing
        form.handle_key(key(KeyCode::Right), &accts);
        assert_eq!(form.focus, Focus::LineCredit(0));
    }

    #[test]
    fn left_moves_through_columns_in_row() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        form.focus = Focus::LineCredit(0);
        form.handle_key(key(KeyCode::Left), &accts);
        assert_eq!(form.focus, Focus::LineDebit(0));
        form.handle_key(key(KeyCode::Left), &accts);
        assert_eq!(form.focus, Focus::LineAccount(0));
        // Left at Account does nothing
        form.handle_key(key(KeyCode::Left), &accts);
        assert_eq!(form.focus, Focus::LineAccount(0));
    }

    #[test]
    fn arrow_keys_ignored_when_picker_is_active() {
        let mut form = JeForm::new();
        let accts = make_accounts();
        form.focus = Focus::LineAccount(0);
        form.handle_key(key(KeyCode::Enter), &accts); // open picker
        assert!(form.picker_active);

        // Arrow keys should go to picker, not move JE form focus
        form.handle_key(key(KeyCode::Down), &accts);
        // Focus should still be LineAccount(0) — picker handled the key
        assert_eq!(form.focus, Focus::LineAccount(0));
        assert!(form.picker_active, "Picker still active");
    }
}
