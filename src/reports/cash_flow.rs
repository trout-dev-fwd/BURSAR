//! Cash Flow Statement report (direct method).
//!
//! Reports cash inflows and outflows over a date range.
//! Identifies cash accounts by name convention (containing "cash", "bank", "checking", "savings").
//! Net cash change must match the change in the cash account balance over the period.

use anyhow::Result;
use chrono::Local;

use crate::db::EntityDb;
use crate::db::journal_repo::DateRange;
use crate::types::Money;

use super::{Align, Report, ReportParams, format_header, format_money, format_table};

pub struct CashFlow;

/// Returns true if an account name looks like a cash/bank account.
fn is_cash_account_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("cash")
        || lower.contains("bank")
        || lower.contains("checking")
        || lower.contains("savings")
}

impl Report for CashFlow {
    fn name(&self) -> &str {
        "CashFlow"
    }

    fn generate(&self, db: &EntityDb, params: &ReportParams) -> Result<String> {
        let today = Local::now().date_naive();
        let (start, end) = params.date_range.unwrap_or((today, today));

        let date_label = format!(
            "{} – {}",
            start.format("%B %-d, %Y"),
            end.format("%B %-d, %Y")
        );
        let header = format_header(&params.entity_name, "Cash Flow Statement", &date_label);

        // Find all cash/bank accounts.
        let accounts = db.accounts().list_all()?;
        let cash_accounts: Vec<_> = accounts
            .iter()
            .filter(|a| !a.is_placeholder && a.is_active && is_cash_account_name(&a.name))
            .collect();

        let headers = ["Date", "JE #", "Description", "Inflow", "Outflow"];
        let alignments = [
            Align::Left,
            Align::Left,
            Align::Left,
            Align::Right,
            Align::Right,
        ];

        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut total_inflow = Money(0);
        let mut total_outflow = Money(0);

        for acct in &cash_accounts {
            let lines = db.journals().list_lines_for_account(
                acct.id,
                Some(DateRange {
                    from: Some(start),
                    to: Some(end),
                }),
            )?;

            if lines.is_empty() {
                continue;
            }

            // Section header per cash account.
            rows.push(vec![
                format!("── {} ──", acct.name),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ]);

            for line in &lines {
                let (inflow_str, outflow_str) = if line.debit.0 > 0 {
                    // Cash received (debit to cash account).
                    total_inflow = total_inflow + line.debit;
                    (format_money(line.debit), String::new())
                } else {
                    // Cash paid out (credit to cash account).
                    total_outflow = total_outflow + line.credit;
                    (String::new(), format_money(line.credit))
                };

                rows.push(vec![
                    line.entry_date.format("%Y-%m-%d").to_string(),
                    line.je_number.clone(),
                    line.memo.clone().unwrap_or_default(),
                    inflow_str,
                    outflow_str,
                ]);
            }

            rows.push(vec![String::new(); 5]);
        }

        // Net cash change row.
        let net = total_inflow - total_outflow;
        rows.push(vec![
            String::new(),
            String::new(),
            "TOTAL INFLOW".to_owned(),
            format_money(total_inflow),
            String::new(),
        ]);
        rows.push(vec![
            String::new(),
            String::new(),
            "TOTAL OUTFLOW".to_owned(),
            String::new(),
            format_money(total_outflow),
        ]);
        rows.push(vec![
            String::new(),
            String::new(),
            "NET CASH CHANGE".to_owned(),
            format_money(net),
            String::new(),
        ]);

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

    fn make_params(entity: &str, start: NaiveDate, end: NaiveDate) -> ReportParams {
        ReportParams {
            entity_name: entity.to_owned(),
            as_of_date: None,
            date_range: Some((start, end)),
            account_id: None,
        }
    }

    #[test]
    fn cash_flow_report_name() {
        assert_eq!(CashFlow.name(), "CashFlow");
    }

    #[test]
    fn cash_flow_contains_required_labels() {
        let (db, _) = make_db();
        let params = make_params(
            "Test Co",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        );
        let output = CashFlow.generate(&db, &params).expect("generate");
        assert!(output.contains("Cash Flow Statement"), "title missing");
        assert!(output.contains("NET CASH CHANGE"), "net cash label missing");
    }

    #[test]
    fn cash_flow_shows_inflows_and_outflows() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Checking Account", AccountType::Asset);
        let revenue = create_account(&db, "4100", "Revenue", AccountType::Revenue);
        let expense_payable = create_account(&db, "2100", "AP", AccountType::Liability);

        let date = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        // Cash inflow: debit cash, credit revenue.
        post_je(
            &db,
            period_id,
            date,
            cash,
            revenue,
            Money::from_dollars(800.0),
        );
        // Cash outflow: debit AP (payment), credit cash.
        post_je(
            &db,
            period_id,
            date,
            expense_payable,
            cash,
            Money::from_dollars(200.0),
        );

        let params = make_params(
            "Acme",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
        );
        let output = CashFlow.generate(&db, &params).expect("generate");

        assert!(output.contains("800.00"), "inflow amount missing");
        assert!(output.contains("200.00"), "outflow amount missing");
        // Net = 800 - 200 = 600
        let net_line = output
            .lines()
            .find(|l| l.contains("NET CASH CHANGE"))
            .expect("NET CASH CHANGE missing");
        assert!(net_line.contains("600.00"), "net cash should be 600.00");
    }

    #[test]
    fn cash_flow_has_box_drawing_chars() {
        let (db, _) = make_db();
        let params = make_params(
            "Test",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        );
        let output = CashFlow.generate(&db, &params).expect("generate");
        assert!(output.contains('┌'));
        assert!(output.contains('│'));
    }
}
