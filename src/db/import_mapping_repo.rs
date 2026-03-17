use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use rusqlite::{Connection, params};

use super::now_str;
use crate::types::{AccountId, ImportMatchSource, ImportMatchType};

/// A row from the `import_mappings` table.
#[derive(Debug, Clone)]
pub struct ImportMapping {
    pub id: i64,
    pub description_pattern: String,
    pub account_id: AccountId,
    pub match_type: ImportMatchType,
    pub source: ImportMatchSource,
    pub bank_name: String,
    pub created_at: NaiveDateTime,
    pub last_used_at: NaiveDateTime,
    pub use_count: i64,
}

/// Repository for the `import_mappings` table.
pub struct ImportMappingRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> ImportMappingRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    /// Finds an exact description match for the given bank.
    /// Returns the mapping id and account id if found.
    pub fn find_exact_match(
        &self,
        bank_name: &str,
        description: &str,
    ) -> Result<Option<(i64, AccountId)>> {
        let result = self.conn.query_row(
            "SELECT id, account_id FROM import_mappings
             WHERE bank_name = ?1
               AND match_type = 'exact'
               AND description_pattern = ?2",
            params![bank_name, description],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        );
        match result {
            Ok((id, account_id)) => Ok(Some((id, AccountId::from(account_id)))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow::Error::from(e).context("find_exact_match failed")),
        }
    }

    /// Finds a substring match for the given bank, returning the longest (most specific) pattern.
    /// Returns the mapping id and account id if found.
    pub fn find_substring_match(
        &self,
        bank_name: &str,
        description: &str,
    ) -> Result<Option<(i64, AccountId)>> {
        let result = self.conn.query_row(
            "SELECT id, account_id FROM import_mappings
             WHERE bank_name = ?1
               AND match_type = 'substring'
               AND ?2 LIKE '%' || description_pattern || '%'
             ORDER BY LENGTH(description_pattern) DESC
             LIMIT 1",
            params![bank_name, description],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        );
        match result {
            Ok((id, account_id)) => Ok(Some((id, AccountId::from(account_id)))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow::Error::from(e).context("find_substring_match failed")),
        }
    }

    /// Creates a new mapping. Returns the new row id.
    /// Returns an error if `(description_pattern, bank_name)` already exists.
    pub fn create(
        &self,
        description_pattern: &str,
        account_id: AccountId,
        match_type: ImportMatchType,
        source: ImportMatchSource,
        bank_name: &str,
    ) -> Result<i64> {
        let now = now_str();
        self.conn
            .execute(
                "INSERT INTO import_mappings
                     (description_pattern, account_id, match_type, source, bank_name,
                      created_at, last_used_at, use_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
                params![
                    description_pattern,
                    i64::from(account_id),
                    match_type.to_string(),
                    source.to_string(),
                    bank_name,
                    now,
                    now,
                ],
            )
            .with_context(|| {
                format!(
                    "Failed to create import mapping for pattern '{}' in bank '{}'",
                    description_pattern, bank_name
                )
            })?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Updates the target account for an existing mapping.
    /// Also updates the `source` to reflect how the change was made.
    pub fn update_account(
        &self,
        id: i64,
        account_id: AccountId,
        source: ImportMatchSource,
    ) -> Result<()> {
        let updated = self.conn.execute(
            "UPDATE import_mappings SET account_id = ?1, source = ?2 WHERE id = ?3",
            params![i64::from(account_id), source.to_string(), id],
        )?;
        if updated == 0 {
            anyhow::bail!("No import mapping found with id {id}");
        }
        Ok(())
    }

    /// Records a successful use of a mapping: increments `use_count` and updates `last_used_at`.
    pub fn record_use(&self, id: i64) -> Result<()> {
        let now = now_str();
        let updated = self.conn.execute(
            "UPDATE import_mappings SET last_used_at = ?1, use_count = use_count + 1 WHERE id = ?2",
            params![now, id],
        )?;
        if updated == 0 {
            anyhow::bail!("No import mapping found with id {id}");
        }
        Ok(())
    }

    /// Returns all mappings for the given bank, ordered by use_count descending.
    pub fn list_by_bank(&self, bank_name: &str) -> Result<Vec<ImportMapping>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description_pattern, account_id, match_type, source,
                    bank_name, created_at, last_used_at, use_count
             FROM import_mappings
             WHERE bank_name = ?1
             ORDER BY use_count DESC, description_pattern",
        )?;
        stmt.query_map(params![bank_name], row_to_mapping)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect()
    }
}

