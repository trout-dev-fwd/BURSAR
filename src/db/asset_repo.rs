//! Fixed asset register: creation, place-in-service, and depreciation generation.
//!
//! Straight-line depreciation only. Rounding rule: the **final** month absorbs the
//! remainder so that `SUM(all monthly amounts) == cost_basis` exactly.

use anyhow::{Result, bail};
use chrono::{Datelike, NaiveDate};
use rusqlite::{Connection, params};

use crate::db::fiscal_repo::FiscalRepo;
use crate::db::journal_repo::{JournalRepo, NewJournalEntry, NewJournalEntryLine};
use crate::db::now_str;
use crate::types::{AccountId, FiscalPeriodId, FixedAssetDetailId, JournalEntryId, Money};

// ── Data structures ───────────────────────────────────────────────────────────

/// Full record from `fixed_asset_details`, including both optional account links.
#[derive(Debug, Clone)]
pub struct FixedAssetDetail {
    pub id: FixedAssetDetailId,
    pub account_id: AccountId,
    pub cost_basis: Money,
    pub in_service_date: Option<NaiveDate>,
    pub useful_life_months: Option<u32>,
    pub is_depreciable: bool,
    pub source_cip_account_id: Option<AccountId>,
    pub accum_depreciation_account_id: Option<AccountId>,
    pub depreciation_expense_account_id: Option<AccountId>,
}

/// Data needed to create a new fixed-asset record directly (without CIP).
#[derive(Debug, Clone)]
pub struct NewFixedAssetDetails {
    pub cost_basis: Money,
    pub in_service_date: Option<NaiveDate>,
    pub useful_life_months: Option<u32>,
    pub is_depreciable: bool,
    /// Linked contra-asset account (e.g. "Accumulated Depreciation — Buildings").
    pub accum_depreciation_account_id: Option<AccountId>,
    /// Linked expense account (e.g. "Depreciation Expense").
    pub depreciation_expense_account_id: Option<AccountId>,
}

/// Fixed asset with derived metrics useful for display.
#[derive(Debug, Clone)]
pub struct FixedAssetWithDetails {
    pub detail: FixedAssetDetail,
    pub account_name: String,
    pub account_number: String,
    /// Sum of posted credits to the accumulated-depreciation account (positive).
    pub accumulated_depreciation: Money,
    /// `cost_basis − accumulated_depreciation`.
    pub book_value: Money,
}

// ── Repository ────────────────────────────────────────────────────────────────

