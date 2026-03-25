use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
};

use crate::db::EntityDb;
use crate::db::account_repo::Account;
use crate::db::fiscal_repo::FiscalYear;
use crate::db::journal_repo::JournalEntryLine;
use crate::db::tax_tag_repo::PostedJeWithTag;
use crate::tabs::{Tab, TabAction};
use crate::types::{AccountId, JournalEntryId, TaxFormTag, TaxReviewStatus};
use crate::widgets::text_input_modal::{TextInputAction, TextInputModal};

// ── Form config modal state ───────────────────────────────────────────────────

/// State for the `c` key form configuration modal.
struct FormConfigModal {
    /// Toggle state for each form in `TaxFormTag::all()` order.
    enabled: Vec<bool>,
    /// Currently highlighted row.
    cursor: usize,
}

impl FormConfigModal {
    fn new(enabled_forms: &[TaxFormTag]) -> Self {
        let all = TaxFormTag::all();
        let enabled: Vec<bool> = all.iter().map(|f| enabled_forms.contains(f)).collect();
        Self { enabled, cursor: 0 }
    }

    /// Returns the list of currently-enabled form tags.
    fn as_enabled_list(&self) -> Vec<TaxFormTag> {
        TaxFormTag::all()
            .into_iter()
            .enumerate()
            .filter_map(|(i, f)| if self.enabled[i] { Some(f) } else { None })
            .collect()
    }

    fn handle_key(&mut self, key: KeyEvent) -> FormConfigAction {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                FormConfigAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < TaxFormTag::all().len() {
                    self.cursor += 1;
                }
                FormConfigAction::None
            }
            KeyCode::Char(' ') => {
                self.enabled[self.cursor] = !self.enabled[self.cursor];
                FormConfigAction::None
            }
            KeyCode::Enter => FormConfigAction::Save(self.as_enabled_list()),
            KeyCode::Esc => FormConfigAction::Cancel,
            _ => FormConfigAction::None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let all = TaxFormTag::all();
        let row_count = all.len();
        let popup_height = (row_count + 4).min(area.height as usize) as u16;
        let popup_width = 60u16.min(area.width);

        let x = area.x + area.width.saturating_sub(popup_width) / 2;
        let y = area.y + area.height.saturating_sub(popup_height) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        let block = Block::default()
            .title(" Configure Tax Forms (Space: toggle, Enter: save, Esc: cancel) ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan).bg(Color::Black));

        let inner = block.inner(popup_area);

        let lines: Vec<Line> = all
            .iter()
            .enumerate()
            .map(|(i, form)| {
                let check = if self.enabled[i] { "[✓]" } else { "[ ]" };
                let is_selected = i == self.cursor;
                let style = if is_selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                Line::from(vec![
                    Span::styled(format!(" {check} "), style),
                    Span::styled(form.display_name(), style),
                ])
            })
            .collect();

        frame.render_widget(Clear, popup_area);
        frame.render_widget(block, popup_area);
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(Color::Black)),
            inner,
        );
    }
}

enum FormConfigAction {
    None,
    Save(Vec<TaxFormTag>),
    Cancel,
}

// ── Tax modal state machine ───────────────────────────────────────────────────

/// Overlay modals for the Tax tab (layered on top of the list / detail view).
enum TaxModal {
    /// Form picker: user is choosing which form to apply (`f` key).
    FormPicker {
        cursor: usize,
        je_id: JournalEntryId,
    },
    /// Reason input after choosing a form.
    FlagReason {
        input: TextInputModal,
        form: TaxFormTag,
        je_id: JournalEntryId,
    },
    /// Reason input for marking as non-deductible (`n` key).
    NonDeductibleReason {
        input: TextInputModal,
        je_id: JournalEntryId,
    },
    /// Single-line text input for editing the JE memo (`m` key).
    MemoEdit {
        input: TextInputModal,
        je_id: JournalEntryId,
    },
}

/// State for the inline JE detail panel (shown below the list via `Enter`).
struct TaxDetailState {
    je_number: String,
    memo: Option<String>,
    lines: Vec<JournalEntryLine>,
    focused_line: usize,
}

