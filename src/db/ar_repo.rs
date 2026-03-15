use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use rusqlite::{Connection, params};

use super::now_str;
use crate::types::{AccountId, ArApStatus, ArItemId, JournalEntryId, Money};

// ── Data structs ──────────────────────────────────────────────────────────────

/// A full accounts-receivable item row loaded from the database.
#[derive(Debug, Clone, PartialEq)]
pub struct ArItem {
    pub id: ArItemId,
    pub account_id: AccountId,
    pub customer_name: String,
    pub description: Option<String>,
    pub amount: Money,
    pub due_date: NaiveDate,
    pub status: ArApStatus,
    pub originating_je_id: JournalEntryId,
    pub created_at: String,
    pub updated_at: String,
}

/// Data required to create a new AR item.
#[derive(Debug, Clone)]
pub struct NewArItem {
    pub account_id: AccountId,
    pub customer_name: String,
    pub description: Option<String>,
    pub amount: Money,
    pub due_date: NaiveDate,
    pub originating_je_id: JournalEntryId,
}

/// A single payment recorded against an AR item.
#[derive(Debug, Clone, PartialEq)]
pub struct ArPayment {
    pub id: i64,
    pub ar_item_id: ArItemId,
    pub je_id: JournalEntryId,
    pub amount: Money,
    pub payment_date: NaiveDate,
    pub created_at: String,
}

/// Filter criteria for `ArRepo::list`. All fields are optional (None = no filter).
#[derive(Debug, Clone, Default)]
pub struct ArFilter {
    pub status: Option<ArApStatus>,
}

// ── Repository ────────────────────────────────────────────────────────────────

/// Repository for the `ar_items` and `ar_payments` tables.
pub struct ArRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> ArRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    /// Creates a new AR item with status `Open`. Returns the new item's ID.
    pub fn create_item(&self, new: &NewArItem) -> Result<ArItemId> {
        let now = now_str();
        self.conn
            .execute(
                "INSERT INTO ar_items
                    (account_id, customer_name, description, amount, due_date,
                     status, originating_je_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'Open', ?6, ?7, ?8)",
                params![
                    i64::from(new.account_id),
                    new.customer_name,
                    new.description,
                    new.amount.0,
                    new.due_date.to_string(),
                    i64::from(new.originating_je_id),
                    now,
                    now,
                ],
            )
            .context("Failed to create AR item")?;
        Ok(ArItemId::from(self.conn.last_insert_rowid()))
    }

    /// Records a payment against an AR item and recomputes its status.
    ///
    /// - Rejects if the item is already `Paid` (terminal state).
    /// - Rejects overpayments (total paid would exceed the item's original amount).
    /// - Status transitions: `Open` → `Partial` (partial payment) or `Paid` (full payment).
    pub fn record_payment(
        &self,
        item_id: ArItemId,
        je_id: JournalEntryId,
        amount: Money,
        date: NaiveDate,
    ) -> Result<()> {
        let item = self.get_by_id(item_id)?;

        if item.status == ArApStatus::Paid {
            bail!(
                "Cannot record payment: AR item {} is already Paid (terminal state)",
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
                "INSERT INTO ar_payments (ar_item_id, je_id, amount, payment_date, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    i64::from(item_id),
                    i64::from(je_id),
                    amount.0,
                    date.to_string(),
                    now,
                ],
            )
            .context("Failed to insert AR payment")?;

        let new_status = if new_total.0 == item.amount.0 {
            ArApStatus::Paid
        } else {
            ArApStatus::Partial
        };

        self.conn
            .execute(
                "UPDATE ar_items SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![new_status.to_string(), now_str(), i64::from(item_id)],
            )
            .context("Failed to update AR item status")?;

        Ok(())
    }

    /// Returns AR items matching `filter`, ordered by due_date ASC, id ASC.
    /// Uses dynamic SQL (consistent with JournalRepo::list pattern).
    pub fn list(&self, filter: &ArFilter) -> Result<Vec<ArItem>> {
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
            "SELECT id, account_id, customer_name, description, amount, due_date,
                    status, originating_je_id, created_at, updated_at
             FROM ar_items
             {where_clause}
             ORDER BY due_date ASC, id ASC"
        );

        let mut stmt = self.conn.prepare(&sql)?;
        stmt.query_map(
            rusqlite::params_from_iter(param_strings.iter()),
            row_to_ar_item,
        )?
        .map(|r| r.map_err(anyhow::Error::from))
        .collect()
    }

    /// Returns an AR item together with all its payments, ordered by payment_date.
    pub fn get_with_payments(&self, id: ArItemId) -> Result<(ArItem, Vec<ArPayment>)> {
        let item = self.get_by_id(id)?;

        let mut stmt = self.conn.prepare(
            "SELECT id, ar_item_id, je_id, amount, payment_date, created_at
             FROM ar_payments
             WHERE ar_item_id = ?1
             ORDER BY payment_date ASC, id ASC",
        )?;

        let payments = stmt
            .query_map(params![i64::from(id)], row_to_ar_payment)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect::<Result<Vec<_>>>()?;

        Ok((item, payments))
    }

    /// Returns the sum of all payments recorded against an AR item.
    /// Returns `Money(0)` if no payments exist.
    pub fn get_total_paid(&self, id: ArItemId) -> Result<Money> {
        let raw: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(amount), 0) FROM ar_payments WHERE ar_item_id = ?1",
                params![i64::from(id)],
                |row| row.get(0),
            )
            .context("Failed to compute total paid for AR item")?;
        Ok(Money(raw))
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn get_by_id(&self, id: ArItemId) -> Result<ArItem> {
        self.conn
            .query_row(
                "SELECT id, account_id, customer_name, description, amount, due_date,
                        status, originating_je_id, created_at, updated_at
                 FROM ar_items WHERE id = ?1",
                params![i64::from(id)],
                row_to_ar_item,
            )
            .with_context(|| format!("AR item not found: {}", i64::from(id)))
    }
}

