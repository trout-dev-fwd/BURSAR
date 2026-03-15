use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use rusqlite::{Connection, params};

use super::now_str;
use crate::types::{AccountId, ApItemId, ArApStatus, JournalEntryId, Money};

// ── Data structs ──────────────────────────────────────────────────────────────

/// A full accounts-payable item row loaded from the database.
#[derive(Debug, Clone, PartialEq)]
pub struct ApItem {
    pub id: ApItemId,
    pub account_id: AccountId,
    pub vendor_name: String,
    pub description: Option<String>,
    pub amount: Money,
    pub due_date: NaiveDate,
    pub status: ArApStatus,
    pub originating_je_id: JournalEntryId,
    pub created_at: String,
    pub updated_at: String,
}

/// Data required to create a new AP item.
#[derive(Debug, Clone)]
pub struct NewApItem {
    pub account_id: AccountId,
    pub vendor_name: String,
    pub description: Option<String>,
    pub amount: Money,
    pub due_date: NaiveDate,
    pub originating_je_id: JournalEntryId,
}

/// A single payment recorded against an AP item.
#[derive(Debug, Clone, PartialEq)]
pub struct ApPayment {
    pub id: i64,
    pub ap_item_id: ApItemId,
    pub je_id: JournalEntryId,
    pub amount: Money,
    pub payment_date: NaiveDate,
    pub created_at: String,
}

/// Filter criteria for `ApRepo::list`. All fields are optional (None = no filter).
#[derive(Debug, Clone, Default)]
pub struct ApFilter {
    pub status: Option<ArApStatus>,
}

// ── Repository ────────────────────────────────────────────────────────────────

/// Repository for the `ap_items` and `ap_payments` tables.
/// Mirrors `ArRepo` with `vendor_name` in place of `customer_name`.
pub struct ApRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> ApRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    /// Creates a new AP item with status `Open`. Returns the new item's ID.
    pub fn create_item(&self, new: &NewApItem) -> Result<ApItemId> {
        let now = now_str();
        self.conn
            .execute(
                "INSERT INTO ap_items
                    (account_id, vendor_name, description, amount, due_date,
                     status, originating_je_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'Open', ?6, ?7, ?8)",
                params![
                    i64::from(new.account_id),
                    new.vendor_name,
                    new.description,
                    new.amount.0,
                    new.due_date.to_string(),
                    i64::from(new.originating_je_id),
                    now,
                    now,
                ],
            )
            .context("Failed to create AP item")?;
        Ok(ApItemId::from(self.conn.last_insert_rowid()))
    }

    /// Records a payment against an AP item and recomputes its status.
    ///
    /// - Rejects if the item is already `Paid` (terminal state).
    /// - Rejects overpayments (total paid would exceed the item's original amount).
    /// - Status transitions: `Open` → `Partial` (partial payment) or `Paid` (full payment).
    pub fn record_payment(
        &self,
        item_id: ApItemId,
        je_id: JournalEntryId,
        amount: Money,
        date: NaiveDate,
    ) -> Result<()> {
        let item = self.get_by_id(item_id)?;

        if item.status == ArApStatus::Paid {
            bail!(
                "Cannot record payment: AP item {} is already Paid (terminal state)",
                i64::from(item_id)
            );
        }

        if amount.0 <= 0 {
            bail!("Payment amount must be positive");
        }

        let total_paid = self.get_total_paid(item_id)?;
        let new_total = Money(total_paid.0 + amount.0);

        if new_total.0 > item.amount.0 {
            bail!(
                "Overpayment: payment of {} would bring total paid ({}) above item amount ({})",
                amount,
                new_total,
                item.amount,
            );
        }

        let now = now_str();
        self.conn
            .execute(
                "INSERT INTO ap_payments (ap_item_id, je_id, amount, payment_date, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    i64::from(item_id),
                    i64::from(je_id),
                    amount.0,
                    date.to_string(),
                    now,
                ],
            )
            .context("Failed to insert AP payment")?;

        let new_status = if new_total.0 == item.amount.0 {
            ArApStatus::Paid
        } else {
            ArApStatus::Partial
        };

        self.conn
            .execute(
                "UPDATE ap_items SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![new_status.to_string(), now_str(), i64::from(item_id)],
            )
            .context("Failed to update AP item status")?;

        Ok(())
    }

    /// Returns AP items matching `filter`, ordered by due_date ASC, id ASC.
    pub fn list(&self, filter: &ApFilter) -> Result<Vec<ApItem>> {
        let mut conditions: Vec<&'static str> = Vec::new();
        let mut param_strings: Vec<String> = Vec::new();

        if let Some(status) = &filter.status {
            conditions.push("status = ?");
            param_strings.push(status.to_string());
        }

        let numbered: Vec<String> = conditions
            .iter()
            .enumerate()
            .map(|(i, c)| c.replace('?', &format!("?{}", i + 1)))
            .collect();

        let where_clause = if numbered.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", numbered.join(" AND "))
        };

        let sql = format!(
            "SELECT id, account_id, vendor_name, description, amount, due_date,
                    status, originating_je_id, created_at, updated_at
             FROM ap_items
             {where_clause}
             ORDER BY due_date ASC, id ASC"
        );

        let mut stmt = self.conn.prepare(&sql)?;
        stmt.query_map(
            rusqlite::params_from_iter(param_strings.iter()),
            row_to_ap_item,
        )?
        .map(|r| r.map_err(anyhow::Error::from))
        .collect()
    }

    /// Returns an AP item together with all its payments, ordered by payment_date.
    pub fn get_with_payments(&self, id: ApItemId) -> Result<(ApItem, Vec<ApPayment>)> {
        let item = self.get_by_id(id)?;

        let mut stmt = self.conn.prepare(
            "SELECT id, ap_item_id, je_id, amount, payment_date, created_at
             FROM ap_payments
             WHERE ap_item_id = ?1
             ORDER BY payment_date ASC, id ASC",
        )?;

        let payments = stmt
            .query_map(params![i64::from(id)], row_to_ap_payment)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect::<Result<Vec<_>>>()?;

        Ok((item, payments))
    }

    /// Returns the sum of all payments recorded against an AP item.
    /// Returns `Money(0)` if no payments exist.
    pub fn get_total_paid(&self, id: ApItemId) -> Result<Money> {
        let raw: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(amount), 0) FROM ap_payments WHERE ap_item_id = ?1",
                params![i64::from(id)],
                |row| row.get(0),
            )
            .context("Failed to compute total paid for AP item")?;
        Ok(Money(raw))
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn get_by_id(&self, id: ApItemId) -> Result<ApItem> {
        self.conn
            .query_row(
                "SELECT id, account_id, vendor_name, description, amount, due_date,
                        status, originating_je_id, created_at, updated_at
                 FROM ap_items WHERE id = ?1",
                params![i64::from(id)],
                row_to_ap_item,
            )
            .with_context(|| format!("AP item not found: {}", i64::from(id)))
    }
}

