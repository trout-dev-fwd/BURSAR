use std::{io, path::Path};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Tabs},
};

use crate::{
    config::{EntityConfig, WorkspaceConfig, save_config},
    db::EntityDb,
    inter_entity::{InterEntityMode, form::InterEntityFormAction, write_protocol},
    tabs::{
        Tab, TabAction, TabId, accounts_payable::AccountsPayableTab,
        accounts_receivable::AccountsReceivableTab, audit_log::AuditLogTab,
        chart_of_accounts::ChartOfAccountsTab, envelopes::EnvelopesTab,
        fixed_assets::FixedAssetsTab, general_ledger::GeneralLedgerTab,
        journal_entries::JournalEntriesTab, reports::ReportsTab,
    },
    widgets::{FiscalModal, FiscalModalAction, StatusBar, UserGuide, UserGuideAction},
};

/// Operating mode of the application.
pub enum AppMode {
    Normal,
    /// User is picking the secondary entity for an inter-entity transaction.
    SecondaryEntityPicker {
        /// Index into `config.entities`, skipping the active entity.
        selected: usize,
        /// Indices of selectable entities (all entities except the active one).
        candidates: Vec<usize>,
    },
    /// Prompting user to create intercompany accounts before opening the form.
    InterEntityAccountSetup {
        mode: Box<InterEntityMode>,
        confirm: crate::widgets::confirmation::Confirmation,
    },
    /// Inter-entity form is open.
    InterEntity(Box<InterEntityMode>),
}

/// Active entity context: database handle, entity name, and the 9 tab instances.
pub struct EntityContext {
    pub db: EntityDb,
    pub name: String,
    pub tabs: Vec<Box<dyn Tab>>,
}

impl EntityContext {
    /// Creates an entity context from an open EntityDb, building all 9 tabs and
    /// performing an initial data load so tabs render content immediately.
    pub fn new(db: EntityDb, name: String, report_output_dir: std::path::PathBuf) -> Self {
        let mut coa = ChartOfAccountsTab::new();
        coa.set_entity_name(&name);
        let mut je = JournalEntriesTab::new();
        je.set_entity_name(&name);
        let mut ar = AccountsReceivableTab::new();
        ar.set_entity_name(&name);
        let mut ap = AccountsPayableTab::new();
        ap.set_entity_name(&name);
        let mut env = EnvelopesTab::new();
        env.set_entity_name(&name);
        let mut reports = ReportsTab::new(report_output_dir);
        reports.set_entity_name(&name);
        let mut tabs: Vec<Box<dyn Tab>> = vec![
            Box::new(coa),
            Box::new(GeneralLedgerTab::new()),
            Box::new(je),
            Box::new(ar),
            Box::new(ap),
            Box::new(env),
            Box::new(FixedAssetsTab::new()),
            Box::new(reports),
            Box::new(AuditLogTab::new()),
        ];
        // Initial data load so tabs show content on first render.
        for tab in &mut tabs {
            tab.refresh(&db);
        }
        Self { db, name, tabs }
    }
}

/// Top-level application struct. Owns the event loop and all state.
pub struct App {
    entity: EntityContext,
    #[allow(dead_code)]
    config: WorkspaceConfig,
    active_tab: usize,
    mode: AppMode,
    status_bar: StatusBar,
    fiscal_modal: Option<FiscalModal>,
    show_help: bool,
    user_guide: Option<UserGuide>,
    should_quit: bool,
}

impl App {
    pub fn new(entity: EntityContext, config: WorkspaceConfig) -> Self {
        let status_bar = StatusBar::new(entity.name.clone(), String::new());
        Self {
            entity,
            config,
            active_tab: 0,
            mode: AppMode::Normal,
            status_bar,
            fiscal_modal: None,
            show_help: false,
            user_guide: None,
            should_quit: false,
        }
    }

    /// Runs the synchronous event loop. Initializes the terminal, runs until quit,
    /// then restores the terminal — including on panic via a drop guard.
    pub fn run(&mut self) -> Result<()> {
        // Set up terminal.
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Panic guard: restores terminal even if a panic occurs.
        let _guard = TerminalGuard;

        let result = self.event_loop(&mut terminal);

        // Explicit cleanup (guard also runs on drop, but this handles the normal path).
        restore_terminal();

        result
    }

