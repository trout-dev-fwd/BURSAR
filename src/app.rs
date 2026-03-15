use std::io;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Tabs},
};

use crate::{
    config::WorkspaceConfig,
    db::EntityDb,
    tabs::{
        Tab, TabAction, TabId, accounts_payable::AccountsPayableTab,
        accounts_receivable::AccountsReceivableTab, audit_log::AuditLogTab,
        chart_of_accounts::ChartOfAccountsTab, envelopes::EnvelopesTab,
        fixed_assets::FixedAssetsTab, general_ledger::GeneralLedgerTab,
        journal_entries::JournalEntriesTab, reports::ReportsTab,
    },
    widgets::StatusBar,
};

/// Operating mode of the application.
pub enum AppMode {
    Normal,
    // TODO(Phase 6): InterEntity(InterEntityMode)
    // TODO(Phase 1+): Modal(ModalKind)  — used for entity picker / creation prompts
}

/// Active entity context: database handle, entity name, and the 9 tab instances.
pub struct EntityContext {
    pub db: EntityDb,
    pub name: String,
    pub tabs: Vec<Box<dyn Tab>>,
}

impl EntityContext {
    /// Creates an entity context from an open EntityDb, building all 9 stub tabs.
    pub fn new(db: EntityDb, name: String) -> Self {
        let tabs: Vec<Box<dyn Tab>> = vec![
            Box::new(ChartOfAccountsTab),
            Box::new(GeneralLedgerTab),
            Box::new(JournalEntriesTab),
            Box::new(AccountsReceivableTab),
            Box::new(AccountsPayableTab),
            Box::new(EnvelopesTab),
            Box::new(FixedAssetsTab),
            Box::new(ReportsTab),
            Box::new(AuditLogTab),
        ];
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
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3), // tab bar
                        Constraint::Min(0),    // content
                        Constraint::Length(1), // status bar
                    ])
                    .split(frame.area());

                self.render_tab_bar(frame, chunks[0]);

                match &self.mode {
                    AppMode::Normal => {
                        self.entity.tabs[self.active_tab].render(frame, chunks[1]);
                    }
                }

                self.status_bar.render(frame, chunks[2]);
            })?;

            // 2. Poll for input (500ms timeout).
            if event::poll(std::time::Duration::from_millis(500))?
                && let Event::Key(key) = event::read()?
            {
                self.handle_key(key);
            }

            // 3. Tick: update status bar timeout.
            self.status_bar.tick();

            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    fn render_tab_bar(&self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        let tab_titles: Vec<Line> = self
            .entity
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                Line::from(vec![Span::styled(
                    format!(" {} ", tab.title()),
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

    fn handle_key(&mut self, key: KeyEvent) {
        // Global hotkeys.
        match key.code {
            KeyCode::Char('q') if key.modifiers == KeyModifiers::NONE => {
                self.should_quit = true;
            }
            // Tab switching: 1–9 keys select tabs by number.
            KeyCode::Char(c @ '1'..='9') if key.modifiers == KeyModifiers::NONE => {
                let idx = (c as usize) - ('1' as usize);
                if idx < self.entity.tabs.len() {
                    self.active_tab = idx;
                }
            }
            _ => {
                // Delegate to active tab.
                let action = self.entity.tabs[self.active_tab].handle_key(key, &self.entity.db);
                self.process_action(action);
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
                // TODO(Phase 6): open inter-entity modal
                self.status_bar
                    .set_message("Inter-entity mode not yet implemented".to_owned());
            }
            TabAction::Quit => {
                self.should_quit = true;
            }
        }
    }
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
