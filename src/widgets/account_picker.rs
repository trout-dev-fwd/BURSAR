//! A reusable popup widget for selecting an account.
//!
//! Filters account list in real-time as the user types. Excludes placeholder
//! and inactive accounts. Returns `Option<AccountId>` when done.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::db::account_repo::Account;
use crate::types::AccountId;

/// Result of a single key event handled by the picker.
#[derive(Debug, Clone, PartialEq)]
pub enum PickerAction {
    /// User confirmed a selection.
    Selected(AccountId),
    /// User pressed Esc — no selection.
    Cancelled,
    /// Key consumed; waiting for more input.
    Pending,
}

/// A popup account-selection widget.
///
/// The caller maintains the full account list and passes it to `handle_key` and `render`.
/// The picker filters to active, non-placeholder accounts matching the current query.
pub struct AccountPicker {
    query: String,
    /// Indexes into the caller's account slice that match the current query.
    matches: Vec<usize>,
    list_state: ListState,
}

impl Default for AccountPicker {
    fn default() -> Self {
        Self::new()
    }
}

impl AccountPicker {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            matches: Vec::new(),
            list_state: ListState::default(),
        }
    }

    /// Resets the picker for a fresh invocation (clears query and results).
    pub fn reset(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.list_state.select(None);
    }

    /// Returns the current query string.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Updates the match list based on `accounts` and the current `query`.
    /// Excludes inactive and placeholder accounts automatically.
    pub fn refresh(&mut self, accounts: &[Account]) {
        let q = self.query.to_lowercase();
        self.matches = accounts
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                a.is_active
                    && !a.is_placeholder
                    && (q.is_empty()
                        || a.name.to_lowercase().contains(&q)
                        || a.number.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect();

        // Clamp selection.
        let len = self.matches.len();
        match self.list_state.selected() {
            Some(i) if i >= len && len > 0 => self.list_state.select(Some(len - 1)),
            None if len > 0 => self.list_state.select(Some(0)),
            _ if len == 0 => self.list_state.select(None),
            _ => {}
        }
    }

    /// Returns the currently selected `AccountId`, if any.
    pub fn selected_id(&self, accounts: &[Account]) -> Option<AccountId> {
        let idx = self.list_state.selected()?;
        let account_idx = *self.matches.get(idx)?;
        accounts.get(account_idx).map(|a| a.id)
    }

    /// Handles a key event. The caller must pass the same `accounts` slice that was
    /// last passed to `refresh()`.
    pub fn handle_key(&mut self, key: KeyEvent, accounts: &[Account]) -> PickerAction {
        match key.code {
            KeyCode::Esc => return PickerAction::Cancelled,

            KeyCode::Enter => {
                return match self.selected_id(accounts) {
                    Some(id) => PickerAction::Selected(id),
                    None => PickerAction::Cancelled,
                };
            }

            KeyCode::Up => {
                let cur = self.list_state.selected().unwrap_or(0);
                if cur > 0 {
                    self.list_state.select(Some(cur - 1));
                }
            }

            KeyCode::Down => {
                let len = self.matches.len();
                if len > 0 {
                    let cur = self.list_state.selected().unwrap_or(0);
                    if cur + 1 < len {
                        self.list_state.select(Some(cur + 1));
                    }
                }
            }

            KeyCode::Backspace => {
                self.query.pop();
                self.refresh(accounts);
            }

            KeyCode::Char(c) => {
                self.query.push(c);
                self.refresh(accounts);
            }

            _ => {}
        }
        PickerAction::Pending
    }

    /// Renders the picker popup centered within `area`.
    /// Pass the same `accounts` slice used for `refresh()` and `handle_key()`.
    pub fn render(&mut self, frame: &mut Frame, area: Rect, accounts: &[Account]) {
        let popup = centered_picker_rect(area);
        frame.render_widget(Clear, popup);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // query input box
                Constraint::Min(0),    // results list
            ])
            .split(popup);

        // ── Query input ────────────────────────────────────────────────────────
        let input_line = Line::from(vec![
            Span::raw(self.query.clone()),
            Span::styled("█", Style::default().fg(Color::Yellow)),
        ]);
        frame.render_widget(
            Paragraph::new(input_line)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Account Search (↑↓ navigate, Enter select, Esc cancel) "),
                )
                .style(Style::default().fg(Color::Yellow)),
            chunks[0],
        );

        // ── Results list ───────────────────────────────────────────────────────
        let items: Vec<ListItem> = self
            .matches
            .iter()
            .filter_map(|&i| accounts.get(i))
            .map(|a| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:>8}  ", a.number),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(a.name.clone()),
                ]))
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} match(es) ", self.matches.len())),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("» ");

        frame.render_stateful_widget(list, chunks[1], &mut self.list_state);
    }
}

