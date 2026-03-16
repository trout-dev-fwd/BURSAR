use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};

use super::now_str;
use crate::types::{AccountId, AccountType, Money};

/// A full account row loaded from the database.
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub id: AccountId,
    pub number: String,
    pub name: String,
    pub account_type: AccountType,
    pub parent_id: Option<AccountId>,
    pub is_active: bool,
    pub is_contra: bool,
    pub is_placeholder: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Data required to create a new account.
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub number: String,
    pub name: String,
    pub account_type: AccountType,
    pub parent_id: Option<AccountId>,
    pub is_contra: bool,
    pub is_placeholder: bool,
}

/// Fields that can be changed on an existing account (None = no change).
#[derive(Debug, Clone, Default)]
pub struct AccountUpdate {
    pub name: Option<String>,
    pub number: Option<String>,
}

/// Repository for the `accounts` table.
pub struct AccountRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> AccountRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    /// Returns all accounts ordered by account number.
    pub fn list_all(&self) -> Result<Vec<Account>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, number, name, account_type, parent_id,
                    is_active, is_contra, is_placeholder, created_at, updated_at
             FROM accounts
             ORDER BY number",
        )?;
        stmt.query_map(params![], row_to_account)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect()
    }

    /// Returns only active accounts ordered by account number.
    pub fn list_active(&self) -> Result<Vec<Account>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, number, name, account_type, parent_id,
                    is_active, is_contra, is_placeholder, created_at, updated_at
             FROM accounts
             WHERE is_active = 1
             ORDER BY number",
        )?;
        stmt.query_map(params![], row_to_account)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect()
    }

    /// Returns the account with the given ID, or an error if not found.
    pub fn get_by_id(&self, id: AccountId) -> Result<Account> {
        self.conn
            .query_row(
                "SELECT id, number, name, account_type, parent_id,
                         is_active, is_contra, is_placeholder, created_at, updated_at
                  FROM accounts WHERE id = ?1",
                params![i64::from(id)],
                row_to_account,
            )
            .with_context(|| format!("Account not found: {}", i64::from(id)))
    }

    /// Creates a new account. Returns the new account's ID.
    /// Errors on duplicate number or non-existent parent.
    pub fn create(&self, new: &NewAccount) -> Result<AccountId> {
        // Verify parent exists if provided (belt-and-suspenders beyond FK constraint).
        if let Some(pid) = new.parent_id {
            let count: i64 = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM accounts WHERE id = ?1",
                    params![i64::from(pid)],
                    |row| row.get(0),
                )
                .context("Failed to check parent account existence")?;
            if count == 0 {
                bail!("Parent account {} does not exist", i64::from(pid));
            }
        }

        let now = now_str();
        self.conn
            .execute(
                "INSERT INTO accounts
                    (number, name, account_type, parent_id, is_active,
                     is_contra, is_placeholder, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8)",
                params![
                    new.number,
                    new.name,
                    new.account_type.to_string(),
                    new.parent_id.map(i64::from),
                    new.is_contra as i64,
                    new.is_placeholder as i64,
                    now,
                    now,
                ],
            )
            .with_context(|| format!("Failed to create account '{}'", new.number))?;

        Ok(AccountId::from(self.conn.last_insert_rowid()))
    }

    /// Updates name and/or number on an existing account. Type cannot change.
    /// Both changes are applied in a single UPDATE for atomicity.
    pub fn update(&self, id: AccountId, changes: &AccountUpdate) -> Result<()> {
        if changes.name.is_none() && changes.number.is_none() {
            return Ok(());
        }
        let now = now_str();
        self.conn
            .execute(
                "UPDATE accounts
                 SET name       = COALESCE(?1, name),
                     number     = COALESCE(?2, number),
                     updated_at = ?3
                 WHERE id = ?4",
                params![changes.name, changes.number, now, i64::from(id)],
            )
            .with_context(|| format!("Failed to update account {}", i64::from(id)))?;
        Ok(())
    }

    /// Sets `is_active = 0` for the given account.
    pub fn deactivate(&self, id: AccountId) -> Result<()> {
        self.conn.execute(
            "UPDATE accounts SET is_active = 0, updated_at = ?1 WHERE id = ?2",
            params![now_str(), i64::from(id)],
        )?;
        Ok(())
    }

    /// Sets `is_active = 1` for the given account.
    pub fn activate(&self, id: AccountId) -> Result<()> {
        self.conn.execute(
            "UPDATE accounts SET is_active = 1, updated_at = ?1 WHERE id = ?2",
            params![now_str(), i64::from(id)],
        )?;
        Ok(())
    }

    /// Permanently deletes an unused account. Returns an error if the account has any
    /// journal entry lines, AR/AP items, child accounts, envelope allocations, or
    /// fixed asset details. Accounts that have been used should be deactivated instead.
    pub fn delete(&self, id: AccountId) -> Result<()> {
        let id_val = i64::from(id);

        // Guard: journal entry lines
        let je_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM journal_entry_lines WHERE account_id = ?1",
            params![id_val],
            |row| row.get(0),
        )?;
        if je_count > 0 {
            bail!(
                "Cannot delete: account has {} journal entr{}",
                je_count,
                if je_count == 1 { "y" } else { "ies" }
            );
        }

        // Guard: AR items
        let ar_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM ar_items WHERE account_id = ?1",
            params![id_val],
            |row| row.get(0),
        )?;
        if ar_count > 0 {
            bail!(
                "Cannot delete: account has {} AR item{}",
                ar_count,
                if ar_count == 1 { "" } else { "s" }
            );
        }

        // Guard: AP items
        let ap_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM ap_items WHERE account_id = ?1",
            params![id_val],
            |row| row.get(0),
        )?;
        if ap_count > 0 {
            bail!(
                "Cannot delete: account has {} AP item{}",
                ap_count,
                if ap_count == 1 { "" } else { "s" }
            );
        }

        // Guard: child accounts
        let child_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM accounts WHERE parent_id = ?1",
            params![id_val],
            |row| row.get(0),
        )?;
        if child_count > 0 {
            bail!(
                "Cannot delete: account has {} child account{}",
                child_count,
                if child_count == 1 { "" } else { "s" }
            );
        }

        // Guard: envelope allocations
        let env_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM envelope_allocations WHERE account_id = ?1",
            params![id_val],
            |row| row.get(0),
        )?;
        if env_count > 0 {
            bail!(
                "Cannot delete: account has {} envelope allocation{}",
                env_count,
                if env_count == 1 { "" } else { "s" }
            );
        }

        // Guard: fixed asset details
        let fa_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM fixed_asset_details WHERE account_id = ?1",
            params![id_val],
            |row| row.get(0),
        )?;
        if fa_count > 0 {
            bail!(
                "Cannot delete: account has {} fixed asset detail{}",
                fa_count,
                if fa_count == 1 { "" } else { "s" }
            );
        }

        let rows_affected = self
            .conn
            .execute("DELETE FROM accounts WHERE id = ?1", params![id_val])?;
        if rows_affected == 0 {
            bail!("Account not found: {}", id_val);
        }

        Ok(())
    }

    /// Returns direct children of `parent_id` ordered by account number.
    pub fn get_children(&self, parent_id: AccountId) -> Result<Vec<Account>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, number, name, account_type, parent_id,
                    is_active, is_contra, is_placeholder, created_at, updated_at
             FROM accounts
             WHERE parent_id = ?1
             ORDER BY number",
        )?;
        stmt.query_map(params![i64::from(parent_id)], row_to_account)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect()
    }

    /// Full-text substring search on name and number, ordered by number.
    pub fn search(&self, query: &str) -> Result<Vec<Account>> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, number, name, account_type, parent_id,
                    is_active, is_contra, is_placeholder, created_at, updated_at
             FROM accounts
             WHERE name LIKE ?1 OR number LIKE ?1
             ORDER BY number",
        )?;
        stmt.query_map(params![pattern], row_to_account)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect()
    }

    /// Returns the net debit balance for an account across posted journal entry lines
    /// whose `entry_date` falls within the given date range (inclusive).
    /// Returns `Money(0)` when no posted lines exist in the range.
    pub fn get_balance_for_date_range(
        &self,
        id: AccountId,
        start: chrono::NaiveDate,
        end: chrono::NaiveDate,
    ) -> Result<Money> {
        let raw: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(jel.debit_amount - jel.credit_amount), 0)
                 FROM journal_entry_lines jel
                 JOIN journal_entries je ON je.id = jel.journal_entry_id
                 WHERE jel.account_id = ?1
                   AND je.status = 'Posted'
                   AND je.entry_date >= ?2
                   AND je.entry_date <= ?3",
                params![i64::from(id), start.to_string(), end.to_string()],
                |row| row.get(0),
            )
            .context("Failed to compute account balance for date range")?;
        Ok(Money(raw))
    }

    /// Returns the net debit balance for an account across all posted journal entry lines.
    /// Returns `Money(0)` when no posted lines exist (correct query, no data yet).
    pub fn get_balance(&self, id: AccountId) -> Result<Money> {
        let raw: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(jel.debit_amount - jel.credit_amount), 0)
                 FROM journal_entry_lines jel
                 JOIN journal_entries je ON je.id = jel.journal_entry_id
                 WHERE jel.account_id = ?1
                   AND je.status = 'Posted'",
                params![i64::from(id)],
                |row| row.get(0),
            )
            .context("Failed to compute account balance")?;
        Ok(Money(raw))
    }

    /// Returns net debit balances for all accounts that have posted JE lines,
    /// in a single query. Accounts with no posted lines are absent from the map
    /// (callers should treat absence as `Money(0)`).
    pub fn get_all_balances(&self) -> Result<HashMap<AccountId, Money>> {
        let mut stmt = self.conn.prepare(
            "SELECT jel.account_id,
                    COALESCE(SUM(jel.debit_amount - jel.credit_amount), 0)
             FROM journal_entry_lines jel
             JOIN journal_entries je ON je.id = jel.journal_entry_id
             WHERE je.status = 'Posted'
             GROUP BY jel.account_id",
        )?;
        let map = stmt
            .query_map(params![], |row| {
                let id: i64 = row.get(0)?;
                let amount: i64 = row.get(1)?;
                Ok((AccountId::from(id), Money(amount)))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(map)
    }

    /// Returns net debit balances for all accounts with posted JE lines on or before `as_of`.
    /// Accounts with no activity up to that date are absent (treat absence as `Money(0)`).
    pub fn get_all_balances_as_of(
        &self,
        as_of: chrono::NaiveDate,
    ) -> Result<HashMap<AccountId, Money>> {
        let mut stmt = self.conn.prepare(
            "SELECT jel.account_id,
                    COALESCE(SUM(jel.debit_amount - jel.credit_amount), 0)
             FROM journal_entry_lines jel
             JOIN journal_entries je ON je.id = jel.journal_entry_id
             WHERE je.status = 'Posted'
               AND je.entry_date <= ?1
             GROUP BY jel.account_id",
        )?;
        let map = stmt
            .query_map(params![as_of.to_string()], |row| {
                let id: i64 = row.get(0)?;
                let amount: i64 = row.get(1)?;
                Ok((AccountId::from(id), Money(amount)))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(map)
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let account_type_str: String = row.get(3)?;
    let account_type = account_type_str.parse::<AccountType>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let parent_id_raw: Option<i64> = row.get(4)?;
    Ok(Account {
        id: AccountId::from(row.get::<_, i64>(0)?),
        number: row.get(1)?,
        name: row.get(2)?,
        account_type,
        parent_id: parent_id_raw.map(AccountId::from),
        is_active: row.get::<_, i32>(5)? != 0,
        is_contra: row.get::<_, i32>(6)? != 0,
        is_placeholder: row.get::<_, i32>(7)? != 0,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::{initialize_schema, seed_default_accounts};
    use rusqlite::Connection;

    fn db_seeded() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema init");
        seed_default_accounts(&conn).expect("seed accounts");
        conn
    }

    fn db_empty() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema init");
        conn
    }

    // ── list_active ───────────────────────────────────────────────────────────

    #[test]
    fn list_active_returns_seeded_accounts() {
        let conn = db_seeded();
        let repo = AccountRepo::new(&conn);
        let accounts = repo.list_active().expect("list_active failed");
        // The seed inserts 5 top-level + multiple children — just verify non-empty.
        assert!(!accounts.is_empty(), "Should have seeded accounts");
        // All returned accounts must be active.
        assert!(
            accounts.iter().all(|a| a.is_active),
            "list_active should only return active accounts"
        );
    }

    #[test]
    fn list_all_includes_inactive() {
        let conn = db_seeded();
        let repo = AccountRepo::new(&conn);

        // Deactivate one account.
        let all_before = repo.list_all().expect("list_all");
        let target = all_before.first().expect("at least one account");
        repo.deactivate(target.id).expect("deactivate");

        let active = repo.list_active().expect("list_active");
        let all_after = repo.list_all().expect("list_all after");

        assert_eq!(
            all_before.len(),
            all_after.len(),
            "list_all count unchanged"
        );
        assert_eq!(
            active.len(),
            all_before.len() - 1,
            "list_active drops the deactivated account"
        );
    }

    // ── create / get_by_id ────────────────────────────────────────────────────

    #[test]
    fn create_and_retrieve_account() {
        let conn = db_empty();
        let repo = AccountRepo::new(&conn);

        let new = NewAccount {
            number: "1010".to_string(),
            name: "Test Cash".to_string(),
            account_type: AccountType::Asset,
            parent_id: None,
            is_contra: false,
            is_placeholder: false,
        };
        let id = repo.create(&new).expect("create failed");
        let account = repo.get_by_id(id).expect("get_by_id failed");

        assert_eq!(account.number, "1010");
        assert_eq!(account.name, "Test Cash");
        assert_eq!(account.account_type, AccountType::Asset);
        assert!(account.is_active);
        assert!(!account.is_contra);
        assert!(!account.is_placeholder);
        assert!(account.parent_id.is_none());
    }

    #[test]
    fn create_sub_account_with_parent() {
        let conn = db_empty();
        let repo = AccountRepo::new(&conn);

        let parent_id = repo
            .create(&NewAccount {
                number: "1000".to_string(),
                name: "Assets".to_string(),
                account_type: AccountType::Asset,
                parent_id: None,
                is_contra: false,
                is_placeholder: true,
            })
            .expect("create parent");

        let child_id = repo
            .create(&NewAccount {
                number: "1010".to_string(),
                name: "Cash".to_string(),
                account_type: AccountType::Asset,
                parent_id: Some(parent_id),
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create child");

        let child = repo.get_by_id(child_id).expect("get child");
        assert_eq!(child.parent_id, Some(parent_id));
    }

    // ── update ────────────────────────────────────────────────────────────────

    #[test]
    fn update_name_and_number() {
        let conn = db_empty();
        let repo = AccountRepo::new(&conn);

        let id = repo
            .create(&NewAccount {
                number: "1010".to_string(),
                name: "Old Name".to_string(),
                account_type: AccountType::Asset,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create");

        repo.update(
            id,
            &AccountUpdate {
                name: Some("New Name".to_string()),
                number: Some("1020".to_string()),
            },
        )
        .expect("update");

        let updated = repo.get_by_id(id).expect("get");
        assert_eq!(updated.name, "New Name");
        assert_eq!(updated.number, "1020");
    }

    #[test]
    fn update_with_none_fields_is_no_op() {
        let conn = db_empty();
        let repo = AccountRepo::new(&conn);

        let id = repo
            .create(&NewAccount {
                number: "1010".to_string(),
                name: "Stable Name".to_string(),
                account_type: AccountType::Asset,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create");

        repo.update(id, &AccountUpdate::default())
            .expect("empty update");

        let account = repo.get_by_id(id).expect("get");
        assert_eq!(account.name, "Stable Name");
        assert_eq!(account.number, "1010");
    }

    // ── deactivate / activate ─────────────────────────────────────────────────

    #[test]
    fn deactivate_excludes_from_list_active() {
        let conn = db_empty();
        let repo = AccountRepo::new(&conn);

        let id = repo
            .create(&NewAccount {
                number: "1010".to_string(),
                name: "Cash".to_string(),
                account_type: AccountType::Asset,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create");

        repo.deactivate(id).expect("deactivate");

        let active = repo.list_active().expect("list_active");
        assert!(
            !active.iter().any(|a| a.id == id),
            "Deactivated account should not appear in list_active"
        );

        let account = repo.get_by_id(id).expect("get_by_id");
        assert!(!account.is_active, "is_active should be false");
    }

    #[test]
    fn activate_restores_account() {
        let conn = db_empty();
        let repo = AccountRepo::new(&conn);

        let id = repo
            .create(&NewAccount {
                number: "1010".to_string(),
                name: "Cash".to_string(),
                account_type: AccountType::Asset,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create");

        repo.deactivate(id).expect("deactivate");
        repo.activate(id).expect("activate");

        let account = repo.get_by_id(id).expect("get_by_id");
        assert!(account.is_active, "is_active should be restored to true");
    }

    // ── get_children ──────────────────────────────────────────────────────────

    #[test]
    fn get_children_returns_direct_children_only() {
        let conn = db_seeded();
        let repo = AccountRepo::new(&conn);

        // Find the Assets root account (1000).
        let all = repo.list_all().expect("list_all");
        let assets = all
            .iter()
            .find(|a| a.number == "1000")
            .expect("assets account");
        let children = repo.get_children(assets.id).expect("get_children");

        // Assets (1000) has direct children: 1100, 1200, 1300, 1400, 1500.
        assert!(!children.is_empty(), "Assets should have direct children");
        // All children have parent_id == assets.id.
        assert!(
            children.iter().all(|c| c.parent_id == Some(assets.id)),
            "All children should have correct parent_id"
        );
        // Grandchildren (e.g. 1110) are NOT returned.
        assert!(
            !children.iter().any(|c| c.number == "1110"),
            "get_children should not return grandchildren"
        );
    }

    // ── search ────────────────────────────────────────────────────────────────

    #[test]
    fn search_finds_cash_accounts() {
        let conn = db_seeded();
        let repo = AccountRepo::new(&conn);

        let results = repo.search("cash").expect("search failed");
        assert!(
            !results.is_empty(),
            "search('cash') should return at least one account"
        );
        assert!(
            results
                .iter()
                .any(|a| a.name.to_lowercase().contains("cash")
                    || a.number.to_lowercase().contains("cash")),
            "Results should contain cash-related accounts"
        );
    }

    #[test]
    fn search_is_case_insensitive_for_sqlite_ascii() {
        let conn = db_seeded();
        let repo = AccountRepo::new(&conn);

        let lower = repo.search("cash").expect("lower");
        let upper = repo.search("Cash").expect("upper");
        // SQLite LIKE is case-insensitive for ASCII.
        assert_eq!(lower.len(), upper.len());
    }

    #[test]
    fn search_by_number_prefix() {
        let conn = db_seeded();
        let repo = AccountRepo::new(&conn);

        let results = repo.search("110").expect("search by number");
        // Should find 1100 (Cash & Bank Accounts) and its children 1110, 1120.
        assert!(
            results.iter().any(|a| a.number == "1100"),
            "Should find 1100"
        );
    }

    // ── error cases ───────────────────────────────────────────────────────────

    #[test]
    fn create_duplicate_number_returns_error() {
        let conn = db_empty();
        let repo = AccountRepo::new(&conn);

        let new = NewAccount {
            number: "1010".to_string(),
            name: "First".to_string(),
            account_type: AccountType::Asset,
            parent_id: None,
            is_contra: false,
            is_placeholder: false,
        };
        repo.create(&new).expect("first create ok");

        let duplicate = NewAccount {
            number: "1010".to_string(),
            name: "Second".to_string(),
            account_type: AccountType::Asset,
            parent_id: None,
            is_contra: false,
            is_placeholder: false,
        };
        let result = repo.create(&duplicate);
        assert!(result.is_err(), "Duplicate number should return an error");
    }

    #[test]
    fn create_with_nonexistent_parent_returns_error() {
        let conn = db_empty();
        let repo = AccountRepo::new(&conn);

        let result = repo.create(&NewAccount {
            number: "1010".to_string(),
            name: "Orphan".to_string(),
            account_type: AccountType::Asset,
            parent_id: Some(AccountId::from(9999)),
            is_contra: false,
            is_placeholder: false,
        });
        assert!(
            result.is_err(),
            "Non-existent parent should return an error"
        );
    }

    // ── delete ────────────────────────────────────────────────────────────────

    #[test]
    fn delete_unused_account_succeeds() {
        let conn = db_empty();
        let repo = AccountRepo::new(&conn);

        let id = repo
            .create(&NewAccount {
                number: "9999".to_string(),
                name: "Temp".to_string(),
                account_type: AccountType::Asset,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create");

        repo.delete(id)
            .expect("delete should succeed for unused account");
        assert!(
            repo.get_by_id(id).is_err(),
            "account should no longer exist"
        );
    }

    #[test]
    fn delete_account_with_children_returns_error() {
        let conn = db_empty();
        let repo = AccountRepo::new(&conn);

        let parent_id = repo
            .create(&NewAccount {
                number: "1000".to_string(),
                name: "Parent".to_string(),
                account_type: AccountType::Asset,
                parent_id: None,
                is_contra: false,
                is_placeholder: true,
            })
            .expect("create parent");

        repo.create(&NewAccount {
            number: "1010".to_string(),
            name: "Child".to_string(),
            account_type: AccountType::Asset,
            parent_id: Some(parent_id),
            is_contra: false,
            is_placeholder: false,
        })
        .expect("create child");

        let err = repo.delete(parent_id).unwrap_err();
        assert!(
            err.to_string().contains("child account"),
            "error should mention child accounts, got: {err}"
        );
    }

    #[test]
    fn delete_account_with_journal_entries_returns_error() {
        use crate::db::{entity_db_from_conn, fiscal_repo::FiscalRepo, journal_repo::*};
        use chrono::NaiveDate;

        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::db::schema::initialize_schema(&conn).expect("schema");
        crate::db::schema::seed_default_accounts(&conn).expect("seed");
        FiscalRepo::new(&conn)
            .create_fiscal_year(1, 2026)
            .expect("fiscal year");
        let db = entity_db_from_conn(conn);

        let accounts: Vec<_> = db
            .accounts()
            .list_all()
            .unwrap()
            .into_iter()
            .filter(|a| !a.is_placeholder && a.is_active)
            .collect();
        let acct_a = &accounts[0];
        let acct_b = &accounts[1];

        let period = db
            .fiscal()
            .get_period_for_date(NaiveDate::from_ymd_opt(2026, 1, 15).unwrap())
            .unwrap();

        db.journals()
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                memo: None,
                fiscal_period_id: period.id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: acct_a.id,
                        debit_amount: Money(100_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: acct_b.id,
                        debit_amount: Money(0),
                        credit_amount: Money(100_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .unwrap();

        let err = db.accounts().delete(acct_a.id).unwrap_err();
        assert!(
            err.to_string().contains("journal entr"),
            "error should mention journal entries, got: {err}"
        );
    }

    // ── get_balance ───────────────────────────────────────────────────────────

    #[test]
    fn get_balance_returns_zero_with_no_journal_entries() {
        let conn = db_seeded();
        let repo = AccountRepo::new(&conn);

        let all = repo.list_all().expect("list_all");
        let account = all.first().expect("at least one account");
        let balance = repo.get_balance(account.id).expect("get_balance");
        assert_eq!(
            balance,
            Money(0),
            "Balance should be zero with no posted JEs"
        );
    }

    #[test]
    fn get_balance_reflects_posted_journal_entries() {
        use crate::db::{
            entity_db_from_conn,
            fiscal_repo::FiscalRepo,
            journal_repo::{NewJournalEntry, NewJournalEntryLine},
        };
        use crate::services::journal::post_journal_entry;
        use chrono::NaiveDate;
        use rusqlite::Connection;

        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::db::schema::initialize_schema(&conn).expect("schema");
        crate::db::schema::seed_default_accounts(&conn).expect("seed");
        FiscalRepo::new(&conn)
            .create_fiscal_year(1, 2026)
            .expect("fiscal year");
        let db = entity_db_from_conn(conn);

        // Find two non-placeholder active accounts.
        let accounts: Vec<_> = db
            .accounts()
            .list_all()
            .unwrap()
            .into_iter()
            .filter(|a| !a.is_placeholder && a.is_active)
            .collect();
        let cash = &accounts[0];
        let revenue = &accounts[1];

        let period = db
            .fiscal()
            .get_period_for_date(NaiveDate::from_ymd_opt(2026, 1, 15).unwrap())
            .unwrap();

        // Before posting: balance is zero.
        assert_eq!(db.accounts().get_balance(cash.id).unwrap(), Money(0));

        // Create draft: debit Cash $100, credit Revenue $100.
        let je_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                memo: None,
                fiscal_period_id: period.id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: cash.id,
                        debit_amount: Money(10_000_000_000), // $100
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: revenue.id,
                        debit_amount: Money(0),
                        credit_amount: Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .unwrap();

        // Draft does NOT affect balance.
        assert_eq!(db.accounts().get_balance(cash.id).unwrap(), Money(0));

        // Post the entry.
        post_journal_entry(&db, je_id, "Test Entity").unwrap();

        // Now Cash has a net debit balance of $100.
        let cash_balance = db.accounts().get_balance(cash.id).unwrap();
        assert_eq!(
            cash_balance,
            Money(10_000_000_000),
            "Cash should show $100 debit balance"
        );

        // Revenue has a net credit balance (negative net debit).
        let rev_balance = db.accounts().get_balance(revenue.id).unwrap();
        assert_eq!(
            rev_balance,
            Money(-10_000_000_000),
            "Revenue should show $100 credit balance"
        );

        // get_all_balances should agree.
        let all_balances = db.accounts().get_all_balances().unwrap();
        assert_eq!(
            all_balances.get(&cash.id).copied().unwrap_or(Money(0)),
            Money(10_000_000_000)
        );
    }

    #[test]
    fn get_balance_for_date_range_filters_by_entry_date() {
        use crate::db::{
            entity_db_from_conn,
            fiscal_repo::FiscalRepo,
            journal_repo::{NewJournalEntry, NewJournalEntryLine},
        };
        use crate::services::journal::post_journal_entry;
        use chrono::NaiveDate;
        use rusqlite::Connection;

        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::db::schema::initialize_schema(&conn).expect("schema");
        crate::db::schema::seed_default_accounts(&conn).expect("seed");
        FiscalRepo::new(&conn)
            .create_fiscal_year(1, 2026)
            .expect("fiscal year");
        let db = entity_db_from_conn(conn);

        let accounts: Vec<_> = db
            .accounts()
            .list_all()
            .unwrap()
            .into_iter()
            .filter(|a| !a.is_placeholder && a.is_active)
            .collect();
        let cash = &accounts[0];
        let revenue = &accounts[1];

        let period = db
            .fiscal()
            .get_period_for_date(NaiveDate::from_ymd_opt(2026, 1, 15).unwrap())
            .unwrap();

        let je_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                memo: None,
                fiscal_period_id: period.id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: cash.id,
                        debit_amount: Money(10_000_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: revenue.id,
                        debit_amount: Money(0),
                        credit_amount: Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .unwrap();

        post_journal_entry(&db, je_id, "Test Entity").unwrap();

        // Date range that includes entry_date should show the balance.
        let in_range = db
            .accounts()
            .get_balance_for_date_range(
                cash.id,
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            )
            .unwrap();
        assert_eq!(in_range, Money(10_000_000_000));

        // Date range outside entry_date should return zero.
        let out_of_range = db
            .accounts()
            .get_balance_for_date_range(
                cash.id,
                NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(),
            )
            .unwrap();
        assert_eq!(out_of_range, Money(0));
    }
}
