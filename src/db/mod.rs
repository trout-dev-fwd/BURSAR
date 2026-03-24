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
        migrate_to_junction_table(&conn)?;
        // Seed default accounts on a fresh database (no rows → first open).
        let account_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))?;
        if account_count == 0 {
            seed_default_accounts(&conn)?;
        }
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

/// Migrates V2-era databases from the `import_ref` column on `journal_entries` to the
/// `journal_entry_import_refs` junction table introduced in V3.
///
/// If `journal_entries` still has `import_ref` (old schema): copies all non-NULL values
/// to the junction table and rebuilds `journal_entries` without the column.
/// If the column is already absent (new schema or pre-V2 DB), this is a no-op.
fn migrate_to_junction_table(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(journal_entries)")?;
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();

    if !columns.contains(&"import_ref".to_string()) {
        return Ok(()); // Already on new schema or fresh DB.
    }

    // V2 schema detected: migrate import_refs to junction table and drop the old column.
    conn.execute_batch("PRAGMA foreign_keys=OFF;")?;
    conn.execute_batch(
        "
        BEGIN;

        INSERT OR IGNORE INTO journal_entry_import_refs (journal_entry_id, import_ref)
        SELECT id, import_ref FROM journal_entries WHERE import_ref IS NOT NULL;

        CREATE TABLE journal_entries_new (
            id                  INTEGER PRIMARY KEY,
            je_number           TEXT    NOT NULL UNIQUE,
            entry_date          TEXT    NOT NULL,
            memo                TEXT,
            status              TEXT    NOT NULL DEFAULT 'Draft',
            is_reversed         INTEGER NOT NULL DEFAULT 0,
            reversed_by_je_id   INTEGER REFERENCES journal_entries_new(id),
            reversal_of_je_id   INTEGER REFERENCES journal_entries_new(id),
            inter_entity_uuid   TEXT,
            source_entity_name  TEXT,
            fiscal_period_id    INTEGER NOT NULL REFERENCES fiscal_periods(id),
            created_at          TEXT    NOT NULL,
            updated_at          TEXT    NOT NULL
        );

        INSERT INTO journal_entries_new
        SELECT id, je_number, entry_date, memo, status, is_reversed,
               reversed_by_je_id, reversal_of_je_id, inter_entity_uuid,
               source_entity_name, fiscal_period_id, created_at, updated_at
        FROM journal_entries;

        DROP TABLE journal_entries;
        ALTER TABLE journal_entries_new RENAME TO journal_entries;

        COMMIT;
        ",
    )?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

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
        let dir = std::env::temp_dir().join("bursar_entity_db_test");
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

    /// Creates an old-schema (V2-era) SQLite file with `import_ref` on `journal_entries`
    /// and no `journal_entry_import_refs` table. Returns the path and a JE id with a ref.
    fn setup_old_schema_db(path: &std::path::Path) -> i64 {
        let conn = Connection::open(path).expect("open");
        conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
        conn.execute_batch(
            "
            CREATE TABLE fiscal_periods (
                id INTEGER PRIMARY KEY,
                fiscal_year_id INTEGER,
                period_number INTEGER,
                start_date TEXT,
                end_date TEXT,
                is_closed INTEGER DEFAULT 0,
                closed_at TEXT,
                reopened_at TEXT,
                created_at TEXT
            );
            INSERT INTO fiscal_periods VALUES (1, 1, 1, '2026-01-01', '2026-01-31', 0, NULL, NULL, '2026-01-01');

            CREATE TABLE journal_entries (
                id INTEGER PRIMARY KEY,
                je_number TEXT NOT NULL UNIQUE,
                entry_date TEXT NOT NULL,
                memo TEXT,
                status TEXT NOT NULL DEFAULT 'Draft',
                is_reversed INTEGER NOT NULL DEFAULT 0,
                reversed_by_je_id INTEGER,
                reversal_of_je_id INTEGER,
                inter_entity_uuid TEXT,
                source_entity_name TEXT,
                fiscal_period_id INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                import_ref TEXT
            );
            INSERT INTO journal_entries VALUES (
                1, 'JE-0001', '2026-01-15', 'Test', 'Draft', 0,
                NULL, NULL, NULL, NULL, 1,
                '2026-01-15T00:00:00', '2026-01-15T00:00:00',
                'TestBank|2026-01-15|DEPOSIT|10000000000'
            );
            INSERT INTO journal_entries VALUES (
                2, 'JE-0002', '2026-01-16', 'No ref', 'Draft', 0,
                NULL, NULL, NULL, NULL, 1,
                '2026-01-16T00:00:00', '2026-01-16T00:00:00',
                NULL
            );
            ",
        )
        .unwrap();
        1 // return je_id with import_ref
    }

    #[test]
    fn migrate_to_junction_table_moves_import_refs() {
        let dir = std::env::temp_dir().join("bursar_junction_migration_test");
        let path = dir.join("migration_test.sqlite");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let je_id_with_ref = setup_old_schema_db(&path);

        // Open with new code: runs initialize_schema (creates junction table) + migration.
        let db = EntityDb::open(&path).expect("open with new code");

        // The import_ref should be in the junction table.
        let ref_count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM journal_entry_import_refs WHERE journal_entry_id = ?1",
                rusqlite::params![je_id_with_ref],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(
            ref_count, 1,
            "import_ref should be migrated to junction table"
        );

        let migrated_ref: String = db
            .conn()
            .query_row(
                "SELECT import_ref FROM journal_entry_import_refs WHERE journal_entry_id = ?1",
                rusqlite::params![je_id_with_ref],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(migrated_ref, "TestBank|2026-01-15|DEPOSIT|10000000000");

        // NULL-ref JE should have no entry in junction table.
        let null_count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM journal_entry_import_refs WHERE journal_entry_id = 2",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(null_count, 0, "NULL import_ref should not be migrated");

        // journal_entries should no longer have the import_ref column.
        let mut stmt = db
            .conn()
            .prepare("PRAGMA table_info(journal_entries)")
            .unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            !cols.contains(&"import_ref".to_string()),
            "import_ref column should be removed from journal_entries after migration"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_to_junction_table_is_noop_on_new_schema() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema");
        // Should not fail — column absent, migration is a no-op.
        migrate_to_junction_table(&conn).expect("migration on new schema should be no-op");
    }
}
