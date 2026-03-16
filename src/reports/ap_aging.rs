//! AP Aging report.
//!
//! Open payables grouped by aging buckets: Current, 1-30, 31-60, 61-90, 90+ days past due.
//! Same structure as AR Aging.

use anyhow::Result;
use chrono::Local;

use crate::db::EntityDb;
use crate::db::ap_repo::ApFilter;
use crate::types::{ArApStatus, Money};

use super::ar_aging::aging_bucket;
use super::{Align, Report, ReportParams, format_header, format_money, format_table}; // reuse the shared bucket function

pub struct ApAging;

impl Report for ApAging {
    fn name(&self) -> &str {
        "ApAging"
    }

    fn generate(&self, db: &EntityDb, params: &ReportParams) -> Result<String> {
        let as_of = params
            .as_of_date
            .unwrap_or_else(|| Local::now().date_naive());

        let date_label = as_of.format("As of %B %-d, %Y").to_string();
        let header = format_header(&params.entity_name, "Accounts Payable Aging", &date_label);

        // Get all open/partial AP items.
        let all_items = db.ap().list(&ApFilter { status: None })?;
        let items: Vec<_> = all_items
            .iter()
            .filter(|i| i.status != ArApStatus::Paid)
            .collect();

        let headers = ["Vendor", "Description", "Due Date", "Bucket", "Amount"];
        let alignments = [
            Align::Left,
            Align::Left,
            Align::Left,
            Align::Left,
            Align::Right,
        ];

        let bucket_names = [
            "Current",
            "1-30 Days",
            "31-60 Days",
            "61-90 Days",
            "90+ Days",
        ];
        let mut bucket_totals: [Money; 5] = [Money(0); 5];

        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut grand_total = Money(0);

        for item in &items {
            let days_past_due = (as_of - item.due_date).num_days();
            let bucket = aging_bucket(days_past_due);

            // For partial items, compute outstanding balance.
            let outstanding = if item.status == ArApStatus::Partial {
                let (_, payments) = db.ap().get_with_payments(item.id)?;
                let paid: Money = payments
                    .iter()
                    .map(|p| p.amount)
                    .fold(Money(0), |a, b| a + b);
                item.amount - paid
            } else {
                item.amount
            };

            grand_total = grand_total + outstanding;

            if let Some(idx) = bucket_names.iter().position(|&b| b == bucket) {
                bucket_totals[idx] = bucket_totals[idx] + outstanding;
            }

            rows.push(vec![
                item.vendor_name.clone(),
                item.description.clone().unwrap_or_default(),
                item.due_date.format("%Y-%m-%d").to_string(),
                bucket.to_owned(),
                format_money(outstanding),
            ]);
        }

        // Bucket summary rows.
        rows.push(vec![String::new(); 5]);
        for (name, total) in bucket_names.iter().zip(bucket_totals.iter()) {
            rows.push(vec![
                format!("  {}", name),
                String::new(),
                String::new(),
                String::new(),
                format_money(*total),
            ]);
        }
        rows.push(vec![
            "TOTAL".to_owned(),
            String::new(),
            String::new(),
            String::new(),
            format_money(grand_total),
        ]);

        let table = format_table(&headers, &rows, &alignments);
        Ok(format!("{}\n{}", header, table))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::account_repo::NewAccount;
    use crate::db::ap_repo::NewApItem;
    use crate::db::entity_db_from_conn;
    use crate::db::journal_repo::{NewJournalEntry, NewJournalEntryLine};
    use crate::db::schema::initialize_schema;
    use crate::services::journal::post_journal_entry;
    use crate::types::{AccountId, AccountType, FiscalPeriodId, JournalEntryId};
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

    fn create_je(
        db: &crate::db::EntityDb,
        period_id: FiscalPeriodId,
        date: NaiveDate,
        debit_id: AccountId,
        credit_id: AccountId,
    ) -> JournalEntryId {
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
                        debit_amount: Money::from_dollars(100.0),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: credit_id,
                        debit_amount: Money(0),
                        credit_amount: Money::from_dollars(100.0),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create draft");
        post_journal_entry(db, je_id, "Test").expect("post");
        je_id
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
    fn ap_aging_report_name() {
        assert_eq!(ApAging.name(), "ApAging");
    }

    #[test]
    fn ap_aging_contains_required_labels() {
        let (db, _) = make_db();
        let params = make_params("Test Co", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = ApAging.generate(&db, &params).expect("generate");
        assert!(output.contains("Accounts Payable Aging"), "title missing");
        assert!(output.contains("TOTAL"), "total row missing");
    }

    #[test]
    fn ap_aging_buckets_overdue_item() {
        let (db, period_id) = make_db();
        let expense = create_account(&db, "5100", "Expense", AccountType::Expense);
        let ap_account = create_account(&db, "2100", "AP", AccountType::Liability);

        let jan1 = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let je_id = create_je(&db, period_id, jan1, expense, ap_account);

        // Due Jan 1 — by March 31 = 89 days past due.
        db.ap()
            .create_item(&NewApItem {
                account_id: ap_account,
                vendor_name: "Vendor X".to_owned(),
                description: None,
                amount: Money::from_dollars(500.0),
                due_date: jan1,
                originating_je_id: je_id,
            })
            .expect("create AP item");

        let params = make_params("Acme", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = ApAging.generate(&db, &params).expect("generate");

        assert!(output.contains("Vendor X"), "vendor missing");
        assert!(output.contains("500.00"), "amount missing");
        assert!(output.contains("61-90 Days"), "wrong bucket");
    }

    #[test]
    fn ap_aging_has_box_drawing_chars() {
        let (db, _) = make_db();
        let params = make_params("Test", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = ApAging.generate(&db, &params).expect("generate");
        assert!(output.contains('┌'));
    }
}
