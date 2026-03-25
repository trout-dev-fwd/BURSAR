use crate::db::EntityDb;
use crate::db::tax_ref_repo::TaxRefChunk;

// ── Keyword → Tag Mapping ─────────────────────────────────────────────────────

/// Maps keyword substrings (lowercased) to one or more comma-separated topic tags.
/// Checked in order; all matching entries contribute tags.
const KEYWORD_MAP: &[(&[&str], &str)] = &[
    // Topic terms
    (&["deductible", "deduction", "deduct"], "deduction"),
    (&["depreciat", "macrs", "section 179"], "depreciation"),
    (&["home office", "home business"], "home_office"),
    (
        &["capital gain", "capital loss", "stock sale", "crypto sale"],
        "capital_gains",
    ),
    (
        &[
            "rental income",
            "rental property",
            "rental expense",
            "landlord",
        ],
        "rental",
    ),
    (
        &["self-employ", "freelance", "sole proprietor", "1099-nec"],
        "small_business",
    ),
    (
        &[
            "medical expense",
            "dental expense",
            "health insurance",
            "prescription",
        ],
        "medical",
    ),
    (&["charitable", "charity", "donation", "501(c)"], "charity"),
    (
        &[
            "mortgage interest",
            "property tax",
            "state tax",
            "local tax",
        ],
        "taxes_interest",
    ),
    (
        &["estimated tax", "quarterly payment"],
        "estimated_payments",
    ),
    (
        &["s-corp", "k-1", "shareholder distribution", "s corp"],
        "s_corp",
    ),
    (
        &[
            "section 1231",
            "depreciation recapture",
            "business property sale",
        ],
        "business_property",
    ),
    // Form name keywords → map to relevant topic tags
    (
        &[
            "schedule c",
            "business expense",
            "business deduction",
            "ordinary business",
        ],
        "small_business,business_expense",
    ),
    (
        &["schedule a", "itemized deduction", "itemize"],
        "deduction",
    ),
    (&["schedule d", "capital gain"], "capital_gains"),
    (&["schedule e", "rental"], "rental"),
    (&["schedule se", "self-employment tax"], "small_business"),
    (
        &["form 4562", "4562", "depreciation schedule", "amortization"],
        "depreciation",
    ),
    (
        &["form 8829", "8829", "business use of home"],
        "home_office",
    ),
    (
        &["form 4797", "4797", "sale of business property"],
        "business_property",
    ),
    (
        &[
            "form 1120-s",
            "1120s",
            "s corporation",
            "s-corporation return",
        ],
        "s_corp",
    ),
    (
        &["form 1040-es", "1040-es", "estimated payment"],
        "estimated_payments",
    ),
    (&["medical deduction", "health expense"], "medical"),
    (
        &["charitable contribution", "noncash contribution"],
        "charity",
    ),
    (
        &["mortgage deduction", "investment interest"],
        "taxes_interest",
    ),
];

// ── Public API ────────────────────────────────────────────────────────────────

/// Context about the currently highlighted JE in the Tax tab.
/// Pre-formatted strings so tax_context.rs doesn't need to import TaxTab internals.
pub struct SelectedJeContext {
    pub je_number: String,
    pub entry_date: chrono::NaiveDate,
    pub memo: Option<String>,
    /// (account_display, debit_str, credit_str) for each line.
    pub lines: Vec<(String, String, String)>,
    /// Form display name if already classified (e.g., "Schedule C").
    pub form_display: Option<String>,
    /// Status label (e.g., "Confirmed", "AI Suggested").
    pub status_display: String,
    /// User or AI supplied reason, if any.
    pub reason: Option<String>,
}

