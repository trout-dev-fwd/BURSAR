//! Tax Summary by Form report.
//!
//! Groups confirmed journal entries by tax form with per-entry reasons and subtotals.
//! Non-deductible and unreviewed entries are shown as counts only.

use anyhow::Result;
use chrono::Local;
use std::collections::HashMap;

use crate::db::EntityDb;
use crate::types::{TaxFormTag, TaxReviewStatus};

use super::{Report, ReportParams, format_header, format_money};

pub struct TaxSummary;

impl Report for TaxSummary {
    fn name(&self) -> &str {
        "TaxSummary"
    }

    fn generate(&self, db: &EntityDb, params: &ReportParams) -> Result<String> {
        let today = Local::now().date_naive();
        let (start, end) = params.date_range.unwrap_or((today, today));

        let all_jes = db.tax_tags().list_all_posted_for_date_range(start, end)?;

        // Partition entries by status.
        let mut confirmed_by_form: HashMap<TaxFormTag, Vec<ConfirmedEntry>> = HashMap::new();
        let mut non_deductible_count: usize = 0;
        let mut unreviewed_count: usize = 0;

        for je in &all_jes {
            match &je.tag {
                Some(tag) if tag.status == TaxReviewStatus::Confirmed => {
                    if let Some(form) = tag.form_tag {
                        confirmed_by_form
                            .entry(form)
                            .or_default()
                            .push(ConfirmedEntry {
                                date: je.entry_date.format("%b %-d").to_string(),
                                je_number: je.je_number.clone(),
                                memo: je.memo.clone().unwrap_or_default(),
                                amount: je.total_debits,
                                reason: tag.reason.clone(),
                            });
                    }
                }
                Some(tag) if tag.status == TaxReviewStatus::NonDeductible => {
                    non_deductible_count += 1;
                }
                _ => {
                    // Unreviewed, ai_pending, ai_suggested all count as unreviewed for report.
                    unreviewed_count += 1;
                }
            }
        }

        let date_label = format!(
            "{} \u{2013} {}",
            start.format("%B %-d, %Y"),
            end.format("%B %-d, %Y")
        );
        let header = format_header(&params.entity_name, "Tax Summary by Form", &date_label);

        // Width for the separator line: match the header width.
        let header_width = header
            .lines()
            .next()
            .map(|l| l.chars().count())
            .unwrap_or(60);
        let sep: String = "\u{2500}".repeat(header_width);

        let mut lines: Vec<String> = Vec::new();
        lines.push(header);
        lines.push(String::new());

        // Output confirmed sections in canonical form order.
        let form_order = TaxFormTag::all();
        let mut grand_total = crate::types::Money(0);

        // Compute column widths across all confirmed entries.
        let date_w = confirmed_by_form
            .values()
            .flat_map(|v| v.iter())
            .map(|e| e.date.chars().count())
            .max()
            .unwrap_or(6)
            .max(6);
        let je_w = confirmed_by_form
            .values()
            .flat_map(|v| v.iter())
            .map(|e| e.je_number.chars().count())
            .max()
            .unwrap_or(7)
            .max(7);
        let memo_w = confirmed_by_form
            .values()
            .flat_map(|v| v.iter())
            .map(|e| e.memo.chars().count().min(40))
            .max()
            .unwrap_or(20)
            .max(20);
        let amt_w = confirmed_by_form
            .values()
            .flat_map(|v| v.iter())
            .map(|e| format_money(e.amount).chars().count())
            .max()
            .unwrap_or(10)
            .max(10);

        let mut any_confirmed = false;

        for form in &form_order {
            if *form == TaxFormTag::NonDeductible {
                continue;
            }
            let Some(entries) = confirmed_by_form.get(form) else {
                continue;
            };

            any_confirmed = true;

            let section_title = format!("{} \u{2014} {}", form.display_name(), form.description());
            lines.push(section_title);
            lines.push(sep.clone());

            let mut section_total = crate::types::Money(0);

            for entry in entries {
                let memo_display: String = entry.memo.chars().take(40).collect();
                let amount_str = format_money(entry.amount);
                let date_col = format!("{:<width$}", entry.date, width = date_w);
                let je_col = format!("{:<width$}", entry.je_number, width = je_w);
                let memo_col = format!("{:<width$}", memo_display, width = memo_w);
                let amt_col = format!("{:>width$}", amount_str, width = amt_w);
                lines.push(format!(
                    "  {}  {}  {}  {}",
                    date_col, je_col, memo_col, amt_col
                ));

                let reason_text = entry.reason.as_deref().unwrap_or("(no reason given)");
                let indent = " ".repeat(2 + date_w + 2 + je_w + 2);
                lines.push(format!("{}Reason: {}", indent, reason_text));
                section_total = section_total + entry.amount;
                grand_total = grand_total + entry.amount;
            }

            let total_label = "Total:";
            let total_str = format_money(section_total);
            let right_edge = 2 + date_w + 2 + je_w + 2 + memo_w + 2 + amt_w;
            let total_line_len = total_label.len() + 1 + total_str.len();
            let pad = " ".repeat(right_edge.saturating_sub(total_line_len));
            lines.push(format!("{}{}  {}", pad, total_label, total_str));
            lines.push(String::new());
        }

        if any_confirmed {
            let total_label = "Grand Total (confirmed):";
            let grand_str = format_money(grand_total);
            let right_edge = 2 + date_w + 2 + je_w + 2 + memo_w + 2 + amt_w;
            let total_line_len = total_label.len() + 2 + grand_str.len();
            let pad = " ".repeat(right_edge.saturating_sub(total_line_len));
            lines.push(format!("{}{}  {}", pad, total_label, grand_str));
            lines.push(sep.clone());
            lines.push(String::new());
        }

        // Counts section.
        lines.push(format!(
            "  Non-Deductible:  {} {}",
            non_deductible_count,
            if non_deductible_count == 1 {
                "entry (not listed)"
            } else {
                "entries (not listed)"
            }
        ));
        lines.push(format!(
            "  Unreviewed:      {} {}",
            unreviewed_count,
            if unreviewed_count == 1 {
                "entry"
            } else {
                "entries"
            }
        ));
        lines.push(String::new());

        // End of report marker.
        let marker = "\u{2014} End of Report \u{2014}";
        let marker_len = marker.chars().count();
        let pad = " ".repeat(header_width.saturating_sub(marker_len) / 2);
        lines.push(format!("{}{}", pad, marker));

        Ok(lines.join("\n"))
    }
}