fn row_to_mapping(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImportMapping> {
    let match_type_str: String = row.get(3)?;
    let source_str: String = row.get(4)?;
    let created_at_str: String = row.get(6)?;
    let last_used_at_str: String = row.get(7)?;

    let match_type = match_type_str.parse::<ImportMatchType>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let source = source_str.parse::<ImportMatchSource>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    })?;

    let created_at =
        NaiveDateTime::parse_from_str(&created_at_str, "%Y-%m-%dT%H:%M:%S").map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
        })?;
    let last_used_at = NaiveDateTime::parse_from_str(&last_used_at_str, "%Y-%m-%dT%H:%M:%S")
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e))
        })?;

    Ok(ImportMapping {
        id: row.get(0)?,
        description_pattern: row.get(1)?,
        account_id: AccountId::from(row.get::<_, i64>(2)?),
        match_type,
        source,
        bank_name: row.get(5)?,
        created_at,
        last_used_at,
        use_count: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::{initialize_schema, seed_default_accounts};

    fn setup() -> (rusqlite::Connection, AccountId) {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        initialize_schema(&conn).expect("schema");
        seed_default_accounts(&conn).expect("seed");
        let account_id: i64 = conn
            .query_row("SELECT id FROM accounts WHERE number = '5100'", [], |r| {
                r.get(0)
            })
            .expect("account 5100");
        (conn, AccountId::from(account_id))
    }

    fn setup2(conn: &rusqlite::Connection) -> AccountId {
        let id: i64 = conn
            .query_row("SELECT id FROM accounts WHERE number = '5200'", [], |r| {
                r.get(0)
            })
            .expect("account 5200");
        AccountId::from(id)
    }

    #[test]
    fn create_and_find_exact_match() {
        let (conn, account_id) = setup();
        let repo = ImportMappingRepo::new(&conn);

        let id = repo
            .create(
                "WELLS FARGO MORTGAGE",
                account_id,
                ImportMatchType::Exact,
                ImportMatchSource::Confirmed,
                "SoFi",
            )
            .expect("create");
        assert!(id > 0);

        let result = repo
            .find_exact_match("SoFi", "WELLS FARGO MORTGAGE")
            .expect("find");
        let (found_id, found_account) = result.expect("should have match");
        assert_eq!(found_id, id);
        assert_eq!(found_account, account_id);
    }

    #[test]
    fn exact_match_returns_none_for_unknown() {
        let (conn, _) = setup();
        let repo = ImportMappingRepo::new(&conn);

        let result = repo.find_exact_match("SoFi", "NONEXISTENT").expect("find");
        assert!(result.is_none());
    }

    #[test]
    fn create_and_find_substring_match() {
        let (conn, account_id) = setup();
        let repo = ImportMappingRepo::new(&conn);

        repo.create(
            "INSURANCE",
            account_id,
            ImportMatchType::Substring,
            ImportMatchSource::AiSuggested,
            "SoFi",
        )
        .expect("create");

        let result = repo
            .find_substring_match("SoFi", "ACME INSURANCE PAYMENT")
            .expect("find");
        let (_, found_account) = result.expect("should match via substring");
        assert_eq!(found_account, account_id);
    }

    #[test]
    fn substring_match_returns_longest_pattern() {
        let (conn, account_id) = setup();
        let account_id2 = setup2(&conn);
        let repo = ImportMappingRepo::new(&conn);

        // Short pattern maps to account1
        repo.create(
            "INSURANCE",
            account_id,
            ImportMatchType::Substring,
            ImportMatchSource::Confirmed,
            "SoFi",
        )
        .expect("create short");
        // Longer pattern maps to account2
        repo.create(
            "ACME INSURANCE",
            account_id2,
            ImportMatchType::Substring,
            ImportMatchSource::Confirmed,
            "SoFi",
        )
        .expect("create long");

        let result = repo
            .find_substring_match("SoFi", "ACME INSURANCE PAYMENT")
            .expect("find");
        let (_, found_account) = result.expect("should have match");
        // Should return the longer (more specific) pattern's account
        assert_eq!(found_account, account_id2);
    }

    #[test]
    fn exact_match_takes_priority_over_substring() {
        let (conn, account_id) = setup();
        let account_id2 = setup2(&conn);
        let repo = ImportMappingRepo::new(&conn);

        // Substring mapping
        repo.create(
            "INSURANCE",
            account_id,
            ImportMatchType::Substring,
            ImportMatchSource::Confirmed,
            "SoFi",
        )
        .expect("create substring");
        // Exact mapping for the full description
        repo.create(
            "ACME INSURANCE PAYMENT",
            account_id2,
            ImportMatchType::Exact,
            ImportMatchSource::Confirmed,
            "SoFi",
        )
        .expect("create exact");

        // Caller should check exact first, then substring
        let exact = repo
            .find_exact_match("SoFi", "ACME INSURANCE PAYMENT")
            .expect("exact");
        assert!(exact.is_some(), "exact match should be found");
        let (_, exact_account) = exact.unwrap();
        assert_eq!(exact_account, account_id2, "exact match should win");
    }

    #[test]
    fn record_use_increments_count() {
        let (conn, account_id) = setup();
        let repo = ImportMappingRepo::new(&conn);

        let id = repo
            .create(
                "MORTGAGE",
                account_id,
                ImportMatchType::Exact,
                ImportMatchSource::Confirmed,
                "SoFi",
            )
            .expect("create");

        // Initial use_count is 1
        let initial_count: i64 = conn
            .query_row(
                "SELECT use_count FROM import_mappings WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .expect("query");
        assert_eq!(initial_count, 1);

        repo.record_use(id).expect("record_use");

        let new_count: i64 = conn
            .query_row(
                "SELECT use_count FROM import_mappings WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .expect("query");
        assert_eq!(new_count, 2);
    }

    #[test]
    fn record_use_unknown_id_returns_error() {
        let (conn, _) = setup();
        let repo = ImportMappingRepo::new(&conn);
        assert!(repo.record_use(99999).is_err());
    }

    #[test]
    fn duplicate_pattern_bank_returns_error() {
        let (conn, account_id) = setup();
        let repo = ImportMappingRepo::new(&conn);

        repo.create(
            "RENT",
            account_id,
            ImportMatchType::Exact,
            ImportMatchSource::Confirmed,
            "SoFi",
        )
        .expect("first create");

        let result = repo.create(
            "RENT",
            account_id,
            ImportMatchType::Exact,
            ImportMatchSource::Confirmed,
            "SoFi",
        );
        assert!(result.is_err(), "duplicate (pattern, bank) should fail");
    }

    #[test]
    fn update_account_changes_target() {
        let (conn, account_id) = setup();
        let account_id2 = setup2(&conn);
        let repo = ImportMappingRepo::new(&conn);

        let id = repo
            .create(
                "PAYROLL",
                account_id,
                ImportMatchType::Exact,
                ImportMatchSource::AiSuggested,
                "SoFi",
            )
            .expect("create");

        repo.update_account(id, account_id2, ImportMatchSource::Confirmed)
            .expect("update");

        let stored_account: i64 = conn
            .query_row(
                "SELECT account_id FROM import_mappings WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .expect("query");
        assert_eq!(AccountId::from(stored_account), account_id2);
    }

    #[test]
    fn list_by_bank_returns_all_for_bank() {
        let (conn, account_id) = setup();
        let repo = ImportMappingRepo::new(&conn);

        repo.create(
            "RENT",
            account_id,
            ImportMatchType::Exact,
            ImportMatchSource::Confirmed,
            "SoFi",
        )
        .unwrap();
        repo.create(
            "INSURANCE",
            account_id,
            ImportMatchType::Substring,
            ImportMatchSource::Confirmed,
            "SoFi",
        )
        .unwrap();
        repo.create(
            "OTHER",
            account_id,
            ImportMatchType::Exact,
            ImportMatchSource::Confirmed,
            "Chase",
        )
        .unwrap();

        let sofi_mappings = repo.list_by_bank("SoFi").expect("list");
        assert_eq!(sofi_mappings.len(), 2);

        let chase_mappings = repo.list_by_bank("Chase").expect("list");
        assert_eq!(chase_mappings.len(), 1);

        let unknown_mappings = repo.list_by_bank("Unknown").expect("list");
        assert!(unknown_mappings.is_empty());
    }

    #[test]
    fn foreign_key_violation_on_invalid_account_id() {
        let (conn, _) = setup();
        let repo = ImportMappingRepo::new(&conn);

        let invalid_account = AccountId::from(99999_i64);
        let result = repo.create(
            "TEST",
            invalid_account,
            ImportMatchType::Exact,
            ImportMatchSource::Confirmed,
            "SoFi",
        );
        assert!(result.is_err(), "invalid account_id should fail FK check");
    }
}
