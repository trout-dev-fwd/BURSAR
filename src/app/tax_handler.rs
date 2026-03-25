use ratatui::Terminal;

use crate::{
    ai::{ApiContent, ApiMessage, ApiRole},
    db::journal_repo::JournalFilter,
    types::{AuditAction, TaxFormTag},
};

use super::App;

/// Max JEs per batch sent to the AI in a single request.
const BATCH_SIZE: usize = 25;

impl App {
    /// Runs AI batch review for all `ai_pending` JEs.
    ///
    /// Called from `process_pending` when `pending_tax_batch_review` is set
    /// by the `R` key in the Tax tab. Sends JEs in batches of 25 to the AI,
    /// parses the pipe-separated response, and updates each JE's tax tag.
    pub(super) fn handle_tax_batch_review<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) {
        // ── Lazy-init AI client ───────────────────────────────────────────────
        if let Err(msg) = self.ensure_ai_client() {
            self.status_bar.set_error(msg);
            return;
        }

        // ── Collect pending JEs ───────────────────────────────────────────────
        let pending = match self.entity.db.tax_tags().get_pending() {
            Ok(p) => p,
            Err(e) => {
                self.status_bar
                    .set_error(format!("Failed to load pending JEs: {e}"));
                return;
            }
        };

        if pending.is_empty() {
            self.status_bar
                .set_message("No JEs queued for AI review.".to_string());
            return;
        }

        // ── Build enabled forms list for system prompt ────────────────────────
        // Get enabled forms from the Tax tab (index 8).
        let enabled_forms_str = {
            use crate::tabs::tax::TaxTab;
            // SAFETY: We know tabs[8] is a TaxTab by construction in EntityContext::new.
            // We use downcast_ref here via Any — but since Tab doesn't implement Any,
            // we access enabled_forms indirectly through the Tab trait's tax method.
            // Instead, we read the form config from the entity TOML so we don't need a downcast.
            // Fall back to all forms enabled.
            let (toml_path, workspace_dir) = self.entity_toml_path();
            let entity_cfg =
                crate::config::load_entity_toml(&toml_path, &workspace_dir).unwrap_or_default();
            if let Some(tax_cfg) = entity_cfg.tax {
                if let Some(forms) = tax_cfg.enabled_forms {
                    forms
                } else {
                    TaxTab::all_form_tags_as_strings()
                }
            } else {
                TaxTab::all_form_tags_as_strings()
            }
        };

        // ── Build system prompt (same across all batches → cached) ────────────
        let system_prompt = build_batch_system_prompt(&enabled_forms_str);

        // ── Resolve pending JE details for prompting ──────────────────────────
        // Collect all JE header data and line details upfront.
        struct PendingJeInfo {
            je_id: crate::types::JournalEntryId,
            je_number: String,
            prompt_text: String,
        }

        let all_jes = self
            .entity
            .db
            .journals()
            .list(&JournalFilter::default())
            .unwrap_or_default();

        let mut je_infos: Vec<PendingJeInfo> = Vec::with_capacity(pending.len());
        for tag in &pending {
            let je_id = tag.journal_entry_id;
            let Some(je) = all_jes.iter().find(|j| j.id == je_id) else {
                continue;
            };
            let (_, lines) = match self.entity.db.journals().get_with_lines(je_id) {
                Ok(r) => r,
                Err(_) => continue,
            };

            let mut line_parts: Vec<String> = Vec::new();
            for line in &lines {
                let account = self
                    .entity
                    .db
                    .accounts()
                    .get_by_id(line.account_id)
                    .map(|a| format!("{} {}", a.number, a.name))
                    .unwrap_or_else(|_| format!("account#{}", i64::from(line.account_id)));
                if !line.debit_amount.is_zero() {
                    line_parts.push(format!("  Debit {} {}", account, line.debit_amount));
                }
                if !line.credit_amount.is_zero() {
                    line_parts.push(format!("  Credit {} {}", account, line.credit_amount));
                }
            }

            let memo_str = je
                .memo
                .as_deref()
                .map(|m| format!(" — {m}"))
                .unwrap_or_default();
            let prompt_text = format!(
                "{} | {}{}\n{}",
                je.je_number,
                je.entry_date,
                memo_str,
                line_parts.join("\n")
            );

            je_infos.push(PendingJeInfo {
                je_id,
                je_number: je.je_number.clone(),
                prompt_text,
            });
        }

        if je_infos.is_empty() {
            self.status_bar
                .set_message("No valid JEs found for batch review.".to_string());
            return;
        }

        // ── Process batches ───────────────────────────────────────────────────
        let total = je_infos.len();
        let batch_count = total.div_ceil(BATCH_SIZE);
        let mut confirmed_count = 0usize;
        let mut parse_fail_count = 0usize;

        let Some(client) = self.ai_client.take() else {
            self.status_bar
                .set_error("AI client not available.".to_string());
            return;
        };

        for (batch_idx, batch) in je_infos.chunks(BATCH_SIZE).enumerate() {
            // Force render so user sees progress.
            self.status_bar.set_ai_status(Some(format!(
                "Classifying batch {}/{batch_count}...",
                batch_idx + 1
            )));
            let _ = terminal.draw(|frame| self.render_frame(frame));

            let user_text = build_batch_user_prompt(batch.iter().map(|j| j.prompt_text.as_str()));

            let messages = vec![ApiMessage {
                role: ApiRole::User,
                content: ApiContent::Text(user_text),
            }];

            let response_text = match client.send_cached_simple(&system_prompt, &messages) {
                Ok(text) => text,
                Err(e) => {
                    tracing::warn!(batch = batch_idx, error = %e, "Batch review API call failed");
                    self.status_bar
                        .set_error(format!("Batch {} failed: {e}", batch_idx + 1));
                    continue;
                }
            };

            // Log AiTaxReview audit entry for this batch.
            let _ = self.entity.db.audit().append(
                AuditAction::AiTaxReview,
                &self.entity.name,
                None,
                None,
                &format!(
                    "Tax batch review, batch {}/{}: {} JEs",
                    batch_idx + 1,
                    batch_count,
                    batch.len()
                ),
            );

            // ── Parse response ────────────────────────────────────────────────
            // Expected format per line:
            //   JE-0004: schedule_c | Office supplies are ordinary business expenses
            for line in response_text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                // Split at first `:` to get je_number and rest.
                let Some((je_prefix, rest)) = line.split_once(':') else {
                    tracing::warn!(response_line = %line, "Batch parse: no colon separator");
                    parse_fail_count += 1;
                    continue;
                };

                let je_number = je_prefix.trim();
                let rest = rest.trim();

                // Find matching JE in this batch.
                let Some(je_info) = batch
                    .iter()
                    .find(|j| j.je_number.eq_ignore_ascii_case(je_number))
                else {
                    tracing::warn!(je_number, "Batch parse: JE not found in batch");
                    parse_fail_count += 1;
                    continue;
                };

                // Split at first `|` to get form_tag and reason.
                let (form_tag_str, reason) =
                    if let Some((tag_part, reason_part)) = rest.split_once('|') {
                        (tag_part.trim(), reason_part.trim())
                    } else {
                        (rest, "")
                    };

                // Parse form tag.
                let form_tag: TaxFormTag = match form_tag_str.parse() {
                    Ok(f) => f,
                    Err(_) => {
                        tracing::warn!(
                            je_number,
                            tag = form_tag_str,
                            "Batch parse: unrecognised form_tag"
                        );
                        parse_fail_count += 1;
                        continue;
                    }
                };

                // Update the database.
                if let Err(e) =
                    self.entity
                        .db
                        .tax_tags()
                        .set_ai_suggested(je_info.je_id, form_tag, reason)
                {
                    tracing::warn!(je_number, error = %e, "Failed to save AI suggestion");
                    parse_fail_count += 1;
                } else {
                    confirmed_count += 1;
                }
            }
        }

        // Return client.
        self.ai_client = Some(client);

        // ── Refresh UI and show summary ───────────────────────────────────────
        self.status_bar.set_ai_status(None);

        for tab in &mut self.entity.tabs {
            tab.refresh(&self.entity.db);
        }

        let failed = total - confirmed_count;
        if parse_fail_count > 0 || failed > 0 {
            self.status_bar.set_message(format!(
                "AI review complete: {confirmed_count}/{total} classified. \
                 {parse_fail_count} parse errors (still queued)."
            ));
        } else {
            self.status_bar.set_message(format!(
                "AI review complete: {confirmed_count} JEs classified."
            ));
        }
    }
}