// ── Internal data struct ───────────────────────────────────────────────────────

struct ConfirmedEntry {
    date: String,
    je_number: String,
    memo: String,
    amount: crate::types::Money,
    reason: Option<String>,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::EntityDb;
    use crate::db::journal_repo::{JournalRepo, NewJournalEntry, NewJournalEntryLine};
    use crate::reports::ReportParams;
    use crate::types::{AccountId, Money, TaxFormTag};
    use chrono::NaiveDate;

    fn make_db() -> EntityDb {
        let db = EntityDb::open_in_memory().expect("in-memory db");
        db.fiscal().create_fiscal_year(1, 2026).expect("fy");
        db
    }

    fn params(start: &str, end: &str) -> ReportParams {
        ReportParams {
            entity_name: "Test LLC".to_owned(),
            as_of_date: None,
            date_range: Some((
                NaiveDate::parse_from_str(start, "%Y-%m-%d").unwrap(),
                NaiveDate::parse_from_str(end, "%Y-%m-%d").unwrap(),
            )),
            account_id: None,
        }
    }

    fn post_je_with_memo(db: &EntityDb, date: &str, memo: &str, amount: Money) -> i64 {
        let conn = db.conn();
        let period_id: i64 = conn
            .query_row(
                "SELECT id FROM fiscal_periods WHERE period_number = 1",
                [],
                |r| r.get(0),
            )
            .expect("period");
        let acct1: i64 = conn
            .query_row("SELECT id FROM accounts WHERE number = '1110'", [], |r| {
                r.get(0)
            })
            .expect("acct1");
        let acct2: i64 = conn
            .query_row("SELECT id FROM accounts WHERE number = '4100'", [], |r| {
                r.get(0)
            })
            .expect("acct2");

        use crate::types::FiscalPeriodId;
        let entry = NewJournalEntry {
            entry_date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            memo: Some(memo.to_owned()),
            fiscal_period_id: FiscalPeriodId::from(period_id),
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: AccountId::from(acct1),
                    debit_amount: amount,
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: AccountId::from(acct2),
                    debit_amount: Money(0),
                    credit_amount: amount,
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };
        let je_id = JournalRepo::new(conn)
            .create_draft(&entry)
            .expect("create_draft");
        conn.execute(
            "UPDATE journal_entries SET status = 'Posted' WHERE id = ?1",
            rusqlite::params![i64::from(je_id)],
        )
        .expect("post");
        i64::from(je_id)
    }

    #[test]
    fn empty_data_generates_report() {
        let db = make_db();
        let report = TaxSummary;
        let output = report
            .generate(&db, &params("2026-01-01", "2026-12-31"))
            .expect("generate");
        assert!(output.contains("Tax Summary by Form"));
        assert!(output.contains("Non-Deductible:  0 entries"));
        assert!(output.contains("Unreviewed:      0 entries"));
        assert!(output.contains("End of Report"));
    }