// ── Row mappers ───────────────────────────────────────────────────────────────

fn row_to_ap_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<ApItem> {
    let status_str: String = row.get(6)?;
    let status = status_str.parse::<ArApStatus>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let date_str: String = row.get(5)?;
    let due_date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(ApItem {
        id: ApItemId::from(row.get::<_, i64>(0)?),
        account_id: AccountId::from(row.get::<_, i64>(1)?),
        vendor_name: row.get(2)?,
        description: row.get(3)?,
        amount: Money(row.get::<_, i64>(4)?),
        due_date,
        status,
        originating_je_id: JournalEntryId::from(row.get::<_, i64>(7)?),
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn row_to_ap_payment(row: &rusqlite::Row<'_>) -> rusqlite::Result<ApPayment> {
    let date_str: String = row.get(4)?;
    let payment_date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(ApPayment {
        id: row.get(0)?,
        ap_item_id: ApItemId::from(row.get::<_, i64>(1)?),
        je_id: JournalEntryId::from(row.get::<_, i64>(2)?),
        amount: Money(row.get::<_, i64>(3)?),
        payment_date,
        created_at: row.get(5)?,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::account_repo::{AccountRepo, NewAccount};
    use crate::db::fiscal_repo::FiscalRepo;
    use crate::db::journal_repo::{JournalRepo, NewJournalEntry, NewJournalEntryLine};
    use crate::db::schema::{initialize_schema, seed_default_accounts};
    use crate::types::{AccountType, FiscalPeriodId};
    use rusqlite::Connection;

    fn setup_db() -> (Connection, FiscalPeriodId, AccountId, AccountId) {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema init");
        seed_default_accounts(&conn).expect("seed accounts");

        let fiscal = FiscalRepo::new(&conn);
        let fy_id = fiscal.create_fiscal_year(1, 2026).expect("create FY");
        let periods = fiscal.list_periods(fy_id).expect("list periods");
        let period_id = periods[0].id;

        let accounts = AccountRepo::new(&conn);
        let all = accounts.list_active().expect("list active");
        let non_placeholder: Vec<_> = all.iter().filter(|a| !a.is_placeholder).collect();
        let acct1 = non_placeholder[0].id;
        let acct2 = non_placeholder[1].id;

        (conn, period_id, acct1, acct2)
    }

    fn make_ap_account(conn: &Connection) -> AccountId {
        AccountRepo::new(conn)
            .create(&NewAccount {
                number: "2100".to_string(),
                name: "Accounts Payable".to_string(),
                account_type: AccountType::Liability,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .unwrap_or_else(|_| {
                AccountRepo::new(conn)
                    .list_all()
                    .unwrap()
                    .into_iter()
                    .find(|a| a.number == "2100")
                    .map(|a| a.id)
                    .expect("AP account 2100 should exist")
            })
    }

    fn make_je(
        conn: &Connection,
        period_id: FiscalPeriodId,
        acct1: AccountId,
        acct2: AccountId,
    ) -> JournalEntryId {
        JournalRepo::new(conn)
            .create_draft(&NewJournalEntry {
                entry_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                memo: None,
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: acct1,
                        debit_amount: Money(10_000_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: acct2,
                        debit_amount: Money(0),
                        credit_amount: Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create je")
    }

    #[test]
    fn create_item_and_list() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let ap_account = make_ap_account(&conn);
        let je_id = make_je(&conn, period_id, acct1, acct2);
        let repo = ApRepo::new(&conn);

        let item_id = repo
            .create_item(&NewApItem {
                account_id: ap_account,
                vendor_name: "Supplies Co".to_string(),
                description: Some("Office supplies".to_string()),
                amount: Money(25_000_000_000),
                due_date: chrono::NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
                originating_je_id: je_id,
            })
            .expect("create_item");

        let items = repo.list(&ApFilter::default()).expect("list");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, item_id);
        assert_eq!(items[0].vendor_name, "Supplies Co");
        assert_eq!(items[0].status, ArApStatus::Open);
    }

    #[test]
    fn partial_payment_sets_status_to_partial() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let ap_account = make_ap_account(&conn);
        let orig_je = make_je(&conn, period_id, acct1, acct2);
        let pay_je = make_je(&conn, period_id, acct1, acct2);
        let repo = ApRepo::new(&conn);

        let item_id = repo
            .create_item(&NewApItem {
                account_id: ap_account,
                vendor_name: "Electric Co".to_string(),
                description: None,
                amount: Money(100_000_000_000),
                due_date: chrono::NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
                originating_je_id: orig_je,
            })
            .expect("create");

        repo.record_payment(
            item_id,
            pay_je,
            Money(40_000_000_000),
            chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        )
        .expect("partial payment");

        let (item, payments) = repo.get_with_payments(item_id).expect("get");
        assert_eq!(item.status, ArApStatus::Partial);
        assert_eq!(payments.len(), 1);
    }

    #[test]
    fn full_payment_sets_status_to_paid() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let ap_account = make_ap_account(&conn);
        let orig_je = make_je(&conn, period_id, acct1, acct2);
        let pay_je = make_je(&conn, period_id, acct1, acct2);
        let repo = ApRepo::new(&conn);

        let item_id = repo
            .create_item(&NewApItem {
                account_id: ap_account,
                vendor_name: "Water Utility".to_string(),
                description: None,
                amount: Money(50_000_000_000),
                due_date: chrono::NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
                originating_je_id: orig_je,
            })
            .expect("create");

        repo.record_payment(
            item_id,
            pay_je,
            Money(50_000_000_000),
            chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        )
        .expect("full payment");

        let (item, _) = repo.get_with_payments(item_id).expect("get");
        assert_eq!(item.status, ArApStatus::Paid);

        let total = repo.get_total_paid(item_id).expect("total");
        assert_eq!(total, Money(50_000_000_000));
    }

    #[test]
    fn paid_item_rejects_further_payments() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let ap_account = make_ap_account(&conn);
        let orig_je = make_je(&conn, period_id, acct1, acct2);
        let pay_je1 = make_je(&conn, period_id, acct1, acct2);
        let pay_je2 = make_je(&conn, period_id, acct1, acct2);
        let repo = ApRepo::new(&conn);

        let item_id = repo
            .create_item(&NewApItem {
                account_id: ap_account,
                vendor_name: "Gas Co".to_string(),
                description: None,
                amount: Money(10_000_000_000),
                due_date: chrono::NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
                originating_je_id: orig_je,
            })
            .expect("create");

        repo.record_payment(
            item_id,
            pay_je1,
            Money(10_000_000_000),
            chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        )
        .expect("full payment");

        let result = repo.record_payment(
            item_id,
            pay_je2,
            Money(1_000_000_000),
            chrono::NaiveDate::from_ymd_opt(2026, 2, 10).unwrap(),
        );
        assert!(result.is_err(), "Paid item should reject further payments");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Paid") && msg.contains("terminal"),
            "Error should mention terminal state: {msg}"
        );
    }

    #[test]
    fn overpayment_rejected() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let ap_account = make_ap_account(&conn);
        let orig_je = make_je(&conn, period_id, acct1, acct2);
        let pay_je = make_je(&conn, period_id, acct1, acct2);
        let repo = ApRepo::new(&conn);

        let item_id = repo
            .create_item(&NewApItem {
                account_id: ap_account,
                vendor_name: "Rent LLC".to_string(),
                description: None,
                amount: Money(10_000_000_000),
                due_date: chrono::NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
                originating_je_id: orig_je,
            })
            .expect("create");

        let result = repo.record_payment(
            item_id,
            pay_je,
            Money(20_000_000_000),
            chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        );
        assert!(result.is_err(), "Overpayment should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Overpayment"),
            "Error should mention overpayment: {msg}"
        );
    }
}
