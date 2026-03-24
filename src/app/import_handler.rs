use super::*;
use crate::ai::{ApiContent, ApiRole};
use crate::widgets::FilePickerAction;
use crate::widgets::chat_panel::ChatAction;

impl App {
    /// Parses the CSV, runs duplicate detection, and advances to the appropriate step.
    ///
    /// Mutates `flow` in place. On CSV parse error, sets step to Failed.
    /// If duplicates found → DuplicateWarning. If none → Pass1Matching.
    pub(super) fn enter_duplicate_check(flow: &mut ImportFlowState, db: &crate::db::EntityDb) {
        use crate::ai::csv_import::{check_duplicates, parse_csv};

        let (file_path, bank_config) = match (&flow.file_path, &flow.bank_config) {
            (Some(p), Some(c)) => (p.clone(), c.clone()),
            _ => {
                flow.step = ImportFlowStep::Failed("Missing file path or bank config".to_string());
                return;
            }
        };

        // Parse the full CSV.
        match parse_csv(&file_path, &bank_config) {
            Err(e) => {
                flow.step = ImportFlowStep::Failed(format!("CSV parse error: {e}"));
            }
            Ok((transactions, parse_warnings)) => {
                flow.warnings = parse_warnings;
                // Get recent import refs for duplicate detection.
                let existing_refs = db.journals().get_recent_import_refs(90).unwrap_or_default();
                let (unique, duplicates) = check_duplicates(&transactions, &existing_refs);
                flow.duplicates = duplicates.clone();
                if !duplicates.is_empty() {
                    // Store all transactions and show warning.
                    flow.transactions = transactions;
                    flow.step = ImportFlowStep::DuplicateWarning;
                } else {
                    // No duplicates: skip directly to matching.
                    flow.transactions = unique;
                    flow.step = ImportFlowStep::Pass1Matching;
                }
            }
        }
    }

