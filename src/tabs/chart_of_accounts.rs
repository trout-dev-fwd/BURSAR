use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};

use crate::db::{EntityDb, account_repo::Account};
use crate::tabs::{RecordId, Tab, TabAction};
use crate::types::{AccountId, Money};

// ── Data structures ───────────────────────────────────────────────────────────

/// One displayable row in the account list (normal or search mode).
#[derive(Debug, Clone)]
struct VisibleRow {
    account: Account,
    depth: usize,
    has_children: bool,
}

// ── Tab struct ────────────────────────────────────────────────────────────────

pub struct ChartOfAccountsTab {
    all_accounts: Vec<Account>,
    balances: HashMap<AccountId, Money>,
    /// Set of collapsed group accounts (has_children but currently folded).
    collapsed: HashSet<AccountId>,
    /// Flattened view for normal mode.
    visible: Vec<VisibleRow>,
    table_state: TableState,
    /// Whether `/` search mode is active.
    search_active: bool,
    search_query: String,
    /// Flattened, filtered view for search mode.
    filtered: Vec<VisibleRow>,
    filtered_state: TableState,
}

impl Default for ChartOfAccountsTab {
    fn default() -> Self {
        Self::new()
    }
}

impl ChartOfAccountsTab {
    pub fn new() -> Self {
        Self {
            all_accounts: Vec::new(),
            balances: HashMap::new(),
            collapsed: HashSet::new(),
            visible: Vec::new(),
            table_state: {
                let mut s = TableState::default();
                s.select(Some(0));
                s
            },
            search_active: false,
            search_query: String::new(),
            filtered: Vec::new(),
            filtered_state: {
                let mut s = TableState::default();
                s.select(Some(0));
                s
            },
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Rebuilds `visible` from `all_accounts` + `collapsed` set.
    fn build_visible(&mut self) {
        // Which accounts are parents?
        let is_parent: HashSet<AccountId> = self
            .all_accounts
            .iter()
            .filter_map(|a| a.parent_id)
            .collect();

        // parent_id → sorted children
        let mut children_map: HashMap<Option<AccountId>, Vec<&Account>> = HashMap::new();
        for acc in &self.all_accounts {
            children_map.entry(acc.parent_id).or_default().push(acc);
        }
        for list in children_map.values_mut() {
            list.sort_by(|a, b| a.number.cmp(&b.number));
        }

        let mut rows = Vec::new();
        flatten_tree(
            None,
            0,
            &children_map,
            &self.collapsed,
            &is_parent,
            &mut rows,
        );
        self.visible = rows;

        // Clamp selection.
        let len = self.visible.len();
        match self.table_state.selected() {
            Some(i) if i >= len && len > 0 => self.table_state.select(Some(len - 1)),
            None if len > 0 => self.table_state.select(Some(0)),
            _ => {}
        }
    }

    /// Rebuilds `filtered` from `all_accounts` using `search_query`.
    fn update_filter(&mut self) {
        let q = self.search_query.to_lowercase();
        self.filtered = self
            .all_accounts
            .iter()
            .filter(|a| {
                q.is_empty()
                    || a.name.to_lowercase().contains(&q)
                    || a.number.to_lowercase().contains(&q)
            })
            .map(|a| VisibleRow {
                account: a.clone(),
                depth: 0, // flat in search mode
                has_children: false,
            })
            .collect();

        let len = self.filtered.len();
        match self.filtered_state.selected() {
            Some(i) if i >= len && len > 0 => self.filtered_state.select(Some(len - 1)),
            None if len > 0 => self.filtered_state.select(Some(0)),
            _ => {}
        }
    }

    fn current_rows(&self) -> &[VisibleRow] {
        if self.search_active {
            &self.filtered
        } else {
            &self.visible
        }
    }

    fn selected_idx(&self) -> Option<usize> {
        if self.search_active {
            self.filtered_state.selected()
        } else {
            self.table_state.selected()
        }
    }

    fn scroll_up(&mut self) {
        if self.search_active {
            let cur = self.filtered_state.selected().unwrap_or(0);
            if cur > 0 {
                self.filtered_state.select(Some(cur - 1));
            }
        } else {
            let cur = self.table_state.selected().unwrap_or(0);
            if cur > 0 {
                self.table_state.select(Some(cur - 1));
            }
        }
    }

    fn scroll_down(&mut self) {
        let len = self.current_rows().len();
        if len == 0 {
            return;
        }
        if self.search_active {
            let cur = self.filtered_state.selected().unwrap_or(0);
            if cur + 1 < len {
                self.filtered_state.select(Some(cur + 1));
            }
        } else {
            let cur = self.table_state.selected().unwrap_or(0);
            if cur + 1 < len {
                self.table_state.select(Some(cur + 1));
            }
        }
    }

    /// Toggles collapsed state for the selected account (if it has children).
    fn toggle_expand(&mut self) {
        if self.search_active {
            return; // no expand/collapse in search mode
        }
        let idx = match self.table_state.selected() {
            Some(i) => i,
            None => return,
        };
        let row = match self.visible.get(idx) {
            Some(r) => r.clone(),
            None => return,
        };
        if !row.has_children {
            return;
        }
        if self.collapsed.contains(&row.account.id) {
            self.collapsed.remove(&row.account.id);
        } else {
            self.collapsed.insert(row.account.id);
        }
        self.build_visible();
    }

    // ── Render helpers ────────────────────────────────────────────────────────

    fn make_table<'a>(
        rows: &'a [VisibleRow],
        balances: &'a HashMap<AccountId, Money>,
        collapsed: &'a HashSet<AccountId>,
    ) -> Table<'a> {
        let header = Row::new(vec![
            Cell::from("Number").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Name").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Type").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Balance").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Flags").style(Style::default().add_modifier(Modifier::BOLD)),
        ])
        .style(Style::default().bg(Color::DarkGray));

        let table_rows: Vec<Row> = rows
            .iter()
            .map(|vr| {
                let acc = &vr.account;

                // Expand/collapse indicator + indentation
                let indent = "  ".repeat(vr.depth);
                let indicator = if vr.has_children {
                    if collapsed.contains(&acc.id) {
                        "▶ "
                    } else {
                        "▼ "
                    }
                } else {
                    "  "
                };
                let name_cell = format!("{}{}{}", indent, indicator, acc.name);

                // Type abbreviation
                let type_str = match acc.account_type {
                    crate::types::AccountType::Asset => "Asset",
                    crate::types::AccountType::Liability => "Liab",
                    crate::types::AccountType::Equity => "Equity",
                    crate::types::AccountType::Revenue => "Rev",
                    crate::types::AccountType::Expense => "Exp",
                };

                // Balance
                let balance = balances.get(&acc.id).copied().unwrap_or(Money(0));

                // Flags: P = placeholder, C = contra, X = inactive
                let mut flags = String::new();
                if acc.is_placeholder {
                    flags.push('P');
                }
                if acc.is_contra {
                    flags.push('C');
                }
                if !acc.is_active {
                    flags.push('x');
                }

                let row_style = if !acc.is_active {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                };

                Row::new(vec![
                    Cell::from(acc.number.clone()),
                    Cell::from(name_cell),
                    Cell::from(type_str),
                    Cell::from(balance.to_string()),
                    Cell::from(flags),
                ])
                .style(row_style)
            })
            .collect();

        Table::new(
            table_rows,
            [
                Constraint::Length(8),
                Constraint::Min(30),
                Constraint::Length(7),
                Constraint::Length(12),
                Constraint::Length(5),
            ],
        )
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Chart of Accounts "),
        )
        .row_highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("» ")
    }
}

