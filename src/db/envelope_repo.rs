use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};
use uuid::Uuid;

use super::now_str;
use crate::types::{
    AccountId, EnvelopeAllocationId, EnvelopeEntryType, EnvelopeLedgerId, JournalEntryId, Money,
    Percentage,
};

// ── Data structs ──────────────────────────────────────────────────────────────

/// One row from `envelope_allocations`.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvelopeAllocation {
    pub id: EnvelopeAllocationId,
    pub account_id: AccountId,
    /// Primary allocation percentage.
    pub percentage: Percentage,
    /// Secondary allocation percentage (receives overflow from capped primary fills).
    pub secondary_percentage: Percentage,
    /// Cap on earmarked balance that gates primary fills. `None` means no cap.
    pub cap_amount: Option<Money>,
    pub created_at: String,
    pub updated_at: String,
}

/// One row from `envelope_ledger`.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvelopeLedgerEntry {
    pub id: EnvelopeLedgerId,
    pub account_id: AccountId,
    pub entry_type: EnvelopeEntryType,
    /// Signed amount: positive = add to balance, negative = remove.
    pub amount: Money,
    pub source_je_id: Option<JournalEntryId>,
    pub related_account_id: Option<AccountId>,
    pub transfer_group_id: Option<String>,
    pub memo: Option<String>,
    pub created_at: String,
}

// ── Repository ────────────────────────────────────────────────────────────────

