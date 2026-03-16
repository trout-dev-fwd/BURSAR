//! Trial Balance report.
//!
//! Lists all non-placeholder accounts with non-zero balances (plus active accounts with zero
//! balance) as of a point in time. Inactive accounts with zero balance are omitted.
//! Final row shows total debits and total credits — they must always be equal.

use anyhow::Result;
use chrono::Local;

use crate::db::EntityDb;
use crate::types::Money;

use super::{Align, Report, ReportParams, format_header, format_money, format_table};

pub struct TrialBalance;

impl Report for TrialBalance {
    fn name(&self) -> &str {
        "TrialBalance"
    }

    fn generate(&self, db: &EntityDb, params: &ReportParams) -> Result<String> {
        let as_of = params
            .as_of_date
            .unwrap_or_else(|| Local::now().date_naive());

        // Load accounts and balances in two queries.
        let accounts = db.accounts().list_all()?;
        let balances = db.accounts().get_all_balances_as_of(as_of)?;

        let date_label = as_of.format("As of %B %-d, %Y").to_string();
        let header = format_header(&params.entity_name, "Trial Balance", &date_label);

        let headers = ["#", "Account Name", "Debit", "Credit"];
        let alignments = [Align::Left, Align::Left, Align::Right, Align::Right];

        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut total_debit = Money(0);
        let mut total_credit = Money(0);

        for account in &accounts {
            // Skip placeholder accounts — they cannot have posted balances.
            if account.is_placeholder {
                continue;
            }

            let balance = balances.get(&account.id).copied().unwrap_or(Money(0));

            // Skip inactive accounts with zero balance.
            if !account.is_active && balance.is_zero() {
                continue;
            }

            let (debit_str, credit_str) = if balance.0 > 0 {
                total_debit = total_debit + balance;
                (format_money(balance), String::new())
            } else if balance.0 < 0 {
                let abs_bal = balance.abs();
                total_credit = total_credit + abs_bal;
                (String::new(), format_money(abs_bal))
            } else {
                // Zero balance on an active account — show blanks.
                (String::new(), String::new())
            };

            rows.push(vec![
                account.number.clone(),
                account.name.clone(),
                debit_str,
                credit_str,
            ]);
        }

        // Totals row.
        rows.push(vec![
            String::new(),
            "TOTAL".to_owned(),
            format_money(total_debit),
            format_money(total_credit),
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
        let fy = db
            .fiscal()
            .create_fiscal_year(1, 2026)
            .expect("fiscal year");
        let periods = db.fiscal().list_periods(fy).expect("periods");
        let jan_period = periods[0].id;
        (db, jan_period)
    }

    fn create_account(
        db: &crate::db::EntityDb,
        number: &str,
        name: &str,
        account_type: AccountType,
        parent_id: Option<AccountId>,
    ) -> crate::db::account_repo::Account {
        let id = db
            .accounts()
            .create(&NewAccount {
                number: number.to_owned(),
                name: name.to_owned(),
                account_type,
                parent_id,
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create account");
        db.accounts().get_by_id(id).expect("get account")
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

    fn make_params(entity: &str, as_of: NaiveDate) -> ReportParams {
        ReportParams {
            entity_name: entity.to_owned(),
            as_of_date: Some(as_of),
            date_range: None,
            account_id: None,
        }
    }

    #[test]
    fn trial_balance_report_name() {
        assert_eq!(TrialBalance.name(), "TrialBalance");
    }

    #[test]
    fn trial_balance_empty_db_shows_zero_totals() {
        let (db, _) = make_db();
        let params = make_params("Test Co", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = TrialBalance.generate(&db, &params).expect("generate");

        assert!(output.contains("Trial Balance"), "title missing");
        assert!(output.contains("Test Co"), "entity name missing");
        assert!(output.contains("TOTAL"), "totals row missing");
        assert!(output.contains("0.00"), "zero totals missing");
    }

    #[test]
    fn trial_balance_balances_after_posting() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Checking", AccountType::Asset, None);
        let revenue = create_account(&db, "4100", "Revenue", AccountType::Revenue, None);

        let amount = Money::from_dollars(500.0);
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        post_je(&db, period_id, date, cash.id, revenue.id, amount);

        let params = make_params("Acme LLC", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = TrialBalance.generate(&db, &params).expect("generate");

        assert!(output.contains("Checking"), "cash account missing");
        assert!(output.contains("Revenue"), "revenue account missing");
        assert!(output.contains("500.00"), "amount missing");

        // Total debits should equal total credits (500.00 each).
        let total_line = output
            .lines()
            .find(|l| l.contains("TOTAL"))
            .expect("TOTAL row missing");
        assert_eq!(
            total_line.matches("500.00").count(),
            2,
            "debits should equal credits in TOTAL row"
        );
    }

    #[test]
    fn trial_balance_excludes_future_entries() {
        let (db, _period_id) = make_db();
        let cash = create_account(&db, "1110", "Cash", AccountType::Asset, None);
        let revenue = create_account(&db, "4100", "Revenue", AccountType::Revenue, None);

        // Post a JE in April period (which is within our fiscal year).
        let april_periods = db
            .fiscal()
            .list_periods(db.fiscal().list_fiscal_years().expect("fy")[0].id)
            .expect("periods");
        let april_period = april_periods[3].id; // period 4 = April
        let future_date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        post_je(
            &db,
            april_period,
            future_date,
            cash.id,
            revenue.id,
            Money::from_dollars(100.0),
        );

        let params = make_params("Test", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = TrialBalance.generate(&db, &params).expect("generate");

        // Totals should both be 0.00 (no entries before as_of).
        let total_line = output
            .lines()
            .find(|l| l.contains("TOTAL"))
            .expect("TOTAL row missing");
        assert!(
            total_line.contains("0.00"),
            "future entry should not appear in past trial balance"
        );
    }

    #[test]
    fn trial_balance_skips_inactive_zero_balance_accounts() {
        let (db, _) = make_db();
        let cash = create_account(&db, "1110", "Cash", AccountType::Asset, None);
        // Deactivate the account.
        db.accounts().deactivate(cash.id).expect("deactivate");

        let params = make_params("Test", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = TrialBalance.generate(&db, &params).expect("generate");

        // Deactivated account with zero balance should NOT appear.
        assert!(
            !output.contains("Cash"),
            "inactive zero-balance account should be excluded"
        );
    }

    #[test]
    fn trial_balance_has_box_drawing_chars() {
        let (db, _) = make_db();
        let params = make_params("Test", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = TrialBalance.generate(&db, &params).expect("generate");
        assert!(output.contains('┌'), "missing TL corner");
        assert!(output.contains('│'), "missing vertical bar");
        assert!(output.contains('─'), "missing horizontal bar");
    }
}
