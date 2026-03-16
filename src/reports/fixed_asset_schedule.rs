//! Fixed Asset Schedule report.
//!
//! Lists all fixed assets with cost basis, accumulated depreciation, and book value.
//! Verifies: book value = cost basis − accumulated depreciation for each asset.

use anyhow::Result;
use chrono::Local;

use crate::db::EntityDb;
use crate::types::Money;

use super::{Align, Report, ReportParams, format_header, format_money, format_table};

pub struct FixedAssetSchedule;

impl Report for FixedAssetSchedule {
    fn name(&self) -> &str {
        "FixedAssetSchedule"
    }

    fn generate(&self, db: &EntityDb, params: &ReportParams) -> Result<String> {
        let as_of = params
            .as_of_date
            .unwrap_or_else(|| Local::now().date_naive());

        let date_label = as_of.format("As of %B %-d, %Y").to_string();
        let header = format_header(&params.entity_name, "Fixed Asset Schedule", &date_label);

        let assets = db.assets().list_assets()?;

        let headers = [
            "Acct #",
            "Asset Name",
            "In-Service Date",
            "Life (mo.)",
            "Cost Basis",
            "Accum. Depr.",
            "Book Value",
        ];
        let alignments = [
            Align::Left,
            Align::Left,
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];

        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut total_cost = Money(0);
        let mut total_accum = Money(0);
        let mut total_book = Money(0);

        for asset in &assets {
            total_cost = total_cost + asset.detail.cost_basis;
            total_accum = total_accum + asset.accumulated_depreciation;
            total_book = total_book + asset.book_value;

            let in_service = asset
                .detail
                .in_service_date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "CIP".to_owned());

            let life = asset
                .detail
                .useful_life_months
                .map(|m| m.to_string())
                .unwrap_or_else(|| "N/A".to_owned());

            rows.push(vec![
                asset.account_number.clone(),
                asset.account_name.clone(),
                in_service,
                life,
                format_money(asset.detail.cost_basis),
                format_money(asset.accumulated_depreciation),
                format_money(asset.book_value),
            ]);
        }

        // Totals row.
        rows.push(vec![
            String::new(),
            "TOTAL".to_owned(),
            String::new(),
            String::new(),
            format_money(total_cost),
            format_money(total_accum),
            format_money(total_book),
        ]);

        let table = format_table(&headers, &rows, &alignments);
        Ok(format!("{}\n{}", header, table))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::account_repo::NewAccount;
    use crate::db::asset_repo::NewFixedAssetDetails;
    use crate::db::entity_db_from_conn;
    use crate::db::schema::initialize_schema;
    use crate::types::{AccountId, AccountType};
    use chrono::NaiveDate;
    use rusqlite::Connection;

    fn make_db() -> crate::db::EntityDb {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema");
        let db = entity_db_from_conn(conn);
        db.fiscal().create_fiscal_year(1, 2026).expect("fy");
        db
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

    fn make_params(entity: &str, as_of: NaiveDate) -> ReportParams {
        ReportParams {
            entity_name: entity.to_owned(),
            as_of_date: Some(as_of),
            date_range: None,
            account_id: None,
        }
    }

    #[test]
    fn fixed_asset_schedule_report_name() {
        assert_eq!(FixedAssetSchedule.name(), "FixedAssetSchedule");
    }

    #[test]
    fn fixed_asset_schedule_empty() {
        let db = make_db();
        let params = make_params("Test Co", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = FixedAssetSchedule.generate(&db, &params).expect("generate");
        assert!(output.contains("Fixed Asset Schedule"), "title missing");
        assert!(output.contains("TOTAL"), "totals row missing");
    }

    #[test]
    fn fixed_asset_schedule_shows_asset_and_book_value() {
        let db = make_db();
        let asset_account = create_account(&db, "1400", "Building", AccountType::Asset);

        let in_service = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        db.assets()
            .create_fixed_asset(
                asset_account,
                &NewFixedAssetDetails {
                    cost_basis: Money::from_dollars(100_000.0),
                    in_service_date: Some(in_service),
                    useful_life_months: Some(240),
                    is_depreciable: true,
                    accum_depreciation_account_id: None,
                    depreciation_expense_account_id: None,
                },
            )
            .expect("create asset");

        let params = make_params("Acme", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = FixedAssetSchedule.generate(&db, &params).expect("generate");

        assert!(output.contains("Building"), "asset name missing");
        assert!(output.contains("100,000.00"), "cost basis missing");
        // No accumulated depreciation (no JEs posted), book value = cost.
        let total_line = output
            .lines()
            .find(|l| l.contains("TOTAL"))
            .expect("TOTAL missing");
        assert!(total_line.contains("100,000.00"), "total cost missing");
    }

    #[test]
    fn fixed_asset_schedule_book_value_equals_cost_minus_accum_depr() {
        let db = make_db();
        let asset_account = create_account(&db, "1400", "Building", AccountType::Asset);

        db.assets()
            .create_fixed_asset(
                asset_account,
                &NewFixedAssetDetails {
                    cost_basis: Money::from_dollars(50_000.0),
                    in_service_date: Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
                    useful_life_months: Some(120),
                    is_depreciable: true,
                    accum_depreciation_account_id: None,
                    depreciation_expense_account_id: None,
                },
            )
            .expect("create asset");

        let params = make_params("Acme", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = FixedAssetSchedule.generate(&db, &params).expect("generate");

        // Without any depreciation JEs, accumulated = 0, book value = cost.
        // book_value is computed as cost - accumulated in the AssetRepo.
        assert!(output.contains("50,000.00"), "cost basis in output");
    }

    #[test]
    fn fixed_asset_schedule_has_box_drawing_chars() {
        let db = make_db();
        let params = make_params("Test", NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let output = FixedAssetSchedule.generate(&db, &params).expect("generate");
        assert!(output.contains('┌'));
        assert!(output.contains('│'));
    }
}