pub struct AssetRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> AssetRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    /// Registers a fixed asset record directly (e.g. for existing assets or land).
    /// Does **not** create a journal entry.
    pub fn create_fixed_asset(
        &self,
        account_id: AccountId,
        details: &NewFixedAssetDetails,
    ) -> Result<FixedAssetDetailId> {
        let now = now_str();
        self.conn.execute(
            "INSERT INTO fixed_asset_details
               (account_id, cost_basis, in_service_date, useful_life_months,
                is_depreciable, accum_depreciation_account_id, depreciation_expense_account_id,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                i64::from(account_id),
                details.cost_basis.0,
                details.in_service_date.map(|d| d.to_string()),
                details.useful_life_months.map(|v| v as i64),
                details.is_depreciable as i64,
                details.accum_depreciation_account_id.map(i64::from),
                details.depreciation_expense_account_id.map(i64::from),
                now,
            ],
        )?;
        Ok(FixedAssetDetailId::from(self.conn.last_insert_rowid()))
    }

    /// Transfers a CIP account balance into a fixed-asset account, creating a Posted journal
    /// entry and populating `fixed_asset_details` for the target account.
    ///
    /// Preconditions:
    /// - `target_asset_account_id` must already have a `fixed_asset_details` row (created with
    ///   `create_fixed_asset`).
    /// - The fiscal period for `in_service_date` must be open.
    pub fn place_in_service(
        &self,
        cip_account_id: AccountId,
        target_asset_account_id: AccountId,
        in_service_date: NaiveDate,
        useful_life_months: u32,
        accum_depreciation_account_id: Option<AccountId>,
        depreciation_expense_account_id: Option<AccountId>,
    ) -> Result<JournalEntryId> {
        // Determine the CIP account balance (that becomes the cost basis).
        let cost_basis = self.get_gl_balance(cip_account_id)?;
        if cost_basis.0 <= 0 {
            bail!("CIP account has no positive balance to transfer");
        }

        // Find the fiscal period for the in_service_date.
        let fiscal_repo = FiscalRepo::new(self.conn);
        let period = fiscal_repo.get_period_for_date(in_service_date)?;
        if period.is_closed {
            bail!("Fiscal period for in_service_date is closed");
        }

        // Get the CIP account name for the memo.
        let cip_name: String = self.conn.query_row(
            "SELECT name FROM accounts WHERE id = ?1",
            params![i64::from(cip_account_id)],
            |row| row.get(0),
        )?;
        let asset_name: String = self.conn.query_row(
            "SELECT name FROM accounts WHERE id = ?1",
            params![i64::from(target_asset_account_id)],
            |row| row.get(0),
        )?;

        // Create and immediately post the transfer JE.
        let je_repo = JournalRepo::new(self.conn);
        let je_id = je_repo.create_draft(&NewJournalEntry {
            entry_date: in_service_date,
            memo: Some(format!(
                "Place in service: {asset_name} (from CIP: {cip_name})"
            )),
            fiscal_period_id: period.id,
            reversal_of_je_id: None,
            lines: vec![
                // Debit fixed asset account (increases asset balance).
                NewJournalEntryLine {
                    account_id: target_asset_account_id,
                    debit_amount: cost_basis,
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                // Credit CIP account (removes balance from CIP).
                NewJournalEntryLine {
                    account_id: cip_account_id,
                    debit_amount: Money(0),
                    credit_amount: cost_basis,
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        })?;
        je_repo.update_status(je_id, crate::types::JournalEntryStatus::Posted)?;

        // Upsert fixed_asset_details for the target account.
        let now = now_str();
        self.conn.execute(
            "INSERT INTO fixed_asset_details
               (account_id, cost_basis, in_service_date, useful_life_months,
                is_depreciable, source_cip_account_id,
                accum_depreciation_account_id, depreciation_expense_account_id,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(account_id) DO UPDATE SET
               cost_basis = excluded.cost_basis,
               in_service_date = excluded.in_service_date,
               useful_life_months = excluded.useful_life_months,
               source_cip_account_id = excluded.source_cip_account_id,
               accum_depreciation_account_id = excluded.accum_depreciation_account_id,
               depreciation_expense_account_id = excluded.depreciation_expense_account_id,
               updated_at = excluded.updated_at",
            params![
                i64::from(target_asset_account_id),
                cost_basis.0,
                in_service_date.to_string(),
                useful_life_months as i64,
                i64::from(cip_account_id),
                accum_depreciation_account_id.map(i64::from),
                depreciation_expense_account_id.map(i64::from),
                now,
            ],
        )?;

        Ok(je_id)
    }

    /// Lists all fixed assets with their GL account info and computed book value.
    pub fn list_assets(&self) -> Result<Vec<FixedAssetWithDetails>> {
        let mut stmt = self.conn.prepare(
            "SELECT fad.id, fad.account_id, fad.cost_basis, fad.in_service_date,
                    fad.useful_life_months, fad.is_depreciable, fad.source_cip_account_id,
                    fad.accum_depreciation_account_id, fad.depreciation_expense_account_id,
                    a.name, a.number
             FROM fixed_asset_details fad
             JOIN accounts a ON a.id = fad.account_id
             ORDER BY a.number",
        )?;

        let rows = stmt.query_map([], |row| {
            let in_service: Option<String> = row.get(3)?;
            let accum_id: Option<i64> = row.get(7)?;
            let expense_id: Option<i64> = row.get(8)?;
            let src_cip: Option<i64> = row.get(6)?;
            Ok((
                FixedAssetDetail {
                    id: FixedAssetDetailId::from(row.get::<_, i64>(0)?),
                    account_id: AccountId::from(row.get::<_, i64>(1)?),
                    cost_basis: Money(row.get::<_, i64>(2)?),
                    in_service_date: in_service
                        .as_deref()
                        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()),
                    useful_life_months: row.get::<_, Option<i64>>(4)?.map(|v| v as u32),
                    is_depreciable: row.get::<_, i64>(5)? != 0,
                    source_cip_account_id: src_cip.map(AccountId::from),
                    accum_depreciation_account_id: accum_id.map(AccountId::from),
                    depreciation_expense_account_id: expense_id.map(AccountId::from),
                },
                row.get::<_, String>(9)?,  // name
                row.get::<_, String>(10)?, // number
            ))
        })?;

        let mut result = Vec::new();
        for row in rows {
            let (detail, name, number) = row?;

            // Compute accumulated depreciation: sum of posted credits to the accum account.
            let accumulated_depreciation =
                if let Some(accum_id) = detail.accum_depreciation_account_id {
                    let credits: i64 = self.conn.query_row(
                        "SELECT COALESCE(SUM(jel.credit_amount - jel.debit_amount), 0)
                         FROM journal_entry_lines jel
                         JOIN journal_entries je ON je.id = jel.journal_entry_id
                         WHERE jel.account_id = ?1
                           AND je.status = 'Posted'",
                        params![i64::from(accum_id)],
                        |row| row.get(0),
                    )?;
                    Money(credits)
                } else {
                    Money(0)
                };

            let book_value = Money(detail.cost_basis.0 - accumulated_depreciation.0);
            result.push(FixedAssetWithDetails {
                detail,
                account_name: name,
                account_number: number,
                accumulated_depreciation,
                book_value,
            });
        }

        Ok(result)
    }

    /// Generates Draft journal entries for all pending depreciation months up to
    /// (and including) the end of `as_of_period`.
    ///
    /// Rounding: the final month absorbs the remainder so the total exactly equals
    /// `cost_basis`.
    pub fn generate_pending_depreciation(
        &self,
        as_of_period: FiscalPeriodId,
    ) -> Result<Vec<NewJournalEntry>> {
        // Get the end date of the as_of_period to determine coverage.
        let fiscal_repo = FiscalRepo::new(self.conn);
        let period_end: NaiveDate = self
            .conn
            .query_row(
                "SELECT end_date FROM fiscal_periods WHERE id = ?1",
                params![i64::from(as_of_period)],
                |row| {
                    let s: String = row.get(0)?;
                    Ok(s)
                },
            )
            .and_then(|s| {
                NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
            })?;

        // Query all depreciable assets that are in service.
        let mut stmt = self.conn.prepare(
            "SELECT fad.account_id, fad.cost_basis, fad.in_service_date,
                    fad.useful_life_months, fad.accum_depreciation_account_id,
                    fad.depreciation_expense_account_id, a.name
             FROM fixed_asset_details fad
             JOIN accounts a ON a.id = fad.account_id
             WHERE fad.is_depreciable = 1
               AND fad.in_service_date IS NOT NULL
               AND fad.useful_life_months IS NOT NULL
               AND fad.accum_depreciation_account_id IS NOT NULL
               AND fad.depreciation_expense_account_id IS NOT NULL",
        )?;

        struct AssetRow {
            #[allow(dead_code)]
            account_id: AccountId,
            cost_basis: Money,
            in_service_date: NaiveDate,
            useful_life_months: u32,
            accum_id: AccountId,
            expense_id: AccountId,
            name: String,
        }

        let assets: Vec<AssetRow> = stmt
            .query_map([], |row| {
                let date_str: String = row.get(2)?;
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    date_str,
                    row.get::<_, i64>(3)? as u32,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(aid, cost, date_s, life, accum, expense, name)| {
                let in_service_date = NaiveDate::parse_from_str(&date_s, "%Y-%m-%d").ok()?;
                Some(AssetRow {
                    account_id: AccountId::from(aid),
                    cost_basis: Money(cost),
                    in_service_date,
                    useful_life_months: life,
                    accum_id: AccountId::from(accum),
                    expense_id: AccountId::from(expense),
                    name,
                })
            })
            .collect();

        let mut new_entries: Vec<NewJournalEntry> = Vec::new();

        for asset in assets {
            // Count months already generated (any status, non-reversed) for this accum account.
            // Assumption: each accum_depreciation_account is dedicated to one asset.
            let months_generated: i64 = self.conn.query_row(
                "SELECT COUNT(DISTINCT je.id)
                 FROM journal_entries je
                 JOIN journal_entry_lines jel ON jel.journal_entry_id = je.id
                 WHERE jel.account_id = ?1
                   AND jel.credit_amount > 0
                   AND je.is_reversed = 0",
                params![i64::from(asset.accum_id)],
                |row| row.get(0),
            )?;
            let months_generated = months_generated as u32;

            if months_generated >= asset.useful_life_months {
                continue; // Fully depreciated.
            }

            // Determine how many months to generate through as_of_period.
            // Month N is dated at the first day of (in_service_date + N months).
            let total_to_generate = (0..asset.useful_life_months)
                .map(|n| month_start_after(asset.in_service_date, n + 1))
                .filter(|&d| d <= period_end)
                .count() as u32;

            let monthly_amount = asset.cost_basis.0 / i64::from(asset.useful_life_months);

            for month_idx in months_generated..total_to_generate {
                let je_date = month_start_after(asset.in_service_date, month_idx + 1);

                // Find or look up the fiscal period for this date.
                let period = fiscal_repo.get_period_for_date(je_date)?;

                // Final month absorbs remainder.
                let is_final = month_idx + 1 == asset.useful_life_months;
                let amount = if is_final {
                    Money(asset.cost_basis.0 - monthly_amount * i64::from(month_idx))
                } else {
                    Money(monthly_amount)
                };

                let memo = format!(
                    "Monthly depreciation: {} — Month {} of {}",
                    asset.name,
                    month_idx + 1,
                    asset.useful_life_months,
                );

                new_entries.push(NewJournalEntry {
                    entry_date: je_date,
                    memo: Some(memo),
                    fiscal_period_id: period.id,
                    reversal_of_je_id: None,
                    lines: vec![
                        NewJournalEntryLine {
                            account_id: asset.expense_id,
                            debit_amount: amount,
                            credit_amount: Money(0),
                            line_memo: None,
                            sort_order: 0,
                        },
                        NewJournalEntryLine {
                            account_id: asset.accum_id,
                            debit_amount: Money(0),
                            credit_amount: amount,
                            line_memo: None,
                            sort_order: 1,
                        },
                    ],
                });
            }
        }

        Ok(new_entries)
    }

    // ── Private helpers ────────────────────────────────────────────────────────

    /// Returns the GL balance (SUM debit - SUM credit) for `account_id` across posted entries.
    fn get_gl_balance(&self, account_id: AccountId) -> Result<Money> {
        let bal: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(jel.debit_amount - jel.credit_amount), 0)
             FROM journal_entry_lines jel
             JOIN journal_entries je ON je.id = jel.journal_entry_id
             WHERE jel.account_id = ?1
               AND je.status = 'Posted'",
            params![i64::from(account_id)],
            |row| row.get(0),
        )?;
        Ok(Money(bal))
    }
}

/// Returns the first day of the Nth month after `start` (1-based: N=1 = next month).
fn month_start_after(start: NaiveDate, n: u32) -> NaiveDate {
    let mut year = start.year();
    let mut month = start.month() + n;
    while month > 12 {
        month -= 12;
        year += 1;
    }
    NaiveDate::from_ymd_opt(year, month, 1)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, month + 1, 1).expect("valid date"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::journal_repo::JournalRepo;
    use crate::db::schema::initialize_schema;
    use crate::types::JournalEntryStatus;
    use rusqlite::Connection;

    fn setup() -> (Connection, FiscalPeriodId) {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema");
        // No seeded accounts — tests create only what they need.

        // Create a fiscal year starting Jan 1.
        let now = "2026-01-01";
        conn.execute(
            "INSERT INTO fiscal_years (start_date, end_date, is_closed, created_at)
             VALUES ('2026-01-01', '2026-12-31', 0, ?1)",
            params![now],
        )
        .expect("fy");
        let fy_id = conn.last_insert_rowid();

        // Period 1: Jan.
        conn.execute(
            "INSERT INTO fiscal_periods (fiscal_year_id, period_number, start_date, end_date, is_closed, created_at)
             VALUES (?1, 1, '2026-01-01', '2026-01-31', 0, ?2)",
            params![fy_id, now],
        )
        .expect("period");
        let p1 = FiscalPeriodId::from(conn.last_insert_rowid());

        // Periods 2-12 for later tests.
        for m in 2u32..=12 {
            let start = NaiveDate::from_ymd_opt(2026, m, 1).unwrap();
            let end = if m < 12 {
                NaiveDate::from_ymd_opt(2026, m + 1, 1)
                    .unwrap()
                    .pred_opt()
                    .unwrap()
            } else {
                NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()
            };
            conn.execute(
                "INSERT INTO fiscal_periods (fiscal_year_id, period_number, start_date, end_date, is_closed, created_at)
                 VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                params![fy_id, m as i64, start.to_string(), end.to_string(), now],
            )
            .expect("period");
        }

        (conn, p1)
    }

    /// Creates a minimal account and returns its id.
    fn make_account(
        conn: &Connection,
        number: &str,
        name: &str,
        acct_type: &str,
        is_contra: bool,
        parent_id: Option<i64>,
    ) -> AccountId {
        conn.execute(
            "INSERT INTO accounts (number, name, account_type, is_placeholder, is_contra, is_active, parent_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, 0, ?4, 1, ?5, '2026-01-01', '2026-01-01')",
            params![number, name, acct_type, is_contra as i64, parent_id],
        )
        .expect("insert account");
        AccountId::from(conn.last_insert_rowid())
    }

    #[test]
    fn create_fixed_asset_and_list() {
        let (conn, _p1) = setup();
        let asset_account = make_account(&conn, "1500", "Buildings", "Asset", false, None);
        let repo = AssetRepo::new(&conn);

        let detail_id = repo
            .create_fixed_asset(
                asset_account,
                &NewFixedAssetDetails {
                    cost_basis: Money(100_000 * 100_000_000),
                    in_service_date: Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
                    useful_life_months: Some(360),
                    is_depreciable: true,
                    accum_depreciation_account_id: None,
                    depreciation_expense_account_id: None,
                },
            )
            .expect("create_fixed_asset");

        assert!(i64::from(detail_id) > 0);

        let assets = repo.list_assets().expect("list_assets");
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].detail.account_id, asset_account);
        assert_eq!(assets[0].detail.cost_basis.0, 100_000 * 100_000_000);
        assert_eq!(assets[0].accumulated_depreciation, Money(0));
        assert_eq!(assets[0].book_value, assets[0].detail.cost_basis);
    }

    #[test]
    fn land_is_non_depreciable() {
        let (conn, _p1) = setup();
        let land = make_account(&conn, "1510", "Land", "Asset", false, None);
        let repo = AssetRepo::new(&conn);

        repo.create_fixed_asset(
            land,
            &NewFixedAssetDetails {
                cost_basis: Money(50_000 * 100_000_000),
                in_service_date: None,
                useful_life_months: None,
                is_depreciable: false,
                accum_depreciation_account_id: None,
                depreciation_expense_account_id: None,
            },
        )
        .expect("create land");

        // generate_pending_depreciation should skip non-depreciable assets.
        let (conn2, _) = setup();
        let repo2 = AssetRepo::new(&conn2);

        // Get a valid period id (period 1).
        let p_id: i64 = conn
            .query_row(
                "SELECT id FROM fiscal_periods WHERE period_number = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let entries = repo
            .generate_pending_depreciation(FiscalPeriodId::from(p_id))
            .expect("generate");
        assert_eq!(entries.len(), 0, "land should generate no depreciation");
        let _ = repo2; // silence unused warning
    }

    #[test]
    fn place_in_service_creates_je_and_updates_details() {
        let (conn, _p1) = setup();

        // Create a CIP account and post some balance into it.
        let cip = make_account(
            &conn,
            "1490",
            "Construction in Progress",
            "Asset",
            false,
            None,
        );
        let asset_acct = make_account(&conn, "1500", "Buildings", "Asset", false, None);
        let equity = make_account(&conn, "3100", "Owner Capital Equity", "Equity", false, None);

        // Post a JE: Debit CIP $200,000, Credit Owner's Capital.
        let cost = Money(200_000 * 100_000_000);
        let je_repo = JournalRepo::new(&conn);
        let p_id: i64 = conn
            .query_row(
                "SELECT id FROM fiscal_periods WHERE period_number = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let p = FiscalPeriodId::from(p_id);

        let je_id = je_repo
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
                memo: None,
                fiscal_period_id: p,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: cip,
                        debit_amount: cost,
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: equity,
                        debit_amount: Money(0),
                        credit_amount: cost,
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .unwrap();
        je_repo
            .update_status(je_id, JournalEntryStatus::Posted)
            .unwrap();

        // Register the fixed asset (without details yet).
        let repo = AssetRepo::new(&conn);
        repo.create_fixed_asset(
            asset_acct,
            &NewFixedAssetDetails {
                cost_basis: Money(0), // will be overwritten by place_in_service
                in_service_date: None,
                useful_life_months: None,
                is_depreciable: true,
                accum_depreciation_account_id: None,
                depreciation_expense_account_id: None,
            },
        )
        .expect("create_fixed_asset");

        // Place in service.
        let transfer_je_id = repo
            .place_in_service(
                cip,
                asset_acct,
                NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                360,
                None,
                None,
            )
            .expect("place_in_service");

        assert!(i64::from(transfer_je_id) > 0);

        // Verify the transfer JE is Posted.
        let je_status: String = conn
            .query_row(
                "SELECT status FROM journal_entries WHERE id = ?1",
                params![i64::from(transfer_je_id)],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(je_status, "Posted");

        // CIP balance should now be zero (original debit offset by credit in transfer JE).
        let cip_balance: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(jel.debit_amount - jel.credit_amount), 0)
                 FROM journal_entry_lines jel
                 JOIN journal_entries je ON je.id = jel.journal_entry_id
                 WHERE jel.account_id = ?1 AND je.status = 'Posted'",
                params![i64::from(cip)],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cip_balance, 0);

        // Asset account balance should equal cost.
        let asset_balance: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(jel.debit_amount - jel.credit_amount), 0)
                 FROM journal_entry_lines jel
                 JOIN journal_entries je ON je.id = jel.journal_entry_id
                 WHERE jel.account_id = ?1 AND je.status = 'Posted'",
                params![i64::from(asset_acct)],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(asset_balance, cost.0);

        // fixed_asset_details should be updated.
        let (db_cost, db_date, db_months): (i64, String, i64) = conn
            .query_row(
                "SELECT cost_basis, in_service_date, useful_life_months
                 FROM fixed_asset_details WHERE account_id = ?1",
                params![i64::from(asset_acct)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(db_cost, cost.0);
        assert_eq!(db_date, "2026-01-15");
        assert_eq!(db_months, 360);
    }

    #[test]
    fn place_in_service_rejects_zero_cip_balance() {
        let (conn, _p1) = setup();
        let cip = make_account(
            &conn,
            "1490",
            "Construction in Progress",
            "Asset",
            false,
            None,
        );
        let asset_acct = make_account(&conn, "1500", "Buildings", "Asset", false, None);

        let repo = AssetRepo::new(&conn);
        repo.create_fixed_asset(
            asset_acct,
            &NewFixedAssetDetails {
                cost_basis: Money(0),
                in_service_date: None,
                useful_life_months: None,
                is_depreciable: true,
                accum_depreciation_account_id: None,
                depreciation_expense_account_id: None,
            },
        )
        .unwrap();

        let err = repo
            .place_in_service(
                cip,
                asset_acct,
                NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                360,
                None,
                None,
            )
            .unwrap_err();
        assert!(err.to_string().contains("no positive balance"));
    }

    #[test]
    fn generate_depreciation_correct_amounts() {
        let (conn, _p1) = setup();

        // $12,000 asset, 12 months → $1,000/month.
        let asset_acct = make_account(&conn, "1500", "Buildings", "Asset", false, None);
        let accum = make_account(
            &conn,
            "1521",
            "Accumulated Depreciation",
            "Asset",
            true,
            None,
        );
        let expense = make_account(
            &conn,
            "5500",
            "Depreciation Expense",
            "Expense",
            false,
            None,
        );

        let repo = AssetRepo::new(&conn);
        let monthly = 1_000 * 100_000_000_i64;
        repo.create_fixed_asset(
            asset_acct,
            &NewFixedAssetDetails {
                cost_basis: Money(12 * monthly),
                in_service_date: Some(NaiveDate::from_ymd_opt(2025, 12, 31).unwrap()),
                useful_life_months: Some(12),
                is_depreciable: true,
                accum_depreciation_account_id: Some(accum),
                depreciation_expense_account_id: Some(expense),
            },
        )
        .unwrap();

        // Get period 3 (March 2026) id.
        let p3_id: i64 = conn
            .query_row(
                "SELECT id FROM fiscal_periods WHERE period_number = 3",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let entries = repo
            .generate_pending_depreciation(FiscalPeriodId::from(p3_id))
            .expect("generate");

        // Months 1, 2, 3 (Jan, Feb, Mar 2026) should be generated.
        assert_eq!(entries.len(), 3);
        for entry in &entries {
            let debit = entry.lines[0].debit_amount;
            assert_eq!(debit.0, monthly, "each month should be $1,000");
        }
    }

    #[test]
    fn generate_depreciation_skips_already_generated_months() {
        let (conn, _p1) = setup();

        let asset_acct = make_account(&conn, "1500", "Buildings", "Asset", false, None);
        let accum = make_account(
            &conn,
            "1521",
            "Accumulated Depreciation",
            "Asset",
            true,
            None,
        );
        let expense = make_account(
            &conn,
            "5500",
            "Depreciation Expense",
            "Expense",
            false,
            None,
        );

        let monthly = 1_000 * 100_000_000_i64;
        let repo = AssetRepo::new(&conn);
        repo.create_fixed_asset(
            asset_acct,
            &NewFixedAssetDetails {
                cost_basis: Money(12 * monthly),
                in_service_date: Some(NaiveDate::from_ymd_opt(2025, 12, 31).unwrap()),
                useful_life_months: Some(12),
                is_depreciable: true,
                accum_depreciation_account_id: Some(accum),
                depreciation_expense_account_id: Some(expense),
            },
        )
        .unwrap();

        // Manually "post" 2 months of depreciation.
        let p1_id: i64 = conn
            .query_row(
                "SELECT id FROM fiscal_periods WHERE period_number = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let p2_id: i64 = conn
            .query_row(
                "SELECT id FROM fiscal_periods WHERE period_number = 2",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let je_repo = JournalRepo::new(&conn);
        for p_id in [p1_id, p2_id] {
            let entry_date = if p_id == p1_id {
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
            } else {
                NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()
            };
            let je_id = je_repo
                .create_draft(&NewJournalEntry {
                    entry_date,
                    memo: Some("Monthly depreciation: Buildings — Month X of 12".to_string()),
                    fiscal_period_id: FiscalPeriodId::from(p_id),
                    reversal_of_je_id: None,
                    lines: vec![
                        NewJournalEntryLine {
                            account_id: expense,
                            debit_amount: Money(monthly),
                            credit_amount: Money(0),
                            line_memo: None,
                            sort_order: 0,
                        },
                        NewJournalEntryLine {
                            account_id: accum,
                            debit_amount: Money(0),
                            credit_amount: Money(monthly),
                            line_memo: None,
                            sort_order: 1,
                        },
                    ],
                })
                .unwrap();
            je_repo
                .update_status(je_id, JournalEntryStatus::Posted)
                .unwrap();
        }

        // Generate through period 3 — only month 3 should appear.
        let p3_id: i64 = conn
            .query_row(
                "SELECT id FROM fiscal_periods WHERE period_number = 3",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let entries = repo
            .generate_pending_depreciation(FiscalPeriodId::from(p3_id))
            .expect("generate");
        assert_eq!(entries.len(), 1, "only month 3 should be pending");
    }

    /// Verify that when cost_basis is not evenly divisible by useful_life_months,
    /// the final month absorbs the rounding remainder so that the sum of all
    /// monthly depreciation amounts equals cost_basis exactly.
    #[test]
    fn generate_depreciation_rounding_totals_cost_basis_exactly() {
        let (conn, _p1) = setup();

        // $10 (1_000_000_000 internal units) over 3 months.
        // 1_000_000_000 / 3 = 333_333_333 remainder 1.
        // Months 1-2: 333_333_333 each; month 3: 333_333_334.
        // Total must equal exactly 1_000_000_000.
        let cost_basis = Money(1_000_000_000);
        let life_months: u32 = 3;
        let monthly = cost_basis.0 / i64::from(life_months);
        let remainder = cost_basis.0 % i64::from(life_months);
        assert_ne!(remainder, 0, "test requires non-zero remainder");

        let asset_acct = make_account(&conn, "1500", "Equipment", "Asset", false, None);
        let accum = make_account(
            &conn,
            "1521",
            "Accumulated Depreciation",
            "Asset",
            true,
            None,
        );
        let expense = make_account(
            &conn,
            "5500",
            "Depreciation Expense",
            "Expense",
            false,
            None,
        );

        let repo = AssetRepo::new(&conn);
        repo.create_fixed_asset(
            asset_acct,
            &NewFixedAssetDetails {
                cost_basis,
                in_service_date: Some(NaiveDate::from_ymd_opt(2025, 12, 31).unwrap()),
                useful_life_months: Some(life_months),
                is_depreciable: true,
                accum_depreciation_account_id: Some(accum),
                depreciation_expense_account_id: Some(expense),
            },
        )
        .unwrap();

        // Generate through period 3 (March 2026) — all 3 months.
        let p3_id: i64 = conn
            .query_row(
                "SELECT id FROM fiscal_periods WHERE period_number = 3",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let entries = repo
            .generate_pending_depreciation(FiscalPeriodId::from(p3_id))
            .expect("generate");

        assert_eq!(entries.len(), 3, "all 3 months should be generated");

        let total: i64 = entries.iter().map(|e| e.lines[0].debit_amount.0).sum();
        assert_eq!(
            total, cost_basis.0,
            "sum of all monthly depreciation must equal cost basis exactly"
        );

        // First life_months-1 entries each get `monthly`; last gets `monthly + remainder`.
        for (i, entry) in entries.iter().enumerate() {
            let debit = entry.lines[0].debit_amount.0;
            if i < (life_months as usize) - 1 {
                assert_eq!(debit, monthly, "month {} should be base amount", i + 1);
            } else {
                assert_eq!(
                    debit,
                    monthly + remainder,
                    "final month should absorb remainder"
                );
            }
        }
    }
}