// ── TaxTab ────────────────────────────────────────────────────────────────────

/// Tax workstation tab.
pub struct TaxTab {
    /// Currently enabled tax forms. Initialized to all-enabled.
    enabled_forms: Vec<TaxFormTag>,
    /// Active form configuration modal (Some when `c` is pressed).
    form_config_modal: Option<FormConfigModal>,
    /// All fiscal years, ordered by start_date ASC.
    fiscal_years: Vec<FiscalYear>,
    /// Index into `fiscal_years` for the currently displayed year.
    selected_fy_index: usize,
    /// Posted JEs for the selected fiscal year with their tax tags.
    rows: Vec<PostedJeWithTag>,
    /// Table selection state.
    table_state: TableState,
    /// Full account list for name resolution in the detail view.
    accounts: Vec<Account>,
    /// Active overlay modal (form picker, reason input, memo edit).
    modal: Option<TaxModal>,
    /// Inline detail panel state (shown alongside the list).
    detail: Option<TaxDetailState>,
}

impl TaxTab {
    pub fn new() -> Self {
        Self {
            enabled_forms: TaxFormTag::all(),
            form_config_modal: None,
            fiscal_years: Vec::new(),
            selected_fy_index: 0,
            rows: Vec::new(),
            table_state: TableState::default(),
            accounts: Vec::new(),
            modal: None,
            detail: None,
        }
    }

    /// Updates enabled forms from a saved list of tag strings.
    /// Call this after loading the entity TOML to restore persisted config.
    pub fn set_enabled_forms_from_strings(&mut self, form_strings: &[String]) {
        self.enabled_forms = TaxFormTag::all()
            .into_iter()
            .filter(|f| form_strings.contains(&f.to_string()))
            .collect();
        // If nothing matched, default to all enabled.
        if self.enabled_forms.is_empty() {
            self.enabled_forms = TaxFormTag::all();
        }
    }

    /// Returns the currently enabled forms.
    pub fn enabled_forms(&self) -> &[TaxFormTag] {
        &self.enabled_forms
    }

    fn reload_fiscal_years(&mut self, db: &EntityDb) {
        match db.fiscal().list_fiscal_years() {
            Ok(years) => {
                let today = chrono::Local::now().date_naive();
                let current_idx = years
                    .iter()
                    .position(|fy| today >= fy.start_date && today <= fy.end_date)
                    .unwrap_or(years.len().saturating_sub(1));
                self.fiscal_years = years;
                self.selected_fy_index = current_idx;
            }
            Err(e) => {
                tracing::error!("Failed to load fiscal years: {e}");
                self.fiscal_years.clear();
                self.selected_fy_index = 0;
            }
        }
    }

    fn reload_rows(&mut self, db: &EntityDb) {
        let range = self
            .fiscal_years
            .get(self.selected_fy_index)
            .map(|fy| (fy.start_date, fy.end_date));

        if let Some((start, end)) = range {
            match db.tax_tags().list_all_posted_for_date_range(start, end) {
                Ok(rows) => self.rows = rows,
                Err(e) => {
                    tracing::error!("Failed to load tax rows: {e}");
                    self.rows.clear();
                }
            }
        } else {
            self.rows.clear();
        }
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        let len = self.rows.len();
        if len == 0 {
            self.table_state.select(None);
        } else if self.table_state.selected().is_none_or(|i| i >= len) {
            self.table_state.select(Some(0));
        }
    }

    fn selected_row(&self) -> Option<&PostedJeWithTag> {
        self.table_state.selected().and_then(|i| self.rows.get(i))
    }

    fn fiscal_year_label(&self) -> String {
        match self.fiscal_years.get(self.selected_fy_index) {
            Some(fy) => fy.start_date.format("FY %Y").to_string(),
            None => "No fiscal year".to_string(),
        }
    }

    fn scroll_up(&mut self) {
        if let Some(i) = self.table_state.selected()
            && i > 0
        {
            self.table_state.select(Some(i - 1));
        }
    }