// ── Row mappers ───────────────────────────────────────────────────────────────

fn row_to_ar_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArItem> {
    let status_str: String = row.get(6)?;
    let status = status_str.parse::<ArApStatus>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let date_str: String = row.get(5)?;
    let due_date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(ArItem {
        id: ArItemId::from(row.get::<_, i64>(0)?),
        account_id: AccountId::from(row.get::<_, i64>(1)?),
        customer_name: row.get(2)?,
        description: row.get(3)?,
        amount: Money(row.get::<_, i64>(4)?),
        due_date,
        status,
        originating_je_id: JournalEntryId::from(row.get::<_, i64>(7)?),
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn row_to_ar_payment(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArPayment> {
    let date_str: String = row.get(4)?;
    let payment_date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(ArPayment {
        id: row.get(0)?,
        ar_item_id: ArItemId::from(row.get::<_, i64>(1)?),
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

    /// Sets up an in-memory DB and returns the connection, a fiscal period ID, and
    /// two non-placeholder account IDs (for creating JEs).
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
        assert!(
            non_placeholder.len() >= 2,
            "Need at least 2 non-placeholder accounts"
        );
        let acct1 = non_placeholder[0].id;
        let acct2 = non_placeholder[1].id;

        (conn, period_id, acct1, acct2)
    }

    /// Creates an AR account (non-placeholder) for use in AR items.
    fn make_ar_account(conn: &Connection) -> AccountId {
        AccountRepo::new(conn)
            .create(&NewAccount {
                number: "1200".to_string(),
                name: "Accounts Receivable".to_string(),
                account_type: AccountType::Asset,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .unwrap_or_else(|_| {
                // Account 1200 may already exist from seeded data — find it.
                AccountRepo::new(conn)
                    .list_all()
                    .unwrap()
                    .into_iter()
                    .find(|a| a.number == "1200")
                    .map(|a| a.id)
                    .expect("AR account 1200 should exist")
            })
    }

    /// Creates a minimal journal entry and returns its ID.
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

    // ── create_item ───────────────────────────────────────────────────────────

    #[test]
    fn create_item_and_list() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let ar_account = make_ar_account(&conn);
        let je_id = make_je(&conn, period_id, acct1, acct2);
        let repo = ArRepo::new(&conn);

        let item_id = repo
            .create_item(&NewArItem {
                account_id: ar_account,
                customer_name: "Acme Corp".to_string(),
                description: Some("Invoice #001".to_string()),
                amount: Money(50_000_000_000), // $500.00
                due_date: chrono::NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
                originating_je_id: je_id,
            })
            .expect("create_item failed");

        let items = repo.list(&ArFilter::default()).expect("list failed");
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, item_id);
        assert_eq!(item.customer_name, "Acme Corp");
        assert_eq!(item.description.as_deref(), Some("Invoice #001"));
        assert_eq!(item.amount, Money(50_000_000_000));
        assert_eq!(item.status, ArApStatus::Open);
    }

    // ── record_payment: partial → paid ────────────────────────────────────────

    #[test]
    fn partial_payment_sets_status_to_partial() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let ar_account = make_ar_account(&conn);
        let orig_je = make_je(&conn, period_id, acct1, acct2);
        let pay_je = make_je(&conn, period_id, acct1, acct2);
        let repo = ArRepo::new(&conn);

        let item_id = repo
            .create_item(&NewArItem {
                account_id: ar_account,
                customer_name: "Beta LLC".to_string(),
                description: None,
                amount: Money(100_000_000_000), // $1000.00
                due_date: chrono::NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
                originating_je_id: orig_je,
            })
            .expect("create");

        // Partial payment: $400
        repo.record_payment(
            item_id,
            pay_je,
            Money(40_000_000_000),
            chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        )
        .expect("record partial payment");

        let (item, payments) = repo.get_with_payments(item_id).expect("get_with_payments");
        assert_eq!(item.status, ArApStatus::Partial);
        assert_eq!(payments.len(), 1);
        assert_eq!(payments[0].amount, Money(40_000_000_000));

        let total = repo.get_total_paid(item_id).expect("get_total_paid");
        assert_eq!(total, Money(40_000_000_000));
    }

    #[test]
    fn full_payment_sets_status_to_paid() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let ar_account = make_ar_account(&conn);
        let orig_je = make_je(&conn, period_id, acct1, acct2);
        let pay_je1 = make_je(&conn, period_id, acct1, acct2);
        let pay_je2 = make_je(&conn, period_id, acct1, acct2);
        let repo = ArRepo::new(&conn);

        let item_id = repo
            .create_item(&NewArItem {
                account_id: ar_account,
                customer_name: "Gamma Inc".to_string(),
                description: None,
                amount: Money(100_000_000_000), // $1000.00
                due_date: chrono::NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
                originating_je_id: orig_je,
            })
            .expect("create");

        // First payment: $600 → Partial
        repo.record_payment(
            item_id,
            pay_je1,
            Money(60_000_000_000),
            chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        )
        .expect("partial payment");

        let items = repo
            .list(&ArFilter {
                status: Some(ArApStatus::Partial),
            })
            .expect("list partial");
        assert_eq!(items.len(), 1, "Should be in Partial state");

        // Second payment: $400 → Paid
        repo.record_payment(
            item_id,
            pay_je2,
            Money(40_000_000_000),
            chrono::NaiveDate::from_ymd_opt(2026, 2, 15).unwrap(),
        )
        .expect("final payment");

        let (item, payments) = repo.get_with_payments(item_id).expect("get");
        assert_eq!(item.status, ArApStatus::Paid);
        assert_eq!(payments.len(), 2);

        let total = repo.get_total_paid(item_id).expect("total");
        assert_eq!(total, Money(100_000_000_000));
    }

    // ── terminal state and overpayment guards ─────────────────────────────────

    #[test]
    fn paid_item_rejects_further_payments() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let ar_account = make_ar_account(&conn);
        let orig_je = make_je(&conn, period_id, acct1, acct2);
        let pay_je1 = make_je(&conn, period_id, acct1, acct2);
        let pay_je2 = make_je(&conn, period_id, acct1, acct2);
        let repo = ArRepo::new(&conn);

        let item_id = repo
            .create_item(&NewArItem {
                account_id: ar_account,
                customer_name: "Delta Co".to_string(),
                description: None,
                amount: Money(10_000_000_000), // $100.00
                due_date: chrono::NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
                originating_je_id: orig_je,
            })
            .expect("create");

        // Full payment → Paid.
        repo.record_payment(
            item_id,
            pay_je1,
            Money(10_000_000_000),
            chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        )
        .expect("full payment");

        // Attempt another payment → should fail.
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
            "Error should mention Paid terminal state: {msg}"
        );
    }

    #[test]
    fn overpayment_rejected() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let ar_account = make_ar_account(&conn);
        let orig_je = make_je(&conn, period_id, acct1, acct2);
        let pay_je = make_je(&conn, period_id, acct1, acct2);
        let repo = ArRepo::new(&conn);

        let item_id = repo
            .create_item(&NewArItem {
                account_id: ar_account,
                customer_name: "Epsilon Ltd".to_string(),
                description: None,
                amount: Money(10_000_000_000), // $100.00
                due_date: chrono::NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
                originating_je_id: orig_je,
            })
            .expect("create");

        // Attempt to pay $150 against a $100 item.
        let result = repo.record_payment(
            item_id,
            pay_je,
            Money(15_000_000_000),
            chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        );
        assert!(result.is_err(), "Overpayment should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Overpayment"),
            "Error should mention overpayment: {msg}"
        );
    }

    // ── list with status filter ───────────────────────────────────────────────

    #[test]
    fn list_status_filter_works() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let ar_account = make_ar_account(&conn);
        let orig1 = make_je(&conn, period_id, acct1, acct2);
        let orig2 = make_je(&conn, period_id, acct1, acct2);
        let pay_je = make_je(&conn, period_id, acct1, acct2);
        let repo = ArRepo::new(&conn);

        let id1 = repo
            .create_item(&NewArItem {
                account_id: ar_account,
                customer_name: "Zeta Corp".to_string(),
                description: None,
                amount: Money(10_000_000_000),
                due_date: chrono::NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
                originating_je_id: orig1,
            })
            .expect("create 1");

        repo.create_item(&NewArItem {
            account_id: ar_account,
            customer_name: "Eta Inc".to_string(),
            description: None,
            amount: Money(20_000_000_000),
            due_date: chrono::NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
            originating_je_id: orig2,
        })
        .expect("create 2");

        // Partially pay item 1 → Partial.
        repo.record_payment(
            id1,
            pay_je,
            Money(5_000_000_000),
            chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        )
        .expect("partial");

        let all = repo.list(&ArFilter::default()).expect("all");
        assert_eq!(all.len(), 2);

        let open = repo
            .list(&ArFilter {
                status: Some(ArApStatus::Open),
            })
            .expect("open");
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].customer_name, "Eta Inc");

        let partial = repo
            .list(&ArFilter {
                status: Some(ArApStatus::Partial),
            })
            .expect("partial");
        assert_eq!(partial.len(), 1);
        assert_eq!(partial[0].customer_name, "Zeta Corp");
    }

    // ── get_with_payments ─────────────────────────────────────────────────────

    #[test]
    fn get_with_payments_returns_correct_data() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let ar_account = make_ar_account(&conn);
        let orig_je = make_je(&conn, period_id, acct1, acct2);
        let pay1 = make_je(&conn, period_id, acct1, acct2);
        let pay2 = make_je(&conn, period_id, acct1, acct2);
        let repo = ArRepo::new(&conn);

        let item_id = repo
            .create_item(&NewArItem {
                account_id: ar_account,
                customer_name: "Theta LLC".to_string(),
                description: None,
                amount: Money(30_000_000_000),
                due_date: chrono::NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
                originating_je_id: orig_je,
            })
            .expect("create");

        let d1 = chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let d2 = chrono::NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

        repo.record_payment(item_id, pay1, Money(10_000_000_000), d1)
            .expect("payment 1");
        repo.record_payment(item_id, pay2, Money(10_000_000_000), d2)
            .expect("payment 2");

        let (item, payments) = repo.get_with_payments(item_id).expect("get_with_payments");
        assert_eq!(item.customer_name, "Theta LLC");
        assert_eq!(item.status, ArApStatus::Partial);
        assert_eq!(payments.len(), 2);
        // Ordered by payment_date ASC.
        assert_eq!(payments[0].payment_date, d1);
        assert_eq!(payments[1].payment_date, d2);
        assert_eq!(payments[0].amount, Money(10_000_000_000));
    }

    #[test]
    fn get_with_payments_nonexistent_id_returns_error() {
        let (conn, _, _, _) = setup_db();
        let repo = ArRepo::new(&conn);
        let result = repo.get_with_payments(ArItemId::from(9999));
        assert!(result.is_err(), "Nonexistent AR item should return error");
    }
}
