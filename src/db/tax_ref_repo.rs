use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use super::now_str;

// ── Data structs ──────────────────────────────────────────────────────────────

/// A single chunk of IRS publication text stored in the tax_reference table.
#[derive(Debug, Clone, PartialEq)]
pub struct TaxRefChunk {
    pub id: i64,
    pub publication: String,
    pub section: String,
    pub topic_tags: String,
    pub content: String,
    pub tax_year: i32,
    pub ingested_at: String,
}

// ── Repository ────────────────────────────────────────────────────────────────

pub struct TaxRefRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> TaxRefRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    /// Deletes all rows from tax_reference.
    pub fn clear(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM tax_reference", [])
            .context("Failed to clear tax_reference")?;
        Ok(())
    }

    /// Inserts one chunk. Caller is responsible for transaction management.
    pub fn insert(
        &self,
        publication: &str,
        section: &str,
        topic_tags: &str,
        content: &str,
        tax_year: i32,
    ) -> Result<()> {
        let now = now_str();
        self.conn
            .execute(
                "INSERT INTO tax_reference (publication, section, topic_tags, content, tax_year, ingested_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![publication, section, topic_tags, content, tax_year, now],
            )
            .context("Failed to insert tax reference chunk")?;
        Ok(())
    }

    /// Returns all chunks whose topic_tags contain the given tag substring.
    pub fn search_by_tag(&self, tag: &str) -> Result<Vec<TaxRefChunk>> {
        let pattern = format!("%{tag}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, publication, section, topic_tags, content, tax_year, ingested_at
             FROM tax_reference
             WHERE topic_tags LIKE ?1
             ORDER BY publication, section",
        )?;
        let chunks = stmt
            .query_map(params![pattern], row_to_chunk)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect::<Result<Vec<_>>>()
            .context("Failed to search tax reference by tag")?;
        Ok(chunks)
    }

    /// Returns the total number of chunks in the table.
    pub fn count(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM tax_reference", [], |row| row.get(0))
            .context("Failed to count tax reference chunks")
    }
}

// ── Row mapping helper ────────────────────────────────────────────────────────

fn row_to_chunk(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaxRefChunk> {
    Ok(TaxRefChunk {
        id: row.get(0)?,
        publication: row.get(1)?,
        section: row.get(2)?,
        topic_tags: row.get(3)?,
        content: row.get(4)?,
        tax_year: row.get(5)?,
        ingested_at: row.get(6)?,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::initialize_schema;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema");
        conn
    }

    #[test]
    fn insert_and_count() {
        let conn = setup_db();
        let repo = TaxRefRepo::new(&conn);

        assert_eq!(repo.count().expect("count"), 0);

        repo.insert(
            "Pub 334",
            "Business Expenses",
            "schedule_c,business_expense",
            "Content here.",
            2026,
        )
        .expect("insert 1");
        repo.insert(
            "Pub 527",
            "Rental Income",
            "schedule_e,rental",
            "Rental content.",
            2026,
        )
        .expect("insert 2");

        assert_eq!(repo.count().expect("count"), 2);
    }

    #[test]
    fn clear_removes_all_rows() {
        let conn = setup_db();
        let repo = TaxRefRepo::new(&conn);

        repo.insert("Pub 17", "General", "general", "Some text.", 2026)
            .expect("insert");
        repo.insert("Pub 17", "Chapter 2", "general", "More text.", 2026)
            .expect("insert 2");

        repo.clear().expect("clear");
        assert_eq!(repo.count().expect("count"), 0);
    }

    #[test]
    fn search_by_tag_returns_matching() {
        let conn = setup_db();
        let repo = TaxRefRepo::new(&conn);

        repo.insert(
            "Pub 334",
            "Expenses",
            "schedule_c,business_expense",
            "Business stuff.",
            2026,
        )
        .expect("insert 1");
        repo.insert(
            "Pub 527",
            "Income",
            "schedule_e,rental",
            "Rental stuff.",
            2026,
        )
        .expect("insert 2");
        repo.insert(
            "Pub 946",
            "MACRS",
            "depreciation,form_4562",
            "Depreciation.",
            2026,
        )
        .expect("insert 3");

        let results = repo.search_by_tag("schedule_c").expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].publication, "Pub 334");

        let results = repo.search_by_tag("depreciation").expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].publication, "Pub 946");
    }

    #[test]
    fn search_by_tag_returns_empty_for_no_match() {
        let conn = setup_db();
        let repo = TaxRefRepo::new(&conn);

        repo.insert("Pub 17", "Chapter 1", "general,income", "Content.", 2026)
            .expect("insert");

        let results = repo.search_by_tag("nonexistent_tag").expect("search");
        assert!(results.is_empty());
    }

    #[test]
    fn search_by_tag_partial_match() {
        let conn = setup_db();
        let repo = TaxRefRepo::new(&conn);

        // "schedule_a" should match "schedule_a_medical"
        repo.insert(
            "Pub 502",
            "Medical",
            "medical,schedule_a",
            "Medical expenses.",
            2026,
        )
        .expect("insert");

        let results = repo.search_by_tag("schedule_a").expect("search");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn retrieved_chunk_has_correct_fields() {
        let conn = setup_db();
        let repo = TaxRefRepo::new(&conn);

        repo.insert(
            "Pub 946",
            "MACRS Section",
            "depreciation,macrs",
            "MACRS content here.",
            2026,
        )
        .expect("insert");

        let results = repo.search_by_tag("macrs").expect("search");
        assert_eq!(results.len(), 1);
        let chunk = &results[0];
        assert_eq!(chunk.publication, "Pub 946");
        assert_eq!(chunk.section, "MACRS Section");
        assert_eq!(chunk.topic_tags, "depreciation,macrs");
        assert_eq!(chunk.content, "MACRS content here.");
        assert_eq!(chunk.tax_year, 2026);
        assert!(!chunk.ingested_at.is_empty());
    }
}
