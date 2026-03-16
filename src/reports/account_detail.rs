//! Account Detail report.
//!
//! Lists all posted transactions for one account within a date range, with running balance.
//! Matches the GL tab display for the same account and date range.

use anyhow::{Result, bail};
use chrono::Local;

use crate::db::EntityDb;
use crate::db::journal_repo::DateRange;
use crate::types::Money;

use super::{Align, Report, ReportParams, format_header, format_money, format_table};

pub struct AccountDetail;

impl Report for AccountDetail {
    fn name(&self) -> &str {
        "AccountDetail"
    }

    fn generate(&self, db: &EntityDb, params: &ReportParams) -> Result<String> {
        let account_id = match params.account_id {
            Some(id) => id,
            None => bail!("AccountDetail report requires account_id in ReportParams"),
        };

        let today = Local::now().date_naive();
        let (start, end) = params.date_range.unwrap_or((today, today));

        let account = db.accounts().get_by_id(account_id)?;
        let lines = db.journals().list_lines_for_account(
            account_id,
            Some(DateRange {
                from: Some(start),
                to: Some(end),
            }),
        )?;

        let date_label = format!(
            "{} – {}",
            start.format("%B %-d, %Y"),
            end.format("%B %-d, %Y")
        );
        let title = format!("Account Detail: {} {}", account.number, account.name);
        let header = format_header(&params.entity_name, &title, &date_label);

        let headers = ["Date", "JE #", "Description", "Debit", "Credit", "Balance"];
        let alignments = [
            Align::Left,
            Align::Left,
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
        ];

        let rows: Vec<Vec<String>> = lines
            .iter()
            .map(|row| {
                vec![
                    row.entry_date.format("%Y-%m-%d").to_string(),
                    row.je_number.clone(),
                    row.memo.clone().unwrap_or_default(),
                    if row.debit.is_zero() {
                        String::new()
                    } else {
                        format_money(row.debit)
                    },
                    if row.credit.is_zero() {
                        String::new()
                    } else {
                        format_money(row.credit)
                    },
                    format_money(row.running_balance),
                ]
            })
            .collect();

        // Summary row: total debits and credits.
        let total_debit: Money = lines.iter().map(|r| r.debit).fold(Money(0), |a, b| a + b);
        let total_credit: Money = lines.iter().map(|r| r.credit).fold(Money(0), |a, b| a + b);
        let mut all_rows = rows;
        all_rows.push(vec![
            String::new(),
            String::new(),
            "TOTAL".to_owned(),
            format_money(total_debit),
            format_money(total_credit),
            String::new(),
        ]);

        let table = format_table(&headers, &all_rows, &alignments);
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
    use crate::types::{AccountId, AccountType, FiscalPeriodId};
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
                memo: Some("Test".to_owned()),
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

    fn make_params(
        entity: &str,
        account_id: AccountId,
        start: NaiveDate,
        end: NaiveDate,
    ) -> ReportParams {
        ReportParams {
            entity_name: entity.to_owned(),
            as_of_date: None,
            date_range: Some((start, end)),
            account_id: Some(account_id),
        }
    }

    #[test]
    fn account_detail_report_name() {
        assert_eq!(AccountDetail.name(), "AccountDetail");
    }

    #[test]
    fn account_detail_requires_account_id() {
        let (db, _) = make_db();
        let params = ReportParams {
            entity_name: "Test".to_owned(),
            as_of_date: None,
            date_range: None,
            account_id: None,
        };
        assert!(AccountDetail.generate(&db, &params).is_err());
    }

    #[test]
    fn account_detail_shows_transactions() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Checking", AccountType::Asset);
        let revenue = create_account(&db, "4100", "Revenue", AccountType::Revenue);

        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        post_je(
            &db,
            period_id,
            date,
            cash,
            revenue,
            Money::from_dollars(500.0),
        );

        let params = make_params(
            "Acme",
            cash,
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
        );
        let output = AccountDetail.generate(&db, &params).expect("generate");

        assert!(output.contains("Account Detail"), "title missing");
        assert!(output.contains("Checking"), "account name missing");
        assert!(output.contains("500.00"), "amount missing");
        assert!(output.contains("TOTAL"), "total row missing");
    }

    #[test]
    fn account_detail_running_balance_matches_gl() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Checking", AccountType::Asset);
        let revenue = create_account(&db, "4100", "Revenue", AccountType::Revenue);
        let expense = create_account(&db, "5100", "Expense", AccountType::Expense);

        let d1 = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        post_je(
            &db,
            period_id,
            d1,
            cash,
            revenue,
            Money::from_dollars(1_000.0),
        );
        post_je(
            &db,
            period_id,
            d2,
            expense,
            cash,
            Money::from_dollars(300.0),
        );

        let params = make_params(
            "Acme",
            cash,
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
        );
        let output = AccountDetail.generate(&db, &params).expect("generate");

        // After first line: balance = 1,000
        assert!(output.contains("1,000.00"), "first running balance missing");
        // After second line: balance = 700
        assert!(output.contains("700.00"), "second running balance missing");
    }

    #[test]
    fn account_detail_has_box_drawing_chars() {
        let (db, _) = make_db();
        let cash = create_account(&db, "1110", "Checking", AccountType::Asset);
        let params = make_params(
            "Test",
            cash,
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        );
        let output = AccountDetail.generate(&db, &params).expect("generate");
        assert!(output.contains('┌'));
        assert!(output.contains('│'));
    }
}
