//! Income Statement report.
//!
//! Revenue − Expenses over a date range. Grouped by account type section with subtotals.
//! Net income = Revenue total − Expense total.

use anyhow::Result;
use chrono::Local;

use crate::db::EntityDb;
use crate::types::{AccountType, Money};

use super::{Align, Report, ReportParams, format_header, format_money, format_table};

pub struct IncomeStatement;

impl Report for IncomeStatement {
    fn name(&self) -> &str {
        "IncomeStatement"
    }

    fn generate(&self, db: &EntityDb, params: &ReportParams) -> Result<String> {
        let today = Local::now().date_naive();
        let (start, end) = params.date_range.unwrap_or((today, today));

        let accounts = db.accounts().list_all()?;

        // Get all balances restricted to this date range.
        // We use get_all_balances_as_of filtered by start too, by subtracting the pre-start
        // balances. Alternatively, query per-account with the date range.
        // For simplicity we use per-account queries via get_balance_for_date_range.
        // To avoid N+1 at scale, use a single bulk query.
        let end_balances = db.accounts().get_all_balances_as_of(end)?;
        // Pre-start balances (the day before start).
        let pre_start = start.pred_opt().unwrap_or(start);
        let pre_balances = db.accounts().get_all_balances_as_of(pre_start)?;

        let date_label = format!(
            "{} – {}",
            start.format("%B %-d, %Y"),
            end.format("%B %-d, %Y")
        );
        let header = format_header(&params.entity_name, "Income Statement", &date_label);

        let headers = ["Account", "Amount"];
        let alignments = [Align::Left, Align::Right];

        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut total_revenue = Money(0);
        let mut total_expenses = Money(0);

        for &acct_type in &[AccountType::Revenue, AccountType::Expense] {
            let section_accounts: Vec<_> = accounts
                .iter()
                .filter(|a| a.account_type == acct_type && !a.is_placeholder && a.is_active)
                .collect();

            rows.push(vec![format!("── {} ──", acct_type), String::new()]);

            let mut section_total = Money(0);

            for account in &section_accounts {
                let end_bal = end_balances.get(&account.id).copied().unwrap_or(Money(0));
                let pre_bal = pre_balances.get(&account.id).copied().unwrap_or(Money(0));
                // Period balance = change over the period.
                let period_raw = end_bal - pre_bal;

                // For credit-normal accounts (Revenue), a credit balance is positive.
                // For debit-normal accounts (Expense), a debit balance is positive.
                let display = match acct_type {
                    AccountType::Revenue => -period_raw, // flip: credit increase = positive
                    AccountType::Expense => period_raw,  // debit increase = positive
                    _ => period_raw,
                };

                section_total = section_total + display;

                if !display.is_zero() {
                    rows.push(vec![format!("  {}", account.name), format_money(display)]);
                }
            }

            rows.push(vec![
                format!("Total {}", acct_type),
                format_money(section_total),
            ]);
            rows.push(vec![String::new(), String::new()]);

            match acct_type {
                AccountType::Revenue => total_revenue = section_total,
                AccountType::Expense => total_expenses = section_total,
                _ => {}
            }
        }

        let net_income = total_revenue - total_expenses;

        rows.push(vec!["NET INCOME".to_owned(), format_money(net_income)]);

        let table = format_table(&headers, &rows, &alignments);
        Ok(format!("{}\n{}", header, table))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::account_repo::NewAccount;
    use crate::db::entity_db_from_conn;
    use crate::db::journal_repo::{NewJournalEntry, NewJournalEntryLine};
    use crate::db::schema::initialize_schema;
    use crate::services::journal::post_journal_entry;
    use crate::types::{AccountId, FiscalPeriodId};
    use chrono::NaiveDate;
    use rusqlite::Connection;

    fn make_db() -> (crate::db::EntityDb, FiscalPeriodId) {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema");
        let db = entity_db_from_conn(conn);
        let fy = db.fiscal().create_fiscal_year(1, 2026).expect("fy");
        let periods = db.fiscal().list_periods(fy).expect("periods");
        (db, periods[0].id)
    }

    fn create_account(
        db: &crate::db::EntityDb,
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
        db: &crate::db::EntityDb,
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
    fn income_statement_report_name() {
        assert_eq!(IncomeStatement.name(), "IncomeStatement");
    }

    #[test]
    fn income_statement_contains_sections() {
        let (db, _) = make_db();
        let params = make_params(
            "Test Co",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        );
        let output = IncomeStatement.generate(&db, &params).expect("generate");
        assert!(output.contains("Income Statement"), "title missing");
        assert!(output.contains("Revenue"), "revenue section missing");
        assert!(output.contains("Expense"), "expense section missing");
        assert!(output.contains("NET INCOME"), "net income missing");
    }

    #[test]
    fn income_statement_net_income_equals_revenue_minus_expenses() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Cash", AccountType::Asset);
        let revenue = create_account(&db, "4100", "Revenue", AccountType::Revenue);
        let expense = create_account(&db, "5100", "Expense", AccountType::Expense);

        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        post_je(
            &db,
            period_id,
            date,
            cash,
            revenue,
            Money::from_dollars(1_000.0),
        );
        post_je(
            &db,
            period_id,
            date,
            expense,
            cash,
            Money::from_dollars(400.0),
        );

        let params = make_params(
            "Acme",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
        );
        let output = IncomeStatement.generate(&db, &params).expect("generate");

        // Net income = 1000 - 400 = 600
        let net_line = output
            .lines()
            .find(|l| l.contains("NET INCOME"))
            .expect("NET INCOME row missing");
        assert!(net_line.contains("600.00"), "net income should be 600.00");
    }

    #[test]
    fn income_statement_has_box_drawing_chars() {
        let (db, _) = make_db();
        let params = make_params(
            "Test",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        );
        let output = IncomeStatement.generate(&db, &params).expect("generate");
        assert!(output.contains('┌'));
        assert!(output.contains('│'));
    }
}