    fn event_loop<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        loop {
            // 1. Render.
            terminal.draw(|frame| {
                let tab_bar_height = self.tab_bar_height(frame.area().width);
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(tab_bar_height), // tab bar
                        Constraint::Min(0),                 // content
                        Constraint::Length(1),              // status bar
                    ])
                    .split(frame.area());

                self.render_tab_bar(frame, chunks[0]);

                match &self.mode {
                    AppMode::Normal => {
                        self.entity.tabs[self.active_tab].render(frame, chunks[1]);
                    }
                    AppMode::SecondaryEntityPicker {
                        selected,
                        candidates,
                    } => {
                        render_secondary_entity_picker(
                            frame,
                            chunks[1],
                            &self.config,
                            *selected,
                            candidates,
                        );
                    }
                    AppMode::InterEntityAccountSetup { mode, confirm } => {
                        // Render the form underneath, confirmation overlay on top.
                        mode.form.render(
                            frame,
                            chunks[1],
                            &mode.primary_name,
                            &mode.secondary_name,
                            &mode.primary_accounts,
                            &mode.secondary_accounts,
                            &std::collections::HashMap::new(),
                            &std::collections::HashMap::new(),
                        );
                        // Center a small confirmation popup.
                        let area = chunks[1];
                        let popup_w = 60u16.min(area.width);
                        let popup_h = 6u16.min(area.height);
                        let px = area.x + area.width.saturating_sub(popup_w) / 2;
                        let py = area.y + area.height.saturating_sub(popup_h) / 2;
                        let popup_area = ratatui::layout::Rect::new(px, py, popup_w, popup_h);
                        frame.render_widget(ratatui::widgets::Clear, popup_area);
                        confirm.render(frame, popup_area);
                    }
                    AppMode::InterEntity(mode) => {
                        mode.form.render(
                            frame,
                            chunks[1],
                            &mode.primary_name,
                            &mode.secondary_name,
                            &mode.primary_accounts,
                            &mode.secondary_accounts,
                            &std::collections::HashMap::new(),
                            &std::collections::HashMap::new(),
                        );
                    }
                }

                // Fiscal period modal overlay (rendered on top of tab content).
                if let Some(ref mut modal) = self.fiscal_modal {
                    modal.render(frame, chunks[1]);
                }

                // Help overlay (rendered topmost).
                if self.show_help {
                    render_help_overlay(
                        frame,
                        chunks[1],
                        self.entity.tabs[self.active_tab].hotkey_help(),
                    );
                }

                // User guide overlay (rendered above everything else).
                if let Some(guide) = &self.user_guide {
                    guide.render(frame, chunks[1]);
                }

                self.status_bar.render(frame, chunks[2]);
            })?;

            // 2. Poll for input (500ms timeout).
            if event::poll(std::time::Duration::from_millis(500))?
                && let Event::Key(key) = event::read()?
            {
                self.handle_key(key);
            }

            // 3. Tick: update status bar timeout + unsaved indicator.
            self.status_bar.tick();
            let unsaved = self.entity.tabs[self.active_tab].has_unsaved_changes();
            self.status_bar.set_unsaved(unsaved);

            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    /// Returns the short label for a tab, abbreviating if `abbreviate` is true.
    fn tab_label(title: &str, abbreviate: bool) -> &str {
        if !abbreviate {
            return title;
        }
        match title {
            "Chart of Accounts" => "CoA",
            "General Ledger" => "GL",
            "Journal Entries" => "Journal",
            "Accounts Receivable" => "AR",
            "Accounts Payable" => "AP",
            "Fixed Assets" => "Assets",
            other => other,
        }
    }

    /// Compute how many rows the tab bar needs (2 border rows + content rows).
    fn tab_bar_height(&self, width: u16) -> u16 {
        let inner_width = width.saturating_sub(2) as usize; // borders
        let labels: Vec<&str> = self.entity.tabs.iter().map(|t| t.title()).collect();

        // Try full names first, then abbreviated.
        for abbreviate in [false, true] {
            let total: usize = labels
                .iter()
                .map(|t| Self::tab_label(t, abbreviate).len() + 3) // " label " + separator
                .sum();
            if total <= inner_width {
                return 3; // single row + 2 borders
            }
        }
        // Need two rows.
        4
    }

    fn render_tab_bar(&self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        let inner_width = area.width.saturating_sub(2) as usize;
        let titles: Vec<&str> = self.entity.tabs.iter().map(|t| t.title()).collect();

        // Decide whether to abbreviate: try full names, fall back to short.
        let abbreviate = {
            let full_total: usize = titles.iter().map(|t| t.len() + 3).sum();
            full_total > inner_width
        };

        let labels: Vec<&str> = titles
            .iter()
            .map(|t| Self::tab_label(t, abbreviate))
            .collect();

        let total_width: usize = labels.iter().map(|l| l.len() + 3).sum();
        let needs_wrap = total_width > inner_width;

        if needs_wrap {
            // Split tabs across two rows, roughly equal.
            let mut split_at = labels.len() / 2;
            // Adjust so first row fits within inner_width.
            let mut row1_width: usize = labels[..split_at].iter().map(|l| l.len() + 3).sum();
            while row1_width > inner_width && split_at > 1 {
                split_at -= 1;
                row1_width = labels[..split_at].iter().map(|l| l.len() + 3).sum();
            }

            let make_spans = |range: std::ops::Range<usize>| -> Vec<Span> {
                let mut spans = Vec::new();
                for i in range {
                    let style = if i == self.active_tab {
                        Style::default().fg(Color::Yellow).bg(Color::DarkGray)
                    } else {
                        Style::default().fg(Color::Gray)
                    };
                    spans.push(Span::styled(format!(" {} ", labels[i]), style));
                    spans.push(Span::raw("│"));
                }
                spans
            };

            let line1 = Line::from(make_spans(0..split_at));
            let line2 = Line::from(make_spans(split_at..labels.len()));

            let block = Block::default().borders(Borders::ALL).title("Tabs");
            let inner = block.inner(area);
            frame.render_widget(block, area);
            if inner.height >= 2 {
                frame.render_widget(Paragraph::new(vec![line1, line2]), inner);
            }
        } else {
            // Single-row: use the Tabs widget.
            let tab_titles: Vec<Line> = labels
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    Line::from(vec![Span::styled(
                        format!(" {label} "),
                        if i == self.active_tab {
                            Style::default().fg(Color::Yellow).bg(Color::DarkGray)
                        } else {
                            Style::default().fg(Color::Gray)
                        },
                    )])
                })
                .collect();

            let tabs_widget = Tabs::new(tab_titles)
                .block(Block::default().borders(Borders::ALL).title("Tabs"))
                .select(self.active_tab)
                .highlight_style(Style::default().fg(Color::Yellow).bg(Color::DarkGray));

            frame.render_widget(tabs_widget, area);
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Ctrl+H toggles the user guide from any context.
        if key.code == KeyCode::Char('h') && key.modifiers == KeyModifiers::CONTROL {
            if self.user_guide.is_some() {
                self.user_guide = None;
            } else {
                self.user_guide = Some(UserGuide::new());
            }
            return;
        }

        // User guide overlay: routes all keys; Esc/Close dismisses it.
        if let Some(guide) = &mut self.user_guide {
            match guide.handle_key(key) {
                UserGuideAction::Close => self.user_guide = None,
                UserGuideAction::Pending => {}
            }
            return;
        }

        // Help overlay: Esc or ? dismisses it; all other keys are consumed.
        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => self.show_help = false,
                _ => {}
            }
            return;
        }

        // Inter-entity mode: all input goes to the form.
        if matches!(self.mode, AppMode::InterEntity(_)) {
            self.handle_inter_entity_key(key);
            return;
        }

        // Intercompany account setup prompt.
        if matches!(self.mode, AppMode::InterEntityAccountSetup { .. }) {
            self.handle_account_setup_key(key);
            return;
        }

        // Secondary entity picker: all input goes to picker.
        if matches!(self.mode, AppMode::SecondaryEntityPicker { .. }) {
            self.handle_secondary_picker_key(key);
            return;
        }

        // If the fiscal modal is open, all input goes to it.
        if self.fiscal_modal.is_some() {
            let action = self
                .fiscal_modal
                .as_mut()
                .expect("checked above")
                .handle_key(key, &self.entity.db);
            self.process_fiscal_modal_action(action);
            return;
        }

        // When the active tab has a form, modal, or search field open,
        // delegate all input directly — suppress global hotkeys.
        if self.entity.tabs[self.active_tab].wants_input() {
            let action = self.entity.tabs[self.active_tab].handle_key(key, &self.entity.db);
            self.process_action(action);
            return;
        }

        // Global hotkeys.
        match key.code {
            KeyCode::Char('q') if key.modifiers == KeyModifiers::NONE => {
                self.should_quit = true;
            }
            // Show help overlay.
            KeyCode::Char('?') => {
                self.show_help = true;
            }
            // Open fiscal period management modal.
            KeyCode::Char('f') if key.modifiers == KeyModifiers::NONE => {
                self.fiscal_modal =
                    Some(FiscalModal::new(self.entity.name.clone(), &self.entity.db));
            }
            // Tab switching: 1–9 keys select tabs by number.
            KeyCode::Char(c @ '1'..='9') if key.modifiers == KeyModifiers::NONE => {
                let idx = (c as usize) - ('1' as usize);
                if idx < self.entity.tabs.len() {
                    self.active_tab = idx;
                }
            }
            // Tab cycling: Ctrl+Right / Ctrl+Left wraps through tabs.
            KeyCode::Right if key.modifiers == KeyModifiers::CONTROL => {
                self.active_tab = (self.active_tab + 1) % self.entity.tabs.len();
            }
            KeyCode::Left if key.modifiers == KeyModifiers::CONTROL => {
                self.active_tab =
                    (self.active_tab + self.entity.tabs.len() - 1) % self.entity.tabs.len();
            }
            _ => {
                // Delegate to active tab.
                let action = self.entity.tabs[self.active_tab].handle_key(key, &self.entity.db);
                self.process_action(action);
            }
        }
    }

    fn handle_inter_entity_key(&mut self, key: KeyEvent) {
        let AppMode::InterEntity(ref mut mode) = self.mode else {
            return;
        };
        let action = mode
            .form
            .handle_key(key, &mode.primary_accounts, &mode.secondary_accounts);

        match action {
            InterEntityFormAction::Pending => {}
            InterEntityFormAction::Cancelled => {
                self.mode = AppMode::Normal;
            }
            InterEntityFormAction::Submitted(output) => {
                let AppMode::InterEntity(ref mode) = self.mode else {
                    return;
                };
                let input = write_protocol::InterEntityInput {
                    entry_date: output.entry_date,
                    memo: output.memo,
                    primary_lines: output.primary_lines,
                    secondary_lines: output.secondary_lines,
                };
                let result = write_protocol::execute(
                    &self.entity.db,
                    &mode.secondary_db,
                    &mode.primary_name,
                    &mode.secondary_name,
                    &input,
                );
                match result {
                    Ok(_) => {
                        self.mode = AppMode::Normal;
                        for tab in &mut self.entity.tabs {
                            tab.refresh(&self.entity.db);
                        }
                        self.status_bar
                            .set_message("Inter-entity transaction posted.".to_owned());
                    }
                    Err(e) => {
                        self.status_bar.set_error(format!("Error: {e}"));
                    }
                }
            }
        }
    }

    fn handle_account_setup_key(&mut self, key: KeyEvent) {
        use crate::widgets::confirmation::ConfirmAction;
        let AppMode::InterEntityAccountSetup {
            ref mut mode,
            ref mut confirm,
        } = self.mode
        else {
            return;
        };
        let action = confirm.handle_key(key);
        match action {
            ConfirmAction::Pending => {}
            ConfirmAction::Confirmed => {
                // Create intercompany accounts for whichever sides need them.
                if mode.primary_needs_accounts
                    && let Err(e) = crate::inter_entity::create_intercompany_accounts(
                        &self.entity.db,
                        &mode.secondary_name.clone(),
                    )
                {
                    self.status_bar
                        .set_error(format!("Failed to create primary accounts: {e}"));
                }
                if mode.secondary_needs_accounts {
                    let primary_name = mode.primary_name.clone();
                    if let Err(e) = crate::inter_entity::create_intercompany_accounts(
                        &mode.secondary_db,
                        &primary_name,
                    ) {
                        self.status_bar
                            .set_error(format!("Failed to create secondary accounts: {e}"));
                    }
                }
                // Refresh account lists (clears needs_account_setup flag).
                let _ = mode.refresh_accounts(&self.entity.db);
                // Transition to form.
                let AppMode::InterEntityAccountSetup { mode, .. } =
                    std::mem::replace(&mut self.mode, AppMode::Normal)
                else {
                    return;
                };
                self.mode = AppMode::InterEntity(mode);
            }
            ConfirmAction::Cancelled => {
                // Skip account creation, go straight to form.
                let AppMode::InterEntityAccountSetup { mode, .. } =
                    std::mem::replace(&mut self.mode, AppMode::Normal)
                else {
                    return;
                };
                self.mode = AppMode::InterEntity(mode);
            }
        }
    }

    fn handle_secondary_picker_key(&mut self, key: KeyEvent) {
        let AppMode::SecondaryEntityPicker {
            ref mut selected,
            ref candidates,
        } = self.mode
        else {
            return;
        };
        let count = candidates.len();

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                *selected = selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if *selected + 1 < count {
                    *selected += 1;
                }
            }
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Enter => {
                let AppMode::SecondaryEntityPicker {
                    selected,
                    ref candidates,
                } = self.mode
                else {
                    return;
                };
                let cfg_idx = candidates[selected];
                let secondary_cfg = self.config.entities[cfg_idx].clone();
                match EntityDb::open(&secondary_cfg.db_path) {
                    Err(e) => {
                        self.mode = AppMode::Normal;
                        self.status_bar
                            .set_error(format!("Failed to open {}: {e}", secondary_cfg.name));
                    }
                    Ok(secondary_db) => {
                        match InterEntityMode::open(
                            &self.entity.db,
                            secondary_db,
                            self.entity.name.clone(),
                            secondary_cfg.name,
                        ) {
                            Err(e) => {
                                self.mode = AppMode::Normal;
                                self.status_bar
                                    .set_error(format!("Failed to open inter-entity mode: {e}"));
                            }
                            Ok(mode) => {
                                if mode.needs_account_setup() {
                                    let msg = build_account_setup_message(&mode);
                                    let confirm =
                                        crate::widgets::confirmation::Confirmation::new(msg);
                                    self.mode = AppMode::InterEntityAccountSetup {
                                        mode: Box::new(mode),
                                        confirm,
                                    };
                                } else {
                                    self.mode = AppMode::InterEntity(Box::new(mode));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn process_fiscal_modal_action(&mut self, action: FiscalModalAction) {
        match action {
            FiscalModalAction::None => {}
            FiscalModalAction::Close => {
                self.fiscal_modal = None;
            }
            FiscalModalAction::Mutated(msg) => {
                // Refresh all tabs so lock indicators and lists reflect the new state.
                for tab in &mut self.entity.tabs {
                    tab.refresh(&self.entity.db);
                }
                self.status_bar.set_message(msg);
            }
        }
    }

    fn process_action(&mut self, action: TabAction) {
        match action {
            TabAction::None => {}
            TabAction::SwitchTab(tab_id) => {
                self.active_tab = tab_id_to_index(tab_id);
            }
            TabAction::NavigateTo(tab_id, record_id) => {
                self.active_tab = tab_id_to_index(tab_id);
                self.entity.tabs[self.active_tab].navigate_to(record_id, &self.entity.db);
            }
            TabAction::ShowMessage(msg) => {
                self.status_bar.set_message(msg);
            }
            TabAction::RefreshData => {
                for tab in &mut self.entity.tabs {
                    tab.refresh(&self.entity.db);
                }
            }
            TabAction::StartInterEntityMode => {
                // Build candidate list: all entities except the active one.
                let active_name = &self.entity.name;
                let candidates: Vec<usize> = self
                    .config
                    .entities
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| &e.name != active_name)
                    .map(|(i, _)| i)
                    .collect();
                if candidates.is_empty() {
                    self.status_bar.set_error(
                        "Inter-entity mode requires at least two entities in workspace config."
                            .to_owned(),
                    );
                } else {
                    self.mode = AppMode::SecondaryEntityPicker {
                        selected: 0,
                        candidates,
                    };
                }
            }
            TabAction::Quit => {
                self.should_quit = true;
            }
        }
    }
}

/// Renders a centered help overlay showing global and tab-specific hotkeys.
fn render_help_overlay(
    frame: &mut ratatui::Frame,
    area: Rect,
    tab_hotkeys: Vec<(&'static str, &'static str)>,
) {
    let global_hotkeys: &[(&str, &str)] = &[
        ("1–9", "Switch to tab"),
        ("Ctrl+← / Ctrl+→", "Previous / next tab"),
        ("f", "Fiscal period management"),
        ("Ctrl+H", "Open user guide"),
        ("q", "Quit"),
        ("?", "Show / hide this help"),
    ];

    // Calculate popup size: width = 60, height = rows + borders + section headers.
    let row_count = global_hotkeys.len() + tab_hotkeys.len() + 3; // +3: two headers + blank line
    let popup_height = (row_count + 2).min(area.height as usize) as u16;
    let popup_width = 66u16.min(area.width);

    // Center the popup.
    let x = area.x + area.width.saturating_sub(popup_width) / 2;
    let y = area.y + area.height.saturating_sub(popup_height) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Build content lines.
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        " Global",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    for (key, desc) in global_hotkeys {
        lines.push(Line::from(vec![
            Span::styled(format!("  {key:<16}"), Style::default().fg(Color::Cyan)),
            Span::raw(*desc),
        ]));
    }

    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        " Tab-specific",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    if tab_hotkeys.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (none)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (key, desc) in &tab_hotkeys {
            lines.push(Line::from(vec![
                Span::styled(format!("  {key:<16}"), Style::default().fg(Color::Cyan)),
                Span::raw(*desc),
            ]));
        }
    }

    let block = Block::default()
        .title(" Help (Esc or ? to close) ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::White).bg(Color::Black));

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(Style::default().bg(Color::Black)),
        popup_area,
    );
}

fn build_account_setup_message(mode: &InterEntityMode) -> String {
    let mut parts = Vec::new();
    if mode.primary_needs_accounts {
        parts.push(format!(
            "• {} is missing Due From/To {} accounts",
            mode.primary_name, mode.secondary_name
        ));
    }
    if mode.secondary_needs_accounts {
        parts.push(format!(
            "• {} is missing Due From/To {} accounts",
            mode.secondary_name, mode.primary_name
        ));
    }
    format!("Create intercompany accounts?\n{}", parts.join("\n"))
}

fn render_secondary_entity_picker(
    frame: &mut ratatui::Frame,
    area: Rect,
    config: &WorkspaceConfig,
    selected: usize,
    candidates: &[usize],
) {
    let block = Block::default()
        .title(" Select Secondary Entity (↑↓ to move, Enter to open, Esc to cancel) ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<ratatui::text::Line> = candidates
        .iter()
        .enumerate()
        .map(|(i, &cfg_idx)| {
            let name = &config.entities[cfg_idx].name;
            if i == selected {
                ratatui::text::Line::from(vec![Span::styled(
                    format!("  ▶ {name}"),
                    Style::default().fg(Color::Yellow).bg(Color::DarkGray),
                )])
            } else {
                ratatui::text::Line::from(vec![Span::raw(format!("    {name}"))])
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

fn tab_id_to_index(tab_id: TabId) -> usize {
    TabId::all()
        .iter()
        .position(|t| *t == tab_id)
        .expect("tab_id_to_index: TabId::all() must contain every variant")
}

/// Drops when out of scope. Ensures terminal is restored even on panic.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
}

// ── Entity Creation Wizard ────────────────────────────────────────────────────

/// Steps in the entity creation multi-step form.
enum CreationStep {
    EntityName,
    DbPath,
    StartMonth,
}

/// State for the entity creation wizard.
struct EntityCreationForm {
    step: CreationStep,
    name: String,
    db_path: String,
    start_month: u32,
    error: Option<String>,
}

impl EntityCreationForm {
    fn new(default_db_dir: &Path) -> Self {
        let default_db_path = default_db_dir
            .join("entity.sqlite")
            .to_string_lossy()
            .into_owned();
        Self {
            step: CreationStep::EntityName,
            name: String::new(),
            db_path: default_db_path,
            start_month: 1,
            error: None,
        }
    }
}

/// Runs the entity creation wizard in the TUI. Returns the newly created `EntityContext`
/// and the updated `WorkspaceConfig` (with the new entity appended).
///
/// This function manages its own terminal setup/teardown so it can be called before
/// `App::run()` when the workspace has no entities.
pub fn run_entity_creation_wizard(
    config_path: &Path,
    config: &mut WorkspaceConfig,
) -> Result<EntityContext> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let _guard = TerminalGuard;

    // Default DB directory: same directory as the config file.
    let default_db_dir = config_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let result = run_wizard_loop(&mut terminal, config, config_path, &default_db_dir);
    restore_terminal();
    result
}

fn run_wizard_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    config: &mut WorkspaceConfig,
    config_path: &Path,
    default_db_dir: &Path,
) -> Result<EntityContext> {
    let mut form = EntityCreationForm::new(default_db_dir);

    loop {
        terminal.draw(|frame| render_wizard(frame, &form))?;

        if event::poll(std::time::Duration::from_millis(500))?
            && let Event::Key(key) = event::read()?
        {
            match wizard_handle_key(key, &mut form, config, config_path) {
                WizardOutcome::Continue => {}
                WizardOutcome::Done(ctx) => return Ok(ctx),
                WizardOutcome::Cancelled => {
                    anyhow::bail!("Entity creation cancelled by user");
                }
            }
        }
    }
}

enum WizardOutcome {
    Continue,
    Done(EntityContext),
    Cancelled,
}

fn wizard_handle_key(
    key: KeyEvent,
    form: &mut EntityCreationForm,
    config: &mut WorkspaceConfig,
    config_path: &Path,
) -> WizardOutcome {
    form.error = None;

    match key.code {
        KeyCode::Esc => return WizardOutcome::Cancelled,

        KeyCode::Backspace => match form.step {
            CreationStep::EntityName => {
                form.name.pop();
            }
            CreationStep::DbPath => {
                form.db_path.pop();
            }
            CreationStep::StartMonth => {}
        },

        KeyCode::Char(c) => match form.step {
            CreationStep::EntityName => form.name.push(c),
            CreationStep::DbPath => form.db_path.push(c),
            CreationStep::StartMonth => {
                if let Some(digit) = c.to_digit(10) {
                    let new_month = form.start_month * 10 + digit;
                    if new_month <= 12 {
                        form.start_month = new_month;
                    }
                }
            }
        },

        KeyCode::Up => {
            if matches!(form.step, CreationStep::StartMonth) && form.start_month < 12 {
                form.start_month += 1;
            }
        }
        KeyCode::Down => {
            if matches!(form.step, CreationStep::StartMonth) && form.start_month > 1 {
                form.start_month -= 1;
            }
        }

        KeyCode::Enter => match form.step {
            CreationStep::EntityName => {
                if form.name.trim().is_empty() {
                    form.error = Some("Entity name cannot be empty.".to_owned());
                } else {
                    form.step = CreationStep::DbPath;
                }
            }
            CreationStep::DbPath => {
                if form.db_path.trim().is_empty() {
                    form.error = Some("Database path cannot be empty.".to_owned());
                } else {
                    form.step = CreationStep::StartMonth;
                }
            }
            CreationStep::StartMonth => {
                // Validate and create entity.
                if !(1..=12).contains(&form.start_month) {
                    form.error = Some("Start month must be between 1 and 12.".to_owned());
                    return WizardOutcome::Continue;
                }
                let db_path = std::path::PathBuf::from(form.db_path.trim());
                match EntityDb::create(&db_path, form.name.trim(), form.start_month) {
                    Err(e) => {
                        form.error = Some(format!("Failed to create database: {e}"));
                    }
                    Ok(db) => {
                        let entity_name = form.name.trim().to_owned();
                        config.entities.push(EntityConfig {
                            name: entity_name.clone(),
                            db_path: db_path.clone(),
                            config_path: None,
                        });
                        if let Err(e) = save_config(config_path, config) {
                            form.error = Some(format!("Failed to save config: {e}"));
                            return WizardOutcome::Continue;
                        }
                        let ctx =
                            EntityContext::new(db, entity_name, config.report_output_dir.clone());
                        return WizardOutcome::Done(ctx);
                    }
                }
            }
        },

        _ => {}
    }
    WizardOutcome::Continue
}

fn render_wizard(frame: &mut ratatui::Frame, form: &EntityCreationForm) {
    let area = frame.area();
    let block = Block::default()
        .title(" New Entity Setup ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(inner);

    // Instructions
    let instructions = match form.step {
        CreationStep::EntityName => {
            "Step 1/3: Enter entity name (Enter to continue, Esc to cancel)"
        }
        CreationStep::DbPath => "Step 2/3: Enter database file path (Enter to continue)",
        CreationStep::StartMonth => {
            "Step 3/3: Fiscal year start month (Up/Down or type 1-12, Enter to create)"
        }
    };
    frame.render_widget(
        Paragraph::new(instructions).alignment(Alignment::Center),
        chunks[0],
    );

    // Entity name field
    let name_style = if matches!(form.step, CreationStep::EntityName) {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Gray)
    };
    frame.render_widget(
        Paragraph::new(format!("  {}", form.name))
            .block(Block::default().borders(Borders::ALL).title("Entity Name"))
            .style(name_style),
        chunks[1],
    );

    // DB path field
    let path_style = if matches!(form.step, CreationStep::DbPath) {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Gray)
    };
    frame.render_widget(
        Paragraph::new(format!("  {}", form.db_path))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Database File Path"),
            )
            .style(path_style),
        chunks[2],
    );

    // Start month field
    let month_style = if matches!(form.step, CreationStep::StartMonth) {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Gray)
    };
    let month_names = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let month_name = if form.start_month >= 1 && form.start_month <= 12 {
        month_names[(form.start_month - 1) as usize]
    } else {
        "Invalid"
    };
    frame.render_widget(
        Paragraph::new(format!("  {} ({})", form.start_month, month_name))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Fiscal Year Start Month"),
            )
            .style(month_style),
        chunks[3],
    );

    // Error message
    if let Some(err) = &form.error {
        frame.render_widget(
            Paragraph::new(err.as_str())
                .style(Style::default().fg(Color::Red))
                .alignment(Alignment::Center),
            chunks[4],
        );
    }
}

// ── Entity Picker ─────────────────────────────────────────────────────────────

/// Runs an entity picker modal when multiple entities are configured.
/// If only one entity is configured, opens it directly without showing the picker.
/// Returns the selected `EntityContext`.
pub fn run_entity_picker(config: &WorkspaceConfig) -> Result<EntityContext> {
    if config.entities.len() == 1 {
        let entity_cfg = &config.entities[0];
        let db = EntityDb::open(&entity_cfg.db_path)?;
        return Ok(EntityContext::new(
            db,
            entity_cfg.name.clone(),
            config.report_output_dir.clone(),
        ));
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let _guard = TerminalGuard;

    let result = run_picker_loop(&mut terminal, config);
    restore_terminal();
    result
}

fn run_picker_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    config: &WorkspaceConfig,
) -> Result<EntityContext> {
    let mut selected: usize = 0;
    let count = config.entities.len();

    loop {
        terminal.draw(|frame| render_picker(frame, config, selected))?;

        if event::poll(std::time::Duration::from_millis(500))?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected + 1 < count {
                        selected += 1;
                    }
                }
                KeyCode::Enter => {
                    let entity_cfg = &config.entities[selected];
                    let db = EntityDb::open(&entity_cfg.db_path)?;
                    return Ok(EntityContext::new(
                        db,
                        entity_cfg.name.clone(),
                        config.report_output_dir.clone(),
                    ));
                }
                KeyCode::Esc => {
                    anyhow::bail!("Entity selection cancelled");
                }
                _ => {}
            }
        }
    }
}

fn render_picker(frame: &mut ratatui::Frame, config: &WorkspaceConfig, selected: usize) {
    let area = frame.area();
    let block = Block::default()
        .title(" Select Entity (↑↓ to move, Enter to open, Esc to quit) ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = config
        .entities
        .iter()
        .enumerate()
        .map(|(i, entity)| {
            if i == selected {
                Line::from(vec![Span::styled(
                    format!("  ▶ {}", entity.name),
                    Style::default().fg(Color::Yellow).bg(Color::DarkGray),
                )])
            } else {
                Line::from(vec![Span::raw(format!("    {}", entity.name))])
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}