    /// Runs bank format detection: reads first 4 CSV lines, sends to Claude, parses response.
    /// Updates `import_flow` with the detected config or moves to Failed step on error.
    pub(super) fn run_bank_detection<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) {
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

        let system = "Respond ONLY with valid JSON.";
        let prompt = format!(
            "Analyze this bank CSV from \"{bank_name}\". Return JSON with: date_column, \
             date_format (chrono: %m/%d/%Y etc), description_column, amount_column (or null), \
             debit_column (or null), credit_column (or null), debit_is_negative (bool).\n\n\
             {csv_sample}"
        );

        let messages = vec![ApiMessage {
            role: ApiRole::User,
            content: ApiContent::Text(prompt),
        }];

        let result = {
            let Some(client) = self.ai_client.as_ref() else {
                self.status_bar
                    .set_error("AI client not available.".to_string());
                if let Some(ref mut f) = self.import_flow {
                    f.step = ImportFlowStep::Failed("AI client not available".to_string());
                }
                return;
            };
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
                            f.confirmation_cursor = 0;
                            f.confirmation_editing = false;
                            f.confirmation_edit_buffer.clear();
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

    /// Runs Pass 1 local matching against `import_mappings`.
    ///
    /// Called from the event loop after `pending_pass1` is set.
    /// Advances to `Pass2AiMatching` if unmatched exist and an API key is available,
    /// otherwise to `ReviewScreen`.
    pub(super) fn run_pass1_step<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) {
        // Extract the data we need before rendering (avoids holding a borrow across draw).
        let (bank_name, transactions) = {
            let Some(ref flow) = self.import_flow else {
                return;
            };
            if flow.step != ImportFlowStep::Pass1Matching {
                return;
            }
            let bank_name = match flow.bank_config.as_ref().map(|c| c.name.clone()) {
                Some(n) => n,
                None => {
                    if let Some(ref mut f) = self.import_flow {
                        f.step = ImportFlowStep::Failed("No bank config set".to_string());
                    }
                    return;
                }
            };
            (bank_name, flow.transactions.clone())
        };

        // Force render so user sees progress indicator before matching begins.
        let _ = terminal.draw(|frame| self.render_frame(frame));

        let matches = crate::ai::csv_import::run_pass1(&transactions, &bank_name, &self.entity.db);

        let has_unmatched = matches.iter().any(|m| {
            m.matched_account_id.is_none()
                && !m.rejected
                && m.match_source != crate::types::MatchSource::TransferMatch
        });

        // Determine next step.
        let next_step = if has_unmatched && self.ensure_ai_client().is_ok() {
            ImportFlowStep::Pass2AiMatching
        } else {
            ImportFlowStep::ReviewScreen
        };

        if let Some(ref mut f) = self.import_flow {
            // Populate transfer_matches before storing matches (so we can borrow both).
            let transfer_matches: Vec<crate::ai::csv_import::TransferMatchRow> = matches
                .iter()
                .filter(|m| {
                    m.match_source == crate::types::MatchSource::TransferMatch
                        && m.transfer_match.is_some()
                })
                .filter_map(|m| {
                    let tm = m.transfer_match.as_ref()?;
                    Some(crate::ai::csv_import::TransferMatchRow {
                        date: m.transaction.date,
                        amount: m.transaction.amount,
                        description: m.transaction.description.clone(),
                        import_ref: m.transaction.import_ref.clone(),
                        matched_je_id: tm.je_id,
                        matched_je_number: tm.je_number.clone(),
                        matched_date: tm.entry_date,
                        matched_amount: tm.amount,
                        matched_bank: tm.bank_name.clone(),
                        confirmed: true,
                    })
                })
                .collect();
            f.matches = matches;
            f.transfer_matches = transfer_matches;
            f.step = next_step.clone();
            if next_step == ImportFlowStep::ReviewScreen {
                f.selected_index = 0;
                f.scroll_offset = 0;
            }
        }
        if next_step == ImportFlowStep::Pass2AiMatching {
            self.pending_pass2 = true;
        }
    }

    /// Runs Pass 2 AI matching: batches unmatched transactions through Claude with tool use.
    ///
    /// Called from the event loop after `pending_pass2` is set.
    /// Advances to `Pass3Clarification` if any Low-confidence matches, else `ReviewScreen`.
    pub(super) fn run_pass2_step<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) {
        use crate::types::{MatchConfidence, MatchSource};

        // Extract needed data before rendering (avoids holding borrow across draw).
        let (bank_name, accounts, unmatched_indices) = {
            let Some(ref flow) = self.import_flow else {
                return;
            };
            if flow.step != ImportFlowStep::Pass2AiMatching {
                return;
            }
            let bank_name = flow
                .bank_config
                .as_ref()
                .map(|c| c.name.clone())
                .unwrap_or_default();
            let accounts = self.entity.db.accounts().list_all().unwrap_or_else(|e| {
                tracing::warn!("Failed to load accounts: {e}");
                Vec::new()
            });
            let unmatched_indices: Vec<usize> = flow
                .matches
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    m.matched_account_id.is_none()
                        && !m.rejected
                        && m.match_source != MatchSource::TransferMatch
                })
                .map(|(i, _)| i)
                .collect();
            (bank_name, accounts, unmatched_indices)
        };

        // Auto-open chat panel if not already visible.
        if !self.chat_panel.is_visible() {
            self.chat_panel.toggle_visible();
        }
        // Keep focus on import, not chat panel.
        self.focus = FocusTarget::MainTab;

        // Ensure AI client is initialized.
        if let Err(msg) = self.ensure_ai_client() {
            if let Some(ref mut f) = self.import_flow {
                f.step = ImportFlowStep::ReviewScreen;
            }
            self.chat_panel
                .add_system_note(&format!("AI matching skipped: {msg}"));
            return;
        }

        let total = unmatched_indices.len();
        if total == 0 {
            if let Some(ref mut f) = self.import_flow {
                f.step = ImportFlowStep::ReviewScreen;
            }
            return;
        }

        self.chat_panel
            .add_system_note(&format!("Matching {total} transactions with AI..."));

        let batches: Vec<Vec<usize>> = unmatched_indices.chunks(25).map(|c| c.to_vec()).collect();
        let mut completed = 0usize;

        let system = "Expert accountant. Use tools to look up accounts. \
            Respond ONLY as JSON array, one object per transaction in order: \
            {\"account_number\": string|null, \"confidence\": \"high\"|\"medium\"|\"low\", \
            \"reasoning\": \"one sentence\"}";

        for batch in &batches {
            // Build transaction list for this batch.
            let transactions_text = {
                let Some(ref flow) = self.import_flow else {
                    break;
                };
                batch
                    .iter()
                    .enumerate()
                    .map(|(i, &idx)| {
                        let txn = &flow.matches[idx].transaction;
                        format!(
                            "{}. {} | {} | {}",
                            i + 1,
                            txn.date,
                            txn.description,
                            txn.amount
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };

            let prompt =
                format!("Match to chart of accounts. Bank: \"{bank_name}\"\n{transactions_text}");
            let messages = vec![ApiMessage {
                role: ApiRole::User,
                content: ApiContent::Text(prompt),
            }];

            // Force render before the blocking call.
            let _ = terminal.draw(|frame| self.render_frame(frame));

            let result = self.run_ai_batch_request(system, messages, terminal, true);

            completed += batch.len();
            self.chat_panel
                .add_system_note(&format!("Matching transactions... {completed}/{total}"));

            // Parse JSON response and update matches.
            if let Some(raw) = result {
                let json_str = extract_json_block(&raw);
                if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&json_str) {
                    let Some(ref mut flow) = self.import_flow else {
                        break;
                    };
                    for (i, &idx) in batch.iter().enumerate() {
                        let Some(obj) = arr.get(i) else { continue };
                        let confidence_str = obj["confidence"].as_str().unwrap_or("low");
                        let confidence = match confidence_str {
                            "high" => MatchConfidence::High,
                            "medium" => MatchConfidence::Medium,
                            _ => MatchConfidence::Low,
                        };
                        let reasoning = obj["reasoning"].as_str().map(|s| s.to_string());
                        let acct_num = obj["account_number"].as_str();
                        let matched = acct_num.and_then(|num| {
                            accounts
                                .iter()
                                .find(|a| a.number == num)
                                .map(|a| (a.id, format!("{} - {}", a.number, a.name)))
                        });
                        if let Some((account_id, display)) = matched {
                            flow.matches[idx].matched_account_id = Some(account_id);
                            flow.matches[idx].matched_account_display = Some(display);
                            flow.matches[idx].match_source = MatchSource::Ai;
                            flow.matches[idx].confidence = Some(confidence);
                            flow.matches[idx].reasoning = reasoning;
                        }
                    }
                } else {
                    tracing::warn!("Pass2: failed to parse AI batch response as JSON array");
                }
            }
        }

        // Determine next step.
        let has_low = {
            let Some(ref flow) = self.import_flow else {
                return;
            };
            flow.matches.iter().any(|m| {
                matches!(m.confidence, Some(MatchConfidence::Low)) && m.matched_account_id.is_some()
            })
        };
        if let Some(ref mut f) = self.import_flow {
            if has_low {
                // Populate clarification queue with indices of Low-confidence matches.
                f.clarification_queue = f
                    .matches
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| {
                        matches!(m.confidence, Some(MatchConfidence::Low))
                            && m.matched_account_id.is_some()
                    })
                    .map(|(i, _)| i)
                    .collect();
                f.clarification_prompted = false;
                f.step = ImportFlowStep::Pass3Clarification;
            } else {
                f.step = ImportFlowStep::ReviewScreen;
                f.selected_index = 0;
                f.scroll_offset = 0;
            }
        }
    }

    /// Batch-creates draft journal entries from the approved import matches.
    ///
    /// Runs within a SQLite savepoint for atomicity. On success: logs audit entries,
    /// saves learned mappings, sets step to Complete, and refreshes all tabs.
    /// On failure: rolls back and returns to ReviewScreen.
    pub(super) fn run_draft_creation_step<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) {
        use crate::ai::csv_import::determine_debit_credit;
        use crate::db::journal_repo::{NewJournalEntry, NewJournalEntryLine};
        use crate::types::{ImportMatchSource, ImportMatchType, MatchSource};

        // Extract data from flow before the terminal.draw() call (to release borrow).
        let (bank_name, bank_account_number, matches_snapshot, is_rematch) = {
            let Some(ref flow) = self.import_flow else {
                return;
            };
            if flow.step != ImportFlowStep::Creating {
                return;
            }
            let bank_name = flow
                .bank_config
                .as_ref()
                .map(|c| c.name.clone())
                .unwrap_or_default();
            let bank_account_number = flow
                .bank_config
                .as_ref()
                .map(|c| c.linked_account.clone())
                .unwrap_or_default();
            let matches_snapshot = flow.matches.clone();
            let is_rematch = flow.is_rematch;
            (bank_name, bank_account_number, matches_snapshot, is_rematch)
        };
        let entity_name = self.entity.name.clone();

        // Force render so user sees "Creating" state.
        let _ = terminal.draw(|frame| self.render_frame(frame));
        let all_accounts = self.entity.db.accounts().list_all().unwrap_or_else(|e| {
            tracing::warn!("Failed to load accounts: {e}");
            Vec::new()
        });
        let bank_account = all_accounts
            .iter()
            .find(|a| a.number == bank_account_number)
            .cloned();

        // Begin savepoint for atomicity.
        let sp_result = self
            .entity
            .db
            .conn()
            .execute("SAVEPOINT import_batch_sp", []);
        if let Err(e) = sp_result {
            self.status_bar
                .set_error(format!("Failed to begin transaction: {e}"));
            if let Some(ref mut f) = self.import_flow {
                f.step = ImportFlowStep::ReviewScreen;
            }
            return;
        }

        let mut created_count = 0usize;
        let mut ai_matched_count = 0usize;
        let mut manual_count = 0usize;
        let mut batch_error: Option<String> = None;
        let mut learned_mappings: Vec<(String, crate::types::AccountId, String, String)> =
            Vec::new(); // (desc, account_id, account_number, account_name)

        'batch: for m in &matches_snapshot {
            if m.rejected || m.match_source == MatchSource::TransferMatch {
                // Rejected by user, or confirmed as transfer (handled in Phase 4 wiring).
                continue;
            }

            // Find fiscal period for this transaction date.
            let fiscal_period = match self
                .entity
                .db
                .fiscal()
                .get_period_for_date(m.transaction.date)
            {
                Ok(fp) => fp,
                Err(e) => {
                    batch_error = Some(format!("No fiscal period for {}: {e}", m.transaction.date));
                    break 'batch;
                }
            };

            let memo_str = format!(
                "Import: {}",
                m.transaction
                    .description
                    .chars()
                    .take(200)
                    .collect::<String>()
            );

            // Determine debit/credit using bank account type.
            let bank_acct_type = bank_account
                .as_ref()
                .map(|a| a.account_type)
                .unwrap_or(crate::types::AccountType::Asset);
            let (bank_debit, bank_credit, _) =
                determine_debit_credit(m.transaction.amount, bank_acct_type);
            let contra_debit = bank_credit; // contra side is opposite
            let contra_credit = bank_debit;

            let bank_line = bank_account.as_ref().map(|a| NewJournalEntryLine {
                account_id: a.id,
                debit_amount: bank_debit,
                credit_amount: bank_credit,
                line_memo: None,
                sort_order: 0,
            });

            let mut lines = Vec::new();
            if let Some(bl) = bank_line {
                lines.push(bl);
            }
            if let Some(account_id) = m.matched_account_id {
                lines.push(NewJournalEntryLine {
                    account_id,
                    debit_amount: contra_debit,
                    credit_amount: contra_credit,
                    line_memo: None,
                    sort_order: 1,
                });

                // Track AI-suggested mappings to save later.
                if matches!(m.match_source, MatchSource::Ai) {
                    ai_matched_count += 1;
                    if let Some(display) = &m.matched_account_display {
                        let parts: Vec<&str> = display.splitn(2, " - ").collect();
                        let num = parts.first().copied().unwrap_or("");
                        let name = parts.get(1).copied().unwrap_or(display.as_str());
                        learned_mappings.push((
                            m.transaction.description.clone(),
                            account_id,
                            num.to_string(),
                            name.to_string(),
                        ));
                    }
                } else if matches!(m.match_source, MatchSource::UserConfirmed) {
                    manual_count += 1;
                }
            }

            let entry = NewJournalEntry {
                entry_date: m.transaction.date,
                memo: Some(memo_str),
                fiscal_period_id: fiscal_period.id,
                reversal_of_je_id: None,
                lines,
            };

            let op_result = if is_rematch {
                if let Some(je_id) = m.existing_je_id {
                    self.entity.db.journals().update_draft(
                        je_id,
                        entry.entry_date,
                        entry.memo,
                        entry.fiscal_period_id,
                        &entry.lines,
                    )
                } else {
                    self.entity
                        .db
                        .journals()
                        .create_draft_with_import_ref(&entry, Some(&m.transaction.import_ref))
                        .map(|_| ())
                }
            } else {
                self.entity
                    .db
                    .journals()
                    .create_draft_with_import_ref(&entry, Some(&m.transaction.import_ref))
                    .map(|_| ())
            };
            match op_result {
                Ok(()) => created_count += 1,
                Err(e) => {
                    batch_error = Some(format!("Failed to create/update draft: {e}"));
                    break 'batch;
                }
            }
        }

        if let Some(err) = batch_error {
            // Rollback entire batch.
            let _ = self
                .entity
                .db
                .conn()
                .execute("ROLLBACK TO SAVEPOINT import_batch_sp", []);
            let _ = self
                .entity
                .db
                .conn()
                .execute("RELEASE SAVEPOINT import_batch_sp", []);
            self.status_bar.set_error(format!("Import failed: {err}"));
            if let Some(ref mut f) = self.import_flow {
                f.step = ImportFlowStep::ReviewScreen;
            }
            return;
        }

        // Commit savepoint.
        let _ = self
            .entity
            .db
            .conn()
            .execute("RELEASE SAVEPOINT import_batch_sp", []);

        // Save AI-suggested mappings.
        for (desc, account_id, acct_num, acct_name) in &learned_mappings {
            let _ = self.entity.db.import_mappings().create(
                desc,
                *account_id,
                ImportMatchType::Exact,
                ImportMatchSource::AiSuggested,
                &bank_name,
            );
            let _ = self.entity.db.audit().log_mapping_learned(
                &entity_name,
                desc,
                acct_num,
                acct_name,
                "ai_suggested",
            );
        }

        // Log CsvImport audit entry.
        let matched = matches_snapshot
            .iter()
            .filter(|m| m.matched_account_id.is_some() && !m.rejected)
            .count();
        let _ = self.entity.db.audit().log_csv_import(
            &entity_name,
            &bank_name,
            matches_snapshot.len(),
            matched,
            ai_matched_count,
            manual_count,
        );

        // Refresh tabs so JE list shows new drafts.
        for tab in &mut self.entity.tabs {
            tab.refresh(&self.entity.db);
        }

        self.status_bar.set_message(format!(
            "Imported {created_count} draft entries from {bank_name}."
        ));

        // Clear all import flow state.
        self.import_flow = None;
        self.file_picker = None;
        self.focus = FocusTarget::MainTab;
    }

    /// Handles key events while the file picker modal is active.
    pub(super) fn handle_file_picker_key(&mut self, key: KeyEvent) {
        let Some(mut picker) = self.file_picker.take() else {
            return;
        };
        match picker.handle_key(key) {
            FilePickerAction::Cancelled => {
                // file_picker stays None (already taken).
            }
            FilePickerAction::Selected(path) => {
                // Save last_import_dir.
                let (toml_path, workspace_dir) = self.entity_toml_path();
                let mut entity_cfg =
                    crate::config::load_entity_toml(&toml_path, &workspace_dir).unwrap_or_default();
                if let Some(parent) = path.parent() {
                    entity_cfg.last_import_dir = Some(parent.to_string_lossy().into_owned());
                    let _ =
                        crate::config::save_entity_toml(&toml_path, &workspace_dir, &entity_cfg);
                }
                // Build import flow starting after the file selection step.
                let mut flow = ImportFlowState::new();
                flow.file_path = Some(path);
                flow.available_banks = entity_cfg.bank_accounts;
                if flow.available_banks.is_empty() {
                    flow.step = ImportFlowStep::NewBankName;
                    flow.is_new_bank = true;
                } else {
                    flow.step = ImportFlowStep::BankSelection;
                }
                flow.selected_index = 0;
                self.import_flow = Some(flow);
            }
            FilePickerAction::Pending => {
                self.file_picker = Some(picker);
            }
        }
    }

    /// Handles all key events while the import wizard modal is active.
    pub(super) fn handle_import_key(&mut self, key: KeyEvent) {
        // Take the flow out to avoid simultaneous self borrows.
        let Some(mut flow) = self.import_flow.take() else {
            return;
        };

        let step = flow.step.clone();
        match step {
            ImportFlowStep::BankSelection => {
                // Handle delete confirmation sub-state first.
                if let Some(del_idx) = flow.delete_confirm {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            let (toml_path, workspace_dir) = self.entity_toml_path();
                            let mut entity_cfg =
                                crate::config::load_entity_toml(&toml_path, &workspace_dir)
                                    .unwrap_or_default();
                            if del_idx < entity_cfg.bank_accounts.len() {
                                entity_cfg.bank_accounts.remove(del_idx);
                                if let Err(e) = crate::config::save_entity_toml(
                                    &toml_path,
                                    &workspace_dir,
                                    &entity_cfg,
                                ) {
                                    self.status_bar
                                        .set_error(format!("Failed to delete bank config: {e}"));
                                } else {
                                    flow.available_banks = entity_cfg.bank_accounts;
                                    // Keep selected_index in bounds.
                                    let max = flow.available_banks.len();
                                    if flow.selected_index > 0 && flow.selected_index >= max {
                                        flow.selected_index = max.saturating_sub(1);
                                    }
                                }
                            }
                            flow.delete_confirm = None;
                        }
                        _ => {
                            flow.delete_confirm = None;
                        }
                    }
                } else {
                    match key.code {
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
                        KeyCode::Char('d') => {
                            let new_idx = flow.available_banks.len();
                            if flow.selected_index < new_idx {
                                flow.delete_confirm = Some(flow.selected_index);
                            }
                        }
                        KeyCode::Char('e') => {
                            let new_idx = flow.available_banks.len();
                            if flow.selected_index < new_idx {
                                let cfg = flow.available_banks[flow.selected_index].clone();
                                flow.detected_config = Some(cfg);
                                flow.is_editing_bank = true;
                                flow.editing_bank_index = Some(flow.selected_index);
                                flow.confirmation_cursor = 0;
                                flow.confirmation_editing = false;
                                flow.confirmation_edit_buffer.clear();
                                flow.step = ImportFlowStep::NewBankConfirmation;
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
                                App::enter_duplicate_check(&mut flow, &self.entity.db);
                                flow.selected_index = 0;
                                if flow.step == ImportFlowStep::Pass1Matching {
                                    self.pending_pass1 = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
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
            ImportFlowStep::NewBankConfirmation => {
                if flow.confirmation_editing {
                    match key.code {
                        KeyCode::Esc => {
                            flow.confirmation_editing = false;
                            flow.confirmation_edit_buffer.clear();
                        }
                        KeyCode::Enter => {
                            // Apply the edit buffer to the appropriate config field.
                            let buf = flow.confirmation_edit_buffer.trim().to_string();
                            let cur = flow.confirmation_cursor;
                            let is_single = flow
                                .detected_config
                                .as_ref()
                                .is_none_or(|c| c.amount_column.is_some());
                            if let Some(ref mut cfg) = flow.detected_config {
                                match cur {
                                    0 => {
                                        if !buf.is_empty() {
                                            cfg.date_column = buf;
                                        }
                                    }
                                    1 => {
                                        if !buf.is_empty() {
                                            cfg.date_format = buf;
                                        }
                                    }
                                    2 => {
                                        if !buf.is_empty() {
                                            cfg.description_column = buf;
                                        }
                                    }
                                    3 => {
                                        if is_single {
                                            cfg.amount_column =
                                                if buf.is_empty() { None } else { Some(buf) };
                                        } else {
                                            cfg.debit_column =
                                                if buf.is_empty() { None } else { Some(buf) };
                                        }
                                    }
                                    4 if !is_single => {
                                        cfg.credit_column =
                                            if buf.is_empty() { None } else { Some(buf) };
                                    }
                                    _ => {}
                                }
                            }
                            flow.confirmation_editing = false;
                            flow.confirmation_edit_buffer.clear();
                        }
                        KeyCode::Backspace => {
                            flow.confirmation_edit_buffer.pop();
                        }
                        KeyCode::Char(c) => {
                            flow.confirmation_edit_buffer.push(c);
                        }
                        _ => {}
                    }
                } else {
                    let is_single = flow
                        .detected_config
                        .as_ref()
                        .is_none_or(|c| c.amount_column.is_some());
                    match key.code {
                        KeyCode::Esc => {
                            if flow.is_editing_bank {
                                // Return to bank selection without cancelling the import.
                                flow.is_editing_bank = false;
                                flow.editing_bank_index = None;
                                flow.detected_config = None;
                                flow.step = ImportFlowStep::BankSelection;
                            } else {
                                return;
                            }
                        }
                        KeyCode::Up => {
                            flow.confirmation_cursor = flow.confirmation_cursor.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            if flow.confirmation_cursor < 5 {
                                flow.confirmation_cursor += 1;
                            }
                        }
                        KeyCode::Enter | KeyCode::Char(' ') => {
                            let cur = flow.confirmation_cursor;
                            if cur == 5 {
                                if flow.is_editing_bank {
                                    // Save edited config back to TOML and return to BankSelection.
                                    if let (Some(updated_cfg), Some(edit_idx)) =
                                        (flow.detected_config.clone(), flow.editing_bank_index)
                                    {
                                        let (toml_path, workspace_dir) = self.entity_toml_path();
                                        let mut entity_cfg = crate::config::load_entity_toml(
                                            &toml_path,
                                            &workspace_dir,
                                        )
                                        .unwrap_or_default();
                                        if edit_idx < entity_cfg.bank_accounts.len() {
                                            entity_cfg.bank_accounts[edit_idx] =
                                                updated_cfg.clone();
                                            if let Err(e) = crate::config::save_entity_toml(
                                                &toml_path,
                                                &workspace_dir,
                                                &entity_cfg,
                                            ) {
                                                self.status_bar.set_error(format!(
                                                    "Failed to save bank config: {e}"
                                                ));
                                            } else {
                                                flow.available_banks =
                                                    entity_cfg.bank_accounts.clone();
                                            }
                                        }
                                        flow.is_editing_bank = false;
                                        flow.editing_bank_index = None;
                                        flow.detected_config = None;
                                        flow.step = ImportFlowStep::BankSelection;
                                    }
                                } else {
                                    // New bank: advance to account picker.
                                    let accounts =
                                        self.entity.db.accounts().list_all().unwrap_or_default();
                                    flow.picker_accounts = accounts;
                                    flow.account_picker.reset();
                                    flow.account_picker.refresh(&flow.picker_accounts);
                                    flow.step = ImportFlowStep::NewBankAccountPicker;
                                }
                            } else if cur == 1 {
                                // Cycle through date formats.
                                if let Some(ref mut cfg) = flow.detected_config {
                                    cfg.date_format =
                                        cycle_date_format(&cfg.date_format).to_string();
                                }
                            } else if cur == 4 && is_single {
                                // Toggle sign convention.
                                if let Some(ref mut cfg) = flow.detected_config {
                                    cfg.debit_is_negative = !cfg.debit_is_negative;
                                }
                            } else {
                                // Open inline edit for text fields (rows 0, 2, 3, 4-split).
                                let val = flow.detected_config.as_ref().map(|cfg| match cur {
                                    0 => cfg.date_column.clone(),
                                    2 => cfg.description_column.clone(),
                                    3 if is_single => cfg.amount_column.clone().unwrap_or_default(),
                                    3 => cfg.debit_column.clone().unwrap_or_default(),
                                    4 => cfg.credit_column.clone().unwrap_or_default(),
                                    _ => String::new(),
                                });
                                flow.confirmation_edit_buffer = val.unwrap_or_default();
                                flow.confirmation_editing = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
            ImportFlowStep::NewBankAccountPicker => {
                if key.code == KeyCode::Esc {
                    // Go back to the confirmation screen instead of cancelling the whole import.
                    flow.account_picker.reset();
                    flow.picker_accounts.clear();
                    flow.step = ImportFlowStep::NewBankConfirmation;
                    self.import_flow = Some(flow);
                    return;
                }
                let picker_accounts = flow.picker_accounts.clone();
                let action = flow.account_picker.handle_key(key, &picker_accounts);
                match action {
                    crate::widgets::account_picker::PickerAction::Selected(account_id) => {
                        // Find account number for linked_account.
                        let number = picker_accounts
                            .iter()
                            .find(|a| a.id == account_id)
                            .map(|a| a.number.clone())
                            .unwrap_or_default();
                        // Complete the BankAccountConfig.
                        if let Some(ref mut cfg) = flow.detected_config {
                            cfg.linked_account = number;
                        }
                        let completed_cfg = flow.detected_config.clone().unwrap_or_else(|| {
                            crate::config::BankAccountConfig {
                                name: flow.new_bank_name.clone().unwrap_or_default(),
                                linked_account: String::new(),
                                date_column: "Date".to_string(),
                                date_format: "%m/%d/%Y".to_string(),
                                description_column: "Description".to_string(),
                                amount_column: Some("Amount".to_string()),
                                debit_column: None,
                                credit_column: None,
                                debit_is_negative: true,
                            }
                        });
                        // Save to entity toml.
                        let (toml_path, workspace_dir) = self.entity_toml_path();
                        let mut entity_cfg =
                            crate::config::load_entity_toml(&toml_path, &workspace_dir)
                                .unwrap_or_default();
                        entity_cfg.bank_accounts.push(completed_cfg.clone());
                        let _ = crate::config::save_entity_toml(
                            &toml_path,
                            &workspace_dir,
                            &entity_cfg,
                        );
                        flow.bank_config = Some(completed_cfg);
                        flow.available_banks = entity_cfg.bank_accounts;
                        // Dismiss the picker widget before advancing.
                        flow.account_picker.reset();
                        flow.picker_accounts.clear();
                        App::enter_duplicate_check(&mut flow, &self.entity.db);
                        flow.selected_index = 0;
                        if flow.step == ImportFlowStep::Pass1Matching {
                            self.pending_pass1 = true;
                        }
                    }
                    // Enter with no selection: stay in picker (don't cancel the import).
                    crate::widgets::account_picker::PickerAction::Cancelled => {}
                    crate::widgets::account_picker::PickerAction::Pending => {}
                }
            }
            ImportFlowStep::DuplicateWarning => match key.code {
                KeyCode::Esc => return,
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    // Skip duplicates: keep only unique transactions.
                    let existing_refs: std::collections::HashSet<String> = flow
                        .duplicates
                        .iter()
                        .map(|t| t.import_ref.clone())
                        .collect();
                    flow.transactions
                        .retain(|t| !existing_refs.contains(&t.import_ref));
                    flow.step = ImportFlowStep::Pass1Matching;
                    self.pending_pass1 = true;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Enter => {
                    // Include all (duplicates included).
                    // All transactions are already in flow.transactions.
                    flow.step = ImportFlowStep::Pass1Matching;
                    self.pending_pass1 = true;
                }
                _ => {}
            },
            // Pass1Matching: show progress; Esc cancels, keys otherwise consumed.
            ImportFlowStep::Pass1Matching => {
                if key.code == KeyCode::Esc {
                    self.pending_pass1 = false;
                    return;
                }
            }
            // Pass2AiMatching: AI matching in progress; Esc cancels.
            ImportFlowStep::Pass2AiMatching => {
                if key.code == KeyCode::Esc {
                    self.pending_pass2 = false;
                    return;
                }
            }
            // Pass3Clarification: route keys to chat panel for one item at a time.
            ImportFlowStep::Pass3Clarification => {
                if key.code == KeyCode::Esc {
                    // Skip remaining — advance to review with what we have.
                    flow.step = ImportFlowStep::ReviewScreen;
                    flow.selected_index = 0;
                    flow.scroll_offset = 0;
                    self.focus = FocusTarget::MainTab;
                    self.import_flow = Some(flow);
                    return;
                }

                // Show prompt for current item if not yet shown.
                if !flow.clarification_prompted {
                    if flow.clarification_queue.is_empty() {
                        flow.step = ImportFlowStep::ReviewScreen;
                        self.focus = FocusTarget::MainTab;
                        self.import_flow = Some(flow);
                        return;
                    }
                    let idx = flow.clarification_queue[0];
                    let m = &flow.matches[idx];
                    let txn = &m.transaction;
                    let acct = m
                        .matched_account_display
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    let reasoning = m.reasoning.clone().unwrap_or_default();
                    let prompt = format!(
                        "Transaction: {} | {} | {}\n\
                         Best guess: {} (Low confidence)\n\
                         Reason: {}\n\n\
                         Type an account number to redirect, \
                         'confirm'/'y' to accept, or 'skip'/'s' to leave unmatched.",
                        txn.date, txn.description, txn.amount, acct, reasoning
                    );
                    self.chat_panel.add_system_note(&prompt);
                    if !self.chat_panel.is_visible() {
                        self.chat_panel.toggle_visible();
                    }
                    self.focus = FocusTarget::ChatPanel;
                    flow.clarification_prompted = true;
                    self.import_flow = Some(flow);
                    return;
                }

                // Route key to chat panel; intercept SendMessage.
                let action = self.chat_panel.handle_key(key);
                match action {
                    ChatAction::SendMessage(messages) => {
                        // Extract the user's text from the last user message.
                        let user_text = messages
                            .iter()
                            .rev()
                            .find(|m| matches!(m.role, ApiRole::User))
                            .and_then(|m| {
                                if let ApiContent::Text(t) = &m.content {
                                    Some(t.trim().to_string())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_default();
                        let input_lower = user_text.to_lowercase();
                        let idx = flow.clarification_queue.remove(0);

                        if input_lower == "skip" || input_lower == "s" {
                            // Leave unmatched.
                            flow.matches[idx].matched_account_id = None;
                            flow.matches[idx].matched_account_display = None;
                            flow.matches[idx].match_source = crate::types::MatchSource::Unmatched;
                        } else if input_lower == "confirm" || input_lower == "y" {
                            // Accept Claude's suggestion.
                            flow.matches[idx].match_source =
                                crate::types::MatchSource::UserConfirmed;
                            // Save mapping to import_mappings.
                            if let Some(account_id) = flow.matches[idx].matched_account_id {
                                let bank_name = flow
                                    .bank_config
                                    .as_ref()
                                    .map(|c| c.name.clone())
                                    .unwrap_or_default();
                                let desc = flow.matches[idx].transaction.description.clone();
                                let _ = self.entity.db.import_mappings().create(
                                    &desc,
                                    account_id,
                                    crate::types::ImportMatchType::Exact,
                                    crate::types::ImportMatchSource::Confirmed,
                                    &bank_name,
                                );
                            }
                        } else {
                            // Try to match by account number or name.
                            let accounts =
                                self.entity.db.accounts().list_all().unwrap_or_else(|e| {
                                    tracing::warn!("Failed to load accounts: {e}");
                                    Vec::new()
                                });
                            let trimmed = user_text.trim();
                            let trimmed_lower = trimmed.to_lowercase();
                            let found = accounts.iter().find(|a| {
                                a.number == trimmed || a.name.to_lowercase() == trimmed_lower
                            });
                            if let Some(acct) = found {
                                flow.matches[idx].matched_account_id = Some(acct.id);
                                flow.matches[idx].matched_account_display =
                                    Some(format!("{} - {}", acct.number, acct.name));
                                flow.matches[idx].match_source =
                                    crate::types::MatchSource::UserConfirmed;
                                flow.matches[idx].confidence =
                                    Some(crate::types::MatchConfidence::High);
                                // Save mapping.
                                let bank_name = flow
                                    .bank_config
                                    .as_ref()
                                    .map(|c| c.name.clone())
                                    .unwrap_or_default();
                                let desc = flow.matches[idx].transaction.description.clone();
                                let _ = self.entity.db.import_mappings().create(
                                    &desc,
                                    acct.id,
                                    crate::types::ImportMatchType::Exact,
                                    crate::types::ImportMatchSource::Confirmed,
                                    &bank_name,
                                );
                            } else {
                                self.chat_panel
                                    .add_system_note("Account not found. Try the account number, 'confirm', or 'skip'.");
                                // Put the index back.
                                flow.clarification_queue.insert(0, idx);
                            }
                        }

                        // Advance to next item or finish.
                        if flow.clarification_queue.is_empty() {
                            flow.step = ImportFlowStep::ReviewScreen;
                            flow.selected_index = 0;
                            flow.scroll_offset = 0;
                            self.focus = FocusTarget::MainTab;
                        } else {
                            flow.clarification_prompted = false;
                        }
                    }
                    ChatAction::Close => {
                        // User closed chat — advance to ReviewScreen.
                        flow.step = ImportFlowStep::ReviewScreen;
                        flow.selected_index = 0;
                        flow.scroll_offset = 0;
                        self.chat_panel.toggle_visible();
                        self.focus = FocusTarget::MainTab;
                    }
                    _ => {}
                }
                self.import_flow = Some(flow);
                return;
            }
            // ReviewScreen: full-screen match review with approve/reject.
            ImportFlowStep::ReviewScreen => {
                let rows = build_review_rows(&flow);
                let row_count = rows.len();
                match key.code {
                    KeyCode::Esc => return, // Cancel — no drafts, discard flow.
                    KeyCode::Up => {
                        flow.selected_index = flow.selected_index.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        if flow.selected_index + 1 < row_count {
                            flow.selected_index += 1;
                        }
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        if let Some(row) = rows.get(flow.selected_index) {
                            match row {
                                ReviewRow::TransferItem { transfer_idx } => {
                                    let idx = *transfer_idx;
                                    flow.transfer_matches[idx].confirmed =
                                        !flow.transfer_matches[idx].confirmed;
                                }
                                ReviewRow::ApproveAction => {
                                    if key.code == KeyCode::Enter {
                                        flow.step = ImportFlowStep::Creating;
                                        self.pending_draft_creation = true;
                                    }
                                }
                                ReviewRow::SectionHeader { section_idx, .. } => {
                                    flow.review_section_expanded[*section_idx] =
                                        !flow.review_section_expanded[*section_idx];
                                }
                                ReviewRow::TransferHeader { .. } | ReviewRow::MatchItem { .. } => {}
                            }
                        }
                    }
                    KeyCode::Char('r') => {
                        if let Some(ReviewRow::MatchItem {
                            match_idx,
                            section_idx,
                        }) = rows.get(flow.selected_index)
                        {
                            // Only AI-matched items (section 1) can be rejected.
                            if *section_idx == 1 {
                                let idx = *match_idx;
                                flow.matches[idx].matched_account_id = None;
                                flow.matches[idx].matched_account_display = None;
                                flow.matches[idx].match_source =
                                    crate::types::MatchSource::Unmatched;
                                flow.matches[idx].confidence = None;
                                flow.matches[idx].reasoning = None;
                                flow.matches[idx].rejected = true;
                            }
                        }
                    }
                    _ => {}
                }
                self.import_flow = Some(flow);
                return;
            }
            // Creating/Complete: keys consumed while batch operation runs.
            ImportFlowStep::Creating | ImportFlowStep::Complete => {
                if key.code == KeyCode::Esc {
                    return;
                }
            }
            // Failed: Esc dismisses.
            ImportFlowStep::Failed(_) => {
                if key.code == KeyCode::Esc {
                    return;
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

/// A row in the review screen list.
#[derive(Debug, Clone)]
enum ReviewRow {
    ApproveAction,
    /// Header row for the transfer matches section.
    TransferHeader {
        count: usize,
    },
    /// A single transfer match item.
    TransferItem {
        transfer_idx: usize,
    },
    SectionHeader {
        label: String,
        section_idx: usize,
        count: usize,
        expanded: bool,
    },
    MatchItem {
        match_idx: usize,
        section_idx: usize,
    },
}

/// Builds the flat list of review rows from the current flow state.
fn build_review_rows(flow: &crate::ai::csv_import::ImportFlowState) -> Vec<ReviewRow> {
    use crate::types::MatchSource;

    let sections: [(MatchSource, &str, usize); 4] = [
        (MatchSource::Local, "Auto-Matched", 0),
        (MatchSource::Ai, "AI-Matched", 1),
        (MatchSource::UserConfirmed, "User-Confirmed", 2),
        (MatchSource::Unmatched, "Unmatched", 3),
    ];

    let mut rows = Vec::new();

    // Transfer matches section appears first, before the approve button.
    if !flow.transfer_matches.is_empty() {
        rows.push(ReviewRow::TransferHeader {
            count: flow.transfer_matches.len(),
        });
        for (i, _) in flow.transfer_matches.iter().enumerate() {
            rows.push(ReviewRow::TransferItem { transfer_idx: i });
        }
    }

    rows.push(ReviewRow::ApproveAction);

    for (source, label, section_idx) in &sections {
        let indices: Vec<usize> = flow
            .matches
            .iter()
            .enumerate()
            .filter(|(_, m)| &m.match_source == source)
            .map(|(i, _)| i)
            .collect();
        let count = indices.len();
        if count == 0 {
            continue;
        }
        let expanded = flow.review_section_expanded[*section_idx];
        rows.push(ReviewRow::SectionHeader {
            label: label.to_string(),
            section_idx: *section_idx,
            count,
            expanded,
        });
        if expanded {
            for match_idx in indices {
                rows.push(ReviewRow::MatchItem {
                    match_idx,
                    section_idx: *section_idx,
                });
            }
        }
    }
    rows
}

/// Renders the import review screen (full-screen view of all matches).
pub(super) fn render_review_screen(
    frame: &mut ratatui::Frame,
    area: Rect,
    flow: &crate::ai::csv_import::ImportFlowState,
    bank_account_type: crate::types::AccountType,
) {
    use crate::ai::csv_import::determine_debit_credit;
    use crate::types::MatchSource;
    use ratatui::{
        layout::{Constraint, Direction, Layout},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    };

    // Clear the full area first so nothing from the underlying tab bleeds through
    // (e.g., an account picker or form that was active in the JE tab).
    frame.render_widget(Clear, area);

    let bank_name = flow
        .bank_config
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or("Import");
    let total = flow.matches.len();

    // Optionally reserve space at the top for parse warnings.
    let warning_height = if flow.warnings.is_empty() {
        0u16
    } else {
        flow.warnings.len() as u16 + 2 // borders
    };

    // Split area: [warnings?] list on top, detail pane on bottom.
    let outer_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if warning_height > 0 {
            vec![
                Constraint::Length(warning_height),
                Constraint::Min(5),
                Constraint::Length(8),
            ]
        } else {
            vec![Constraint::Min(5), Constraint::Length(8)]
        })
        .split(area);

    let (list_area, detail_area) = if warning_height > 0 {
        // Render warnings panel.
        let warn_area = outer_chunks[0];
        let warning_lines: Vec<Line> = flow
            .warnings
            .iter()
            .map(|w| {
                Line::from(Span::styled(
                    format!("  \u{26a0} {w}"),
                    Style::default().fg(Color::Yellow),
                ))
            })
            .collect();
        frame.render_widget(
            Paragraph::new(warning_lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Parse Warnings ")
                    .style(Style::default().fg(Color::Yellow)),
            ),
            warn_area,
        );
        (outer_chunks[1], outer_chunks[2])
    } else {
        (outer_chunks[0], outer_chunks[1])
    };

    // Build rows.
    let rows = build_review_rows(flow);
    let selected = flow.selected_index.min(rows.len().saturating_sub(1));

    // Scroll offset: keep selected visible.
    let visible_height = list_area.height.saturating_sub(2) as usize;
    let scroll = if selected < flow.scroll_offset {
        selected
    } else if selected >= flow.scroll_offset + visible_height {
        selected + 1 - visible_height
    } else {
        flow.scroll_offset
    };

    // Build list items.
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(i, row)| {
            let is_selected = i == selected;
            let base_style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let text = match row {
                ReviewRow::TransferHeader { count } => {
                    let s = if is_selected {
                        base_style
                    } else {
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD)
                    };
                    Line::from(Span::styled(
                        format!(
                            " \u{2500}\u{2500}\u{2500} Transfer Matches ({count}) \
                             \u{2500}\u{2500}\u{2500}"
                        ),
                        s,
                    ))
                }
                ReviewRow::TransferItem { transfer_idx } => {
                    let tm = &flow.transfer_matches[*transfer_idx];
                    let (indicator, indicator_style) = if tm.confirmed {
                        (
                            "\u{2713}",
                            if is_selected {
                                base_style
                            } else {
                                Style::default().fg(Color::Green)
                            },
                        )
                    } else {
                        (
                            "\u{2717}",
                            if is_selected {
                                base_style
                            } else {
                                Style::default().fg(Color::Red)
                            },
                        )
                    };
                    let desc = tm.description.chars().take(24).collect::<String>();
                    let amt_sign = if tm.amount >= crate::types::Money(0) {
                        "+"
                    } else {
                        ""
                    };
                    let matched_amt_sign = if tm.matched_amount >= crate::types::Money(0) {
                        "+"
                    } else {
                        ""
                    };
                    let label = format!(
                        "  {indicator}  {}  {amt_sign}{}  \"{desc}\"  \u{2192}  \
                         JE #{} ({}, {matched_amt_sign}{}, {})",
                        tm.date.format("%b %d"),
                        tm.amount,
                        tm.matched_je_number,
                        tm.matched_bank,
                        tm.matched_amount,
                        tm.matched_date.format("%b %d"),
                    );
                    Line::from(Span::styled(label, indicator_style))
                }
                ReviewRow::ApproveAction => {
                    let s = if is_selected {
                        base_style
                    } else {
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD)
                    };
                    Line::from(Span::styled(
                        " \u{2713} Approve All & Create Drafts  [Enter]",
                        s,
                    ))
                }
                ReviewRow::SectionHeader {
                    label,
                    count,
                    expanded,
                    ..
                } => {
                    let arrow = if *expanded { "\u{25BE}" } else { "\u{25B8}" };
                    let s = if is_selected {
                        base_style
                    } else {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    };
                    Line::from(Span::styled(format!(" {arrow} {label} ({count})"), s))
                }
                ReviewRow::MatchItem {
                    match_idx,
                    section_idx,
                } => {
                    let m = &flow.matches[*match_idx];
                    let desc = m
                        .transaction
                        .description
                        .chars()
                        .take(30)
                        .collect::<String>();
                    let acct = m.matched_account_display.as_deref().unwrap_or("(no match)");
                    let suffix = match *section_idx {
                        1 => {
                            // AI-matched: show confidence
                            let conf = match m.confidence {
                                Some(crate::types::MatchConfidence::High) => "high",
                                Some(crate::types::MatchConfidence::Medium) => "med",
                                _ => "low",
                            };
                            format!("  {} \u{2192} {} ({conf})  [r: reject]", desc, acct)
                        }
                        3 => format!("  {} \u{2192} (unmatched)", desc),
                        _ => format!("  {} \u{2192} {}", desc, acct),
                    };
                    let style = if is_selected {
                        base_style
                    } else if *section_idx == 0 {
                        Style::default().fg(Color::DarkGray)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    Line::from(Span::styled(suffix, style))
                }
            };
            ListItem::new(text)
        })
        .collect();

    let title = format!(" Import Review \u{2014} {bank_name} \u{2014} {total} transactions ");
    frame.render_widget(Clear, list_area);
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(Style::default().fg(Color::Cyan)),
        ),
        list_area,
    );

    // Detail pane: show transfer match info or the proposed JE.
    frame.render_widget(Clear, detail_area);
    let detail_lines = if let Some(ReviewRow::TransferItem { transfer_idx }) = rows.get(selected) {
        let tm = &flow.transfer_matches[*transfer_idx];
        let action = if tm.confirmed {
            "Skip import — link import_ref to existing draft JE (no new draft created)"
        } else {
            "Reject — send to Pass 2 for AI matching instead"
        };
        vec![
            Line::from(Span::styled(
                format!(
                    " Transfer: {} | {} | {}",
                    tm.date.format("%b %d"),
                    tm.description,
                    tm.amount
                ),
                Style::default().fg(Color::Gray),
            )),
            Line::from(Span::styled(
                format!(
                    " Matched: JE #{} | {} | {} | {}",
                    tm.matched_je_number,
                    tm.matched_bank,
                    tm.matched_amount,
                    tm.matched_date.format("%b %d")
                ),
                Style::default().fg(Color::Gray),
            )),
            Line::from(Span::styled(
                format!(" Action: {action}"),
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                "  Enter/Space: toggle confirm/reject   \u{2191}/\u{2193}: navigate   Esc: cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ]
    } else if let Some(ReviewRow::MatchItem { match_idx, .. }) = rows.get(selected) {
        let m = &flow.matches[*match_idx];
        let txn = &m.transaction;
        let bank_acct = flow
            .bank_config
            .as_ref()
            .map(|c| c.linked_account.as_str())
            .unwrap_or("bank");
        let contra_acct = m
            .matched_account_display
            .as_deref()
            .unwrap_or("(unmatched)");

        // Determine debit/credit using the actual bank account type.
        let (debit, credit, _) = determine_debit_credit(txn.amount, bank_account_type);
        let memo = format!(
            "Import: {}",
            txn.description.chars().take(200).collect::<String>()
        );

        vec![
            Line::from(Span::styled(
                format!(" Date: {}  Ref: {}", txn.date, txn.import_ref),
                Style::default().fg(Color::Gray),
            )),
            Line::from(Span::styled(
                format!(" Memo: {}", memo.chars().take(60).collect::<String>()),
                Style::default().fg(Color::Gray),
            )),
            Line::from(Span::styled(
                format!(
                    " Dr {bank_acct}: {debit}  Cr {contra_acct}: {credit}",
                    debit = if debit > crate::types::Money(0) {
                        format!("{debit}")
                    } else {
                        "\u{2014}".to_string()
                    },
                    credit = if credit > crate::types::Money(0) {
                        format!("{credit}")
                    } else {
                        "\u{2014}".to_string()
                    },
                ),
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                "  \u{2191}/\u{2193}: navigate   Enter: select/approve   r: reject AI match   Esc: cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ]
    } else {
        vec![
            Line::from(Span::raw("")),
            Line::from(Span::styled(
                "  Select a transaction to preview the draft journal entry.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::raw("")),
            Line::from(Span::styled(
                "  \u{2191}/\u{2193}: navigate   Enter: approve all   Esc: cancel import",
                Style::default().fg(Color::DarkGray),
            )),
        ]
    };

    frame.render_widget(
        Paragraph::new(detail_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Draft Preview ")
                .style(Style::default().fg(Color::DarkGray)),
        ),
        detail_area,
    );

    // Suppress unused import warning.
    let _ = MatchSource::Local;
}

/// Renders the import wizard modal overlay.
pub(super) fn render_import_modal(
    frame: &mut ratatui::Frame,
    area: Rect,
    flow: &crate::ai::csv_import::ImportFlowState,
    bank_account_type: crate::types::AccountType,
) {
    match &flow.step {
        ImportFlowStep::BankSelection => render_bank_selection_modal(frame, area, flow),
        ImportFlowStep::NewBankName => render_new_bank_name_modal(frame, area, flow),
        ImportFlowStep::NewBankDetection => {
            render_new_bank_detection_modal(frame, area, "Initializing \u{21BB}")
        }
        ImportFlowStep::NewBankConfirmation => {
            render_new_bank_confirmation_modal(frame, area, flow)
        }
        ImportFlowStep::NewBankAccountPicker => render_account_picker_modal(frame, area, flow),
        ImportFlowStep::DuplicateWarning => render_duplicate_warning_modal(frame, area, flow),
        ImportFlowStep::Pass1Matching => {
            let total = flow.transactions.len();
            let msg = format!("Importing \u{263A} {total}/{total}");
            render_new_bank_detection_modal(frame, area, &msg);
        }
        ImportFlowStep::Pass2AiMatching => {
            let matched = flow
                .matches
                .iter()
                .filter(|m| m.matched_account_id.is_some())
                .count();
            let total = flow.matches.len();
            let msg = format!("Matching with AI \u{263A} {matched}/{total}");
            render_new_bank_detection_modal(frame, area, &msg);
        }
        ImportFlowStep::ReviewScreen | ImportFlowStep::Creating => {
            render_review_screen(frame, area, flow, bank_account_type)
        }
        ImportFlowStep::Failed(msg) => render_new_bank_detection_modal(frame, area, msg),
        // Future steps render their own modals (implemented in later tasks).
        _ => {}
    }
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

    if let Some(del_idx) = flow.delete_confirm {
        let bank_name = flow
            .available_banks
            .get(del_idx)
            .map(|b| b.name.as_str())
            .unwrap_or("?");
        lines.push(Line::from(Span::styled(
            format!("  Delete bank config '{bank_name}'? Y/N"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  \u{2191}/\u{2193}: navigate  Enter: select  e: edit  d: delete  Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )));
    }

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

/// Standard date formats available in the cycle selector, in cycle order.
const DATE_FORMAT_CYCLE: &[&str] = &[
    "%m/%d/%Y", "%Y-%m-%d", "%d/%m/%Y", "%m-%d-%Y", "%Y/%m/%d", "%d-%m-%Y",
];

/// Advances to the next date format in the cycle.
///
/// If `current` matches a standard format, returns the next one (wrapping).
/// If `current` is an unrecognised format, returns the first standard format
/// (preserving the unknown value as the implicit starting point).
fn cycle_date_format(current: &str) -> &'static str {
    if let Some(pos) = DATE_FORMAT_CYCLE.iter().position(|&f| f == current) {
        DATE_FORMAT_CYCLE[(pos + 1) % DATE_FORMAT_CYCLE.len()]
    } else {
        DATE_FORMAT_CYCLE[0]
    }
}

/// Returns a human-readable label for a chrono date format string.
fn friendly_date_format(fmt: &str) -> String {
    match fmt {
        "%m/%d/%Y" => "MM/DD/YYYY e.g., 01/27/2026".to_string(),
        "%Y-%m-%d" => "YYYY-MM-DD e.g., 2026-01-27".to_string(),
        "%d/%m/%Y" => "DD/MM/YYYY e.g., 27/01/2026".to_string(),
        "%m-%d-%Y" => "MM-DD-YYYY e.g., 01-27-2026".to_string(),
        "%Y/%m/%d" => "YYYY/MM/DD e.g., 2026/01/27".to_string(),
        "%d-%m-%Y" => "DD-MM-YYYY e.g., 27-01-2026".to_string(),
        other => other.to_string(),
    }
}

/// Renders the editable bank detection confirmation step.
fn render_new_bank_confirmation_modal(
    frame: &mut ratatui::Frame,
    area: Rect,
    flow: &crate::ai::csv_import::ImportFlowState,
) {
    use ratatui::{
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Paragraph},
    };

    let modal = crate::widgets::centered_rect(70, 70, area);
    frame.render_widget(Clear, modal);

    let cursor = flow.confirmation_cursor;
    let is_editing = flow.confirmation_editing;
    let edit_buf = &flow.confirmation_edit_buffer;
    let is_editing_bank = flow.is_editing_bank;
    let is_single = flow
        .detected_config
        .as_ref()
        .is_none_or(|c| c.amount_column.is_some());

    let sel_s = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let edit_s = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let normal_s = Style::default().fg(Color::White);
    let label_s = Style::default().fg(Color::Gray);
    let dim_s = Style::default().fg(Color::DarkGray);

    // Builds one editable text-field row.
    let make_row = |idx: usize, label: &str, display_val: &str| -> Line {
        let sel = cursor == idx;
        let mark = if sel { "\u{25b6}" } else { " " };
        let lbl_text = format!("  {} {:<14}", mark, label);
        if sel && is_editing {
            Line::from(vec![
                Span::styled(lbl_text, label_s),
                Span::styled(format!("{}_", edit_buf), edit_s),
            ])
        } else if sel {
            Line::from(vec![
                Span::styled(lbl_text, sel_s),
                Span::styled(format!("\"{}\"", display_val), sel_s),
            ])
        } else {
            Line::from(vec![
                Span::styled(lbl_text, label_s),
                Span::styled(format!("\"{}\"", display_val), normal_s),
            ])
        }
    };

    // Builds the date format cycle row (row 1).
    let make_date_format_row = |fmt: &str| -> Line {
        let sel = cursor == 1;
        let mark = if sel { "\u{25b6}" } else { " " };
        let lbl_text = format!("  {} {:<14}", mark, "Date format:");
        let display = friendly_date_format(fmt);
        if sel {
            Line::from(vec![
                Span::styled(lbl_text, sel_s),
                Span::styled(display, sel_s),
                Span::styled("  [Enter/Space: cycle]", dim_s),
            ])
        } else {
            Line::from(vec![
                Span::styled(lbl_text, label_s),
                Span::styled(display, normal_s),
            ])
        }
    };

    let section_label = if is_editing_bank {
        "  Edit Bank Format:"
    } else {
        "  Detected Format:"
    };
    let mut lines: Vec<Line> = vec![
        Line::from(Span::raw("")),
        Line::from(Span::styled(section_label, label_s)),
        Line::from(Span::raw("")),
    ];

    if let Some(cfg) = &flow.detected_config {
        // Row 0: date column
        lines.push(make_row(0, "Date column:", &cfg.date_column));
        // Row 1: date format — cycle selector (not free-text edit)
        lines.push(make_date_format_row(&cfg.date_format));
        // Row 2: description column
        lines.push(make_row(2, "Description:", &cfg.description_column));

        if is_single {
            // Row 3: amount column
            lines.push(make_row(
                3,
                "Amount col:",
                cfg.amount_column.as_deref().unwrap_or(""),
            ));
            // Row 4: sign convention (toggle, not an editable text field)
            let sign_val = if cfg.debit_is_negative {
                "negative (- or parens) = withdrawal"
            } else {
                "positive = withdrawal"
            };
            let mark = if cursor == 4 { "\u{25b6}" } else { " " };
            let sign_line = if cursor == 4 {
                Line::from(vec![
                    Span::styled(format!("  {} {:<14}", mark, "Sign:"), sel_s),
                    Span::styled(sign_val, sel_s),
                    Span::styled("  [Enter/Space: toggle]", dim_s),
                ])
            } else {
                Line::from(vec![
                    Span::styled(format!("  {} {:<14}", mark, "Sign:"), label_s),
                    Span::styled(sign_val, normal_s),
                ])
            };
            lines.push(sign_line);
        } else {
            // Row 3: debit column, Row 4: credit column
            lines.push(make_row(
                3,
                "Debit col:",
                cfg.debit_column.as_deref().unwrap_or(""),
            ));
            lines.push(make_row(
                4,
                "Credit col:",
                cfg.credit_column.as_deref().unwrap_or(""),
            ));
        }
    }

    // Row 5: Confirm option
    lines.push(Line::from(Span::raw("")));
    let confirm_line = if cursor == 5 {
        Line::from(Span::styled(
            "  \u{25b6} [ Confirm ]",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from(Span::styled("    [ Confirm ]", dim_s))
    };
    lines.push(confirm_line);
    lines.push(Line::from(Span::raw("")));

    let hint = if is_editing {
        "  Enter: apply  Esc: discard"
    } else {
        "  \u{2191}\u{2193}: navigate  Enter: edit  Esc: cancel"
    };
    lines.push(Line::from(Span::styled(hint, dim_s)));

    let modal_title = if is_editing_bank {
        " Edit Bank Format "
    } else {
        " Confirm Column Mapping "
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(modal_title)
                .style(Style::default().fg(Color::Cyan)),
        ),
        modal,
    );
}

/// Renders the duplicate warning confirmation modal.
fn render_duplicate_warning_modal(
    frame: &mut ratatui::Frame,
    area: Rect,
    flow: &crate::ai::csv_import::ImportFlowState,
) {
    use ratatui::{
        style::{Color, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Paragraph},
    };

    let modal = crate::widgets::centered_rect(70, 35, area);
    frame.render_widget(Clear, modal);

    let dup_count = flow.duplicates.len();
    let total = flow.transactions.len();

    let lines = vec![
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            format!("  {dup_count} of {total} transactions appear to already be imported."),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  Skip duplicates?",
            Style::default().fg(Color::White),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  Y: skip duplicates   N/Enter: include all   Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Duplicate Detection ")
                .style(Style::default().fg(Color::Yellow)),
        ),
        modal,
    );
}

/// Renders the account picker for the new bank account link step.
/// Only renders when picker_accounts is non-empty (i.e., the picker was actively opened).
fn render_account_picker_modal(
    frame: &mut ratatui::Frame,
    area: Rect,
    flow: &crate::ai::csv_import::ImportFlowState,
) {
    // Guard: only render if picker state was explicitly populated for this step.
    // This prevents stale picker state from leaking into other screens.
    if flow.picker_accounts.is_empty() {
        return;
    }
    flow.account_picker
        .render(frame, area, &flow.picker_accounts);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::csv_import::{ImportFlowState, TransferMatchRow};
    use crate::types::Money;
    use chrono::NaiveDate;

    fn make_transfer_row(confirmed: bool) -> TransferMatchRow {
        TransferMatchRow {
            date: NaiveDate::from_ymd_opt(2026, 1, 14).unwrap(),
            amount: Money(50_000_000_000), // $500
            description: "ACH Deposit Chase".to_string(),
            import_ref: "Ally|2026-01-14|ACH Deposit Chase|500".to_string(),
            matched_je_id: crate::types::JournalEntryId::from(47),
            matched_je_number: "47".to_string(),
            matched_date: NaiveDate::from_ymd_opt(2026, 1, 14).unwrap(),
            matched_amount: Money(-50_000_000_000),
            matched_bank: "Chase".to_string(),
            confirmed,
        }
    }

    // ── toggle tests ─────────────────────────────────────────────────────────

    #[test]
    fn transfer_match_toggle_confirmed_to_rejected() {
        let mut flow = ImportFlowState::new();
        flow.transfer_matches.push(make_transfer_row(true));

        // Simulate toggle.
        flow.transfer_matches[0].confirmed = !flow.transfer_matches[0].confirmed;
        assert!(!flow.transfer_matches[0].confirmed);
    }

    #[test]
    fn transfer_match_toggle_rejected_back_to_confirmed() {
        let mut flow = ImportFlowState::new();
        flow.transfer_matches.push(make_transfer_row(false));

        flow.transfer_matches[0].confirmed = !flow.transfer_matches[0].confirmed;
        assert!(flow.transfer_matches[0].confirmed);
    }

    // ── navigation tests (via build_review_rows structure) ───────────────────

    #[test]
    fn review_rows_with_transfer_matches_start_with_transfer_header() {
        let mut flow = ImportFlowState::new();
        flow.transfer_matches.push(make_transfer_row(true));
        flow.transfer_matches.push(make_transfer_row(false));

        let rows = build_review_rows(&flow);

        // First row must be the transfer header.
        assert!(
            matches!(rows[0], ReviewRow::TransferHeader { count: 2 }),
            "expected TransferHeader(2), got {:?}",
            rows[0]
        );
        // Next two rows are the transfer items.
        assert!(matches!(
            rows[1],
            ReviewRow::TransferItem { transfer_idx: 0 }
        ));
        assert!(matches!(
            rows[2],
            ReviewRow::TransferItem { transfer_idx: 1 }
        ));
        // Then the approve button.
        assert!(matches!(rows[3], ReviewRow::ApproveAction));
    }

    #[test]
    fn nav_down_from_last_transfer_item_reaches_approve_action() {
        let mut flow = ImportFlowState::new();
        flow.transfer_matches.push(make_transfer_row(true));

        let rows = build_review_rows(&flow);
        // Transfer section: index 0 = header, index 1 = item, index 2 = ApproveAction.
        let last_transfer_idx = 1usize;
        let approve_idx = last_transfer_idx + 1;

        assert!(matches!(
            rows[last_transfer_idx],
            ReviewRow::TransferItem { .. }
        ));
        assert!(matches!(rows[approve_idx], ReviewRow::ApproveAction));
    }

    #[test]
    fn nav_up_from_approve_action_reaches_last_transfer_item() {
        let mut flow = ImportFlowState::new();
        flow.transfer_matches.push(make_transfer_row(true));
        flow.transfer_matches.push(make_transfer_row(true));

        let rows = build_review_rows(&flow);
        // header(0), item(1), item(2), ApproveAction(3)
        let approve_idx = 3usize;
        let last_transfer_idx = approve_idx - 1;

        assert!(matches!(rows[approve_idx], ReviewRow::ApproveAction));
        assert!(matches!(
            rows[last_transfer_idx],
            ReviewRow::TransferItem { transfer_idx: 1 }
        ));
        // Pressing ↑ from approve_idx: approve_idx.saturating_sub(1) == last_transfer_idx.
        assert_eq!(approve_idx.saturating_sub(1), last_transfer_idx);
    }

    // ── empty transfer section ────────────────────────────────────────────────

    #[test]
    fn review_rows_without_transfer_matches_start_with_approve_action() {
        let flow = ImportFlowState::new();
        let rows = build_review_rows(&flow);

        assert!(
            matches!(rows[0], ReviewRow::ApproveAction),
            "expected ApproveAction at index 0, got {:?}",
            rows[0]
        );
    }

    #[test]
    fn empty_transfer_section_navigation_unchanged() {
        let flow = ImportFlowState::new();
        let rows = build_review_rows(&flow);

        // No transfer rows at all — first row is ApproveAction, normal navigation.
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r, ReviewRow::TransferHeader { .. }))
        );
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r, ReviewRow::TransferItem { .. }))
        );
    }
}
