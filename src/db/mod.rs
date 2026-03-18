pub mod account_repo;
pub mod ap_repo;
pub mod ar_repo;
pub mod asset_repo;
pub mod audit_repo;
pub mod envelope_repo;
pub mod fiscal_repo;
pub mod import_mapping_repo;
pub mod journal_repo;
pub mod recurring_repo;
pub mod schema;

use std::path::Path;

use anyhow::{Context, Result};
use chrono::Datelike;
use rusqlite::Connection;

use crate::db::account_repo::AccountRepo;
use crate::db::ap_repo::ApRepo;
use crate::db::ar_repo::ArRepo;
use crate::db::asset_repo::AssetRepo;
use crate::db::audit_repo::AuditRepo;
use crate::db::envelope_repo::EnvelopeRepo;
use crate::db::fiscal_repo::FiscalRepo;
use crate::db::import_mapping_repo::ImportMappingRepo;
use crate::db::journal_repo::JournalRepo;
use crate::db::recurring_repo::RecurringRepo;
use crate::db::schema::{initialize_schema, seed_default_accounts};

/// Holds the SQLite connection for one entity database.
/// All repository accessors borrow `&self.conn`.
pub struct EntityDb {
    conn: Connection,
}

impl EntityDb {
    /// Opens an existing entity database file. Enables WAL mode and foreign keys.
    /// Runs any pending migrations for schema drift.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open database: {}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        // Initialize schema first (IF NOT EXISTS — safe on both new and existing DBs).
        // Migrations run after so they only ADD missing columns to pre-existing tables.
        initialize_schema(&conn)?;
        migrate_fixed_asset_details(&conn)?;
        migrate_journal_import_ref(&conn)?;
        Ok(Self { conn })
    }

    /// Creates a new entity database: creates the file, runs schema init, seeds default accounts.
    /// `fiscal_year_start_month` and the initial fiscal year creation happen in Task 12.
    pub fn create(path: &Path, _entity_name: &str, fiscal_year_start_month: u32) -> Result<Self> {
        // Create parent directories if needed.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create database directory: {}", parent.display())
            })?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to create database: {}", path.display()))?;
        initialize_schema(&conn)?;
        seed_default_accounts(&conn)?;
        let db = Self { conn };
        let current_year = chrono::Local::now().year();
        db.fiscal()
            .create_fiscal_year(fiscal_year_start_month, current_year)?;
        Ok(db)
    }

    /// Direct connection access for transactions that span multiple repos.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Returns a FiscalRepo borrowing this connection.
    pub fn fiscal(&self) -> FiscalRepo<'_> {
        FiscalRepo::new(&self.conn)
    }

    /// Returns an AccountRepo borrowing this connection.
    pub fn accounts(&self) -> AccountRepo<'_> {
        AccountRepo::new(&self.conn)
    }

    /// Returns an AuditRepo borrowing this connection.
    pub fn audit(&self) -> AuditRepo<'_> {
        AuditRepo::new(&self.conn)
    }

    /// Returns a JournalRepo borrowing this connection.
    pub fn journals(&self) -> JournalRepo<'_> {
        JournalRepo::new(&self.conn)
    }

    /// Returns an ArRepo borrowing this connection.
    pub fn ar(&self) -> ArRepo<'_> {
        ArRepo::new(&self.conn)
    }

    /// Returns an ApRepo borrowing this connection.
    pub fn ap(&self) -> ApRepo<'_> {
        ApRepo::new(&self.conn)
    }

    /// Returns an EnvelopeRepo borrowing this connection.
    pub fn envelopes(&self) -> EnvelopeRepo<'_> {
        EnvelopeRepo::new(&self.conn)
    }

    /// Returns an AssetRepo borrowing this connection.
    pub fn assets(&self) -> AssetRepo<'_> {
        AssetRepo::new(&self.conn)
    }

    /// Returns a RecurringRepo borrowing this connection.
    pub fn recurring(&self) -> RecurringRepo<'_> {
        RecurringRepo::new(&self.conn)
    }

    /// Returns an ImportMappingRepo borrowing this connection.
    pub fn import_mappings(&self) -> ImportMappingRepo<'_> {
        ImportMappingRepo::new(&self.conn)
    }

    /// Create an in-memory database with schema and default accounts.
    /// Only available in tests so production code cannot accidentally use it.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        use crate::db::schema::{initialize_schema, seed_default_accounts};
        let conn =
            Connection::open_in_memory().with_context(|| "Failed to open in-memory database")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        initialize_schema(&conn)?;
        seed_default_accounts(&conn)?;
        Ok(Self { conn })
    }
}

/// Ensures `fixed_asset_details` has the `accum_depreciation_account_id` and
/// `depreciation_expense_account_id` columns added in Phase 4. Databases created
/// before that phase have the table but lack these columns.
fn migrate_fixed_asset_details(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(fixed_asset_details)")?;
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();

    if !columns.contains(&"accum_depreciation_account_id".to_string()) {
        conn.execute_batch(
            "ALTER TABLE fixed_asset_details
             ADD COLUMN accum_depreciation_account_id INTEGER REFERENCES accounts(id)",
        )?;
    }
    if !columns.contains(&"depreciation_expense_account_id".to_string()) {
        conn.execute_batch(
            "ALTER TABLE fixed_asset_details
             ADD COLUMN depreciation_expense_account_id INTEGER REFERENCES accounts(id)",
        )?;
    }
    Ok(())
}

/// Adds the `import_ref` column to `journal_entries` for databases created before V2.
fn migrate_journal_import_ref(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(journal_entries)")?;
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();

    if !columns.contains(&"import_ref".to_string()) {
        conn.execute_batch("ALTER TABLE journal_entries ADD COLUMN import_ref TEXT")?;
    }
    Ok(())
}

/// Returns the current local timestamp as an ISO 8601 string (no timezone).
/// Shared by all repos that store `created_at` / `updated_at` columns.
pub(crate) fn now_str() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// Test-only constructor that wraps an in-memory `Connection` inside an `EntityDb`.
/// This avoids the need for a temp file in unit tests that require cross-repo operations.
#[cfg(test)]
pub fn entity_db_from_conn(conn: Connection) -> EntityDb {
    EntityDb { conn }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_reopen_has_schema_and_accounts() {
        let dir = std::env::temp_dir().join("accounting_entity_db_test");
        let path = dir.join("test_entity.sqlite");

        // Clean up from any previous test run.
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);

        // Create new entity database.
        EntityDb::create(&path, "Test Entity", 1).expect("create failed");

        // Reopen with open().
        let db = EntityDb::open(&path).expect("open failed");

        // Schema exists: accounts table should have seeded data.
        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
            .expect("query failed");
        assert!(count > 0, "Seeded accounts should be present after reopen");

        // Five placeholder top-level accounts.
        let placeholders: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE parent_id IS NULL AND is_placeholder = 1",
                [],
                |row| row.get(0),
            )
            .expect("query failed");
        assert_eq!(placeholders, 5);

        // Fiscal year and 12 periods should exist.
        let period_count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM fiscal_periods", [], |row| row.get(0))
            .expect("query failed");
        assert_eq!(
            period_count, 12,
            "Should have 12 fiscal periods after create"
        );

        // Cleanup.
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn conn_returns_working_connection() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema init");
        let db = EntityDb { conn };
        let result: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
            .expect("query");
        assert_eq!(result, 0); // no seeded data in this test
    }
}