// ── Tab trait ─────────────────────────────────────────────────────────────────

impl Tab for ChartOfAccountsTab {
    fn title(&self) -> &str {
        "Chart of Accounts"
    }

    fn handle_key(&mut self, key: KeyEvent, _db: &EntityDb) -> TabAction {
        if key.modifiers != KeyModifiers::NONE && key.modifiers != KeyModifiers::SHIFT {
            return TabAction::None;
        }

        if self.search_active {
            match key.code {
                KeyCode::Esc => {
                    self.search_active = false;
                    self.search_query.clear();
                    self.update_filter();
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.update_filter();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.update_filter();
                }
                KeyCode::Up => self.scroll_up(),
                KeyCode::Down => self.scroll_down(),
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.scroll_up(),
                KeyCode::Down | KeyCode::Char('j') => self.scroll_down(),
                KeyCode::Enter => self.toggle_expand(),
                KeyCode::Char('/') => {
                    self.search_active = true;
                    self.search_query.clear();
                    self.update_filter();
                }
                _ => {}
            }
        }
        TabAction::None
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        // Split area: table on top, hint bar at bottom (+ search bar if active).
        let hint_height = if self.search_active { 2 } else { 1 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(hint_height)])
            .split(area);

        // ── Table ──────────────────────────────────────────────────────────────
        let rows = self.current_rows();
        let table = Self::make_table(rows, &self.balances, &self.collapsed);