    fn scroll_down(&mut self) {
        let len = self.rows.len();
        if len == 0 {
            return;
        }
        match self.table_state.selected() {
            None => self.table_state.select(Some(0)),
            Some(i) if i + 1 < len => self.table_state.select(Some(i + 1)),
            _ => {}
        }
    }

    fn account_display(&self, id: AccountId) -> String {
        self.accounts
            .iter()
            .find(|a| a.id == id)
            .map(|a| format!("{} {}", a.number, a.name))
            .unwrap_or_else(|| format!("Account #{}", i64::from(id)))
    }

    fn open_detail(&mut self, db: &EntityDb) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let je_id = row.je_id;
        let je_number = row.je_number.clone();
        let memo = row.memo.clone();
        match db.journals().get_with_lines(je_id) {
            Ok((_, lines)) => {
                self.detail = Some(TaxDetailState {
                    je_number,
                    memo,
                    lines,
                    focused_line: 0,
                });
            }
            Err(e) => {
                tracing::error!("Failed to load JE lines for {}: {e}", i64::from(je_id));
            }
        }
    }

    fn close_detail(&mut self) {
        self.detail = None;
    }

    // ── Modal key handlers ────────────────────────────────────────────────────

    /// Routes keys when a TaxModal is active. Returns a TabAction and whether the
    /// modal should be cleared (a `None` modal means clear it).
    fn handle_modal_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        let Some(modal) = self.modal.take() else {
            return TabAction::None;
        };

        match modal {
            TaxModal::FormPicker { mut cursor, je_id } => {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        cursor = cursor.saturating_sub(1);
                        self.modal = Some(TaxModal::FormPicker { cursor, je_id });
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let max = self.enabled_forms.len().saturating_sub(1);
                        cursor = (cursor + 1).min(max);
                        self.modal = Some(TaxModal::FormPicker { cursor, je_id });
                    }
                    KeyCode::Enter => {
                        if let Some(form) = self.enabled_forms.get(cursor).copied() {
                            self.modal = Some(TaxModal::FlagReason {
                                input: TextInputModal::new("Reason (optional)", ""),
                                form,
                                je_id,
                            });
                        }
                        // else: no forms enabled — dismiss picker
                    }
                    KeyCode::Esc => {
                        // modal already taken (None)
                    }
                    _ => {
                        self.modal = Some(TaxModal::FormPicker { cursor, je_id });
                    }
                }
                TabAction::None
            }

            TaxModal::FlagReason {
                mut input,
                form,
                je_id,
            } => match input.handle_key(key) {
                TextInputAction::Confirm(reason) => {
                    let reason_opt = if reason.is_empty() {
                        None
                    } else {
                        Some(reason.as_str())
                    };
                    match db.tax_tags().set_manual(je_id, form, reason_opt) {
                        Ok(()) => {
                            self.reload_rows(db);
                            TabAction::ShowMessage(format!("Flagged as {}.", form.display_name()))
                        }
                        Err(e) => TabAction::ShowMessage(format!("Error: {e}")),
                    }
                }
                TextInputAction::Cancel => TabAction::None,
                TextInputAction::None => {
                    self.modal = Some(TaxModal::FlagReason { input, form, je_id });
                    TabAction::None
                }
            },

            TaxModal::NonDeductibleReason { mut input, je_id } => match input.handle_key(key) {
                TextInputAction::Confirm(reason) => {
                    let reason_opt = if reason.is_empty() {
                        None
                    } else {
                        Some(reason.as_str())
                    };
                    match db.tax_tags().set_non_deductible(je_id, reason_opt) {
                        Ok(()) => {
                            self.reload_rows(db);
                            TabAction::ShowMessage("Marked as non-deductible.".to_string())
                        }
                        Err(e) => TabAction::ShowMessage(format!("Error: {e}")),
                    }
                }
                TextInputAction::Cancel => TabAction::None,
                TextInputAction::None => {
                    self.modal = Some(TaxModal::NonDeductibleReason { input, je_id });
                    TabAction::None
                }
            },

            TaxModal::MemoEdit { mut input, je_id } => match input.handle_key(key) {
                TextInputAction::Confirm(memo) => {
                    let memo_opt = if memo.is_empty() {
                        None
                    } else {
                        Some(memo.as_str())
                    };
                    match db.journals().update_memo(je_id, memo_opt) {
                        Ok(()) => {
                            self.reload_rows(db);
                            // Re-sync the detail panel if open.
                            if self.detail.is_some() {
                                self.open_detail(db);
                            }
                            TabAction::ShowMessage("Memo updated.".to_string())
                        }
                        Err(e) => TabAction::ShowMessage(format!("Error: {e}")),
                    }
                }
                TextInputAction::Cancel => TabAction::None,
                TextInputAction::None => {
                    self.modal = Some(TaxModal::MemoEdit { input, je_id });
                    TabAction::None
                }
            },
        }
    }

    // ── Render helpers ────────────────────────────────────────────────────────

    fn render_list(&self, frame: &mut Frame, area: Rect) {
        let total = self.rows.len();
        let reviewed = self
            .rows
            .iter()
            .filter(|r| {
                r.tag
                    .as_ref()
                    .is_some_and(|t| t.status != TaxReviewStatus::Unreviewed)
            })
            .count();
        let pct = if total > 0 { reviewed * 100 / total } else { 0 };
        let fy_label = self.fiscal_year_label();
        let title = format!(
            " Tax Workstation — {fy_label}  |  Tax Review: {reviewed}/{total} ({pct}%)  [←/→ year] "
        );

        let block = Block::default().borders(Borders::ALL).title(title);

        if self.rows.is_empty() {
            frame.render_widget(
                Paragraph::new("  No posted journal entries for this fiscal year.")
                    .style(Style::default().fg(Color::DarkGray))
                    .block(block),
                area,
            );
            return;
        }

        let header = Row::new(vec![
            Cell::from("Date").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("JE #").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Memo").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Amount").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Form").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Status").style(Style::default().add_modifier(Modifier::BOLD)),
        ]);

        let rows: Vec<Row> = self
            .rows
            .iter()
            .map(|row| {
                let (status_label, form_label, row_style) = status_display(row);
                let date_str = row.entry_date.format("%b %d").to_string();
                let memo_display = truncate_to_chars(row.memo.as_deref().unwrap_or(""), 30);
                Row::new(vec![
                    Cell::from(date_str),
                    Cell::from(row.je_number.clone()),
                    Cell::from(memo_display),
                    Cell::from(row.total_debits.to_string()),
                    Cell::from(form_label),
                    Cell::from(status_label),
                ])
                .style(row_style)
            })
            .collect();

        let widths = [
            Constraint::Length(7),  // Date  "Jan 15"
            Constraint::Length(9),  // JE #  "JE-0004"
            Constraint::Min(20),    // Memo
            Constraint::Length(12), // Amount
            Constraint::Length(18), // Form
            Constraint::Length(16), // Status
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(block)
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        let mut state = self.table_state.clone();
        frame.render_stateful_widget(table, area, &mut state);
    }

    fn render_detail(&self, frame: &mut Frame, area: Rect) {
        let Some(d) = &self.detail else {
            return;
        };

        let title = format!(
            " {} — {} line(s)  ↑↓: scroll  Esc: close ",
            d.je_number,
            d.lines.len()
        );

        let header = Row::new(vec![
            Cell::from("#").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Account").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Debit").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Credit").style(Style::default().add_modifier(Modifier::BOLD)),
        ]);

        let rows: Vec<Row> = d
            .lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let acct = self.account_display(line.account_id);
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
                ])
                .style(row_style)
            })
            .collect();

        let widths = [
            Constraint::Length(3),
            Constraint::Percentage(55),
            Constraint::Length(14),
            Constraint::Length(14),
        ];

        let block = Block::default().title(title).borders(Borders::ALL);

        if let Some(memo) = &d.memo {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(3)])
                .split(area);

            frame.render_widget(
                Paragraph::new(format!("  Memo: {memo}"))
                    .style(Style::default().fg(Color::DarkGray))
                    .wrap(Wrap { trim: false }),
                chunks[0],
            );
            frame.render_widget(
                Table::new(rows, widths).header(header).block(block),
                chunks[1],
            );
        } else {
            frame.render_widget(Table::new(rows, widths).header(header).block(block), area);
        }
    }

    fn render_form_picker(&self, frame: &mut Frame, area: Rect, cursor: usize) {
        let row_count = self.enabled_forms.len();
        let popup_height = (row_count + 4).min(area.height as usize) as u16;
        let popup_width = 60u16.min(area.width);

        let x = area.x + area.width.saturating_sub(popup_width) / 2;
        let y = area.y + area.height.saturating_sub(popup_height) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        let block = Block::default()
            .title(" Select Tax Form (↑↓: move, Enter: confirm, Esc: cancel) ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan).bg(Color::Black));

        let inner = block.inner(popup_area);

        let lines: Vec<Line> = self
            .enabled_forms
            .iter()
            .enumerate()
            .map(|(i, form)| {
                let is_selected = i == cursor;
                let style = if is_selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                Line::from(Span::styled(format!("  {}", form.display_name()), style))
            })
            .collect();

        frame.render_widget(Clear, popup_area);
        frame.render_widget(block, popup_area);
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(Color::Black)),
            inner,
        );
    }
}

