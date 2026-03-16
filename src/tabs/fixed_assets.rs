use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};

use crate::db::EntityDb;
use crate::db::asset_repo::FixedAssetWithDetails;
use crate::db::journal_repo::LedgerRow;
use crate::tabs::{RecordId, Tab, TabAction};
use crate::types::{FiscalPeriodId, Money};

// ── Sub-view ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Register,
    Schedule,
}

// ── Tab struct ────────────────────────────────────────────────────────────────

pub struct FixedAssetsTab {
    view: View,
    /// All fixed assets, loaded on refresh.
    assets: Vec<FixedAssetWithDetails>,
    /// Depreciation ledger rows for the currently selected asset (Schedule view).
    /// Filtered to credit-only rows (i.e. monthly depreciation entries).
    schedule: Vec<LedgerRow>,
    table_state: TableState,
    schedule_state: TableState,
    /// Status message shown at the bottom (e.g. after generating depreciation).
    status: Option<String>,
}

impl Default for FixedAssetsTab {
    fn default() -> Self {
        Self::new()
    }
}

impl FixedAssetsTab {
    pub fn new() -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));
        Self {
            view: View::Register,
            assets: Vec::new(),
            schedule: Vec::new(),
            table_state,
            schedule_state: TableState::default(),
            status: None,
        }
    }

    fn selected_asset(&self) -> Option<&FixedAssetWithDetails> {
        self.table_state.selected().and_then(|i| self.assets.get(i))
    }

    fn scroll_down(&mut self) {
        match self.view {
            View::Register => {
                let len = self.assets.len();
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
            View::Schedule => {
                let len = self.schedule.len();
                if len == 0 {
                    return;
                }
                let next = self
                    .schedule_state
                    .selected()
                    .map(|i| (i + 1).min(len - 1))
                    .unwrap_or(0);
                self.schedule_state.select(Some(next));
            }
        }
    }

    fn scroll_up(&mut self) {
        match self.view {
            View::Register => {
                let next = self
                    .table_state
                    .selected()
                    .map(|i| i.saturating_sub(1))
                    .unwrap_or(0);
                self.table_state.select(Some(next));
            }
            View::Schedule => {
                let next = self
                    .schedule_state
                    .selected()
                    .map(|i| i.saturating_sub(1))
                    .unwrap_or(0);
                self.schedule_state.select(Some(next));
            }
        }
    }

    /// Loads the depreciation schedule (posted JE credits to the AccumDepreciation account)
    /// for the currently selected asset.
    fn load_schedule(&mut self, db: &EntityDb) {
        self.schedule.clear();
        let asset = match self.selected_asset() {
            Some(a) => a,
            None => return,
        };
        let accum_id = match asset.detail.accum_depreciation_account_id {
            Some(id) => id,
            None => return,
        };

        match db.journals().list_lines_for_account(accum_id, None) {
            Ok(rows) => {
                // Keep only credit rows (depreciation entries credit the accum account).
                self.schedule = rows.into_iter().filter(|r| r.credit.0 > 0).collect();
            }
            Err(e) => tracing::error!("Failed to load depreciation schedule: {e}"),
        }

        let mut ss = TableState::default();
        if !self.schedule.is_empty() {
            ss.select(Some(0));
        }
        self.schedule_state = ss;
    }

    /// Generates pending depreciation drafts through the current fiscal period.
    fn generate_depreciation(&mut self, db: &EntityDb) {
        let today = chrono::Local::now().naive_local().date();
        let period = match db.fiscal().get_period_for_date(today) {
            Ok(p) => p,
            Err(e) => {
                self.status = Some(format!("No fiscal period for today: {e}"));
                return;
            }
        };
        self.generate_for_period(period.id, db);
    }

    fn generate_for_period(&mut self, period_id: FiscalPeriodId, db: &EntityDb) {
        let entries = match db.assets().generate_pending_depreciation(period_id) {
            Ok(e) => e,
            Err(e) => {
                self.status = Some(format!("Failed to generate depreciation: {e}"));
                return;
            }
        };

        if entries.is_empty() {
            self.status = Some("No pending depreciation to generate.".to_string());
            return;
        }

        let total = entries.len();
        let je_repo = db.journals();
        let mut created = 0usize;
        for entry in entries {
            match je_repo.create_draft(&entry) {
                Ok(_) => created += 1,
                Err(e) => tracing::error!("Failed to create depreciation draft: {e}"),
            }
        }

        self.status = Some(format!(
            "Generated {created}/{total} depreciation draft JEs — review and post from JE tab."
        ));
    }

    // ── Rendering ─────────────────────────────────────────────────────────────

    fn render_register(&self, frame: &mut Frame, area: Rect) {
        let rows: Vec<Row> = self
            .assets
            .iter()
            .map(|fa| {
                let in_service = fa
                    .detail
                    .in_service_date
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "—".to_string());

                let (life_str, monthly_str) = if fa.detail.is_depreciable {
                    let life = fa
                        .detail
                        .useful_life_months
                        .map(|m| format!("{m} mo"))
                        .unwrap_or_else(|| "—".to_string());
                    let monthly = fa
                        .detail
                        .useful_life_months
                        .map(|m| Money(fa.detail.cost_basis.0 / i64::from(m)).to_string());
                    (life, monthly.unwrap_or_else(|| "—".to_string()))
                } else {
                    ("Non-Dep".to_string(), "—".to_string())
                };

                let row_style = if !fa.detail.is_depreciable {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                };

                Row::new(vec![
                    Cell::from(fa.account_number.as_str()),
                    Cell::from(fa.account_name.as_str()),
                    Cell::from(fa.detail.cost_basis.to_string()),
                    Cell::from(in_service),
                    Cell::from(life_str),
                    Cell::from(monthly_str),
                    Cell::from(fa.accumulated_depreciation.to_string()),
                    Cell::from(fa.book_value.to_string()),
                ])
                .style(row_style)
            })
            .collect();

        let widths = [
            Constraint::Length(8),
            Constraint::Min(22),
            Constraint::Length(14),
            Constraint::Length(12),
            Constraint::Length(9),
            Constraint::Length(12),
            Constraint::Length(14),
            Constraint::Length(14),
        ];

        let table = Table::new(rows, widths)
            .header(
                Row::new(vec![
                    "#",
                    "Name",
                    "Cost Basis",
                    "In Service",
                    "Life",
                    "Mo. Dep.",
                    "Accum. Dep.",
                    "Book Value",
                ])
                .style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Fixed Asset Register"),
            )
            .row_highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            );

        let mut ts = self.table_state.clone();
        frame.render_stateful_widget(table, area, &mut ts);
    }

    fn render_schedule(&self, frame: &mut Frame, area: Rect) {
        let title = self
            .selected_asset()
            .map(|a| format!("Depreciation Schedule: {}", a.account_name))
            .unwrap_or_else(|| "Depreciation Schedule".to_string());

        let rows: Vec<Row> = self
            .schedule
            .iter()
            .map(|row| {
                Row::new(vec![
                    Cell::from(row.entry_date.to_string()),
                    Cell::from(row.memo.as_deref().unwrap_or("").to_string()),
                    Cell::from(row.credit.to_string()),
                    Cell::from(row.running_balance.to_string()),
                ])
            })
            .collect();

        let widths = [
            Constraint::Length(12),
            Constraint::Min(40),
            Constraint::Length(14),
            Constraint::Length(14),
        ];

        let table = Table::new(rows, widths)
            .header(
                Row::new(vec!["Date", "Memo", "Amount", "Accum. Balance"])
                    .style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .block(Block::default().borders(Borders::ALL).title(title))
            .row_highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            );

        let mut ss = self.schedule_state.clone();
        frame.render_stateful_widget(table, area, &mut ss);
    }

    fn hint_text(&self) -> &'static str {
        match self.view {
            View::Register => "↑↓ Navigate  Enter Schedule  g Generate Depreciation",
            View::Schedule => "↑↓ Navigate  Esc Back",
        }
    }
}

