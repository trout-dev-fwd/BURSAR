pub mod account_repo;
pub mod audit_repo;
pub mod fiscal_repo;
pub mod schema;

use std::path::Path;

use anyhow::{Context, Result};
use chrono::Datelike;
use rusqlite::Connection;

use crate::db::account_repo::AccountRepo;
use crate::db::audit_repo::AuditRepo;
use crate::db::fiscal_repo::FiscalRepo;
use crate::db::schema::{initialize_schema, seed_default_accounts};

/// Holds the SQLite connection for one entity database.
/// All repository accessors borrow `&self.conn`.
pub struct EntityDb {
    conn: Connection,
}

impl EntityDb {
    /// Opens an existing entity database file. Enables WAL mode and foreign keys.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open database: {}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
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

    // ── Stub repo accessors (filled in later phases) ──────────────────────────
    // TODO(Phase 2b): fn journals(&self) -> JournalRepo<'_>
    // TODO(Phase 3):  fn ar(&self) -> ArRepo<'_>
    // TODO(Phase 3):  fn ap(&self) -> ApRepo<'_>
    // TODO(Phase 4):  fn envelopes(&self) -> EnvelopeRepo<'_>
    // TODO(Phase 4):  fn assets(&self) -> AssetRepo<'_>
    // TODO(Phase 5):  fn recurring(&self) -> RecurringRepo<'_>
}

/// Returns the current local timestamp as an ISO 8601 string (no timezone).
/// Shared by all repos that store `created_at` / `updated_at` columns.
pub(crate) fn now_str() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
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