/// Returns a Rect suitable for the picker popup (80% wide, 60% tall, centered).
fn centered_picker_rect(area: Rect) -> Rect {
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(10),
            Constraint::Percentage(80),
            Constraint::Percentage(10),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(60),
            Constraint::Percentage(20),
        ])
        .split(horizontal[1])[1]
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::account_repo::Account;
    use crate::types::{AccountId, AccountType};

    fn make_account(
        id: i64,
        number: &str,
        name: &str,
        is_active: bool,
        is_placeholder: bool,
    ) -> Account {
        Account {
            id: AccountId::from(id),
            number: number.to_string(),
            name: name.to_string(),
            account_type: AccountType::Asset,
            parent_id: None,
            is_active,
            is_contra: false,
            is_placeholder,
            created_at: "2025-01-01T00:00:00".to_string(),
            updated_at: "2025-01-01T00:00:00".to_string(),
        }
    }

    fn accounts() -> Vec<Account> {
        vec![
            make_account(1, "1000", "Assets", true, true), // placeholder → excluded
            make_account(2, "1100", "Cash & Bank Accounts", true, true), // placeholder → excluded
            make_account(3, "1110", "Checking Account", true, false),
            make_account(4, "1120", "Savings Account", true, false),
            make_account(5, "1200", "Accounts Receivable", true, false),
            make_account(6, "5000", "Expenses", true, true), // placeholder → excluded
            make_account(7, "5100", "Rent Expense", true, false),
            make_account(8, "9999", "Inactive Account", false, false), // inactive → excluded
        ]
    }

    #[test]
    fn refresh_excludes_placeholder_and_inactive() {
        let accs = accounts();
        let mut picker = AccountPicker::new();
        picker.refresh(&accs);

        // 8 total - 3 placeholder - 1 inactive = 4 matches
        assert_eq!(picker.matches.len(), 4);

        // Verify none of the matches are placeholders or inactive
        for &idx in &picker.matches {
            let acc = &accs[idx];
            assert!(!acc.is_placeholder, "placeholder should be excluded");
            assert!(acc.is_active, "inactive should be excluded");
        }
    }

    #[test]
    fn refresh_filters_by_name_substring() {
        let accs = accounts();
        let mut picker = AccountPicker::new();
        picker.query = "cash".to_string();
        picker.refresh(&accs);

        // Should match "Checking Account" (no "cash") and... actually "Cash & Bank Accounts" is
        // placeholder so excluded. "Checking Account" has "check" not "cash". Let me check
        // "Savings" has no "cash" either. So only "Checking Account" if it contains "cash"?
        // Actually none contain "cash" substring in non-placeholder accounts.
        // But number search: none contain "cash" in number either.
        // So 0 results for "cash" among non-placeholder accounts.
        // This is a valid test: search returns 0 when no match.
        assert_eq!(
            picker.matches.len(),
            0,
            "No non-placeholder account names contain 'cash'"
        );
    }

    #[test]
    fn refresh_filters_by_name_checking() {
        let accs = accounts();
        let mut picker = AccountPicker::new();
        picker.query = "checking".to_string();
        picker.refresh(&accs);

        assert_eq!(picker.matches.len(), 1);
        assert_eq!(accs[picker.matches[0]].name, "Checking Account");
    }

    #[test]
    fn refresh_filters_by_number_prefix() {
        let accs = accounts();
        let mut picker = AccountPicker::new();
        picker.query = "11".to_string();
        picker.refresh(&accs);

        // 1110 (Checking) and 1120 (Savings) — non-placeholder, active
        assert_eq!(picker.matches.len(), 2);
    }

    #[test]
    fn empty_query_shows_all_eligible_accounts() {
        let accs = accounts();
        let mut picker = AccountPicker::new();
        picker.refresh(&accs);

        // 4 eligible: Checking, Savings, Accounts Receivable, Rent Expense
        assert_eq!(picker.matches.len(), 4);
    }

    #[test]
    fn handle_key_char_updates_query_and_filter() {
        let accs = accounts();
        let mut picker = AccountPicker::new();
        picker.refresh(&accs);

        let key = KeyEvent::new(KeyCode::Char('r'), crossterm::event::KeyModifiers::NONE);
        let action = picker.handle_key(key, &accs);
        assert_eq!(action, PickerAction::Pending);
        assert_eq!(picker.query(), "r");
        // "Accounts Receivable" and "Rent Expense" contain 'r'
        assert!(!picker.matches.is_empty());
    }

    #[test]
    fn handle_key_backspace_removes_char() {
        let accs = accounts();
        let mut picker = AccountPicker::new();
        picker.query = "checking".to_string();
        picker.refresh(&accs);
        assert_eq!(picker.matches.len(), 1);

        let key = KeyEvent::new(KeyCode::Backspace, crossterm::event::KeyModifiers::NONE);
        picker.handle_key(key, &accs);
        // "checkin" — still matches Checking Account
        assert_eq!(picker.query(), "checkin");
    }

    #[test]
    fn handle_key_enter_returns_selected_id() {
        let accs = accounts();
        let mut picker = AccountPicker::new();
        picker.query = "checking".to_string();
        picker.refresh(&accs);
        assert_eq!(picker.matches.len(), 1);

        let key = KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
        let action = picker.handle_key(key, &accs);
        assert_eq!(action, PickerAction::Selected(AccountId::from(3)));
    }

    #[test]
    fn handle_key_esc_returns_cancelled() {
        let accs = accounts();
        let mut picker = AccountPicker::new();
        picker.refresh(&accs);

        let key = KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE);
        let action = picker.handle_key(key, &accs);
        assert_eq!(action, PickerAction::Cancelled);
    }

    #[test]
    fn handle_key_enter_with_no_match_returns_cancelled() {
        let accs = accounts();
        let mut picker = AccountPicker::new();
        picker.query = "zzznomatch".to_string();
        picker.refresh(&accs);
        assert!(picker.matches.is_empty());

        let key = KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
        let action = picker.handle_key(key, &accs);
        assert_eq!(action, PickerAction::Cancelled);
    }

    #[test]
    fn navigation_wraps_within_bounds() {
        let accs = accounts();
        let mut picker = AccountPicker::new();
        picker.refresh(&accs); // 4 matches

        // Start at 0, scroll down 3 times to get to last item
        for _ in 0..3 {
            let key = KeyEvent::new(KeyCode::Down, crossterm::event::KeyModifiers::NONE);
            picker.handle_key(key, &accs);
        }
        assert_eq!(picker.list_state.selected(), Some(3));

        // Scrolling down again should stay at last item
        let key = KeyEvent::new(KeyCode::Down, crossterm::event::KeyModifiers::NONE);
        picker.handle_key(key, &accs);
        assert_eq!(picker.list_state.selected(), Some(3));

        // Scroll back up to first
        for _ in 0..10 {
            let key = KeyEvent::new(KeyCode::Up, crossterm::event::KeyModifiers::NONE);
            picker.handle_key(key, &accs);
        }
        assert_eq!(picker.list_state.selected(), Some(0));
    }

    #[test]
    fn reset_clears_state() {
        let accs = accounts();
        let mut picker = AccountPicker::new();
        picker.query = "checking".to_string();
        picker.refresh(&accs);
        assert!(!picker.matches.is_empty());

        picker.reset();
        assert_eq!(picker.query(), "");
        assert!(picker.matches.is_empty());
        assert_eq!(picker.list_state.selected(), None);
    }
}
