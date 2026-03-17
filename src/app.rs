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
    ai::{
        AiError, AiResponse, ApiContent, ApiMessage, ApiRole, RoundResult, ToolResult,
        client::AiClient,
        context::read_context,
        csv_import::{ImportFlowState, ImportFlowStep},
        tools::{fulfill_tool_call, tool_definitions},
    },
    config::{
        EntityConfig, WorkspaceConfig, load_entity_toml, load_secrets, save_config,
        save_entity_toml,
    },
    db::EntityDb,
    inter_entity::{InterEntityMode, form::InterEntityFormAction, write_protocol},
    tabs::{
        Tab, TabAction, TabId, accounts_payable::AccountsPayableTab,
        accounts_receivable::AccountsReceivableTab, audit_log::AuditLogTab,
        chart_of_accounts::ChartOfAccountsTab, envelopes::EnvelopesTab,
        fixed_assets::FixedAssetsTab, general_ledger::GeneralLedgerTab,
        journal_entries::JournalEntriesTab, reports::ReportsTab,
    },
    types::{AiRequestState, FocusTarget},
    widgets::{
        FiscalModal, FiscalModalAction, StatusBar, UserGuide, UserGuideAction,
        chat_panel::{ChatAction, ChatPanel, SlashCommand},
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
    /// Active CSV import wizard state (Some while import is in progress).
    import_flow: Option<ImportFlowState>,
    /// Set when NewBankDetection step begins; consumed by event_loop to run the API call.
    pending_bank_detection: bool,
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
            user_guide: None,
            should_quit: false,
            chat_panel,
            focus: FocusTarget::MainTab,
            ai_state: AiRequestState::Idle,
            ai_client: None,
            pending_ai_messages: None,
            pending_slash_command: None,
            import_flow: None,
            pending_bank_detection: false,
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
            terminal.draw(|frame| self.render_frame(frame))?;

            // 2. Poll for input (500ms timeout).
            if event::poll(std::time::Duration::from_millis(500))?
                && let Event::Key(key) = event::read()?
            {
                self.handle_key(key);
            }

            // 2b. If a SendMessage action was queued, fire the AI request now.
            if let Some(messages) = self.pending_ai_messages.take() {
                self.handle_ai_request(terminal, messages);
            }

            // 2c. If a SlashCommand was queued, execute it now.
            if let Some(cmd) = self.pending_slash_command.take() {
                self.execute_slash_command(terminal, cmd);
            }

            // 2d. If bank format detection is pending, run it now (blocking API call).
            if self.pending_bank_detection {
                self.pending_bank_detection = false;
                self.run_bank_detection(terminal);
            }

            // 3. Tick: advance typewriter, update status bar timeout + unsaved indicator.
            self.chat_panel.tick();
            self.status_bar.tick();
            let unsaved = self.entity.tabs[self.active_tab].has_unsaved_changes();
            self.status_bar.set_unsaved(unsaved);

            if self.should_quit {
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
                    &std::collections::HashMap::new(),
                    &std::collections::HashMap::new(),
                );
                let popup_w = 60u16.min(tab_area.width);
                let popup_h = 6u16.min(tab_area.height);
                let px = tab_area.x + tab_area.width.saturating_sub(popup_w) / 2;
                let py = tab_area.y + tab_area.height.saturating_sub(popup_h) / 2;
                let popup_area = ratatui::layout::Rect::new(px, py, popup_w, popup_h);
                frame.render_widget(ratatui::widgets::Clear, popup_area);
                confirm.render(frame, popup_area);
            }
            AppMode::InterEntity(mode) => {
                mode.form.render(
                    frame,
                    tab_area,
                    &mode.primary_name,
                    &mode.secondary_name,
                    &mode.primary_accounts,
                    &mode.secondary_accounts,
                    &std::collections::HashMap::new(),
                    &std::collections::HashMap::new(),
                );
            }
        }

        if let Some(ref mut modal) = self.fiscal_modal {
            modal.render(frame, tab_area);
        }
        if self.show_help {
            render_help_overlay(
                frame,
                tab_area,
                self.entity.tabs[self.active_tab].hotkey_help(),
                self.chat_panel.is_visible(),
            );
        }
        if let Some(guide) = &self.user_guide {
            guide.render(frame, tab_area);
        }
        if let Some(ref flow) = self.import_flow {
            render_import_modal(frame, tab_area, flow);
        }
        if let Some(area) = panel_area {
            let is_focused = matches!(self.focus, FocusTarget::ChatPanel);
            self.chat_panel.render(frame, area, is_focused);
        }

        self.status_bar.render(frame, chunks[2]);
    }

    /// Initialise `self.ai_client` from secrets on first use.
    ///
    /// Returns `Ok(())` when the client is ready, or `Err(message)` if the
    /// API key could not be loaded.
    fn ensure_ai_client(&mut self) -> Result<(), String> {
        if self.ai_client.is_some() {
            return Ok(());
        }
        let secrets = load_secrets()
            .map_err(|_| "No API key — see ~/.config/bookkeeper/secrets.toml".to_string())?;
        let model = self
            .config
            .ai
            .as_ref()
            .map(|ai| ai.model.clone())
            .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
        self.ai_client = Some(AiClient::new(secrets.anthropic_api_key, model));
        Ok(())
    }

    /// Executes an AI chat request: loads secrets, builds the system prompt, issues
    /// blocking API calls round by round, and routes the response back to the chat
    /// panel.  Between tool-use rounds, logs `AiToolUse` to audit, updates the
    /// status bar, and calls `terminal.draw()` so the user sees
    /// "Checking the books 🕮".
    ///
    /// Must be called from the event loop (not from handle_key) so `terminal` is
    /// available for forced renders between rounds.
    fn handle_ai_request<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        messages: Vec<ApiMessage>,
    ) {
        // ── Lazy-init AiClient ────────────────────────────────────────────────
        if let Err(msg) = self.ensure_ai_client() {
            self.status_bar.set_error(msg);
            return;
        }

        // ── Load entity context ───────────────────────────────────────────────
        let context_dir = self.context_dir();
        let context = read_context(&self.entity.name, &context_dir).unwrap_or_default();

        // ── Build system prompt ───────────────────────────────────────────────
        let persona = self.chat_panel.current_persona.clone();
        let entity_name = self.entity.name.clone();
        let system_prompt = AiClient::build_system_prompt(&persona, &entity_name, &context);

        // ── Log AiPrompt (last user message) ─────────────────────────────────
        if let Some(msg) = messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, ApiRole::User))
            && let ApiContent::Text(text) = &msg.content
        {
            let _ = self.entity.db.audit().log_ai_prompt(&entity_name, text);
        }

        // ── Force render with loading state before first call ────────────────
        self.ai_state = AiRequestState::CallingApi;
        self.status_bar
            .set_ai_status(Some("Calling Accountant \u{260F}".to_string()));
        let _ = terminal.draw(|frame| self.render_frame(frame));

        // ── Tool use loop (up to 5 follow-up rounds) ─────────────────────────
        let tools = tool_definitions();
        let client = self.ai_client.take().expect("just initialized");
        let max_depth: usize = 5;
        let mut msgs = messages;
        let mut accumulated_text: Option<String> = None;
        let mut result: Result<AiResponse, AiError> = Err(AiError::MaxToolDepth);

        for round in 0..=max_depth {
            match client.send_single_round(&system_prompt, &msgs, &tools, accumulated_text.take()) {
                Ok(RoundResult::Done(response)) => {
                    result = Ok(response);
                    break;
                }
                Ok(RoundResult::NeedsToolCall {
                    tool_calls,
                    messages: updated_msgs,
                    accumulated_text: acc,
                }) => {
                    if round == max_depth {
                        tracing::warn!(
                            "Tool use loop exceeded max depth ({max_depth}); \
                             returning partial response"
                        );
                        let fallback = acc.unwrap_or_else(|| {
                            "I reached the maximum number of tool calls. \
                             Please try a simpler question."
                                .to_string()
                        });
                        let (content, summary) = AiClient::parse_summary(&fallback);
                        result = Ok(AiResponse::Text { content, summary });
                        break;
                    }

                    // Log AiToolUse for each tool call.
                    for tc in &tool_calls {
                        let _ = self.entity.db.audit().log_ai_tool_use(
                            &entity_name,
                            &tc.name,
                            &tc.input.to_string(),
                        );
                    }

                    // Render "Checking the books" between rounds.
                    self.ai_state = AiRequestState::FulfillingTools;
                    self.status_bar
                        .set_ai_status(Some("Checking the books \u{1F56E}".to_string()));
                    let _ = terminal.draw(|frame| self.render_frame(frame));

                    // Fulfill tool calls.
                    let tool_results: Vec<ToolResult> = tool_calls
                        .iter()
                        .map(|tc| {
                            let content =
                                fulfill_tool_call(tc, &self.entity.db).unwrap_or_else(|e| {
                                    tracing::warn!(
                                        tool = %tc.name, error = %e,
                                        "Tool fulfillment error"
                                    );
                                    format!("Error: {e}")
                                });
                            ToolResult {
                                tool_use_id: tc.id.clone(),
                                content,
                            }
                        })
                        .collect();

                    msgs = updated_msgs;
                    msgs.push(ApiMessage {
                        role: ApiRole::User,
                        content: ApiContent::ToolResult(tool_results),
                    });
                    accumulated_text = acc;

                    // Restore CallingApi state before next round.
                    self.ai_state = AiRequestState::CallingApi;
                    self.status_bar
                        .set_ai_status(Some("Calling Accountant \u{260F}".to_string()));
                    let _ = terminal.draw(|frame| self.render_frame(frame));
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }

        // Return client to self.
        self.ai_client = Some(client);

        // ── Handle result ─────────────────────────────────────────────────────
        self.ai_state = AiRequestState::Idle;
        self.status_bar.set_ai_status(None);

        match result {
            Ok(AiResponse::Text { content, summary }) => {
                let _ = self
                    .entity
                    .db
                    .audit()
                    .log_ai_response(&entity_name, &summary);
                self.chat_panel.add_response(content);
            }
            Ok(AiResponse::ToolUse(_)) => {
                // Should not reach here — loop always terminates with Text.
                self.chat_panel
                    .add_system_note("[Unexpected tool use termination]");
            }
            Err(AiError::Timeout) => {
                self.status_bar
                    .set_error("The Call Dropped \u{2639} (timeout)".to_string());
            }
            Err(AiError::NoApiKey) => {
                self.status_bar
                    .set_error("No API key — see ~/.config/bookkeeper/secrets.toml".to_string());
            }
            Err(e) => {
                self.status_bar
                    .set_error(format!("The Call Dropped \u{2639}: {e}"));
            }
        }
    }

    /// Runs bank format detection: reads first 4 CSV lines, sends to Claude, parses response.
    /// Updates `import_flow` with the detected config or moves to Failed step on error.
    fn run_bank_detection<B: ratatui::backend::Backend>(&mut self, terminal: &mut Terminal<B>) {
        // Ensure import_flow is active and in the NewBankDetection step.
        let Some(ref flow) = self.import_flow else {
            return;
        };
        if flow.step != ImportFlowStep::NewBankDetection {
            return;
        }

        let file_path = match &flow.file_path {
            Some(p) => p.clone(),
            None => {
                if let Some(ref mut f) = self.import_flow {
                    f.step = ImportFlowStep::Failed("No file path set".to_string());
                }
                return;
            }
        };
        let bank_name = flow.new_bank_name.clone().unwrap_or_default();

        // Force render so user sees "Initializing ↻" before blocking.
        let _ = terminal.draw(|frame| self.render_frame(frame));

        // Read first 4 lines of the CSV file.
        let csv_sample = match std::fs::read_to_string(&file_path) {
            Ok(contents) => contents.lines().take(4).collect::<Vec<_>>().join("\n"),
            Err(e) => {
                if let Some(ref mut f) = self.import_flow {
                    f.step = ImportFlowStep::Failed(format!("Failed to read file: {e}"));
                }
                return;
            }
        };

        // Lazy-init AI client.
        if let Err(msg) = self.ensure_ai_client() {
            if let Some(ref mut f) = self.import_flow {
                f.step = ImportFlowStep::Failed(format!("Failed \u{2328}: {msg}"));
            }
            return;
        }

        let system = "You are a CSV format analyzer. Respond ONLY with valid JSON, no other text.";
        let prompt = format!(
            "Analyze this CSV header and sample rows from a bank statement named \"{bank_name}\".\n\
             Identify: date column name, date format (chrono-compatible like %m/%d/%Y or %Y-%m-%d),\n\
             description/memo column name, and either a single amount column (with sign convention)\n\
             or separate debit/credit columns.\n\
             Respond ONLY with JSON containing these exact fields:\n\
             {{\"date_column\": \"...\", \"date_format\": \"...\", \"description_column\": \"...\",\n\
             \"amount_column\": \"...\" or null, \"debit_column\": \"...\" or null,\n\
             \"credit_column\": \"...\" or null, \"debit_is_negative\": true/false}}\n\n\
             CSV sample:\n{csv_sample}"
        );

        let messages = vec![ApiMessage {
            role: ApiRole::User,
            content: ApiContent::Text(prompt),
        }];

        let result = {
            let client = self.ai_client.as_ref().expect("just initialized");
            client.send_simple(system, &messages)
        };

        match result {
            Ok(json_str) => {
                // Extract JSON from response (Claude may wrap it in markdown).
                let json_str = extract_json_block(&json_str);
                match serde_json::from_str::<serde_json::Value>(&json_str) {
                    Ok(v) => {
                        let cfg = crate::config::BankAccountConfig {
                            name: bank_name,
                            linked_account: String::new(), // filled in Task 5
                            date_column: v["date_column"].as_str().unwrap_or("Date").to_string(),
                            date_format: v["date_format"]
                                .as_str()
                                .unwrap_or("%m/%d/%Y")
                                .to_string(),
                            description_column: v["description_column"]
                                .as_str()
                                .unwrap_or("Description")
                                .to_string(),
                            amount_column: v["amount_column"].as_str().map(|s| s.to_string()),
                            debit_column: v["debit_column"].as_str().map(|s| s.to_string()),
                            credit_column: v["credit_column"].as_str().map(|s| s.to_string()),
                            debit_is_negative: v["debit_is_negative"].as_bool().unwrap_or(true),
                        };
                        if let Some(ref mut f) = self.import_flow {
                            f.detected_config = Some(cfg);
                            f.step = ImportFlowStep::NewBankConfirmation;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse bank detection JSON: {e}\nRaw: {json_str}");
                        if let Some(ref mut f) = self.import_flow {
                            f.step = ImportFlowStep::Failed(
                                "Failed \u{2328}: invalid JSON response".to_string(),
                            );
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Bank detection API error: {e}");
                if let Some(ref mut f) = self.import_flow {
                    f.step = ImportFlowStep::Failed("Failed \u{2328}".to_string());
                }
            }
        }
    }

    /// Executes a slash command entered in the chat panel.
    fn execute_slash_command<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        cmd: SlashCommand,
    ) {
        match cmd {
            SlashCommand::Clear => {
                self.chat_panel.messages.clear();
                self.chat_panel.typewriter = None;
                let context_dir = self.context_dir();
                let context = read_context(&self.entity.name, &context_dir).unwrap_or_default();
                self.chat_panel.rebuild_system_prompt(
                    &self.chat_panel.current_persona.clone(),
                    &self.entity.name.clone(),
                    &context,
                );
                self.chat_panel.build_welcome();
                self.chat_panel.add_system_note("[Conversation cleared]");
            }
            SlashCommand::Context => {
                let context_dir = self.context_dir();
                let context = read_context(&self.entity.name, &context_dir).unwrap_or_default();
                let tab_name = self.entity.tabs[self.active_tab].title().to_string();
                self.chat_panel.rebuild_system_prompt(
                    &self.chat_panel.current_persona.clone(),
                    &self.entity.name.clone(),
                    &context,
                );
                self.chat_panel
                    .add_system_note(&format!("[Context refreshed from {tab_name} tab]"));
            }
            SlashCommand::Compact => {
                let msg_count = self.chat_panel.messages.len();
                if msg_count < 5 {
                    self.chat_panel
                        .add_system_note("Not enough conversation to compact (need ≥ 5 messages)");
                    return;
                }
                if let Err(msg) = self.ensure_ai_client() {
                    self.status_bar.set_error(msg);
                    return;
                }
                // Build compaction request.
                let history = self
                    .chat_panel
                    .api_messages()
                    .iter()
                    .map(|m| match &m.content {
                        ApiContent::Text(t) => format!(
                            "{}: {t}",
                            match m.role {
                                ApiRole::User => "User",
                                ApiRole::Assistant => "Accountant",
                            }
                        ),
                        _ => String::new(),
                    })
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let system = "You are a helpful assistant. Summarise the following conversation in a concise paragraph, preserving the key accounting facts and conclusions.";
                let compaction_messages = vec![ApiMessage {
                    role: ApiRole::User,
                    content: ApiContent::Text(format!(
                        "Please summarise this conversation:\n\n{history}"
                    )),
                }];
                self.status_bar
                    .set_ai_status(Some("Calling Accountant ☏".to_string()));
                let _ = terminal.draw(|frame| self.render_frame(frame));
                let result = {
                    let client = self.ai_client.as_ref().expect("just initialized");
                    client.send_simple(system, &compaction_messages)
                };
                self.status_bar.set_ai_status(None);
                match result {
                    Ok(summary) => {
                        self.chat_panel.replace_with_summary(summary, msg_count);
                    }
                    Err(e) => {
                        self.status_bar
                            .set_error(format!("The Call Dropped ☹: {e}"));
                    }
                }
            }
            SlashCommand::Persona(None) => {
                let persona = self.chat_panel.current_persona.clone();
                self.chat_panel
                    .add_system_note(&format!("Current persona: {persona}"));
            }
            SlashCommand::Persona(Some(new_persona)) => {
                // Save to entity toml.
                let (toml_path, workspace_dir) = self.entity_toml_path();
                let mut entity_cfg =
                    load_entity_toml(&toml_path, &workspace_dir).unwrap_or_default();
                entity_cfg.ai_persona = Some(new_persona.clone());
                if let Err(e) = save_entity_toml(&toml_path, &workspace_dir, &entity_cfg) {
                    self.status_bar
                        .set_error(format!("Failed to save persona: {e}"));
                    return;
                }
                // Rebuild system prompt with new persona.
                let context_dir = self.context_dir();
                let context = read_context(&self.entity.name, &context_dir).unwrap_or_default();
                self.chat_panel.rebuild_system_prompt(
                    &new_persona,
                    &self.entity.name.clone(),
                    &context,
                );
                self.chat_panel.add_system_note("[Persona updated]");
            }
            SlashCommand::Match => {
                // Full /match implementation deferred to Phase 3 (needs import infrastructure).
                self.chat_panel.add_system_note(
                    "Select an incomplete import draft in the Journal Entries tab first",
                );
            }
            SlashCommand::Unknown(name) => {
                self.chat_panel.add_system_note(&format!(
                    "Unknown command '/{name}'. Available: /clear, /context, /compact, /persona, /match"
                ));
            }
        }
    }

    /// Returns the context directory, falling back to `~/.config/bookkeeper/context`.
    fn context_dir(&self) -> String {
        self.config.context_dir.clone().unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            format!("{home}/.config/bookkeeper/context")
        })
    }

    /// Returns `(config_path_str, workspace_dir)` for the active entity's TOML file.
    /// The path is derived from the entity's db_path if no explicit config_path is set.
    fn entity_toml_path(&self) -> (String, std::path::PathBuf) {
        let entity_config = self
            .config
            .entities
            .iter()
            .find(|e| e.name == self.entity.name);
        if let Some(ec) = entity_config {
            if let Some(cp) = &ec.config_path {
                let dir = ec
                    .db_path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                return (cp.clone(), dir);
            }
            // Derive from db_path: same directory, same stem with .toml extension.
            let dir = ec
                .db_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let stem = ec
                .db_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "entity".to_string());
            return (format!("{stem}.toml"), dir);
        }
        ("entity.toml".to_string(), std::path::PathBuf::from("."))
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

        // Import wizard: all keys go to the wizard when it is active.
        if self.import_flow.is_some() {
            self.handle_import_key(key);
            return;
        }

        // Chat panel focus model.
        if self.chat_panel.is_visible() {
            if matches!(self.focus, FocusTarget::ChatPanel) {
                // Tab → switch focus back to main tab (not forwarded to panel).
                if key.code == KeyCode::Tab {
                    self.focus = FocusTarget::MainTab;
                    return;
                }
                // All other keys go to the panel.
                let action = self.chat_panel.handle_key(key);
                match action {
                    ChatAction::None => {}
                    ChatAction::Close => {
                        self.chat_panel.toggle_visible();
                        self.focus = FocusTarget::MainTab;
                    }
                    ChatAction::SkipTypewriter => {
                        self.chat_panel.skip_typewriter();
                    }
                    ChatAction::SendMessage(messages) => {
                        self.pending_ai_messages = Some(messages);
                    }
                    ChatAction::SlashCommand(cmd) => {
                        self.pending_slash_command = Some(cmd);
                    }
                }
                return;
            } else {
                // Panel visible, focus on MainTab.
                // Tab or Ctrl+K → hand focus to chat panel.
                let switch_focus = key.code == KeyCode::Tab
                    || (key.code == KeyCode::Char('k')
                        && key.modifiers.contains(KeyModifiers::CONTROL));
                if switch_focus {
                    self.focus = FocusTarget::ChatPanel;
                    return;
                }
            }
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
            // Ctrl+K — open AI chat panel and give it focus.
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.chat_panel.toggle_visible();
                if self.chat_panel.is_visible() {
                    self.focus = FocusTarget::ChatPanel;
                } else {
                    self.focus = FocusTarget::MainTab;
                }
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
            TabAction::StartImport => {
                let mut flow = ImportFlowState::new();
                // Pre-fill from last_import_dir if available.
                let (toml_path, workspace_dir) = self.entity_toml_path();
                if let Ok(entity_cfg) = crate::config::load_entity_toml(&toml_path, &workspace_dir)
                    && let Some(ref dir) = entity_cfg.last_import_dir
                {
                    let mut pre = dir.clone();
                    if !pre.ends_with('/') {
                        pre.push('/');
                    }
                    flow.input_buffer = pre;
                }
                self.import_flow = Some(flow);
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

    /// Handles all key events while the import wizard modal is active.
    fn handle_import_key(&mut self, key: KeyEvent) {
        // Take the flow out to avoid simultaneous self borrows.
        let Some(mut flow) = self.import_flow.take() else {
            return;
        };

        let step = flow.step.clone();
        match step {
            ImportFlowStep::FilePathInput => match key.code {
                KeyCode::Esc => {
                    // Cancel: leave import_flow as None (already taken).
                    return;
                }
                KeyCode::Enter => {
                    let raw = flow.input_buffer.trim().to_string();
                    let expanded = expand_tilde_str(&raw);
                    let path = std::path::PathBuf::from(&expanded);
                    if path.is_file() {
                        flow.file_path = Some(path.clone());
                        flow.modal_error = None;
                        let (toml_path, workspace_dir) = self.entity_toml_path();
                        let entity_cfg =
                            crate::config::load_entity_toml(&toml_path, &workspace_dir)
                                .unwrap_or_default();
                        // Update last_import_dir in entity toml.
                        if let Some(parent) = path.parent() {
                            let mut cfg = entity_cfg.clone();
                            cfg.last_import_dir = Some(parent.to_string_lossy().into_owned());
                            let _ =
                                crate::config::save_entity_toml(&toml_path, &workspace_dir, &cfg);
                        }
                        flow.available_banks = entity_cfg.bank_accounts;
                        if flow.available_banks.is_empty() {
                            // No configured banks: go straight to new bank setup.
                            flow.step = ImportFlowStep::NewBankName;
                            flow.is_new_bank = true;
                        } else {
                            flow.step = ImportFlowStep::BankSelection;
                        }
                        flow.selected_index = 0;
                        flow.input_buffer = String::new();
                    } else {
                        flow.modal_error =
                            Some("File not found. Check the path and try again.".to_string());
                    }
                }
                KeyCode::Backspace => {
                    flow.input_buffer.pop();
                    flow.modal_error = None;
                }
                KeyCode::Char(c) => {
                    flow.input_buffer.push(c);
                    flow.modal_error = None;
                }
                _ => {}
            },
            ImportFlowStep::BankSelection => match key.code {
                KeyCode::Esc => return,
                KeyCode::Up => {
                    flow.selected_index = flow.selected_index.saturating_sub(1);
                }
                KeyCode::Down => {
                    // +1 for the "New Bank Account" option at the bottom.
                    let max = flow.available_banks.len(); // index of "New" option
                    if flow.selected_index < max {
                        flow.selected_index += 1;
                    }
                }
                KeyCode::Enter => {
                    let new_idx = flow.available_banks.len();
                    if flow.selected_index == new_idx {
                        // "New Bank Account" selected.
                        flow.step = ImportFlowStep::NewBankName;
                        flow.is_new_bank = true;
                        flow.input_buffer = String::new();
                    } else {
                        // Known bank selected.
                        let cfg = flow.available_banks[flow.selected_index].clone();
                        flow.bank_config = Some(cfg);
                        flow.is_new_bank = false;
                        flow.step = ImportFlowStep::DuplicateWarning;
                    }
                }
                _ => {}
            },
            ImportFlowStep::NewBankName => match key.code {
                KeyCode::Esc => return,
                KeyCode::Enter => {
                    let name = flow.input_buffer.trim().to_string();
                    if !name.is_empty() {
                        flow.new_bank_name = Some(name);
                        flow.input_buffer = String::new();
                        flow.step = ImportFlowStep::NewBankDetection;
                        self.pending_bank_detection = true;
                    }
                }
                KeyCode::Backspace => {
                    flow.input_buffer.pop();
                }
                KeyCode::Char(c) => {
                    flow.input_buffer.push(c);
                }
                _ => {}
            },
            // NewBankDetection: UI shows "Initializing ↻" while event_loop calls the API.
            // Keys are consumed (user must wait for detection to complete).
            ImportFlowStep::NewBankDetection => {
                if key.code == KeyCode::Esc {
                    self.pending_bank_detection = false;
                    return;
                }
            }
            // Placeholder dispatch for subsequent steps (implemented in later tasks).
            _ => {
                if key.code == KeyCode::Esc {
                    return; // Cancel: leave import_flow as None.
                }
            }
        }

        // Restore the flow (it was taken above).
        self.import_flow = Some(flow);
    }
}

/// Extracts a JSON object from a string that may be wrapped in markdown code fences.
fn extract_json_block(s: &str) -> String {
    // Look for ```json ... ``` or ``` ... ``` fences.
    let stripped = if let Some(start) = s.find("```json") {
        let after = &s[start + 7..];
        after
            .find("```")
            .map(|end| after[..end].trim().to_string())
            .unwrap_or_else(|| s.to_string())
    } else if let Some(start) = s.find("```") {
        let after = &s[start + 3..];
        after
            .find("```")
            .map(|end| after[..end].trim().to_string())
            .unwrap_or_else(|| s.to_string())
    } else {
        s.to_string()
    };
    // Find first `{` to last `}`.
    if let Some(start) = stripped.find('{')
        && let Some(end) = stripped.rfind('}')
    {
        return stripped[start..=end].to_string();
    }
    stripped
}

/// Expands a leading `~` to the user's `$HOME` directory.
fn expand_tilde_str(s: &str) -> String {
    if s.starts_with("~/") || s == "~" {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        format!("{home}/{}", s.strip_prefix("~/").unwrap_or(""))
    } else {
        s.to_string()
    }
}

/// Renders the import wizard modal overlay.
fn render_import_modal(
    frame: &mut ratatui::Frame,
    area: Rect,
    flow: &crate::ai::csv_import::ImportFlowState,
) {
    match &flow.step {
        ImportFlowStep::FilePathInput => render_file_path_modal(frame, area, flow),
        ImportFlowStep::BankSelection => render_bank_selection_modal(frame, area, flow),
        ImportFlowStep::NewBankName => render_new_bank_name_modal(frame, area, flow),
        ImportFlowStep::NewBankDetection => {
            render_new_bank_detection_modal(frame, area, "Initializing \u{21BB}")
        }
        ImportFlowStep::Failed(msg) => render_new_bank_detection_modal(frame, area, msg),
        // Future steps render their own modals (implemented in later tasks).
        _ => {}
    }
}

/// Renders the file path input step of the import wizard.
fn render_file_path_modal(
    frame: &mut ratatui::Frame,
    area: Rect,
    flow: &crate::ai::csv_import::ImportFlowState,
) {
    use ratatui::{
        style::{Color, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Paragraph},
    };

    let modal = crate::widgets::centered_rect(70, 40, area);
    frame.render_widget(Clear, modal);

    let input_line = format!(" > {}", flow.input_buffer);
    let mut lines = vec![
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  Enter the full path to the CSV bank statement:",
            Style::default().fg(Color::Gray),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(input_line, Style::default().fg(Color::White))),
        Line::from(Span::raw("")),
    ];
    if let Some(ref err) = flow.modal_error {
        lines.push(Line::from(Span::styled(
            format!("  ⚠ {err}"),
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(Span::raw("")));
    }
    lines.push(Line::from(Span::styled(
        "  Enter: confirm   Esc: cancel",
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Import CSV Statement ")
                .style(Style::default().fg(Color::Cyan)),
        ),
        modal,
    );
}

/// Renders the bank selection step of the import wizard.
fn render_bank_selection_modal(
    frame: &mut ratatui::Frame,
    area: Rect,
    flow: &crate::ai::csv_import::ImportFlowState,
) {
    use ratatui::{
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Paragraph},
    };

    let modal = crate::widgets::centered_rect(70, 60, area);
    frame.render_widget(Clear, modal);

    let new_idx = flow.available_banks.len();
    let mut lines = vec![Line::from(Span::raw(""))];

    for (i, bank) in flow.available_banks.iter().enumerate() {
        let style = if i == flow.selected_index {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(
            format!("  {} — {}", bank.name, bank.linked_account),
            style,
        )));
    }

    // "New Bank Account" option.
    let new_style = if flow.selected_index == new_idx {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };
    lines.push(Line::from(Span::styled(
        "  \u{2795} New Bank Account",
        new_style,
    )));
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        "  \u{2191}/\u{2193}: navigate   Enter: select   Esc: cancel",
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Select Bank Account ")
                .style(Style::default().fg(Color::Cyan)),
        ),
        modal,
    );
}

/// Renders the new bank name input step.
fn render_new_bank_name_modal(
    frame: &mut ratatui::Frame,
    area: Rect,
    flow: &crate::ai::csv_import::ImportFlowState,
) {
    use ratatui::{
        style::{Color, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Paragraph},
    };

    let modal = crate::widgets::centered_rect(70, 30, area);
    frame.render_widget(Clear, modal);

    let input_line = format!(" > {}", flow.input_buffer);
    let lines = vec![
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  Bank account name:",
            Style::default().fg(Color::Gray),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(input_line, Style::default().fg(Color::White))),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  Enter: confirm   Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" New Bank Account ")
                .style(Style::default().fg(Color::Cyan)),
        ),
        modal,
    );
}