    #[test]
    fn confirmed_entry_appears_with_reason() {
        let db = make_db();
        let je_id = post_je_with_memo(&db, "2026-01-15", "Office supplies", Money(6_799_000_000));

        use crate::types::JournalEntryId;
        db.tax_tags()
            .set_manual(
                JournalEntryId::from(je_id),
                TaxFormTag::ScheduleC,
                Some("Printer paper and ink"),
            )
            .expect("tag");

        let report = TaxSummary;
        let output = report
            .generate(&db, &params("2026-01-01", "2026-12-31"))
            .expect("generate");

        assert!(output.contains("Schedule C"), "section header missing");
        assert!(output.contains("Office supplies"), "memo missing");
        assert!(output.contains("Printer paper and ink"), "reason missing");
        assert!(output.contains("67.99"), "amount missing");
    }

    #[test]
    fn non_deductible_shown_as_count() {
        let db = make_db();
        let je_id = post_je_with_memo(&db, "2026-02-01", "Personal expense", Money(10_000_000_000));

        use crate::types::JournalEntryId;
        db.tax_tags()
            .set_non_deductible(JournalEntryId::from(je_id), Some("Personal"))
            .expect("tag");

        let report = TaxSummary;
        let output = report
            .generate(&db, &params("2026-01-01", "2026-12-31"))
            .expect("generate");

        assert!(output.contains("Non-Deductible:  1 entry"), "count missing");
        // Should NOT list the JE individually.
        assert!(
            !output.contains("Personal expense"),
            "non-deductible JE should not be listed"
        );
    }

    #[test]
    fn unreviewed_shown_as_count() {
        let db = make_db();
        post_je_with_memo(&db, "2026-03-01", "Unknown expense", Money(5_000_000_000));

        let report = TaxSummary;
        let output = report
            .generate(&db, &params("2026-01-01", "2026-12-31"))
            .expect("generate");

        assert!(output.contains("Unreviewed:      1 entry"), "count missing");
    }

    #[test]
    fn multiple_forms_grouped_with_subtotals() {
        let db = make_db();

        let je1 = post_je_with_memo(&db, "2026-01-15", "Supplies", Money(24_500_000_000));
        let je2 = post_je_with_memo(&db, "2026-01-20", "More supplies", Money(6_799_000_000));
        let je3 = post_je_with_memo(&db, "2026-03-15", "Donation", Money(50_000_000_000));

        use crate::types::JournalEntryId;
        let repo = db.tax_tags();
        repo.set_manual(
            JournalEntryId::from(je1),
            TaxFormTag::ScheduleC,
            Some("Business"),
        )
        .expect("tag1");
        repo.set_manual(
            JournalEntryId::from(je2),
            TaxFormTag::ScheduleC,
            Some("Office"),
        )
        .expect("tag2");
        repo.set_manual(
            JournalEntryId::from(je3),
            TaxFormTag::ScheduleACharity,
            Some("Annual donation"),
        )
        .expect("tag3");

        let report = TaxSummary;
        let output = report
            .generate(&db, &params("2026-01-01", "2026-12-31"))
            .expect("generate");

        assert!(output.contains("Schedule C"), "Schedule C section missing");
        assert!(output.contains("Schedule A"), "Schedule A section missing");
        // Both Schedule C entries should appear.
        assert!(output.contains("Supplies"), "je1 memo missing");
        assert!(output.contains("More supplies"), "je2 memo missing");
        assert!(output.contains("Donation"), "je3 memo missing");
        // Subtotals: Schedule C = 245.00 + 67.99 = 312.99
        assert!(output.contains("312.99"), "Schedule C subtotal missing");
        // Subtotal: Schedule A Charity = 500.00
        assert!(output.contains("500.00"), "Schedule A subtotal missing");
    }

    #[test]
    fn entries_outside_date_range_excluded() {
        let db = make_db();
        let je_in = post_je_with_memo(&db, "2026-06-15", "In range", Money(10_000_000_000));
        let je_out = post_je_with_memo(&db, "2026-12-31", "Out of range", Money(20_000_000_000));

        use crate::types::JournalEntryId;
        let repo = db.tax_tags();
        repo.set_manual(JournalEntryId::from(je_in), TaxFormTag::ScheduleC, None)
            .expect("tag1");
        repo.set_manual(JournalEntryId::from(je_out), TaxFormTag::ScheduleC, None)
            .expect("tag2");

        let report = TaxSummary;
        let output = report
            .generate(&db, &params("2026-01-01", "2026-11-30"))
            .expect("generate");

        assert!(output.contains("In range"), "in-range JE missing");
        assert!(
            !output.contains("Out of range"),
            "out-of-range JE should not appear"
        );
    }
}
