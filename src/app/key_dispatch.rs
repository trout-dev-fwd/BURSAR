use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{
    ai::csv_import::ImportFlowState,
    config::WorkspaceConfig,
    db::EntityDb,
    inter_entity::{InterEntityMode, form::InterEntityFormAction, write_protocol},
    tabs::{TabAction, TabId},
    types::{FocusTarget, MatchSource},
    widgets::{
        FeedbackAction, FeedbackModal, FeedbackType, FiscalModal, FiscalModalAction, UserGuide,
        UserGuideAction,
        chat_panel::ChatAction,
        feedback_modal::{build_issue_url, open_in_browser},
    },
};

use super::{App, AppMode};

impl App {
    pub(super) fn handle_key(&mut self, key: KeyEvent) {
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

        // Help overlay: Esc or ? dismisses it; b/f open feedback modal; all other keys consumed.
        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => {
                    self.show_help = false;
                    self.inter_entity_help = false;
                }
                KeyCode::Char('b') => {
                    self.show_help = false;
                    self.inter_entity_help = false;
                    self.feedback_modal = Some(FeedbackModal::new(FeedbackType::Bug));
                }
                KeyCode::Char('f') => {
                    self.show_help = false;
                    self.inter_entity_help = false;
                    self.feedback_modal = Some(FeedbackModal::new(FeedbackType::Feature));
                }
                _ => {}
            }
            return;
        }

        // Feedback modal: handles all keys when open.
        if let Some(ref mut modal) = self.feedback_modal {
            match modal.handle_key(key) {
                FeedbackAction::Submit(feedback_type, description) => {
                    let audit_entries: Vec<String> = self
                        .entity
                        .db
                        .audit()
                        .list_recent(5)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|e| {
                            format!("{} | {} | {}", e.created_at, e.action_type, e.description)
                        })
                        .collect();
                    let url = build_issue_url(
                        &feedback_type,
                        &description,
                        Some(&self.entity.name),
                        &audit_entries,
                    );
                    match open_in_browser(&url) {
                        Ok(()) => {
                            let msg = match feedback_type {
                                FeedbackType::Bug => "Bug report opened in browser",
                                FeedbackType::Feature => "Feature request opened in browser",
                            };
                            self.status_bar.set_message(msg.to_string());
                        }
                        Err(e) => {
                            self.status_bar.set_error(format!(
                                "{e}. File manually at https://github.com/trout-dev-fwd/bursar/issues"
                            ));
                        }
                    }
                    self.feedback_modal = None;
                }
                FeedbackAction::Cancel => {
                    self.feedback_modal = None;
                }
                FeedbackAction::None => {}
            }
            return;
        }

        // File picker: shown at the start of the import flow.
        if self.file_picker.is_some() {
            self.handle_file_picker_key(key);
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

        // Inter-entity mode: all input goes to the form (except ? and Ctrl+K).
        if matches!(self.mode, AppMode::InterEntity(_)) {
            if key.code == KeyCode::Char('?') {
                self.show_help = true;
                self.inter_entity_help = true;
                return;
            }
            if key.code == KeyCode::Char('k') && key.modifiers.contains(KeyModifiers::CONTROL) {
                self.chat_panel.toggle_visible();
                if self.chat_panel.is_visible() {
                    self.focus = FocusTarget::ChatPanel;
                } else {
                    self.focus = FocusTarget::MainTab;
                }
                return;
            }
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
            // Tab switching: 0–9 keys select tabs by number.
            KeyCode::Char(c @ '0'..='9') if key.modifiers == KeyModifiers::NONE => {
                let idx = (c as usize) - ('0' as usize);
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

    pub(super) fn process_action(&mut self, action: TabAction) {
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
                let (toml_path, workspace_dir) = self.entity_toml_path();
                let start_dir = crate::config::load_entity_toml(&toml_path, &workspace_dir)
                    .ok()
                    .and_then(|cfg| cfg.last_import_dir)
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| {
                        std::env::var("HOME")
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|_| std::path::PathBuf::from("."))
                    });
                self.file_picker = Some(crate::widgets::FilePicker::new(start_dir));
            }
            TabAction::StartRematch => {
                // Collect incomplete import drafts, parse their import_refs, and
                // start the re-match flow at Pass 2 (skipping local match).
                let incomplete = self
                    .entity
                    .db
                    .journals()
                    .get_incomplete_imports()
                    .unwrap_or_default();
                if incomplete.is_empty() {
                    self.status_bar
                        .set_message("No incomplete imports to re-match.".to_string());
                } else {
                    use crate::ai::csv_import::parse_import_ref;

                    // Look up bank config from entity toml using the bank name in
                    // the first import_ref. All incomplete drafts from one import
                    // share the same bank, so we only need to match once.
                    let (toml_path, workspace_dir) = self.entity_toml_path();
                    let entity_cfg = crate::config::load_entity_toml(&toml_path, &workspace_dir)
                        .unwrap_or_default();

                    // Extract bank name from first import_ref to find the config.
                    let first_bank_name = incomplete
                        .iter()
                        .filter_map(|je| je.import_ref.as_deref())
                        .find_map(|r| r.split('|').next().map(|s| s.to_string()));

                    let bank_config = first_bank_name.as_deref().and_then(|name| {
                        entity_cfg
                            .bank_accounts
                            .iter()
                            .find(|b| b.name == name)
                            .cloned()
                    });

                    if bank_config.is_none() {
                        let bank_display = first_bank_name.as_deref().unwrap_or("unknown");
                        self.status_bar.set_error(format!(
                            "Bank config for '{}' not found in entity toml. Cannot re-match.",
                            bank_display
                        ));
                    } else {
                        let matches: Vec<crate::ai::ImportMatch> = incomplete
                            .into_iter()
                            .filter_map(|je| {
                                let import_ref = je.import_ref?;
                                let txn = parse_import_ref(&import_ref)?;
                                Some(crate::ai::ImportMatch {
                                    transaction: txn,
                                    matched_account_id: None,
                                    matched_account_display: None,
                                    match_source: MatchSource::Unmatched,
                                    confidence: None,
                                    reasoning: None,
                                    rejected: false,
                                    existing_je_id: Some(je.id),
                                    transfer_match: None,
                                })
                            })
                            .collect();
                        let mut flow = ImportFlowState::new();
                        flow.matches = matches;
                        flow.is_rematch = true;
                        flow.bank_config = bank_config;
                        flow.step = crate::ai::csv_import::ImportFlowStep::Pass2AiMatching;
                        self.import_flow = Some(flow);
                        self.pending_pass2 = true;
                    }
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
                    self.status_bar
                        .set_error("Need 2+ entities for inter-entity mode.".to_owned());
                } else {
                    self.mode = AppMode::SecondaryEntityPicker {
                        selected: 0,
                        candidates,
                    };
                }
            }
            TabAction::StartTaxIngestion => {
                self.pending_tax_ingestion = true;
                self.status_bar
                    .set_message("Preparing to ingest IRS publications...".to_string());
            }
            TabAction::RunAiBatchReview => {
                self.pending_tax_batch_review = true;
                self.status_bar
                    .set_message("Starting AI batch review...".to_string());
            }
            TabAction::SaveTaxFormConfig(forms) => {
                let (toml_path, workspace_dir) = self.entity_toml_path();
                let mut entity_cfg =
                    crate::config::load_entity_toml(&toml_path, &workspace_dir).unwrap_or_default();
                entity_cfg.tax = Some(crate::config::TaxConfig {
                    enabled_forms: Some(forms),
                });
                match crate::config::save_entity_toml(&toml_path, &workspace_dir, &entity_cfg) {
                    Ok(()) => self
                        .status_bar
                        .set_message("Tax form config saved.".to_string()),
                    Err(e) => self
                        .status_bar
                        .set_error(format!("Failed to save tax config: {e}")),
                }
            }
            TabAction::Quit => {
                self.should_quit = true;
            }
        }
    }
}