/// Repository for `envelope_allocations` and `envelope_ledger`.
pub struct EnvelopeRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> EnvelopeRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    // ── Allocations ───────────────────────────────────────────────────────────

    /// Upserts an allocation for an account.
    /// Stores primary percentage, secondary percentage, and optional cap amount atomically.
    pub fn set_allocation(
        &self,
        account_id: AccountId,
        percentage: Percentage,
        secondary_percentage: Percentage,
        cap_amount: Option<Money>,
    ) -> Result<()> {
        let now = now_str();
        self.conn
            .execute(
                "INSERT INTO envelope_allocations
                     (account_id, percentage, secondary_percentage, cap_amount, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(account_id) DO UPDATE SET
                     percentage = excluded.percentage,
                     secondary_percentage = excluded.secondary_percentage,
                     cap_amount = excluded.cap_amount,
                     updated_at = excluded.updated_at",
                params![
                    i64::from(account_id),
                    percentage.0,
                    secondary_percentage.0,
                    cap_amount.map(|m| m.0),
                    now,
                    now
                ],
            )
            .context("Failed to set envelope allocation")?;
        Ok(())
    }

    /// Returns the sum of all primary percentages across all allocations.
    pub fn total_primary_percentage(&self) -> Result<Percentage> {
        let raw: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(percentage), 0) FROM envelope_allocations",
                [],
                |row| row.get(0),
            )
            .context("Failed to sum primary percentages")?;
        Ok(Percentage(raw))
    }

    /// Returns the sum of all secondary percentages across all allocations.
    pub fn total_secondary_percentage(&self) -> Result<Percentage> {
        let raw: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(secondary_percentage), 0) FROM envelope_allocations",
                [],
                |row| row.get(0),
            )
            .context("Failed to sum secondary percentages")?;
        Ok(Percentage(raw))
    }

    /// Removes the allocation for an account (deletes the row).
    pub fn remove_allocation(&self, account_id: AccountId) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM envelope_allocations WHERE account_id = ?1",
                params![i64::from(account_id)],
            )
            .context("Failed to remove envelope allocation")?;
        Ok(())
    }

    /// Returns all configured allocations, ordered by account_id.
    pub fn get_all_allocations(&self) -> Result<Vec<EnvelopeAllocation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account_id, percentage, secondary_percentage, cap_amount,
                    created_at, updated_at
             FROM envelope_allocations
             ORDER BY account_id ASC",
        )?;
        stmt.query_map([], row_to_allocation)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect()
    }

    // ── Ledger mutations ──────────────────────────────────────────────────────

    /// Records a Fill entry: cash receipt triggered an envelope fill.
    /// `amount` must be positive.
    pub fn record_fill(
        &self,
        account_id: AccountId,
        amount: Money,
        je_id: JournalEntryId,
    ) -> Result<()> {
        if amount.0 <= 0 {
            bail!("Fill amount must be positive, got {}", amount);
        }
        let now = now_str();
        self.conn
            .execute(
                "INSERT INTO envelope_ledger
                    (account_id, entry_type, amount, source_je_id, related_account_id,
                     transfer_group_id, memo, created_at)
                 VALUES (?1, ?2, ?3, ?4, NULL, NULL, NULL, ?5)",
                params![
                    i64::from(account_id),
                    EnvelopeEntryType::Fill.to_string(),
                    amount.0,
                    i64::from(je_id),
                    now,
                ],
            )
            .context("Failed to record envelope fill")?;
        Ok(())
    }

    /// Records a Transfer between two accounts.
    /// Creates two paired ledger rows sharing a `transfer_group_id` (UUID).
    /// Validates that `source` has sufficient balance.
    /// Returns the UUID used for the transfer group.
    pub fn record_transfer(
        &self,
        source: AccountId,
        dest: AccountId,
        amount: Money,
    ) -> Result<Uuid> {
        if amount.0 <= 0 {
            bail!("Transfer amount must be positive, got {}", amount);
        }

        let source_balance = self.get_balance(source)?;
        if source_balance.0 < amount.0 {
            bail!(
                "Insufficient envelope balance: {} available, {} requested",
                source_balance,
                amount,
            );
        }

        let group_id = Uuid::new_v4();
        let group_str = group_id.to_string();
        let now = now_str();

        self.conn
            .execute(
                "INSERT INTO envelope_ledger
                    (account_id, entry_type, amount, source_je_id, related_account_id,
                     transfer_group_id, memo, created_at)
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, NULL, ?6)",
                params![
                    i64::from(source),
                    EnvelopeEntryType::Transfer.to_string(),
                    -amount.0,
                    i64::from(dest),
                    group_str,
                    now,
                ],
            )
            .context("Failed to insert source transfer row")?;

        self.conn
            .execute(
                "INSERT INTO envelope_ledger
                    (account_id, entry_type, amount, source_je_id, related_account_id,
                     transfer_group_id, memo, created_at)
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, NULL, ?6)",
                params![
                    i64::from(dest),
                    EnvelopeEntryType::Transfer.to_string(),
                    amount.0,
                    i64::from(source),
                    group_str,
                    now,
                ],
            )
            .context("Failed to insert destination transfer row")?;

        Ok(group_id)
    }

    /// Records a Reversal entry for a previously-filled JE.
    /// `amount` must be positive (it will be stored as negative to reduce the balance).
    pub fn record_reversal(
        &self,
        account_id: AccountId,
        amount: Money,
        je_id: JournalEntryId,
    ) -> Result<()> {
        if amount.0 <= 0 {
            bail!("Reversal amount must be positive, got {}", amount);
        }
        let now = now_str();
        self.conn
            .execute(
                "INSERT INTO envelope_ledger
                    (account_id, entry_type, amount, source_je_id, related_account_id,
                     transfer_group_id, memo, created_at)
                 VALUES (?1, ?2, ?3, ?4, NULL, NULL, NULL, ?5)",
                params![
                    i64::from(account_id),
                    EnvelopeEntryType::Reversal.to_string(),
                    -amount.0,
                    i64::from(je_id),
                    now,
                ],
            )
            .context("Failed to record envelope reversal")?;
        Ok(())
    }

    // ── Ledger queries ────────────────────────────────────────────────────────

    /// Returns the envelope balance for an account within a date range: `SUM(amount)`
    /// where `created_at` falls between `start` and `end` (inclusive, date-only comparison).
    /// Returns `Money(0)` if no entries exist in the range.
    pub fn get_balance_for_date_range(
        &self,
        account_id: AccountId,
        start: chrono::NaiveDate,
        end: chrono::NaiveDate,
    ) -> Result<Money> {
        let raw: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(el.amount), 0)
                 FROM envelope_ledger el
                 WHERE el.account_id = ?1
                   AND date(el.created_at) >= ?2
                   AND date(el.created_at) <= ?3",
                params![i64::from(account_id), start.to_string(), end.to_string(),],
                |row| row.get(0),
            )
            .context("Failed to compute envelope balance for date range")?;
        Ok(Money(raw))
    }

    /// Returns the current envelope balance for an account: `SUM(amount)`.
    /// Returns `Money(0)` if no entries exist.
    pub fn get_balance(&self, account_id: AccountId) -> Result<Money> {
        let raw: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(amount), 0) FROM envelope_ledger WHERE account_id = ?1",
                params![i64::from(account_id)],
                |row| row.get(0),
            )
            .context("Failed to compute envelope balance")?;
        Ok(Money(raw))
    }

    /// Returns all ledger entries for an account, ordered by created_at ASC.
    pub fn get_ledger(&self, account_id: AccountId) -> Result<Vec<EnvelopeLedgerEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account_id, entry_type, amount, source_je_id, related_account_id,
                    transfer_group_id, memo, created_at
             FROM envelope_ledger
             WHERE account_id = ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        stmt.query_map(params![i64::from(account_id)], row_to_ledger_entry)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect()
    }

    /// Returns all Fill entries for a specific journal entry (used by reversal wiring).
    pub fn get_fills_for_je(&self, je_id: JournalEntryId) -> Result<Vec<EnvelopeLedgerEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account_id, entry_type, amount, source_je_id, related_account_id,
                    transfer_group_id, memo, created_at
             FROM envelope_ledger
             WHERE source_je_id = ?1 AND entry_type = ?2
             ORDER BY id ASC",
        )?;
        stmt.query_map(
            params![i64::from(je_id), EnvelopeEntryType::Fill.to_string()],
            row_to_ledger_entry,
        )?
        .map(|r| r.map_err(anyhow::Error::from))
        .collect()
    }
}

