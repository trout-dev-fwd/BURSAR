use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};

use crate::db::{
    EntityDb,
    account_repo::Account,
    journal_repo::{JournalEntry, JournalEntryLine, JournalFilter},
};
use crate::tabs::{RecordId, Tab, TabAction};
use crate::types::{AccountId, JournalEntryId, JournalEntryStatus, ReconcileState};

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
}

// ── Tab ───────────────────────────────────────────────────────────────────────

pub struct JournalEntriesTab {
    entries: Vec<JournalEntry>,
    table_state: TableState,
    status_filter: StatusFilter,
    detail: Option<DetailState>,
    /// Full account list (including inactive) for name resolution in detail view.
    accounts: Vec<Account>,
}

impl Default for JournalEntriesTab {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            table_state: TableState::default(),
            status_filter: StatusFilter::All,
            detail: None,
            accounts: Vec::new(),
        }
    }
}

impl JournalEntriesTab {
    pub fn new() -> Self {
        Self::default()
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
            Ok((_, lines)) => self.detail = Some(DetailState { lines }),
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

    // ── Render helpers ────────────────────────────────────────────────────────

    fn render_list(&self, frame: &mut Frame, area: Rect) {
        let title = format!(
            " Journal Entries  [f] filter: {}  ↑↓: scroll  Enter: detail ",
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
            " {} — {} line(s)  Esc: close ",
            entry.je_number,
            d.lines.len()
        );

        if let Some(memo) = &entry.memo {
            // Reserve top line for memo when present
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

                Row::new(vec![
                    Cell::from(format!("{}", i + 1)),
                    Cell::from(acct),
                    Cell::from(debit),
                    Cell::from(credit),
                    Cell::from(note),
                    Cell::from(rec),
                ])
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
}

impl Tab for JournalEntriesTab {
    fn title(&self) -> &str {
        "Journal Entries"
    }

    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        match key.code {
            KeyCode::Up => self.scroll_up(),
            KeyCode::Down => self.scroll_down(),
            KeyCode::Enter => {
                if self.detail.is_some() {
                    self.close_detail();
                } else {
                    self.open_detail(db);
                }
            }
            KeyCode::Esc => self.close_detail(),
            KeyCode::Char('f') | KeyCode::Char('F') => {
                self.status_filter = self.status_filter.next();
                self.close_detail();
                self.refresh(db);
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
                Ok((_, lines)) => self.detail = Some(DetailState { lines }),
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
    use chrono::NaiveDate;
    use rusqlite::Connection;

    fn make_db() -> EntityDb {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();
        seed_default_accounts(&conn).unwrap();
        FiscalRepo::new(&conn).create_fiscal_year(1, 2026).unwrap();
        entity_db_from_conn(conn)
    }

    fn non_placeholder_accounts(db: &EntityDb) -> Vec<crate::types::AccountId> {
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
        // Post the entry so we have both Draft (second) and Posted (first).
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
}