/// Renders a centered help overlay showing global and tab-specific hotkeys.
/// When `panel_visible` is true, also renders the Chat Panel section.
pub(super) fn render_help_overlay(
    frame: &mut ratatui::Frame,
    area: Rect,
    tab_hotkeys: Vec<(&'static str, &'static str)>,
    panel_visible: bool,
) {
    let global_hotkeys: &[(&str, &str)] = &[
        ("0–9", "Switch to tab"),
        ("Ctrl+← / Ctrl+→", "Previous / next tab"),
        ("Ctrl+K", "AI Accountant panel"),
        ("f", "Fiscal period management"),
        ("Ctrl+H", "Open user guide (& form guide)"),
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

    let feedback_hotkeys: &[(&str, &str)] = &[("b", "Report bug"), ("f", "Request feature")];

    // Calculate popup size: width = 60, height = rows + borders + section headers.
    let chat_section_rows = if panel_visible {
        chat_hotkeys.len() + 2 // header + blank line before
    } else {
        0
    };
    // +3: two headers (Global, Tab-specific) + blank line between; +3 for Feedback section
    let row_count = global_hotkeys.len()
        + tab_hotkeys.len()
        + 3
        + chat_section_rows
        + feedback_hotkeys.len()
        + 2; // Feedback header + blank line before
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

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Feedback",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    for (key, desc) in feedback_hotkeys {
        lines.push(Line::from(vec![
            Span::styled(format!("  {key:<16}"), Style::default().fg(Color::Cyan)),
            Span::raw(*desc),
        ]));
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

pub(super) fn render_secondary_entity_picker(
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crossterm::event::KeyModifiers;

    use super::*;
    use crate::{
        app::{App, EntityContext},
        config::WorkspaceConfig,
        db::EntityDb,
        widgets::FeedbackType,
    };

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn test_app() -> App {
        let db = EntityDb::open_in_memory().expect("in-memory db");
        let ctx = EntityContext::new(
            db,
            "Test Entity".to_string(),
            std::path::PathBuf::from("/tmp"),
        );
        App::new(ctx, WorkspaceConfig::default())
    }

    #[test]
    fn b_key_in_help_overlay_opens_bug_feedback_modal() {
        let mut app = test_app();
        app.show_help = true;
        app.handle_key(key(KeyCode::Char('b')));
        assert!(!app.show_help, "help overlay should be closed");
        assert!(
            app.feedback_modal.is_some(),
            "feedback modal should be open"
        );
        if let Some(ref modal) = app.feedback_modal {
            assert_eq!(modal.feedback_type, FeedbackType::Bug);
        }
    }

    #[test]
    fn f_key_in_help_overlay_opens_feature_feedback_modal() {
        let mut app = test_app();
        app.show_help = true;
        app.handle_key(key(KeyCode::Char('f')));
        assert!(!app.show_help, "help overlay should be closed");
        assert!(
            app.feedback_modal.is_some(),
            "feedback modal should be open"
        );
        if let Some(ref modal) = app.feedback_modal {
            assert_eq!(modal.feedback_type, FeedbackType::Feature);
        }
    }

    #[test]
    fn b_key_when_help_not_open_does_not_open_feedback_modal() {
        let mut app = test_app();
        // show_help is false by default
        app.handle_key(key(KeyCode::Char('b')));
        assert!(
            app.feedback_modal.is_none(),
            "feedback modal should NOT open when help overlay is closed"
        );
    }

    #[test]
    fn esc_in_feedback_modal_closes_it() {
        let mut app = test_app();
        app.show_help = true;
        app.handle_key(key(KeyCode::Char('b')));
        assert!(app.feedback_modal.is_some());
        app.handle_key(key(KeyCode::Esc));
        assert!(
            app.feedback_modal.is_none(),
            "Esc should close the feedback modal"
        );
    }
}