impl Default for TaxTab {
    fn default() -> Self {
        Self::new()
    }
}

impl Tab for TaxTab {
    fn title(&self) -> &str {
        "Tax"
    }

    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        // Form config modal gets priority over everything.
        if let Some(ref mut modal) = self.form_config_modal {
            match modal.handle_key(key) {
                FormConfigAction::None => {}
                FormConfigAction::Save(forms) => {
                    self.enabled_forms = forms.clone();
                    self.form_config_modal = None;
                    let tags: Vec<String> = forms.iter().map(|f| f.to_string()).collect();
                    return TabAction::SaveTaxFormConfig(tags);
                }
                FormConfigAction::Cancel => {
                    self.form_config_modal = None;
                }
            }
            return TabAction::None;
        }

        // Route to tax modal if active.
        if self.modal.is_some() {
            return self.handle_modal_key(key, db);
        }

        // Detail panel navigation.
        if self.detail.is_some() {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(ref mut d) = self.detail
                        && d.focused_line > 0
                    {
                        d.focused_line -= 1;
                    }
                    return TabAction::None;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(ref mut d) = self.detail {
                        let max = d.lines.len().saturating_sub(1);
                        if d.focused_line < max {
                            d.focused_line += 1;
                        }
                    }
                    return TabAction::None;
                }
                KeyCode::Esc | KeyCode::Enter => {
                    self.close_detail();
                    return TabAction::None;
                }
                // Fall through to base hotkeys (f, n, a, m still work with detail open).
                _ => {}
            }
        }

        // Base key handling.
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_up();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_down();
            }
            KeyCode::Left => {
                if !self.fiscal_years.is_empty() {
                    self.selected_fy_index = self.selected_fy_index.saturating_sub(1);
                    self.detail = None;
                    self.reload_rows(db);
                }
            }
            KeyCode::Right => {
                if !self.fiscal_years.is_empty() {
                    let max = self.fiscal_years.len().saturating_sub(1);
                    self.selected_fy_index = (self.selected_fy_index + 1).min(max);
                    self.detail = None;
                    self.reload_rows(db);
                }
            }
            KeyCode::Enter => {
                if self.detail.is_some() {
                    self.close_detail();
                } else {
                    self.open_detail(db);
                }
            }
            KeyCode::Esc => {
                self.close_detail();
            }
            KeyCode::Char('f') => {
                if let Some(row) = self.selected_row() {
                    let je_id = row.je_id;
                    if !self.enabled_forms.is_empty() {
                        self.modal = Some(TaxModal::FormPicker { cursor: 0, je_id });
                    }
                }
            }
            KeyCode::Char('n') => {
                if let Some(row) = self.selected_row() {
                    let je_id = row.je_id;
                    self.modal = Some(TaxModal::NonDeductibleReason {
                        input: TextInputModal::new("Reason (optional)", ""),
                        je_id,
                    });
                }
            }
            KeyCode::Char('a') => {
                if let Some(row) = self.selected_row() {
                    let je_id = row.je_id;
                    match db.tax_tags().set_ai_pending(je_id) {
                        Ok(()) => {
                            self.reload_rows(db);
                            return TabAction::ShowMessage("Queued for AI review.".to_string());
                        }
                        Err(e) => return TabAction::ShowMessage(format!("Error: {e}")),
                    }
                }
            }
            KeyCode::Char('m') => {
                if let Some(row) = self.selected_row() {
                    let je_id = row.je_id;
                    let prefill = row.memo.clone().unwrap_or_default();
                    self.modal = Some(TaxModal::MemoEdit {
                        input: TextInputModal::new("Edit Memo", prefill),
                        je_id,
                    });
                }
            }
            KeyCode::Char('c') => {
                self.form_config_modal = Some(FormConfigModal::new(&self.enabled_forms));
            }
            KeyCode::Char('u') => {
                return TabAction::StartTaxIngestion;
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

        // Form config modal overlays everything.
        if let Some(ref modal) = self.form_config_modal {
            modal.render(frame, area);
        }

        // Tax modals overlay.
        if let Some(ref modal) = self.modal {
            match modal {
                TaxModal::FormPicker { cursor, .. } => {
                    self.render_form_picker(frame, area, *cursor);
                }
                TaxModal::FlagReason { input, .. }
                | TaxModal::NonDeductibleReason { input, .. }
                | TaxModal::MemoEdit { input, .. } => {
                    input.render(frame, area);
                }
            }
        }
    }

    fn refresh(&mut self, db: &EntityDb) {
        self.reload_fiscal_years(db);
        self.reload_rows(db);
        match db.accounts().list_all() {
            Ok(accts) => self.accounts = accts,
            Err(e) => tracing::error!("Failed to load accounts: {e}"),
        }
    }

    fn wants_input(&self) -> bool {
        self.form_config_modal.is_some() || self.modal.is_some()
    }

    fn hotkey_help(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("↑/↓ or k/j", "Navigate entries"),
            ("←/→", "Cycle fiscal year"),
            ("Enter", "View JE detail lines"),
            ("f", "Flag with tax form + reason"),
            ("n", "Mark as non-deductible"),
            ("a", "Queue for AI review"),
            ("m", "Edit memo"),
            ("c", "Configure enabled tax forms"),
            ("u", "Update tax reference library"),
        ]
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns `(status_label, form_label, row_style)` for a tax list row.
fn status_display(row: &PostedJeWithTag) -> (&'static str, String, Style) {
    match &row.tag {
        None => (
            "Unreviewed",
            String::new(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
        Some(tag) => match tag.status {
            TaxReviewStatus::Unreviewed => (
                "Unreviewed",
                String::new(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ),
            TaxReviewStatus::AiPending => (
                "AI Pending",
                String::new(),
                Style::default().fg(Color::Yellow),
            ),
            TaxReviewStatus::AiSuggested => {
                let form = tag
                    .ai_suggested_form
                    .as_ref()
                    .map(|f| format!("{}?", f.display_name()))
                    .unwrap_or_default();
                ("AI Suggested", form, Style::default().fg(Color::Cyan))
            }
            TaxReviewStatus::Confirmed => {
                let form = tag
                    .form_tag
                    .as_ref()
                    .map(|f| f.display_name().to_string())
                    .unwrap_or_default();
                ("Confirmed", form, Style::default().fg(Color::Green))
            }
            TaxReviewStatus::NonDeductible => (
                "Non-Deductible",
                String::new(),
                Style::default().fg(Color::Gray),
            ),
        },
    }
}

/// Truncates a string to at most `max_chars` Unicode scalar values.
/// Appends `…` if truncation occurred.
fn truncate_to_chars(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = chars[..max_chars.saturating_sub(1)].iter().collect();
        format!("{truncated}…")
    }
}