// ── Prompt Builders ───────────────────────────────────────────────────────────

/// Builds the system prompt for batch tax classification.
///
/// The system prompt is identical across batches, so it gets cached by
/// the Anthropic API (`cache_control: ephemeral` via `send_cached_simple`).
fn build_batch_system_prompt(enabled_forms: &[String]) -> String {
    let form_list: String = enabled_forms
        .iter()
        .filter_map(|s| {
            s.parse::<TaxFormTag>()
                .ok()
                .map(|f| format!("  - {} ({}): {}", s, f.display_name(), f.description()))
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are a tax classification assistant. For each journal entry provided, \
         classify it with the most appropriate IRS tax form from the enabled list below.\n\n\
         Enabled Tax Forms:\n{form_list}\n\n\
         Output exactly one line per journal entry in this format:\n\
         JE-XXXX: form_tag | One sentence reason for this classification\n\n\
         Rules:\n\
         - Use the exact form_tag string (e.g. schedule_c, non_deductible).\n\
         - If no deduction applies, use non_deductible.\n\
         - Output ONLY the classification lines, no preamble or summary.\n\
         - One line per JE, in the same order as input."
    )
}

/// Builds the user message listing JE details for a batch.
fn build_batch_user_prompt<'a>(je_texts: impl Iterator<Item = &'a str>) -> String {
    let mut out = String::from("Classify these journal entries:\n\n");
    for text in je_texts {
        out.push_str(text);
        out.push_str("\n\n");
    }
    out.push_str("Respond with one classification line per JE.");
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_system_prompt_contains_non_deductible_rule() {
        let forms = vec!["schedule_c".to_string(), "non_deductible".to_string()];
        let prompt = build_batch_system_prompt(&forms);
        assert!(
            prompt.contains("non_deductible"),
            "should mention non_deductible"
        );
        assert!(prompt.contains("JE-XXXX"), "should show format example");
    }

    #[test]
    fn batch_system_prompt_skips_unknown_form_tags() {
        // Unknown tags don't crash — they're just silently omitted.
        let forms = vec!["schedule_c".to_string(), "totally_unknown".to_string()];
        let prompt = build_batch_system_prompt(&forms);
        assert!(prompt.contains("schedule_c"));
        assert!(!prompt.contains("totally_unknown"));
    }

    #[test]
    fn batch_user_prompt_includes_je_texts() {
        let texts = [
            "JE-0001 | 2026-01-15 — Office supplies",
            "JE-0002 | 2026-01-16 — Utilities",
        ];
        let prompt = build_batch_user_prompt(texts.iter().copied());
        assert!(prompt.contains("JE-0001"));
        assert!(prompt.contains("JE-0002"));
        assert!(prompt.contains("Classify these journal entries"));
    }

    #[test]
    fn batch_user_prompt_one_je() {
        let prompt = build_batch_user_prompt(std::iter::once("JE-0005 | 2026-02-01 — Rent"));
        assert!(prompt.contains("JE-0005"));
    }

    /// Verify that the pipe-separator parsing logic works correctly.
    #[test]
    fn pipe_parse_extracts_form_and_reason() {
        let line = "JE-0004: schedule_c | Office supplies are ordinary business expenses";
        let (je_prefix, rest) = line.split_once(':').unwrap();
        assert_eq!(je_prefix.trim(), "JE-0004");
        let (tag, reason) = rest.trim().split_once('|').unwrap();
        assert_eq!(tag.trim(), "schedule_c");
        assert_eq!(
            reason.trim(),
            "Office supplies are ordinary business expenses"
        );
    }

    #[test]
    fn pipe_parse_handles_missing_pipe() {
        // When no pipe: treat whole rest as form_tag, reason is empty.
        let line = "JE-0005: non_deductible";
        let (_, rest) = line.split_once(':').unwrap();
        let rest = rest.trim();
        let (tag, reason) = if let Some((t, r)) = rest.split_once('|') {
            (t.trim(), r.trim())
        } else {
            (rest, "")
        };
        assert_eq!(tag, "non_deductible");
        assert_eq!(reason, "");
    }

    #[test]
    fn parse_fails_gracefully_for_no_colon() {
        // Lines without a colon are parse failures — should not panic.
        let line = "some garbage line without colon";
        let result = line.split_once(':');
        assert!(result.is_none());
    }
}