        // Ratatui requires a mutable state for stateful render; we clone for immutable render.
        let mut state = if self.search_active {
            self.filtered_state.clone()
        } else {
            self.table_state.clone()
        };
        frame.render_stateful_widget(table, chunks[0], &mut state);

        // ── Bottom bar ─────────────────────────────────────────────────────────
        if self.search_active {
            let bottom = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Length(1)])
                .split(chunks[1]);

            let search_line = Line::from(vec![
                Span::styled(" Search: ", Style::default().fg(Color::Yellow)),
                Span::raw(self.search_query.clone()),
                Span::styled("█", Style::default().fg(Color::Yellow)), // cursor
            ]);
            frame.render_widget(Paragraph::new(search_line), bottom[0]);

            let hint = Paragraph::new(Line::from(vec![Span::styled(
                " Esc: cancel search  ↑↓: navigate",
                Style::default().fg(Color::DarkGray),
            )]));
            frame.render_widget(hint, bottom[1]);
        } else {
            let count = self.visible.len();
            let selected = self.selected_idx().map(|i| i + 1).unwrap_or(0);
            let hint = Paragraph::new(Line::from(vec![
                Span::styled(
                    " ↑↓/jk: navigate  Enter: expand/collapse  /: search",
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("  [{}/{}]", selected, count),
                    Style::default().fg(Color::Gray),
                ),
            ]));
            frame.render_widget(hint, chunks[1]);
        }
    }

    fn refresh(&mut self, db: &EntityDb) {
        let repo = db.accounts();
        match repo.list_all() {
            Err(e) => {
                tracing::error!("CoA tab: failed to load accounts: {e}");
                return;
            }
            Ok(accounts) => {
                self.all_accounts = accounts;
            }
        }

        // Load balances for all accounts.
        self.balances.clear();
        for acc in &self.all_accounts {
            match repo.get_balance(acc.id) {
                Ok(bal) => {
                    self.balances.insert(acc.id, bal);
                }
                Err(e) => {
                    tracing::error!(
                        "CoA tab: failed to load balance for {}: {e}",
                        i64::from(acc.id)
                    );
                }
            }
        }

        self.build_visible();
        if self.search_active {
            self.update_filter();
        }
    }

    fn navigate_to(&mut self, record_id: RecordId, _db: &EntityDb) {
        if let RecordId::Account(aid) = record_id
            && let Some(idx) = self.visible.iter().position(|r| r.account.id == aid)
        {
            self.table_state.select(Some(idx));
        }
    }
}

// ── Free functions ────────────────────────────────────────────────────────────

/// Recursively flattens the account tree into a display list.
/// Only recurses into children when the parent is not collapsed.
fn flatten_tree(
    parent: Option<AccountId>,
    depth: usize,
    children_map: &HashMap<Option<AccountId>, Vec<&Account>>,
    collapsed: &HashSet<AccountId>,
    is_parent: &HashSet<AccountId>,
    result: &mut Vec<VisibleRow>,
) {
    let Some(children) = children_map.get(&parent) else {
        return;
    };
    for acc in children {
        let has_children = is_parent.contains(&acc.id);
        result.push(VisibleRow {
            account: (*acc).clone(),
            depth,
            has_children,
        });
        if has_children && !collapsed.contains(&acc.id) {
            flatten_tree(
                Some(acc.id),
                depth + 1,
                children_map,
                collapsed,
                is_parent,
                result,
            );
        }
    }
}