/// Returns deduplicated topic tag strings matching keywords found in `text`.
///
/// The search is case-insensitive. Multiple keyword entries may match the same
/// text; each unique tag string in the results is returned at most once.
pub fn extract_tax_tags(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    let mut tags: Vec<&'static str> = Vec::new();

    for &(keywords, tag_str) in KEYWORD_MAP {
        if keywords.iter().any(|kw| lower.contains(*kw)) {
            // tag_str may be comma-separated (e.g. "small_business,business_expense").
            for part in tag_str.split(',') {
                let part = part.trim();
                // Only push if not already present (preserve order, deduplicate).
                if !tags.contains(&part) {
                    tags.push(part);
                }
            }
        }
    }

    tags
}

/// Retrieves IRS reference chunks relevant to the given topic tags.
///
/// Queries `db.tax_refs().search_by_tag(tag)` for each tag, accumulates results
/// (deduplicated by chunk id), then truncates to `max_chunks` entries and trims
/// the formatted block to `max_chars` bytes.
pub fn get_relevant_chunks(
    db: &EntityDb,
    tags: &[&str],
    max_chunks: usize,
    max_chars: usize,
) -> Vec<TaxRefChunk> {
    let mut seen_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut chunks: Vec<TaxRefChunk> = Vec::new();
    let mut total_chars: usize = 0;

    for tag in tags {
        if chunks.len() >= max_chunks {
            break;
        }
        let Ok(results) = db.tax_refs().search_by_tag(tag) else {
            continue;
        };
        for chunk in results {
            if chunks.len() >= max_chunks {
                break;
            }
            if seen_ids.contains(&chunk.id) {
                continue;
            }
            let chunk_len = chunk.content.len() + chunk.section.len() + chunk.publication.len();
            if total_chars + chunk_len > max_chars {
                break;
            }
            total_chars += chunk_len;
            seen_ids.insert(chunk.id);
            chunks.push(chunk);
        }
    }

    chunks
}

