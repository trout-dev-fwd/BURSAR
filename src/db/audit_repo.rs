use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::types::{AuditAction, AuditLogId};

/// A single row from the `audit_log` table.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditEntry {
    pub id: AuditLogId,
    pub action_type: AuditAction,
    pub entity_name: String,
    pub record_type: Option<String>,
    pub record_id: Option<i64>,
    pub description: String,
    pub created_at: String,
}

/// Filter criteria for `AuditRepo::list`. All fields are optional (None = no filter).
#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    /// ISO 8601 date string — include only entries on or after this timestamp.
    pub from: Option<String>,
    /// ISO 8601 date string — include only entries on or before this timestamp.
    pub to: Option<String>,
    /// Include only entries with this action type.
    pub action_type: Option<AuditAction>,
}

/// Repository for the append-only `audit_log` table.
/// Only `append` and `list` are provided; no update or delete methods exist by design.
pub struct AuditRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> AuditRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    /// Appends an audit log entry. Returns the new entry's ID.
    pub fn append(
        &self,
        action: AuditAction,
        entity_name: &str,
        record_type: &str,
        record_id: i64,
        description: &str,
    ) -> Result<AuditLogId> {
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        self.conn
            .execute(
                "INSERT INTO audit_log
                    (action_type, entity_name, record_type, record_id, description, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    action.to_string(),
                    entity_name,
                    record_type,
                    record_id,
                    description,
                    now,
                ],
            )
            .context("Failed to append audit log entry")?;
        Ok(AuditLogId::from(self.conn.last_insert_rowid()))
    }

    /// Returns audit log entries matching `filter`, ordered by `created_at` ascending.
    /// With an empty `AuditFilter` (all None) all entries are returned.
    ///
    /// Uses empty-string sentinels for optional params so the query always takes exactly
    /// 3 positional parameters, avoiding rusqlite param-count mismatches with dynamic SQL.
    pub fn list(&self, filter: &AuditFilter) -> Result<Vec<AuditEntry>> {
        // Empty string sentinels: condition is skipped when the value is "".
        let from_str = filter.from.as_deref().unwrap_or("").to_string();
        let to_str = filter.to.as_deref().unwrap_or("").to_string();
        let action_str = filter
            .action_type
            .map(|a| a.to_string())
            .unwrap_or_default();

        let mut stmt = self.conn.prepare(
            "SELECT id, action_type, entity_name, record_type, record_id, description, created_at
             FROM audit_log
             WHERE (?1 = '' OR created_at >= ?1)
               AND (?2 = '' OR created_at <= ?2)
               AND (?3 = '' OR action_type = ?3)
             ORDER BY created_at ASC",
        )?;
        stmt.query_map(params![from_str, to_str, action_str], row_to_entry)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEntry> {
    let action_str: String = row.get(1)?;
    let action_type = action_str.parse::<AuditAction>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(AuditEntry {
        id: AuditLogId::from(row.get::<_, i64>(0)?),
        action_type,
        entity_name: row.get(2)?,
        record_type: row.get(3)?,
        record_id: row.get(4)?,
        description: row.get(5)?,
        created_at: row.get(6)?,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::initialize_schema;
    use rusqlite::Connection;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema init");
        conn
    }

    fn append_three(repo: &AuditRepo<'_>) {
        repo.append(
            AuditAction::AccountCreated,
            "Test Entity",
            "Account",
            1,
            "Created account 1000 Assets",
        )
        .expect("append 1");

        repo.append(
            AuditAction::AccountModified,
            "Test Entity",
            "Account",
            1,
            "Renamed account 1000",
        )
        .expect("append 2");

        repo.append(
            AuditAction::AccountDeactivated,
            "Test Entity",
            "Account",
            2,
            "Deactivated account 2000 Liabilities",
        )
        .expect("append 3");
    }

    #[test]
    fn append_returns_sequential_ids() {
        let conn = db();
        let repo = AuditRepo::new(&conn);

        let id1 = repo
            .append(AuditAction::AccountCreated, "Ent", "Account", 1, "first")
            .expect("append 1");
        let id2 = repo
            .append(AuditAction::AccountModified, "Ent", "Account", 1, "second")
            .expect("append 2");

        assert_ne!(id1, id2, "Each append should produce a unique ID");
        let raw1: i64 = id1.into();
        let raw2: i64 = id2.into();
        assert!(raw2 > raw1, "IDs should be increasing");
    }

    #[test]
    fn list_no_filter_returns_all_entries() {
        let conn = db();
        let repo = AuditRepo::new(&conn);
        append_three(&repo);

        let entries = repo.list(&AuditFilter::default()).expect("list all");
        assert_eq!(entries.len(), 3, "Should return all 3 entries");
    }

    #[test]
    fn list_filter_by_action_type() {
        let conn = db();
        let repo = AuditRepo::new(&conn);
        append_three(&repo);

        let entries = repo
            .list(&AuditFilter {
                action_type: Some(AuditAction::AccountCreated),
                ..Default::default()
            })
            .expect("list by action");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action_type, AuditAction::AccountCreated);
    }

    #[test]
    fn list_filter_by_action_returns_zero_when_no_match() {
        let conn = db();
        let repo = AuditRepo::new(&conn);
        append_three(&repo);

        let entries = repo
            .list(&AuditFilter {
                action_type: Some(AuditAction::JournalEntryPosted),
                ..Default::default()
            })
            .expect("list");

        assert_eq!(entries.len(), 0, "No JournalEntryPosted entries exist");
    }

    #[test]
    fn list_filter_by_date_range() {
        let conn = db();
        let repo = AuditRepo::new(&conn);

        // Insert entry at a controlled timestamp by inserting raw SQL.
        conn.execute(
            "INSERT INTO audit_log
                (action_type, entity_name, record_type, record_id, description, created_at)
             VALUES ('AccountCreated', 'E', 'Account', 1, 'early', '2025-01-01T00:00:00')",
            [],
        )
        .expect("insert early");
        conn.execute(
            "INSERT INTO audit_log
                (action_type, entity_name, record_type, record_id, description, created_at)
             VALUES ('AccountModified', 'E', 'Account', 1, 'late', '2025-06-01T00:00:00')",
            [],
        )
        .expect("insert late");

        // Filter: from 2025-03-01 should exclude the January entry.
        let entries = repo
            .list(&AuditFilter {
                from: Some("2025-03-01T00:00:00".to_string()),
                ..Default::default()
            })
            .expect("list");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].description, "late");
    }

    #[test]
    fn list_filter_by_to_date() {
        let conn = db();
        let repo = AuditRepo::new(&conn);

        conn.execute(
            "INSERT INTO audit_log
                (action_type, entity_name, record_type, record_id, description, created_at)
             VALUES ('AccountCreated', 'E', 'Account', 1, 'early', '2025-01-01T00:00:00')",
            [],
        )
        .expect("insert early");
        conn.execute(
            "INSERT INTO audit_log
                (action_type, entity_name, record_type, record_id, description, created_at)
             VALUES ('AccountModified', 'E', 'Account', 1, 'late', '2025-06-01T00:00:00')",
            [],
        )
        .expect("insert late");

        // Filter: to 2025-03-01 should exclude the June entry.
        let entries = repo
            .list(&AuditFilter {
                to: Some("2025-03-01T00:00:00".to_string()),
                ..Default::default()
            })
            .expect("list");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].description, "early");
    }

    #[test]
    fn entry_fields_round_trip_correctly() {
        let conn = db();
        let repo = AuditRepo::new(&conn);

        repo.append(
            AuditAction::YearEndClose,
            "My Entity",
            "FiscalYear",
            42,
            "Closed FY2025",
        )
        .expect("append");

        let entries = repo.list(&AuditFilter::default()).expect("list");
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.action_type, AuditAction::YearEndClose);
        assert_eq!(e.entity_name, "My Entity");
        assert_eq!(e.record_type.as_deref(), Some("FiscalYear"));
        assert_eq!(e.record_id, Some(42));
        assert_eq!(e.description, "Closed FY2025");
    }

    #[test]
    fn no_update_or_delete_methods_exist() {
        // Compile-time test: AuditRepo has no update/delete methods.
        // This test documents the intentional absence and passes trivially.
        let conn = db();
        let _repo = AuditRepo::new(&conn);
        // If someone adds update/delete, this test won't catch it — but the boundaries
        // spec and code review are the enforcement mechanism.
    }
}