// ── Row mappers ───────────────────────────────────────────────────────────────

fn row_to_allocation(row: &rusqlite::Row<'_>) -> rusqlite::Result<EnvelopeAllocation> {
    Ok(EnvelopeAllocation {
        id: EnvelopeAllocationId::from(row.get::<_, i64>(0)?),
        account_id: AccountId::from(row.get::<_, i64>(1)?),
        percentage: Percentage(row.get::<_, i64>(2)?),
        secondary_percentage: Percentage(row.get::<_, i64>(3)?),
        cap_amount: row.get::<_, Option<i64>>(4)?.map(Money),
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn row_to_ledger_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<EnvelopeLedgerEntry> {
    let type_str: String = row.get(2)?;
    let entry_type = type_str.parse::<EnvelopeEntryType>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(EnvelopeLedgerEntry {
        id: EnvelopeLedgerId::from(row.get::<_, i64>(0)?),
        account_id: AccountId::from(row.get::<_, i64>(1)?),
        entry_type,
        amount: Money(row.get::<_, i64>(3)?),
        source_je_id: row.get::<_, Option<i64>>(4)?.map(JournalEntryId::from),
        related_account_id: row.get::<_, Option<i64>>(5)?.map(AccountId::from),
        transfer_group_id: row.get(6)?,
        memo: row.get(7)?,
        created_at: row.get(8)?,
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
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .expect("fk on");
        initialize_schema(&conn).expect("schema init");
        seed_default_accounts(&conn).expect("seed");

        let fiscal = FiscalRepo::new(&conn);
        let fy_id = fiscal.create_fiscal_year(1, 2026).expect("create FY");
        let periods = fiscal.list_periods(fy_id).expect("list periods");
        let period_id = periods[0].id;

        // Create two non-placeholder accounts.
        let accounts = AccountRepo::new(&conn);
        let acct1 = accounts
            .create(&NewAccount {
                number: "1110".to_string(),
                name: "Checking Account".to_string(),
                account_type: AccountType::Asset,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .unwrap_or_else(|_| {
                AccountRepo::new(&conn)
                    .list_all()
                    .unwrap()
                    .into_iter()
                    .find(|a| a.number == "1110")
                    .map(|a| a.id)
                    .expect("account 1110")
            });
        let acct2 = accounts
            .create(&NewAccount {
                number: "5100".to_string(),
                name: "Maintenance Reserve".to_string(),
                account_type: AccountType::Expense,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .unwrap_or_else(|_| {
                AccountRepo::new(&conn)
                    .list_all()
                    .unwrap()
                    .into_iter()
                    .find(|a| a.number == "5100")
                    .map(|a| a.id)
                    .expect("account 5100")
            });

        (conn, period_id, acct1, acct2)
    }

    fn make_je(
        conn: &Connection,
        period_id: FiscalPeriodId,
        a: AccountId,
        b: AccountId,
    ) -> JournalEntryId {
        JournalRepo::new(conn)
            .create_draft(&NewJournalEntry {
                entry_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                memo: None,
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: a,
                        debit_amount: Money(10_000_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: b,
                        debit_amount: Money(0),
                        credit_amount: Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create je")
    }

    // ── set_allocation / get_all_allocations ──────────────────────────────────

    #[test]
    fn set_and_get_allocation() {
        let (conn, _, acct1, _) = setup_db();
        let repo = EnvelopeRepo::new(&conn);

        repo.set_allocation(acct1, Percentage(15_000_000), Percentage(0), None) // 15%
            .expect("set allocation");

        let allocs = repo.get_all_allocations().expect("get allocations");
        assert_eq!(allocs.len(), 1);
        assert_eq!(allocs[0].account_id, acct1);
        assert_eq!(allocs[0].percentage, Percentage(15_000_000));
        assert_eq!(allocs[0].secondary_percentage, Percentage(0));
        assert_eq!(allocs[0].cap_amount, None);
    }

    #[test]
    fn set_allocation_with_secondary_and_cap() {
        let (conn, _, acct1, _) = setup_db();
        let repo = EnvelopeRepo::new(&conn);

        repo.set_allocation(
            acct1,
            Percentage(10_000_000),       // 10% primary
            Percentage(5_000_000),        // 5% secondary
            Some(Money(500_000_000_000)), // $5,000 cap
        )
        .expect("set allocation");

        let allocs = repo.get_all_allocations().expect("get allocations");
        assert_eq!(allocs.len(), 1);
        assert_eq!(allocs[0].percentage, Percentage(10_000_000));
        assert_eq!(allocs[0].secondary_percentage, Percentage(5_000_000));
        assert_eq!(allocs[0].cap_amount, Some(Money(500_000_000_000)));
    }

    #[test]
    fn set_allocation_upserts_all_fields() {
        let (conn, _, acct1, _) = setup_db();
        let repo = EnvelopeRepo::new(&conn);

        repo.set_allocation(acct1, Percentage(10_000_000), Percentage(0), None)
            .expect("first set");
        repo.set_allocation(
            acct1,
            Percentage(20_000_000),
            Percentage(5_000_000),
            Some(Money(1_000_000_000_000)),
        )
        .expect("second set");

        let allocs = repo.get_all_allocations().expect("get");
        assert_eq!(allocs.len(), 1, "Upsert should not create duplicate rows");
        assert_eq!(allocs[0].percentage, Percentage(20_000_000));
        assert_eq!(allocs[0].secondary_percentage, Percentage(5_000_000));
        assert_eq!(allocs[0].cap_amount, Some(Money(1_000_000_000_000)));
    }

    #[test]
    fn set_allocation_cap_none_stored_as_null() {
        let (conn, _, acct1, _) = setup_db();
        let repo = EnvelopeRepo::new(&conn);

        repo.set_allocation(acct1, Percentage(10_000_000), Percentage(0), None)
            .expect("set");

        let allocs = repo.get_all_allocations().expect("get");
        assert!(
            allocs[0].cap_amount.is_none(),
            "None cap should be NULL in DB"
        );
    }

    #[test]
    fn remove_allocation() {
        let (conn, _, acct1, acct2) = setup_db();
        let repo = EnvelopeRepo::new(&conn);

        repo.set_allocation(acct1, Percentage(10_000_000), Percentage(0), None)
            .expect("set 1");
        repo.set_allocation(acct2, Percentage(5_000_000), Percentage(0), None)
            .expect("set 2");

        let before = repo.get_all_allocations().expect("before");
        assert_eq!(before.len(), 2);

        repo.remove_allocation(acct1).expect("remove");

        let after = repo.get_all_allocations().expect("after");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].account_id, acct2);
    }

    #[test]
    fn total_primary_and_secondary_percentages() {
        let (conn, _, acct1, acct2) = setup_db();
        let repo = EnvelopeRepo::new(&conn);

        // Empty — both should be zero.
        assert_eq!(
            repo.total_primary_percentage().expect("primary total"),
            Percentage(0)
        );
        assert_eq!(
            repo.total_secondary_percentage().expect("secondary total"),
            Percentage(0)
        );

        repo.set_allocation(acct1, Percentage(10_000_000), Percentage(40_000_000), None)
            .expect("set 1");
        repo.set_allocation(acct2, Percentage(15_000_000), Percentage(35_000_000), None)
            .expect("set 2");

        assert_eq!(
            repo.total_primary_percentage().expect("primary total"),
            Percentage(25_000_000) // 25%
        );
        assert_eq!(
            repo.total_secondary_percentage().expect("secondary total"),
            Percentage(75_000_000) // 75%
        );
    }

    // ── record_fill / get_balance ─────────────────────────────────────────────

    #[test]
    fn record_fill_increases_balance() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let je_id = make_je(&conn, period_id, acct1, acct2);
        let repo = EnvelopeRepo::new(&conn);

        let fill_amount = Money(5_000_000_000); // $50.00
        repo.record_fill(acct1, fill_amount, je_id)
            .expect("record fill");

        let balance = repo.get_balance(acct1).expect("get balance");
        assert_eq!(balance, fill_amount);
    }

    #[test]
    fn get_balance_zero_when_no_entries() {
        let (conn, _, acct1, _) = setup_db();
        let repo = EnvelopeRepo::new(&conn);
        assert_eq!(repo.get_balance(acct1).expect("balance"), Money(0));
    }

    #[test]
    fn record_fill_rejects_nonpositive_amount() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let je_id = make_je(&conn, period_id, acct1, acct2);
        let repo = EnvelopeRepo::new(&conn);

        let result = repo.record_fill(acct1, Money(0), je_id);
        assert!(result.is_err(), "Zero fill should be rejected");

        let result = repo.record_fill(acct1, Money(-100), je_id);
        assert!(result.is_err(), "Negative fill should be rejected");
    }

    // ── record_transfer ───────────────────────────────────────────────────────

    #[test]
    fn transfer_updates_both_balances() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let je_id = make_je(&conn, period_id, acct1, acct2);
        let repo = EnvelopeRepo::new(&conn);

        // Fund acct1 with $100.
        repo.record_fill(acct1, Money(10_000_000_000), je_id)
            .expect("fill");

        // Transfer $30 from acct1 to acct2.
        let transfer_amt = Money(3_000_000_000);
        repo.record_transfer(acct1, acct2, transfer_amt)
            .expect("transfer");

        let bal1 = repo.get_balance(acct1).expect("bal1");
        let bal2 = repo.get_balance(acct2).expect("bal2");
        assert_eq!(bal1, Money(7_000_000_000), "Source should decrease");
        assert_eq!(bal2, Money(3_000_000_000), "Dest should increase");
    }

    #[test]
    fn transfer_pair_sums_to_zero() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let je_id = make_je(&conn, period_id, acct1, acct2);
        let repo = EnvelopeRepo::new(&conn);

        repo.record_fill(acct1, Money(10_000_000_000), je_id)
            .expect("fill");
        let group_uuid = repo
            .record_transfer(acct1, acct2, Money(4_000_000_000))
            .expect("transfer");
        let group_str = group_uuid.to_string();

        // The two rows with this transfer_group_id should sum to zero.
        let sum: i64 = conn
            .query_row(
                "SELECT SUM(amount) FROM envelope_ledger WHERE transfer_group_id = ?1",
                params![group_str],
                |row| row.get(0),
            )
            .expect("sum query");
        assert_eq!(sum, 0, "Transfer pair must sum to zero (invariant 7)");
    }

    #[test]
    fn transfer_rejects_insufficient_balance() {
        let (conn, _, acct1, acct2) = setup_db();
        let repo = EnvelopeRepo::new(&conn);

        // acct1 has $0 balance; try to transfer $10.
        let result = repo.record_transfer(acct1, acct2, Money(1_000_000_000));
        assert!(result.is_err(), "Should reject insufficient balance");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Insufficient"),
            "Error should mention Insufficient: {msg}"
        );
    }

    // ── record_reversal ───────────────────────────────────────────────────────

    #[test]
    fn record_reversal_decreases_balance() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let je_id = make_je(&conn, period_id, acct1, acct2);
        let repo = EnvelopeRepo::new(&conn);

        // Fill $100, then reverse $100 → net balance = $0.
        repo.record_fill(acct1, Money(10_000_000_000), je_id)
            .expect("fill");
        repo.record_reversal(acct1, Money(10_000_000_000), je_id)
            .expect("reversal");

        let balance = repo.get_balance(acct1).expect("balance");
        assert_eq!(balance, Money(0), "Fill + reversal should net to zero");
    }

    // ── get_ledger ────────────────────────────────────────────────────────────

    #[test]
    fn get_ledger_returns_all_entries_for_account() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let je_id = make_je(&conn, period_id, acct1, acct2);
        let repo = EnvelopeRepo::new(&conn);

        repo.record_fill(acct1, Money(10_000_000_000), je_id)
            .expect("fill");
        repo.record_fill(acct1, Money(5_000_000_000), je_id)
            .expect("fill2");

        let ledger = repo.get_ledger(acct1).expect("ledger");
        assert_eq!(ledger.len(), 2);
        assert_eq!(ledger[0].entry_type, EnvelopeEntryType::Fill);
        assert_eq!(ledger[1].entry_type, EnvelopeEntryType::Fill);

        // acct2's ledger should be empty.
        let ledger2 = repo.get_ledger(acct2).expect("ledger2");
        assert_eq!(ledger2.len(), 0);
    }

    // ── get_fills_for_je ──────────────────────────────────────────────────────

    #[test]
    fn get_fills_for_je_returns_correct_entries() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let je1 = make_je(&conn, period_id, acct1, acct2);
        let je2 = make_je(&conn, period_id, acct1, acct2);
        let repo = EnvelopeRepo::new(&conn);

        repo.record_fill(acct1, Money(5_000_000_000), je1)
            .expect("fill je1 acct1");
        repo.record_fill(acct2, Money(3_000_000_000), je1)
            .expect("fill je1 acct2");
        repo.record_fill(acct1, Money(2_000_000_000), je2)
            .expect("fill je2");

        let fills_je1 = repo.get_fills_for_je(je1).expect("fills for je1");
        assert_eq!(fills_je1.len(), 2);
        assert!(fills_je1.iter().all(|e| e.source_je_id == Some(je1)));

        let fills_je2 = repo.get_fills_for_je(je2).expect("fills for je2");
        assert_eq!(fills_je2.len(), 1);
        assert_eq!(fills_je2[0].account_id, acct1);
    }

    // ── get_balance_for_date_range ───────────────────────────────────────────

    #[test]
    fn get_balance_for_date_range_filters_by_created_at() {
        let (conn, period_id, acct1, acct2) = setup_db();
        let je_id = make_je(&conn, period_id, acct1, acct2);
        let repo = EnvelopeRepo::new(&conn);

        // Fill $50 — created_at is today (2026-ish based on test setup).
        repo.record_fill(acct1, Money(5_000_000_000), je_id)
            .expect("fill");

        // Date range that includes today should show the balance.
        let today = chrono::Local::now().date_naive();
        let start = today - chrono::Duration::days(1);
        let end = today + chrono::Duration::days(1);
        let in_range = repo
            .get_balance_for_date_range(acct1, start, end)
            .expect("in range");
        assert_eq!(in_range, Money(5_000_000_000));

        // Date range in the past should return zero.
        let old_start = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).expect("date");
        let old_end = chrono::NaiveDate::from_ymd_opt(2020, 12, 31).expect("date");
        let out_of_range = repo
            .get_balance_for_date_range(acct1, old_start, old_end)
            .expect("out of range");
        assert_eq!(out_of_range, Money(0));
    }
}
