use ratatui::Terminal;

use crate::{
    ai::{
        AiError, AiResponse, ApiContent, ApiMessage, ApiRole, RoundResult, ToolResult,
        client::AiClient,
        context::read_context,
        tools::{fulfill_tool_call, tax_tool_definition, tool_definitions},
    },
    config::{load_entity_toml, load_secrets, save_entity_toml},
    types::AiRequestState,
    widgets::chat_panel::SlashCommand,
};

use super::App;

impl App {
    /// Initialise `self.ai_client` from secrets on first use.
    ///
    /// Returns `Ok(())` when the client is ready, or `Err(message)` if the
    /// API key could not be loaded.
    pub(super) fn ensure_ai_client(&mut self) -> Result<(), String> {
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
    pub(super) fn handle_ai_request<B: ratatui::backend::Backend>(
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

        // ── Tax tab: extract last user message for keyword-based reference lookup ──
        let last_user_message: String = messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, ApiRole::User))
            .and_then(|m| {
                if let ApiContent::Text(t) = &m.content {
                    Some(t.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        // Tax tab is at index 9. When active, append IRS reference chunks and
        // selected JE context to the system prompt, and add get_tax_tag tool.
        const TAX_TAB_INDEX: usize = 9;
        let on_tax_tab = self.active_tab == TAX_TAB_INDEX;
        let tax_context_block: Option<String> = if on_tax_tab {
            self.entity.tabs[TAX_TAB_INDEX]
                .build_tax_ai_context(&self.entity.db, &last_user_message)
        } else {
            None
        };

        // ── Build system prompt ───────────────────────────────────────────────
        let persona = self.chat_panel.current_persona.clone();
        let entity_name = self.entity.name.clone();
        let base_prompt = AiClient::build_system_prompt(&persona, &entity_name, &context);
        let system_prompt = if let Some(tax_block) = tax_context_block {
            format!("{base_prompt}\n\n{tax_block}")
        } else {
            base_prompt
        };

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
        let mut tools = tool_definitions();
        if on_tax_tab {
            tools.push(tax_tool_definition());
        }
        let Some(client) = self.ai_client.take() else {
            self.ai_state = AiRequestState::Idle;
            self.status_bar.set_ai_status(None);
            self.status_bar
                .set_error("AI client not available.".to_string());
            return;
        };
        let max_depth: usize = 5;
        let mut msgs = messages;
        let mut accumulated_text: Option<String> = None;
        let mut result: Result<AiResponse, AiError> = Err(AiError::MaxToolDepth);

        for round in 0..=max_depth {
            match client.send_single_round(
                &system_prompt,
                &msgs,
                &tools,
                accumulated_text.take(),
                true,
            ) {
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

    /// Runs a single AI request through the tool-use loop (no chat panel routing).
    ///
    /// Returns the final text response, or `None` on error/max-depth.
    /// Used for batch AI matching in Pass 2.
    pub(super) fn run_ai_batch_request<B: ratatui::backend::Backend>(
        &mut self,
        system: &str,
        messages: Vec<ApiMessage>,
        terminal: &mut Terminal<B>,
        use_cache: bool,
    ) -> Option<String> {
        let tools = tool_definitions();
        let client = self.ai_client.take()?;
        let max_depth: usize = 5;
        let mut msgs = messages;
        let mut accumulated_text: Option<String> = None;
        let mut result_text: Option<String> = None;

        for _round in 0..=max_depth {
            match client.send_single_round(
                system,
                &msgs,
                &tools,
                accumulated_text.take(),
                use_cache,
            ) {
                Ok(RoundResult::Done(AiResponse::Text { content, .. })) => {
                    result_text = Some(content);
                    break;
                }
                Ok(RoundResult::NeedsToolCall {
                    tool_calls,
                    messages: updated_msgs,
                    accumulated_text: acc,
                }) => {
                    let tool_results: Vec<ToolResult> = tool_calls
                        .iter()
                        .map(|tc| {
                            let content = fulfill_tool_call(tc, &self.entity.db)
                                .unwrap_or_else(|e| format!("Error: {e}"));
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
                    let _ = terminal.draw(|frame| self.render_frame(frame));
                }
                Ok(RoundResult::Done(_)) | Err(_) => break,
            }
        }
        self.ai_client = Some(client);
        result_text
    }

    /// Executes a slash command entered in the chat panel.
    pub(super) fn execute_slash_command<B: ratatui::backend::Backend>(
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
                let system = "";
                let compaction_messages = vec![ApiMessage {
                    role: ApiRole::User,
                    content: ApiContent::Text(format!(
                        "Summarize this conversation in one paragraph. Preserve all account \
                         numbers, amounts, dates, and decisions:\n\n{history}"
                    )),
                }];
                self.status_bar
                    .set_ai_status(Some("Calling Accountant ☏".to_string()));
                let _ = terminal.draw(|frame| self.render_frame(frame));
                let result = {
                    let Some(client) = self.ai_client.as_ref() else {
                        self.status_bar.set_ai_status(None);
                        self.status_bar
                            .set_error("AI client not available.".to_string());
                        return;
                    };
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
                // Get selected draft's import_ref from the active JE tab.
                let import_ref = self.entity.tabs[self.active_tab].selected_draft_import_ref();
                let Some(import_ref) = import_ref else {
                    self.chat_panel.add_system_note(
                        "Select an incomplete Draft entry with an import reference in \
                         the Journal Entries tab first.",
                    );
                    return;
                };
                use crate::ai::csv_import::parse_import_ref;
                let Some(txn) = parse_import_ref(&import_ref) else {
                    self.chat_panel
                        .add_system_note("Could not parse import_ref for this entry.");
                    return;
                };
                if let Err(msg) = self.ensure_ai_client() {
                    self.status_bar.set_error(msg);
                    return;
                }
                let system = self.chat_panel.system_prompt.clone();
                let prompt = format!(
                    "What account should this transaction map to? {} | {} | {}. \
                     Give account_number, confidence, and one-sentence reasoning.",
                    txn.date, txn.description, txn.amount
                );
                let messages = vec![ApiMessage {
                    role: ApiRole::User,
                    content: ApiContent::Text(prompt),
                }];
                self.status_bar
                    .set_ai_status(Some("Calling Accountant \u{260F}".to_string()));
                let _ = terminal.draw(|frame| self.render_frame(frame));
                let result = self.run_ai_batch_request(&system, messages, terminal, false);
                self.status_bar.set_ai_status(None);
                match result {
                    Some(response) => {
                        self.chat_panel.add_system_note(&format!(
                            "Match suggestion for \"{}\": {}",
                            txn.description.chars().take(40).collect::<String>(),
                            response
                        ));
                    }
                    None => {
                        self.chat_panel
                            .add_system_note("AI match request failed. Try again.");
                    }
                }
            }
            SlashCommand::Unknown(name) => {
                self.chat_panel.add_system_note(&format!(
                    "Unknown command '/{name}'. Available: /clear, /context, /compact, /persona, /match"
                ));
            }
        }
    }

    /// Returns the context directory, falling back to `~/.config/bookkeeper/context`.
    pub(super) fn context_dir(&self) -> String {
        self.config.context_dir.clone().unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            format!("{home}/.config/bookkeeper/context")
        })
    }

    /// Returns `(config_path_str, workspace_dir)` for the active entity's TOML file.
    /// The path is derived from the entity's db_path if no explicit config_path is set.
    pub(super) fn entity_toml_path(&self) -> (String, std::path::PathBuf) {
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
}