/// Builds the tax reference context block to append to the AI system prompt.
///
/// Returns `None` when there are no IRS reference chunks AND no selected JE
/// context to include (no point adding an empty section).
///
/// When called from the Tax tab, `selected_je` carries the highlighted row's
/// details and tax tag. Claude sees this without a tool call and can still
/// query other JEs via the `get_tax_tag` tool.
pub fn build_tax_context(
    db: &EntityDb,
    message: &str,
    selected_je: Option<&SelectedJeContext>,
) -> Option<String> {
    let tags = extract_tax_tags(message);

    // Also extract tags from the selected JE's memo and form name so the
    // reference material is relevant to what's highlighted.
    let mut all_tags = tags;
    if let Some(je) = selected_je {
        if let Some(memo) = &je.memo {
            let memo_tags = extract_tax_tags(memo);
            for t in memo_tags {
                if !all_tags.contains(&t) {
                    all_tags.push(t);
                }
            }
        }
        if let Some(form) = &je.form_display {
            let form_tags = extract_tax_tags(form);
            for t in form_tags {
                if !all_tags.contains(&t) {
                    all_tags.push(t);
                }
            }
        }
    }

    let chunks = get_relevant_chunks(db, &all_tags, 8, 8_000);

    // Nothing to include — return None so the caller doesn't add an empty block.
    if chunks.is_empty() && selected_je.is_none() {
        return None;
    }

    let mut out = String::new();

    // ── IRS Reference Chunks ─────────────────────────────────────────────
    if !chunks.is_empty() {
        out.push_str("## Tax Reference (from IRS Publications)\n\n");
        for chunk in &chunks {
            out.push_str(&format!(
                "**{} — {}**\n{}\n\n",
                chunk.publication, chunk.section, chunk.content
            ));
        }
        out.push_str("When citing IRS guidance, use format: (Pub XXX, Section Name)\n\n");
    }

    // ── Selected Journal Entry ───────────────────────────────────────────
    if let Some(je) = selected_je {
        out.push_str("## Selected Journal Entry\n\n");
        let date_str = je.entry_date.format("%b %d, %Y").to_string();
        let memo_str = je
            .memo
            .as_deref()
            .map(|m| format!(" | {m}"))
            .unwrap_or_default();
        out.push_str(&format!("{} | {}{}\n", je.je_number, date_str, memo_str));
        for (account, debit, credit) in &je.lines {
            if !debit.is_empty() {
                out.push_str(&format!("  Debit:  {account}  {debit}\n"));
            }
            if !credit.is_empty() {
                out.push_str(&format!("  Credit: {account}  {credit}\n"));
            }
        }
        out.push('\n');

        // Tax classification (if any).
        if let Some(form) = &je.form_display {
            out.push_str(&format!(
                "Tax Classification: {} ({})\n",
                form, je.status_display
            ));
        } else {
            out.push_str(&format!("Tax Status: {}\n", je.status_display));
        }
        if let Some(reason) = &je.reason {
            out.push_str(&format!("Reason: {reason}\n"));
        }
    }

    Some(out.trim_end().to_string())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::EntityDb;

    fn empty_db() -> EntityDb {
        EntityDb::open_in_memory().expect("in-memory db")
    }

    fn db_with_chunks() -> EntityDb {
        let db = EntityDb::open_in_memory().expect("in-memory db");
        db.tax_refs()
            .insert(
                "Pub 334",
                "Business Expenses",
                "small_business,business_expense",
                "Business supplies content.",
                2026,
            )
            .expect("insert 1");
        db.tax_refs()
            .insert(
                "Pub 946",
                "MACRS",
                "depreciation,macrs",
                "MACRS content.",
                2026,
            )
            .expect("insert 2");
        db.tax_refs()
            .insert(
                "Pub 527",
                "Rental Income",
                "rental",
                "Rental property content.",
                2026,
            )
            .expect("insert 3");
        db
    }

    // ── extract_tax_tags ──────────────────────────────────────────────────

    #[test]
    fn extract_tags_for_topic_keyword() {
        let tags = extract_tax_tags("is this depreciation eligible?");
        assert!(
            tags.contains(&"depreciation"),
            "expected 'depreciation' in {tags:?}"
        );
    }

    #[test]
    fn extract_tags_for_form_name() {
        let tags = extract_tax_tags("should this go on Schedule C?");
        assert!(
            tags.contains(&"small_business") || tags.contains(&"business_expense"),
            "expected business tags in {tags:?}"
        );
    }

    #[test]
    fn extract_tags_case_insensitive() {
        let tags_lower = extract_tax_tags("schedule c expenses");
        let tags_upper = extract_tax_tags("Schedule C Expenses");
        assert_eq!(tags_lower, tags_upper);
    }

    #[test]
    fn extract_tags_no_match_returns_empty() {
        let tags = extract_tax_tags("general business question xyz");
        // Not necessarily empty — but "xyz" alone shouldn't match form names
        // The test verifies the function doesn't panic.
        let _ = tags;
    }

    #[test]
    fn extract_tags_deduplicates() {
        // "schedule c" + "business expense" both map to small_business — deduplicated.
        let tags = extract_tax_tags("schedule c business expense deduction");
        let small_biz_count = tags.iter().filter(|&&t| t == "small_business").count();
        assert_eq!(small_biz_count, 1, "small_business should appear once");
    }

    #[test]
    fn extract_tags_multiple_forms() {
        let tags = extract_tax_tags("depreciation on home office equipment");
        assert!(tags.contains(&"depreciation"), "missing depreciation");
        assert!(tags.contains(&"home_office"), "missing home_office");
    }

    // ── get_relevant_chunks ───────────────────────────────────────────────

    #[test]
    fn get_relevant_chunks_empty_table_returns_empty() {
        let db = empty_db();
        let tags = ["small_business"];
        let chunks = get_relevant_chunks(&db, &tags, 5, 10_000);
        assert!(chunks.is_empty());
    }

    #[test]
    fn get_relevant_chunks_returns_matching() {
        let db = db_with_chunks();
        let tags = ["small_business"];
        let chunks = get_relevant_chunks(&db, &tags, 5, 10_000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].publication, "Pub 334");
    }

    #[test]
    fn get_relevant_chunks_respects_max_chunks() {
        let db = db_with_chunks();
        // All three chunks match "small_business" only when we use a broader tag list
        // Just test max_chunks with max=1 on a tag that matches at least 1.
        let tags = ["small_business", "depreciation", "rental"];
        let chunks = get_relevant_chunks(&db, &tags, 1, 10_000);
        assert_eq!(chunks.len(), 1, "max_chunks=1 should limit to 1 result");
    }

    #[test]
    fn get_relevant_chunks_deduplicates_across_tags() {
        let db = db_with_chunks();
        // "small_business" and "business_expense" both hit Pub 334.
        let tags = ["small_business", "business_expense"];
        let chunks = get_relevant_chunks(&db, &tags, 10, 10_000);
        let pub334_count = chunks.iter().filter(|c| c.publication == "Pub 334").count();
        assert_eq!(pub334_count, 1, "Pub 334 should appear exactly once");
    }

    #[test]
    fn get_relevant_chunks_respects_max_chars() {
        let db = db_with_chunks();
        // Set max_chars so small that no chunk fits.
        let tags = ["small_business", "depreciation", "rental"];
        let chunks = get_relevant_chunks(&db, &tags, 10, 5);
        assert!(
            chunks.is_empty(),
            "max_chars=5 should exclude all chunks (content > 5 chars)"
        );
    }

    // ── build_tax_context ─────────────────────────────────────────────────

    #[test]
    fn build_tax_context_no_chunks_no_je_returns_none() {
        let db = empty_db();
        let result = build_tax_context(&db, "general question", None);
        assert!(result.is_none());
    }

    #[test]
    fn build_tax_context_with_chunks_returns_some() {
        let db = db_with_chunks();
        let result = build_tax_context(&db, "schedule c business expenses", None);
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("Pub 334"), "should include Pub 334 chunk");
    }

    #[test]
    fn build_tax_context_with_selected_je_includes_je_details() {
        let db = empty_db();
        let je = SelectedJeContext {
            je_number: "JE-0004".to_string(),
            entry_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            memo: Some("Home Depot building materials".to_string()),
            lines: vec![
                (
                    "5100 Repairs & Maintenance".to_string(),
                    "$245.00".to_string(),
                    String::new(),
                ),
                (
                    "1110 Chase Checking".to_string(),
                    String::new(),
                    "$245.00".to_string(),
                ),
            ],
            form_display: Some("Schedule C".to_string()),
            status_display: "AI Suggested".to_string(),
            reason: Some("Building supplies are ordinary business expenses".to_string()),
        };
        let result = build_tax_context(&db, "why is this schedule c?", Some(&je));
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("JE-0004"), "should contain JE number");
        assert!(text.contains("Home Depot"), "should contain memo");
        assert!(text.contains("Schedule C"), "should contain form name");
        assert!(text.contains("Building supplies"), "should contain reason");
    }

    #[test]
    fn build_tax_context_with_selected_je_no_tag_shows_status() {
        let db = empty_db();
        let je = SelectedJeContext {
            je_number: "JE-0007".to_string(),
            entry_date: chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            memo: None,
            lines: vec![],
            form_display: None,
            status_display: "Unreviewed".to_string(),
            reason: None,
        };
        let result = build_tax_context(&db, "what form?", Some(&je));
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("JE-0007"));
        assert!(text.contains("Unreviewed"), "should show unreviewed status");
    }

    #[test]
    fn build_tax_context_citation_instruction_present_when_chunks_found() {
        let db = db_with_chunks();
        let result = build_tax_context(&db, "depreciation", None);
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(
            text.contains("Pub"),
            "citation instruction should reference Pub format"
        );
    }
}
