mod ai_handler;
mod import_handler;
mod key_dispatch;
use import_handler::render_import_modal;
use key_dispatch::{render_help_overlay, render_secondary_entity_picker};

use std::{io, path::Path};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
};

use crate::{
    ai::{
        ApiMessage,
        client::AiClient,
        csv_import::{ImportFlowState, ImportFlowStep},
    },
    config::{EntityConfig, WorkspaceConfig, save_config},
    db::EntityDb,
    inter_entity::InterEntityMode,
    tabs::{
        Tab, accounts_payable::AccountsPayableTab, accounts_receivable::AccountsReceivableTab,
        audit_log::AuditLogTab, chart_of_accounts::ChartOfAccountsTab, envelopes::EnvelopesTab,
        fixed_assets::FixedAssetsTab, general_ledger::GeneralLedgerTab,
        journal_entries::JournalEntriesTab, reports::ReportsTab,
    },
    types::{AiRequestState, FocusTarget},
    widgets::{
        FeedbackModal, FilePicker, FiscalModal, StatusBar, UserGuide,
        chat_panel::{ChatPanel, SlashCommand},
    },
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
    /// When true, the help overlay shows inter-entity hotkeys instead of tab hotkeys.
    inter_entity_help: bool,
    user_guide: Option<UserGuide>,
    should_quit: bool,
    chat_panel: ChatPanel,
    focus: FocusTarget,
    /// Current AI API interaction state (Idle / CallingApi / FulfillingTools).
    ai_state: AiRequestState,
    /// Lazily initialized on the first AI request.
    ai_client: Option<AiClient>,
    /// Set by handle_key when a SendMessage action arrives; consumed by event_loop.
    pending_ai_messages: Option<Vec<ApiMessage>>,
    /// Set by handle_key when a SlashCommand action arrives; consumed by event_loop.
    pending_slash_command: Option<SlashCommand>,
    /// File browser shown at the first step of the CSV import flow.
    file_picker: Option<FilePicker>,
    /// Active CSV import wizard state (Some while import is in progress).
    import_flow: Option<ImportFlowState>,
    /// Set when NewBankDetection step begins; consumed by event_loop to run the API call.
    pending_bank_detection: bool,
    /// Set when Pass1Matching step begins; consumed by event_loop to run local matching.
    pending_pass1: bool,
    /// Set when Pass2AiMatching step begins; consumed by event_loop to run AI matching.
    pending_pass2: bool,
    /// Set when Creating step begins; consumed by event_loop to run batch draft creation.
    pending_draft_creation: bool,
    /// Multi-line feedback modal (bug report or feature request), shown over the main UI.
    feedback_modal: Option<FeedbackModal>,
}

impl App {
    pub fn new(entity: EntityContext, config: WorkspaceConfig) -> Self {
        let status_bar = StatusBar::new(entity.name.clone(), String::new());
        let persona = config
            .ai
            .as_ref()
            .map(|ai| ai.persona.clone())
            .unwrap_or_else(|| "Professional Tax Accountant".to_string());
        let chat_panel = ChatPanel::new(&entity.name, &persona);
        Self {
            entity,
            config,
            active_tab: 0,
            mode: AppMode::Normal,
            status_bar,
            fiscal_modal: None,
            show_help: false,
            inter_entity_help: false,
            user_guide: None,
            should_quit: false,
            chat_panel,
            focus: FocusTarget::MainTab,
            ai_state: AiRequestState::Idle,
            ai_client: None,
            pending_ai_messages: None,
            pending_slash_command: None,
            file_picker: None,
            import_flow: None,
            pending_bank_detection: false,
            pending_pass1: false,
            pending_pass2: false,
            pending_draft_creation: false,
            feedback_modal: None,
        }
    }

    // ── Public methods for external event-loop drivers ───────────────────────