/// Renders a status-only modal (used for Initializing/Failed steps).
fn render_new_bank_detection_modal(frame: &mut ratatui::Frame, area: Rect, message: &str) {
    use ratatui::{
        style::{Color, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Paragraph},
    };

    let modal = crate::widgets::centered_rect(50, 20, area);
    frame.render_widget(Clear, modal);

    let lines = vec![
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Bank Format Detection ")
                .style(Style::default().fg(Color::Cyan)),
        ),
        modal,
    );
}

/// Renders a centered help overlay showing global and tab-specific hotkeys.
/// When `panel_visible` is true, also renders the Chat Panel section.
fn render_help_overlay(
    frame: &mut ratatui::Frame,
    area: Rect,
    tab_hotkeys: Vec<(&'static str, &'static str)>,
    panel_visible: bool,
) {
    let global_hotkeys: &[(&str, &str)] = &[
        ("1–9", "Switch to tab"),
        ("Ctrl+← / Ctrl+→", "Previous / next tab"),
        ("Ctrl+K", "AI Accountant panel"),
        ("f", "Fiscal period management"),
        ("Ctrl+H", "Open user guide"),
        ("q", "Quit"),
        ("?", "Show / hide this help"),
    ];

    let chat_hotkeys: &[(&str, &str)] = &[
        ("Ctrl+K / Esc", "Open / close panel"),
        ("Tab", "Switch focus (panel ↔ tab)"),
        ("/clear", "Reset conversation"),
        ("/context", "Refresh tab data"),
        ("/compact", "Compress history"),
        ("/persona", "View / change persona"),
        ("/match", "Re-match selected draft"),
    ];

    // Calculate popup size: width = 60, height = rows + borders + section headers.
    let chat_section_rows = if panel_visible {
        chat_hotkeys.len() + 2 // header + blank line before
    } else {
        0
    };
    let row_count = global_hotkeys.len() + tab_hotkeys.len() + 3 + chat_section_rows; // +3: two headers + blank line
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

    if panel_visible {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Chat Panel",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        for (key, desc) in chat_hotkeys {
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
