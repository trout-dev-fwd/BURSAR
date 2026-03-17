use anyhow::{Context, Result};
use chrono::NaiveDate;
use rusqlite::{Connection, params};

use super::now_str;
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
    /// `record_type` and `record_id` are optional (NULL in the schema) for events
    /// that are entity-level rather than tied to a specific record.
    pub fn append(
        &self,
        action: AuditAction,
        entity_name: &str,
        record_type: Option<&str>,
        record_id: Option<i64>,
        description: &str,
    ) -> Result<AuditLogId> {
        let now = now_str();
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

    // ── AI convenience methods ─────────────────────────────────────────────────

    /// Logs an AI prompt. `description` is truncated to 500 characters.
    pub fn log_ai_prompt(&self, entity_name: &str, description: &str) -> Result<()> {
        let truncated = truncate_chars(description, 500);
        self.append(AuditAction::AiPrompt, entity_name, None, None, truncated)?;
        Ok(())
    }

    /// Logs an AI response summary (single-line summary extracted from the response).
    pub fn log_ai_response(&self, entity_name: &str, summary: &str) -> Result<()> {
        self.append(AuditAction::AiResponse, entity_name, None, None, summary)?;
        Ok(())
    }

    /// Logs an AI tool use. Description format: "Used {tool_name}({key_params})".
    pub fn log_ai_tool_use(
        &self,
        entity_name: &str,
        tool_name: &str,
        key_params: &str,
    ) -> Result<()> {
        let description = format!("Used {tool_name}({key_params})");
        self.append(
            AuditAction::AiToolUse,
            entity_name,
            None,
            None,
            &description,
        )?;
        Ok(())
    }

    /// Logs a CSV import summary.
    pub fn log_csv_import(
        &self,
        entity_name: &str,
        bank_name: &str,
        total: usize,
        matched: usize,
        ai_matched: usize,
        manual: usize,
    ) -> Result<()> {
        let description = format!(
            "Imported {total} rows from {bank_name}: {matched} matched ({ai_matched} AI, {manual} manual)"
        );
        self.append(
            AuditAction::CsvImport,
            entity_name,
            None,
            None,
            &description,
        )?;
        Ok(())
    }

    /// Logs a learned import mapping.
    pub fn log_mapping_learned(
        &self,
        entity_name: &str,
        description: &str,
        account_number: &str,
        account_name: &str,
        source: &str,
    ) -> Result<()> {
        let entry = format!(
            "Learned mapping: '{description}' → {account_number} {account_name} ({source})"
        );
        self.append(AuditAction::MappingLearned, entity_name, None, None, &entry)?;
        Ok(())
    }

    /// Returns audit entries for AI actions (AiPrompt, AiResponse, AiToolUse),
    /// optionally filtered by date range and capped by `limit`.
    pub fn get_ai_entries(
        &self,
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
        limit: Option<usize>,
    ) -> Result<Vec<AuditEntry>> {
        let from_str = start_date
            .map(|d| format!("{}T00:00:00", d))
            .unwrap_or_default();
        let to_str = end_date
            .map(|d| format!("{}T23:59:59", d))
            .unwrap_or_default();

        let limit_clause = match limit {
            Some(n) => format!("LIMIT {n}"),
            None => String::new(),
        };

        let sql = format!(
            "SELECT id, action_type, entity_name, record_type, record_id, description, created_at
             FROM audit_log
             WHERE action_type IN ('AiPrompt', 'AiResponse', 'AiToolUse')
               AND (?1 = '' OR created_at >= ?1)
               AND (?2 = '' OR created_at <= ?2)
             ORDER BY created_at ASC
             {limit_clause}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        stmt.query_map(params![from_str, to_str], row_to_entry)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect()
    }

    /// Returns audit log entries matching `filter`, ordered by `created_at` ascending.
    /// With an empty `AuditFilter` (all None) all entries are returned.
    ///
    /// Uses empty-string sentinels for optional params so the query always takes exactly
    /// 3 positional parameters, avoiding rusqlite param-count mismatches with dynamic SQL.
    ///
    /// NOTE: This sentinel approach is acceptable for the audit log (small table, rare queries)
    /// but should NOT be used as a template for high-volume repos (e.g., JournalRepo). Those
    /// should use dynamic SQL building instead, which allows proper index usage.
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

/// Truncates a string to at most `max_chars` Unicode scalar values.
fn truncate_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &s[..byte_idx],
        None => s,
    }
}

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
            Some("Account"),
            Some(1),
            "Created account 1000 Assets",
        )
        .expect("append 1");

        repo.append(
            AuditAction::AccountModified,
            "Test Entity",
            Some("Account"),
            Some(1),
            "Renamed account 1000",
        )
        .expect("append 2");

        repo.append(
            AuditAction::AccountDeactivated,
            "Test Entity",
            Some("Account"),
            Some(2),
            "Deactivated account 2000 Liabilities",
        )
        .expect("append 3");
    }

    #[test]
    fn append_returns_sequential_ids() {
        let conn = db();
        let repo = AuditRepo::new(&conn);

        let id1 = repo
            .append(
                AuditAction::AccountCreated,
                "Ent",
                Some("Account"),
                Some(1),
                "first",
            )
            .expect("append 1");
        let id2 = repo
            .append(
                AuditAction::AccountModified,
                "Ent",
                Some("Account"),
                Some(1),
                "second",
            )
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
            Some("FiscalYear"),
            Some(42),
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

    // ── AI convenience methods ─────────────────────────────────────────────────

    #[test]
    fn log_ai_prompt_truncates_long_description() {
        let conn = db();
        let repo = AuditRepo::new(&conn);
        let long = "x".repeat(600);
        repo.log_ai_prompt("Ent", &long).expect("log_ai_prompt");
        let entries = repo.list(&AuditFilter::default()).expect("list");
        assert_eq!(entries[0].description.len(), 500);
        assert_eq!(entries[0].action_type, AuditAction::AiPrompt);
    }

    #[test]
    fn log_ai_prompt_short_description_stored_as_is() {
        let conn = db();
        let repo = AuditRepo::new(&conn);
        repo.log_ai_prompt("Ent", "short prompt").expect("log");
        let entries = repo.list(&AuditFilter::default()).expect("list");
        assert_eq!(entries[0].description, "short prompt");
    }

    #[test]
    fn log_ai_tool_use_formats_description() {
        let conn = db();
        let repo = AuditRepo::new(&conn);
        repo.log_ai_tool_use("Ent", "get_account", "5100")
            .expect("log");
        let entries = repo.list(&AuditFilter::default()).expect("list");
        assert_eq!(entries[0].description, "Used get_account(5100)");
        assert_eq!(entries[0].action_type, AuditAction::AiToolUse);
    }

    #[test]
    fn log_csv_import_formats_description() {
        let conn = db();
        let repo = AuditRepo::new(&conn);
        repo.log_csv_import("Ent", "SoFi Checking", 10, 8, 5, 3)
            .expect("log");
        let entries = repo.list(&AuditFilter::default()).expect("list");
        assert!(entries[0].description.contains("SoFi Checking"));
        assert!(entries[0].description.contains("10 rows"));
        assert_eq!(entries[0].action_type, AuditAction::CsvImport);
    }

    #[test]
    fn get_ai_entries_filters_by_ai_action_types() {
        let conn = db();
        let repo = AuditRepo::new(&conn);
        repo.log_ai_prompt("Ent", "a prompt").expect("log prompt");
        repo.log_ai_response("Ent", "a response")
            .expect("log response");
        repo.log_ai_tool_use("Ent", "tool", "arg")
            .expect("log tool");
        repo.log_csv_import("Ent", "Bank", 1, 1, 0, 1)
            .expect("log import");

        let ai = repo.get_ai_entries(None, None, None).expect("get_ai");
        assert_eq!(ai.len(), 3, "Should only return AI-action entries");
        let types: Vec<_> = ai.iter().map(|e| e.action_type).collect();
        assert!(types.contains(&AuditAction::AiPrompt));
        assert!(types.contains(&AuditAction::AiResponse));
        assert!(types.contains(&AuditAction::AiToolUse));
    }

    #[test]
    fn get_ai_entries_no_entries_returns_empty() {
        let conn = db();
        let repo = AuditRepo::new(&conn);
        let ai = repo.get_ai_entries(None, None, None).expect("get_ai");
        assert!(ai.is_empty());
    }

    #[test]
    fn get_ai_entries_respects_limit() {
        let conn = db();
        let repo = AuditRepo::new(&conn);
        for i in 0..5 {
            repo.log_ai_prompt("Ent", &format!("prompt {i}"))
                .expect("log");
        }
        let ai = repo.get_ai_entries(None, None, Some(3)).expect("get_ai");
        assert_eq!(ai.len(), 3);
    }

    #[test]
    fn get_ai_entries_filters_by_date_range() {
        let conn = db();
        let repo = AuditRepo::new(&conn);

        conn.execute(
            "INSERT INTO audit_log (action_type, entity_name, record_type, record_id, description, created_at)
             VALUES ('AiPrompt', 'Ent', NULL, NULL, 'old prompt', '2025-01-01T00:00:00')",
            [],
        ).expect("insert old");
        conn.execute(
            "INSERT INTO audit_log (action_type, entity_name, record_type, record_id, description, created_at)
             VALUES ('AiPrompt', 'Ent', NULL, NULL, 'new prompt', '2026-01-01T00:00:00')",
            [],
        ).expect("insert new");

        let start = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let ai = repo
            .get_ai_entries(Some(start), None, None)
            .expect("get_ai");
        assert_eq!(ai.len(), 1);
        assert_eq!(ai[0].description, "new prompt");
    }
}