    /// Draws one frame to the terminal.
    pub fn render<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        terminal.draw(|frame| self.render_frame(frame))?;
        Ok(())
    }

    /// Processes one input event (currently handles Key events only).
    pub fn handle_event(&mut self, event: &Event) {
        if let Event::Key(key) = event {
            self.handle_key(*key);
        }
    }

    /// Periodic work: advance typewriter, expire status bar messages, update unsaved indicator.
    pub fn tick(&mut self) {
        self.chat_panel.tick();
        self.status_bar.tick();
        let unsaved = self.entity.tabs[self.active_tab].has_unsaved_changes();
        self.status_bar.set_unsaved(unsaved);
    }

    /// Returns `true` when the application should exit.
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Dispatches any pending AI requests, slash commands, or import pipeline steps.
    /// Must be called once per tick after `handle_event`.
    pub fn process_pending<B: ratatui::backend::Backend>(&mut self, terminal: &mut Terminal<B>) {
        if let Some(messages) = self.pending_ai_messages.take() {
            self.handle_ai_request(terminal, messages);
        }
        if let Some(cmd) = self.pending_slash_command.take() {
            self.execute_slash_command(terminal, cmd);
        }
        if self.pending_bank_detection {
            self.pending_bank_detection = false;
            self.run_bank_detection(terminal);
        }
        if self.pending_pass1 {
            self.pending_pass1 = false;
            self.run_pass1_step(terminal);
        }
        if self.pending_pass2 {
            self.pending_pass2 = false;
            self.run_pass2_step(terminal);
        }
        if self.pending_draft_creation {
            self.pending_draft_creation = false;
            self.run_draft_creation_step(terminal);
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
            self.render(terminal)?;

            if event::poll(std::time::Duration::from_millis(500))? {
                let evt = event::read()?;
                self.handle_event(&evt);
            }

            self.process_pending(terminal);
            self.tick();

            if self.should_quit() {
                break;
            }
        }
        Ok(())
    }

    /// Renders the complete UI frame. Called from the event loop draw closure
    /// and from `handle_ai_request` before issuing blocking API calls.
    fn render_frame(&mut self, frame: &mut ratatui::Frame) {
        let tab_bar_height = self.tab_bar_height(frame.area().width);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(tab_bar_height),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(frame.area());

        self.render_tab_bar(frame, chunks[0]);

        // Split content area when the AI panel is visible (70% tab / 30% panel).
        let (tab_area, panel_area) = if self.chat_panel.is_visible() {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
                .split(chunks[1]);
            (split[0], Some(split[1]))
        } else {
            (chunks[1], None)
        };

        match &self.mode {
            AppMode::Normal => {
                self.entity.tabs[self.active_tab].render(frame, tab_area);
            }
            AppMode::SecondaryEntityPicker {
                selected,
                candidates,
            } => {
                render_secondary_entity_picker(
                    frame,
                    tab_area,
                    &self.config,
                    *selected,
                    candidates,
                );
            }
            AppMode::InterEntityAccountSetup { mode, confirm } => {
                mode.form.render(
                    frame,
                    tab_area,
                    &mode.primary_name,
                    &mode.secondary_name,
                    &mode.primary_accounts,
                    &mode.secondary_accounts,
                );
                confirm.render(frame, tab_area);
            }
            AppMode::InterEntity(mode) => {
                mode.form.render(
                    frame,
                    tab_area,
                    &mode.primary_name,
                    &mode.secondary_name,
                    &mode.primary_accounts,
                    &mode.secondary_accounts,
                );
            }
        }

        if let Some(ref mut modal) = self.fiscal_modal {
            modal.render(frame, tab_area);
        }
        if self.show_help {
            let hotkeys = if self.inter_entity_help {
                vec![
                    ("Tab / Shift+Tab", "Next / previous field"),
                    ("↑ / ↓", "Move between rows and entities"),
                    ("← / →", "Move between columns"),
                    ("Enter", "Open account picker"),
                    ("F2", "Add line row"),
                    ("F3 / Del", "Remove line row"),
                    ("Ctrl+S", "Submit inter-entity JE"),
                    ("Esc", "Cancel / close"),
                    ("?", "Show / hide this help"),
                ]
            } else {
                self.entity.tabs[self.active_tab].hotkey_help()
            };
            render_help_overlay(frame, tab_area, hotkeys, self.chat_panel.is_visible());
        }
        if let Some(guide) = &self.user_guide {
            guide.render(frame, tab_area);
        }
        if let Some(ref picker) = self.file_picker {
            picker.render(frame, tab_area);
        }
        if let Some(ref flow) = self.import_flow {
            // Look up bank account type for review screen preview.
            let bank_account_type = flow
                .bank_config
                .as_ref()
                .and_then(|cfg| {
                    self.entity
                        .db
                        .accounts()
                        .list_all()
                        .ok()?
                        .into_iter()
                        .find(|a| a.number == cfg.linked_account)
                        .map(|a| a.account_type)
                })
                .unwrap_or(crate::types::AccountType::Asset);
            render_import_modal(frame, tab_area, flow, bank_account_type);
        }
        if let Some(ref mut modal) = self.feedback_modal {
            modal.render(frame, tab_area);
        }
        if let Some(area) = panel_area {
            let is_focused = matches!(self.focus, FocusTarget::ChatPanel);
            self.chat_panel.render(frame, area, is_focused);
        }

        self.status_bar.render(frame, chunks[2]);
    }

    // Import-related methods (enter_duplicate_check, run_bank_detection, run_pass1_step,
    // run_pass2_step, run_draft_creation_step, handle_file_picker_key, handle_import_key)
    // live in import_handler.rs.
    // Standalone import rendering functions also live there.

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