impl Tab for FixedAssetsTab {
    fn title(&self) -> &str {
        "Fixed Assets"
    }

    fn hotkey_help(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("↑/↓", "Navigate assets"),
            ("Enter", "View depreciation schedule"),
            ("Esc", "Back to asset list"),
            ("g", "Generate pending depreciation"),
        ]
    }

    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        self.status = None;

        match self.view {
            View::Register => match key.code {
                KeyCode::Down => self.scroll_down(),
                KeyCode::Up => self.scroll_up(),
                KeyCode::Enter => {
                    if self.selected_asset().is_some() {
                        self.load_schedule(db);
                        self.view = View::Schedule;
                    }
                }
                KeyCode::Char('g') | KeyCode::Char('G') => {
                    self.generate_depreciation(db);
                    if let Ok(assets) = db.assets().list_assets() {
                        self.assets = assets;
                    }
                }
                _ => {}
            },
            View::Schedule => match key.code {
                KeyCode::Down => self.scroll_down(),
                KeyCode::Up => self.scroll_up(),
                KeyCode::Esc => {
                    self.view = View::Register;
                }
                _ => {}
            },
        }

        TabAction::None
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let status_height: u16 = if self.status.is_some() { 1 } else { 0 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(1),
                Constraint::Length(status_height),
            ])
            .split(area);

        match self.view {
            View::Register => self.render_register(frame, chunks[0]),
            View::Schedule => self.render_schedule(frame, chunks[0]),
        }

        frame.render_widget(
            Paragraph::new(self.hint_text()).style(Style::default().fg(Color::DarkGray)),
            chunks[1],
        );

        if let Some(ref msg) = self.status {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(msg.as_str(), Style::default().fg(Color::Cyan)),
                ])),
                chunks[2],
            );
        }
    }

    fn refresh(&mut self, db: &EntityDb) {
        match db.assets().list_assets() {
            Ok(assets) => self.assets = assets,
            Err(e) => {
                tracing::error!("Fixed Assets tab: failed to load assets: {e}");
                self.assets.clear();
            }
        }

        let len = self.assets.len();
        match self.table_state.selected() {
            Some(i) if i >= len && len > 0 => self.table_state.select(Some(len - 1)),
            None if len > 0 => self.table_state.select(Some(0)),
            _ => {}
        }
    }

    fn navigate_to(&mut self, _record_id: RecordId, _db: &EntityDb) {}
}
