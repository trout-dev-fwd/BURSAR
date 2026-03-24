use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::types::JournalEntryId;

/// Repository for the `journal_entry_import_refs` junction table.
/// Each row links a journal entry to a bank-statement import reference string,
/// supporting multiple import_refs per JE (e.g. both sides of a transfer).
pub struct ImportRefRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> ImportRefRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    /// Inserts a new import_ref for a journal entry.
    /// Returns an error if the import_ref already exists (UNIQUE constraint).
    pub fn insert(&self, je_id: JournalEntryId, import_ref: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO journal_entry_import_refs (journal_entry_id, import_ref)
                 VALUES (?1, ?2)",
                params![i64::from(je_id), import_ref],
            )
            .context("Failed to insert import_ref into junction table")?;
        Ok(())
    }

    /// Returns `true` if the given import_ref exists in the junction table.
    pub fn exists(&self, import_ref: &str) -> Result<bool> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM journal_entry_import_refs WHERE import_ref = ?1",
                params![import_ref],
                |row| row.get(0),
            )
            .context("Failed to check import_ref existence")?;
        Ok(count > 0)
    }

    /// Returns all import_ref strings stored for a given journal entry.
    pub fn get_for_je(&self, je_id: JournalEntryId) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT import_ref FROM journal_entry_import_refs
             WHERE journal_entry_id = ?1
             ORDER BY id",
        )?;
        let refs = stmt
            .query_map(params![i64::from(je_id)], |row| row.get::<_, String>(0))?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect::<Result<Vec<_>>>()?;
        Ok(refs)
    }

    /// Returns the journal entry ID associated with the given import_ref,
    /// or `None` if not found.
    pub fn get_je_id(&self, import_ref: &str) -> Result<Option<JournalEntryId>> {
        let result: rusqlite::Result<i64> = self.conn.query_row(
            "SELECT journal_entry_id FROM journal_entry_import_refs WHERE import_ref = ?1",
            params![import_ref],
            |row| row.get(0),
        );
        match result {
            Ok(id) => Ok(Some(JournalEntryId::from(id))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow::Error::from(e).context("Failed to query journal_entry_id")),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::fiscal_repo::FiscalRepo;
    use crate::db::journal_repo::{JournalRepo, NewJournalEntry, NewJournalEntryLine};
    use crate::db::schema::{initialize_schema, seed_default_accounts};
    use crate::types::{FiscalPeriodId, Money};
    use rusqlite::Connection;

    fn db_with_fiscal_year() -> (Connection, FiscalPeriodId) {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        initialize_schema(&conn).expect("schema");
        seed_default_accounts(&conn).expect("seed");
        let fiscal = FiscalRepo::new(&conn);
        fiscal.create_fiscal_year(1, 2026).expect("fiscal year");
        let period_id: i64 = conn
            .query_row(
                "SELECT id FROM fiscal_periods WHERE period_number = 1",
                [],
                |row| row.get(0),
            )
            .expect("period");
        (conn, FiscalPeriodId::from(period_id))
    }

    fn make_je(conn: &Connection, period_id: FiscalPeriodId) -> JournalEntryId {
        let acct1: i64 = conn
            .query_row("SELECT id FROM accounts WHERE number = '1110'", [], |row| {
                row.get(0)
            })
            .expect("acct1");
        let acct2: i64 = conn
            .query_row("SELECT id FROM accounts WHERE number = '4100'", [], |row| {
                row.get(0)
            })
            .expect("acct2");
        use crate::types::AccountId;
        let entry = NewJournalEntry {
            entry_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            memo: None,
            fiscal_period_id: period_id,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: AccountId::from(acct1),
                    debit_amount: Money(10_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: AccountId::from(acct2),
                    debit_amount: Money(0),
                    credit_amount: Money(10_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };
        JournalRepo::new(conn)
            .create_draft(&entry)
            .expect("create_draft")
    }

    #[test]
    fn insert_and_exists_round_trip() {
        let (conn, period_id) = db_with_fiscal_year();
        let je_id = make_je(&conn, period_id);
        let repo = ImportRefRepo::new(&conn);

        assert!(!repo.exists("ref-1").expect("exists"));
        repo.insert(je_id, "ref-1").expect("insert");
        assert!(repo.exists("ref-1").expect("exists after insert"));
    }

    #[test]
    fn exists_returns_false_for_unknown_ref() {
        let (conn, _) = db_with_fiscal_year();
        let repo = ImportRefRepo::new(&conn);
        assert!(!repo.exists("does-not-exist").expect("exists"));
    }

    #[test]
    fn duplicate_import_ref_returns_error() {
        let (conn, period_id) = db_with_fiscal_year();
        let je_id = make_je(&conn, period_id);
        let repo = ImportRefRepo::new(&conn);

        repo.insert(je_id, "ref-dup").expect("first insert");
        let result = repo.insert(je_id, "ref-dup");
        assert!(result.is_err(), "duplicate import_ref should fail");
    }

    #[test]
    fn get_for_je_returns_all_refs() {
        let (conn, period_id) = db_with_fiscal_year();
        let je_id = make_je(&conn, period_id);
        let repo = ImportRefRepo::new(&conn);

        repo.insert(je_id, "ref-a").expect("insert a");
        repo.insert(je_id, "ref-b").expect("insert b");

        let refs = repo.get_for_je(je_id).expect("get_for_je");
        assert_eq!(refs.len(), 2);
        assert!(refs.contains(&"ref-a".to_string()));
        assert!(refs.contains(&"ref-b".to_string()));
    }

    #[test]
    fn get_for_je_empty_when_no_refs() {
        let (conn, period_id) = db_with_fiscal_year();
        let je_id = make_je(&conn, period_id);
        let repo = ImportRefRepo::new(&conn);

        let refs = repo.get_for_je(je_id).expect("get_for_je");
        assert!(refs.is_empty());
    }

    #[test]
    fn get_je_id_returns_correct_je() {
        let (conn, period_id) = db_with_fiscal_year();
        let je_id = make_je(&conn, period_id);
        let repo = ImportRefRepo::new(&conn);

        repo.insert(je_id, "chase|2026-01-15|DEPOSIT|100")
            .expect("insert");

        let found = repo
            .get_je_id("chase|2026-01-15|DEPOSIT|100")
            .expect("get_je_id");
        assert_eq!(found, Some(je_id));
    }

    #[test]
    fn get_je_id_returns_none_for_unknown_ref() {
        let (conn, _) = db_with_fiscal_year();
        let repo = ImportRefRepo::new(&conn);
        let result = repo.get_je_id("not-there").expect("get_je_id");
        assert!(result.is_none());
    }
}
