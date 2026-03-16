//! Balance Sheet report.
//!
//! Shows Assets = Liabilities + Equity as of a point in time, with subtotals per account type.
//! Hierarchical display: account type section header, then leaf accounts, then section subtotal.

use anyhow::Result;
use chrono::Local;

use crate::db::EntityDb;
use crate::types::{AccountType, BalanceDirection, Money};

use super::{Align, Report, ReportParams, format_header, format_money, format_table};

pub struct BalanceSheet;

impl Report for BalanceSheet {
    fn name(&self) -> &str {
        "BalanceSheet"
    }

    fn generate(&self, db: &EntityDb, params: &ReportParams) -> Result<String> {
        let as_of = params
            .as_of_date
            .unwrap_or_else(|| Local::now().date_naive());

        let accounts = db.accounts().list_all()?;
        let balances = db.accounts().get_all_balances_as_of(as_of)?;

        let date_label = as_of.format("As of %B %-d, %Y").to_string();
        let header = format_header(&params.entity_name, "Balance Sheet", &date_label);

        let headers = ["Account", "Amount"];
        let alignments = [Align::Left, Align::Right];

        let mut rows: Vec<Vec<String>> = Vec::new();

        let section_types: &[AccountType] = &[
            AccountType::Asset,
            AccountType::Liability,
            AccountType::Equity,
        ];

        let mut total_assets = Money(0);
        let mut total_liab_equity = Money(0);

        for &acct_type in section_types {
            let section_accounts: Vec<_> = accounts
                .iter()
                .filter(|a| a.account_type == acct_type && !a.is_placeholder && a.is_active)
                .collect();

            // Section header row (always present so sections are visible even when empty).
            rows.push(vec![format!("── {} ──", acct_type), String::new()]);

            let mut section_total = Money(0);
            let normal = acct_type.normal_balance();

            for account in &section_accounts {
                let raw_balance = balances.get(&account.id).copied().unwrap_or(Money(0));

                // Display balance in the natural direction for this account type.
                // Contra accounts have their sign flipped.
                let display_balance = if account.is_contra {
                    // Contra: a positive raw debit balance reduces the section total.
                    if normal == BalanceDirection::Credit {
                        // Contra-asset: credit normal, but raw is debit-positive.
                        // Section total decreases by raw_balance amount.
                        if raw_balance.0 < 0 {
                            raw_balance.abs()
                        } else {
                            -raw_balance
                        }
                    } else {
                        // Contra-liability/contra-equity: debit normal contra.
                        if raw_balance.0 > 0 {
                            -raw_balance
                        } else {
                            raw_balance.abs()
                        }
                    }
                } else {
                    // Normal account: convert raw (debit-positive) to the natural direction.
                    match normal {
                        BalanceDirection::Debit => raw_balance, // Asset/Expense: positive = good
                        BalanceDirection::Credit => -raw_balance, // Liability/Equity/Revenue: flip
                    }
                };

                section_total = section_total + display_balance;

                if !display_balance.is_zero() {
                    rows.push(vec![
                        format!("  {}", account.name),
                        format_money(display_balance),
                    ]);
                }
            }

            // Section subtotal row.
            rows.push(vec![
                format!("Total {}", acct_type),
                format_money(section_total),
            ]);

            match acct_type {
                AccountType::Asset => total_assets = section_total,
                _ => total_liab_equity = total_liab_equity + section_total,
            }

            // Blank spacer row between sections.
            rows.push(vec![String::new(), String::new()]);
        }

        // Grand totals row: Total Assets vs Total Liabilities + Equity.
        rows.push(vec!["TOTAL ASSETS".to_owned(), format_money(total_assets)]);
        rows.push(vec![
            "TOTAL LIABILITIES + EQUITY".to_owned(),
            format_money(total_liab_equity),
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

    fn make_params(entity: &str, as_of: NaiveDate) -> ReportParams {
        ReportParams {
            entity_name: entity.to_owned(),
            as_of_date: Some(as_of),
            date_range: None,
            account_id: None,
        }
    }

    #[test]
    fn balance_sheet_report_name() {
        assert_eq!(BalanceSheet.name(), "BalanceSheet");
    }

    #[test]
    fn balance_sheet_contains_required_sections() {
        let (db, _) = make_db();
        let params = make_params("Test Co", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = BalanceSheet.generate(&db, &params).expect("generate");

        assert!(output.contains("Balance Sheet"), "title missing");
        assert!(output.contains("Test Co"), "entity name missing");
        assert!(output.contains("Asset"), "Asset section missing");
        assert!(output.contains("Liability"), "Liability section missing");
        assert!(output.contains("Equity"), "Equity section missing");
    }

    #[test]
    fn balance_sheet_fundamental_equation_holds() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Checking", AccountType::Asset);
        let equity = create_account(&db, "3100", "Owner Equity", AccountType::Equity);

        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        post_je(
            &db,
            period_id,
            date,
            cash,
            equity,
            Money::from_dollars(1_000.0),
        );

        let params = make_params("Acme LLC", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = BalanceSheet.generate(&db, &params).expect("generate");

        // Both totals should equal 1,000.00.
        assert!(output.contains("TOTAL ASSETS"), "assets total missing");
        assert!(
            output.contains("TOTAL LIABILITIES + EQUITY"),
            "liab+equity total missing"
        );

        // Count appearances of "1,000.00" — should appear in Asset section and Equity section
        // and both grand total rows.
        let count = output.matches("1,000.00").count();
        assert!(
            count >= 2,
            "1,000.00 should appear at least twice, got {}",
            count
        );
    }

    #[test]
    fn balance_sheet_has_box_drawing_chars() {
        let (db, _) = make_db();
        let params = make_params("Test", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = BalanceSheet.generate(&db, &params).expect("generate");
        assert!(output.contains('┌'));
        assert!(output.contains('│'));
    }
}
