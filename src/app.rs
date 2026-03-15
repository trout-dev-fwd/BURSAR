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
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
};

use crate::{
    config::{EntityConfig, WorkspaceConfig, save_config},
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
    /// Creates an entity context from an open EntityDb, building all 9 tabs and
    /// performing an initial data load so tabs render content immediately.
    pub fn new(db: EntityDb, name: String) -> Self {
        let mut coa = ChartOfAccountsTab::new();
        coa.set_entity_name(&name);
        let mut je = JournalEntriesTab::new();
        je.set_entity_name(&name);
        let mut ar = AccountsReceivableTab::new();
        ar.set_entity_name(&name);
        let mut ap = AccountsPayableTab::new();
        ap.set_entity_name(&name);
        let mut tabs: Vec<Box<dyn Tab>> = vec![
            Box::new(coa),
            Box::new(GeneralLedgerTab::new()),
            Box::new(je),
            Box::new(ar),
            Box::new(ap),
            Box::new(EnvelopesTab),
            Box::new(FixedAssetsTab),
            Box::new(ReportsTab),
            Box::new(AuditLogTab),
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
                        });
                        if let Err(e) = save_config(config_path, config) {
                            form.error = Some(format!("Failed to save config: {e}"));
                            return WizardOutcome::Continue;
                        }
                        let ctx = EntityContext::new(db, entity_name);
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
        return Ok(EntityContext::new(db, entity_cfg.name.clone()));
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
                    return Ok(EntityContext::new(db, entity_cfg.name.clone()));
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
