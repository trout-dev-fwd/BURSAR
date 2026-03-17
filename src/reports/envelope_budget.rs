//! Envelope Budget Summary report.
//!
//! Shows all accounts with envelope allocations, their allocation percentage,
//! total earmarked (fills/transfers/reversals in the period), GL spending for
//! the period, and available balance (Earmarked − GL Balance).
//!
//! Sorted by account number. Includes a totals row and an "Unallocated" line
//! showing the percentage of revenue not yet assigned to any envelope.

use anyhow::Result;

use crate::db::EntityDb;
use crate::types::{Money, Percentage};

use super::{Align, Report, ReportParams, format_header, format_money, format_table};

pub struct EnvelopeBudgetSummary;

impl Report for EnvelopeBudgetSummary {
    fn name(&self) -> &str {
        "EnvelopeBudgetSummary"
    }

    fn generate(&self, db: &EntityDb, params: &ReportParams) -> Result<String> {
        let today = chrono::Local::now().date_naive();
        let (start, end) = params.date_range.unwrap_or((today, today));

        let date_label = format!(
            "{} – {}",
            start.format("%B %-d, %Y"),
            end.format("%B %-d, %Y")
        );
        let header = format_header(&params.entity_name, "Envelope Budget Summary", &date_label);

        // Load all allocations and accounts.
        let allocations = db.envelopes().get_all_allocations()?;
        let all_accounts = db.accounts().list_all()?;

        // Build a map from AccountId → Account for quick lookup.
        let account_map: std::collections::HashMap<_, _> =
            all_accounts.iter().map(|a| (a.id, a)).collect();

        // Collect rows for allocated accounts, sorted by account number.
        let mut alloc_rows: Vec<_> = allocations
            .iter()
            .filter_map(|alloc| {
                account_map
                    .get(&alloc.account_id)
                    .map(|acct| (alloc, *acct))
            })
            .collect();
        alloc_rows.sort_by(|(_, a), (_, b)| a.number.cmp(&b.number));

        let headers = [
            "Account #",
            "Account Name",
            "Allocation %",
            "Earmarked",
            "GL Balance",
            "Available",
        ];
        let alignments = [
            Align::Left,
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];

        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut total_earmarked = Money(0);
        let mut total_gl = Money(0);
        let mut total_available = Money(0);
        let mut total_alloc_pct = Percentage(0);

        for (alloc, account) in &alloc_rows {
            let earmarked = db
                .envelopes()
                .get_balance_for_date_range(account.id, start, end)?;
            let gl_balance = db
                .accounts()
                .get_balance_for_date_range(account.id, start, end)?;
            let available = earmarked - gl_balance;

            total_earmarked = total_earmarked + earmarked;
            total_gl = total_gl + gl_balance;
            total_available = total_available + available;
            total_alloc_pct = Percentage(total_alloc_pct.0 + alloc.percentage.0);

            rows.push(vec![
                account.number.clone(),
                account.name.clone(),
                alloc.percentage.to_string(),
                format_money(earmarked),
                format_money(gl_balance),
                format_money(available),
            ]);
        }

        // Totals row.
        rows.push(vec![
            String::new(),
            "TOTAL".to_owned(),
            total_alloc_pct.to_string(),
            format_money(total_earmarked),
            format_money(total_gl),
            format_money(total_available),
        ]);

        // Unallocated line: 100% minus sum of all allocation percentages.
        let unallocated_pct = Percentage(
            Percentage::from_display(100.0)
                .0
                .saturating_sub(total_alloc_pct.0),
        );
        rows.push(vec![
            String::new(),
            "Unallocated".to_owned(),
            unallocated_pct.to_string(),
            String::new(),
            String::new(),
            String::new(),
        ]);

        let table = format_table(&headers, &rows, &alignments);
        Ok(format!("{}\n{}", header, table))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::account_repo::NewAccount;
    use crate::db::entity_db_from_conn;
    use crate::db::journal_repo::{NewJournalEntry, NewJournalEntryLine};
    use crate::db::schema::initialize_schema;
    use crate::services::journal::post_journal_entry;
    use crate::types::{AccountId, AccountType, FiscalPeriodId};
    use chrono::NaiveDate;
    use rusqlite::Connection;

    fn make_db() -> (EntityDb, FiscalPeriodId) {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema");
        let db = entity_db_from_conn(conn);
        let fy = db.fiscal().create_fiscal_year(1, 2026).expect("fy");
        let periods = db.fiscal().list_periods(fy).expect("periods");
        (db, periods[0].id)
    }

    fn create_account(
        db: &EntityDb,
        number: &str,
        name: &str,
        account_type: AccountType,
    ) -> AccountId {
        db.accounts()
            .create(&NewAccount {
                number: number.to_owned(),
                name: name.to_owned(),
                account_type,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create account")
    }

    fn post_je(
        db: &EntityDb,
        period_id: FiscalPeriodId,
        date: NaiveDate,
        debit_id: AccountId,
        credit_id: AccountId,
        amount: Money,
    ) {
        let je_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: date,
                memo: None,
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: debit_id,
                        debit_amount: amount,
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: credit_id,
                        debit_amount: Money(0),
                        credit_amount: amount,
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create draft");
        post_journal_entry(db, je_id, "Test Entity").expect("post");
    }

    fn make_params(entity: &str, start: NaiveDate, end: NaiveDate) -> ReportParams {
        ReportParams {
            entity_name: entity.to_owned(),
            as_of_date: None,
            date_range: Some((start, end)),
            account_id: None,
        }
    }

    #[test]
    fn envelope_budget_report_name() {
        assert_eq!(EnvelopeBudgetSummary.name(), "EnvelopeBudgetSummary");
    }

    #[test]
    fn envelope_budget_empty_when_no_allocations() {
        let (db, _) = make_db();
        let params = make_params(
            "Test Co",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        );
        let output = EnvelopeBudgetSummary
            .generate(&db, &params)
            .expect("generate");
        assert!(output.contains("Envelope Budget Summary"), "title missing");
        assert!(output.contains("TOTAL"), "totals row missing");
        assert!(output.contains("Unallocated"), "unallocated row missing");
        // Unallocated should be 100% when nothing is allocated.
        assert!(output.contains("100.00%"), "unallocated should be 100%");
    }

    #[test]
    fn envelope_budget_shows_allocated_accounts() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Checking", AccountType::Asset);
        let expense = create_account(&db, "5100", "Maintenance", AccountType::Expense);
        let revenue = create_account(&db, "4100", "Rental Income", AccountType::Revenue);

        // Set a 25% allocation on the expense account.
        db.envelopes()
            .set_allocation(expense, Percentage::from_display(25.0))
            .expect("set allocation");

        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        // Record income (cash debit → revenue credit). This triggers envelope fill.
        post_je(
            &db,
            period_id,
            date,
            cash,
            revenue,
            Money::from_dollars(1_000.0),
        );

        // Record expense spending.
        post_je(
            &db,
            period_id,
            date,
            expense,
            cash,
            Money::from_dollars(100.0),
        );

        let params = make_params(
            "Acme",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        );
        let output = EnvelopeBudgetSummary
            .generate(&db, &params)
            .expect("generate");

        assert!(output.contains("5100"), "account number missing");
        assert!(output.contains("Maintenance"), "account name missing");
        assert!(output.contains("25.00%"), "allocation % missing");
        // GL balance should show 100.00 (the expense debit).
        assert!(output.contains("100.00"), "gl balance missing");
    }

    #[test]
    fn envelope_budget_unallocated_decreases_with_allocations() {
        let (db, _) = make_db();
        let expense = create_account(&db, "5100", "Maintenance", AccountType::Expense);

        db.envelopes()
            .set_allocation(expense, Percentage::from_display(30.0))
            .expect("set allocation");

        let params = make_params(
            "Acme",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        );
        let output = EnvelopeBudgetSummary
            .generate(&db, &params)
            .expect("generate");

        // 100% - 30% = 70% unallocated.
        assert!(output.contains("70.00%"), "unallocated should be 70%");
    }

    #[test]
    fn envelope_budget_has_box_drawing_chars() {
        let (db, _) = make_db();
        let params = make_params(
            "Test",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        );
        let output = EnvelopeBudgetSummary
            .generate(&db, &params)
            .expect("generate");
        assert!(output.contains('┌'));
        assert!(output.contains('│'));
        assert!(output.contains("— End of Report —"));
    }

    #[test]
    fn envelope_budget_sorted_by_account_number() {
        let (db, _) = make_db();
        let b_acct = create_account(&db, "5200", "Insurance", AccountType::Expense);
        let a_acct = create_account(&db, "5100", "Maintenance", AccountType::Expense);

        db.envelopes()
            .set_allocation(b_acct, Percentage::from_display(10.0))
            .expect("set b");
        db.envelopes()
            .set_allocation(a_acct, Percentage::from_display(15.0))
            .expect("set a");

        let params = make_params(
            "Acme",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        );
        let output = EnvelopeBudgetSummary
            .generate(&db, &params)
            .expect("generate");

        let pos_5100 = output.find("5100").expect("5100 missing");
        let pos_5200 = output.find("5200").expect("5200 missing");
        assert!(pos_5100 < pos_5200, "5100 should appear before 5200");
    }
}
