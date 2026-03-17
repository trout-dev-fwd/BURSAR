use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use rusqlite::{Connection, params};

use super::now_str;
use crate::types::{
    AccountId, FiscalPeriodId, JournalEntryId, JournalEntryLineId, JournalEntryStatus, Money,
    ReconcileState,
};

// ── Data structs ──────────────────────────────────────────────────────────────

/// A journal entry header row loaded from the database.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalEntry {
    pub id: JournalEntryId,
    pub je_number: String,
    pub entry_date: NaiveDate,
    pub memo: Option<String>,
    pub status: JournalEntryStatus,
    pub is_reversed: bool,
    pub reversed_by_je_id: Option<JournalEntryId>,
    pub reversal_of_je_id: Option<JournalEntryId>,
    pub inter_entity_uuid: Option<String>,
    pub source_entity_name: Option<String>,
    pub fiscal_period_id: FiscalPeriodId,
    pub created_at: String,
    pub updated_at: String,
    /// Composite reference string for tracing a draft back to its source bank statement line.
    /// Format: `"{bank_name}|{date}|{description}|{amount}"`. Null for non-imported entries.
    pub import_ref: Option<String>,
}

/// A single debit/credit line within a journal entry.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalEntryLine {
    pub id: JournalEntryLineId,
    pub journal_entry_id: JournalEntryId,
    pub account_id: AccountId,
    pub debit_amount: Money,
    pub credit_amount: Money,
    pub line_memo: Option<String>,
    pub reconcile_state: ReconcileState,
    pub sort_order: i32,
    pub created_at: String,
}

/// Data required to create a new journal entry (draft).
#[derive(Debug, Clone)]
pub struct NewJournalEntry {
    pub entry_date: NaiveDate,
    pub memo: Option<String>,
    /// Must reference an existing open fiscal period.
    pub fiscal_period_id: FiscalPeriodId,
    /// Set when this entry is a reversal of another. `None` for normal entries.
    pub reversal_of_je_id: Option<JournalEntryId>,
    pub lines: Vec<NewJournalEntryLine>,
}

/// Data for a single line within a new journal entry.
#[derive(Debug, Clone)]
pub struct NewJournalEntryLine {
    pub account_id: AccountId,
    /// One of debit_amount / credit_amount must be zero per the schema design.
    pub debit_amount: Money,
    pub credit_amount: Money,
    pub line_memo: Option<String>,
    pub sort_order: i32,
}

/// Filter criteria for `JournalRepo::list`. All fields are optional (None = no filter).
#[derive(Debug, Clone, Default)]
pub struct JournalFilter {
    pub status: Option<JournalEntryStatus>,
    pub from_date: Option<NaiveDate>,
    pub to_date: Option<NaiveDate>,
}

/// Date range filter used in ledger and reporting queries.
#[derive(Debug, Clone, Copy, Default)]
pub struct DateRange {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
}

/// A single row in a per-account General Ledger view.
/// Combines data from `journal_entries` and `journal_entry_lines`.
#[derive(Debug, Clone)]
pub struct LedgerRow {
    pub je_id: JournalEntryId,
    pub je_number: String,
    pub entry_date: NaiveDate,
    /// Line-level memo if set, otherwise falls back to the JE-level memo.
    pub memo: Option<String>,
    pub debit: Money,
    pub credit: Money,
    pub reconcile_state: ReconcileState,
    /// Cumulative net balance (Σ debit − Σ credit) through this row, ordered
    /// chronologically. Credit-normal account display logic should negate this.
    pub running_balance: Money,
}

// ── Repository ────────────────────────────────────────────────────────────────

/// Repository for the `journal_entries` and `journal_entry_lines` tables.
/// This is the data-access layer only. Business logic (post/reverse) lives in
/// the orchestration functions in `src/services/journal.rs` (Phase 2b, Task 2).
pub struct JournalRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> JournalRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    /// Generates the next sequential JE number in "JE-NNNN" format.
    /// The first entry returns "JE-0001". Numbers never reuse even after deletes.
    pub fn get_next_je_number(&self) -> Result<String> {
        let max: Option<String> = self
            .conn
            .query_row("SELECT MAX(je_number) FROM journal_entries", [], |row| {
                row.get(0)
            })
            .context("Failed to query max JE number")?;

        let next = match max {
            None => 1,
            Some(s) => {
                // Format: "JE-NNNN" — extract the numeric suffix after "JE-".
                let num: u32 = s
                    .strip_prefix("JE-")
                    .and_then(|suffix| suffix.parse().ok())
                    .unwrap_or(0);
                num + 1
            }
        };

        Ok(format!("JE-{next:04}"))
    }

    /// Creates a draft journal entry with its lines in a single operation.
    /// Returns the new JE's ID.
    pub fn create_draft(&self, entry: &NewJournalEntry) -> Result<JournalEntryId> {
        self.create_draft_inner(entry, None)
    }

    /// Creates a draft journal entry with an optional import reference.
    /// Used by the CSV import pipeline to link drafts back to their source bank statement lines.
    pub fn create_draft_with_import_ref(
        &self,
        entry: &NewJournalEntry,
        import_ref: Option<&str>,
    ) -> Result<JournalEntryId> {
        self.create_draft_inner(entry, import_ref)
    }

    fn create_draft_inner(
        &self,
        entry: &NewJournalEntry,
        import_ref: Option<&str>,
    ) -> Result<JournalEntryId> {
        // Reject if the target fiscal period is closed — drafts in closed periods cannot
        // be posted, so we refuse at creation to avoid orphaned un-postable entries.
        let is_closed: i32 = self
            .conn
            .query_row(
                "SELECT is_closed FROM fiscal_periods WHERE id = ?1",
                params![i64::from(entry.fiscal_period_id)],
                |row| row.get(0),
            )
            .with_context(|| {
                format!(
                    "Fiscal period {} not found",
                    i64::from(entry.fiscal_period_id)
                )
            })?;
        if is_closed != 0 {
            anyhow::bail!(
                "Cannot create journal entry in closed fiscal period {}",
                i64::from(entry.fiscal_period_id)
            );
        }

        let je_number = self.get_next_je_number()?;
        let now = now_str();
        let date_str = entry.entry_date.to_string();

        self.conn
            .execute(
                "INSERT INTO journal_entries
                    (je_number, entry_date, memo, status, is_reversed,
                     reversal_of_je_id, fiscal_period_id, created_at, updated_at, import_ref)
                 VALUES (?1, ?2, ?3, 'Draft', 0, ?4, ?5, ?6, ?7, ?8)",
                params![
                    je_number,
                    date_str,
                    entry.memo,
                    entry.reversal_of_je_id.map(i64::from),
                    i64::from(entry.fiscal_period_id),
                    now,
                    now,
                    import_ref,
                ],
            )
            .context("Failed to insert journal entry")?;

        let je_id = JournalEntryId::from(self.conn.last_insert_rowid());

        for line in &entry.lines {
            self.conn
                .execute(
                    "INSERT INTO journal_entry_lines
                        (journal_entry_id, account_id, debit_amount, credit_amount,
                         line_memo, reconcile_state, sort_order, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'Uncleared', ?6, ?7)",
                    params![
                        i64::from(je_id),
                        i64::from(line.account_id),
                        line.debit_amount.0,
                        line.credit_amount.0,
                        line.line_memo,
                        line.sort_order,
                        now,
                    ],
                )
                .context("Failed to insert journal entry line")?;
        }

        Ok(je_id)
    }

    /// Sets `inter_entity_uuid` and `source_entity_name` on an existing journal entry.
    ///
    /// Called immediately after `create_draft` during the inter-entity write protocol
    /// to tag the entry with the cross-database linkage before posting.
    pub fn set_inter_entity_metadata(
        &self,
        id: JournalEntryId,
        uuid: &str,
        source_entity_name: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE journal_entries
             SET inter_entity_uuid = ?1, source_entity_name = ?2, updated_at = ?3
             WHERE id = ?4",
            params![uuid, source_entity_name, now_str(), i64::from(id)],
        )?;
        Ok(())
    }

    /// Permanently deletes a Draft journal entry and all its lines.
    ///
    /// **Only permitted during inter-entity rollback recovery (Phase 6).**
    /// Returns an error if the entry is not in Draft status — Posted entries
    /// must be reversed, never deleted.
    pub fn delete_draft(&self, id: JournalEntryId) -> Result<()> {
        let status: String = self
            .conn
            .query_row(
                "SELECT status FROM journal_entries WHERE id = ?1",
                params![i64::from(id)],
                |row| row.get(0),
            )
            .with_context(|| format!("delete_draft: journal entry {} not found", i64::from(id)))?;
        if status != "Draft" {
            anyhow::bail!(
                "delete_draft: entry {} has status '{status}', expected Draft",
                i64::from(id)
            );
        }
        self.conn.execute(
            "DELETE FROM journal_entry_lines WHERE journal_entry_id = ?1",
            params![i64::from(id)],
        )?;
        self.conn.execute(
            "DELETE FROM journal_entries WHERE id = ?1",
            params![i64::from(id)],
        )?;
        Ok(())
    }

    /// Updates an existing Draft journal entry in place.
    ///
    /// Validates that the entry is still in Draft status, then atomically:
    /// 1. Updates the header row (date, memo, fiscal_period_id).
    /// 2. Deletes all existing lines.
    /// 3. Re-inserts the new lines.
    ///
    /// A savepoint wraps steps 1–3 so a failure mid-way leaves the entry unchanged.
    pub fn update_draft(
        &self,
        id: JournalEntryId,
        entry_date: NaiveDate,
        memo: Option<String>,
        fiscal_period_id: FiscalPeriodId,
        lines: &[NewJournalEntryLine],
    ) -> Result<()> {
        let status: String = self
            .conn
            .query_row(
                "SELECT status FROM journal_entries WHERE id = ?1",
                params![i64::from(id)],
                |row| row.get(0),
            )
            .with_context(|| format!("update_draft: journal entry {} not found", i64::from(id)))?;
        if status != "Draft" {
            anyhow::bail!(
                "Cannot edit entry {}: only Draft entries can be edited (got {status})",
                i64::from(id)
            );
        }

        let now = now_str();
        self.conn
            .execute("SAVEPOINT update_draft_sp", [])
            .context("Failed to create savepoint")?;

        let result = self.do_update_draft(id, entry_date, memo, fiscal_period_id, lines, &now);

        if result.is_err() {
            let _ = self
                .conn
                .execute("ROLLBACK TO SAVEPOINT update_draft_sp", []);
            let _ = self.conn.execute("RELEASE SAVEPOINT update_draft_sp", []);
            return result;
        }
        self.conn
            .execute("RELEASE SAVEPOINT update_draft_sp", [])
            .context("Failed to release savepoint")?;
        Ok(())
    }

    fn do_update_draft(
        &self,
        id: JournalEntryId,
        entry_date: NaiveDate,
        memo: Option<String>,
        fiscal_period_id: FiscalPeriodId,
        lines: &[NewJournalEntryLine],
        now: &str,
    ) -> Result<()> {
        self.conn
            .execute(
                "UPDATE journal_entries
                 SET entry_date = ?1, memo = ?2, fiscal_period_id = ?3, updated_at = ?4
                 WHERE id = ?5",
                params![
                    entry_date.to_string(),
                    memo,
                    i64::from(fiscal_period_id),
                    now,
                    i64::from(id),
                ],
            )
            .context("Failed to update journal entry header")?;

        self.conn
            .execute(
                "DELETE FROM journal_entry_lines WHERE journal_entry_id = ?1",
                params![i64::from(id)],
            )
            .context("Failed to delete existing lines")?;

        for line in lines {
            self.conn
                .execute(
                    "INSERT INTO journal_entry_lines
                        (journal_entry_id, account_id, debit_amount, credit_amount,
                         line_memo, reconcile_state, sort_order, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'Uncleared', ?6, ?7)",
                    params![
                        i64::from(id),
                        i64::from(line.account_id),
                        line.debit_amount.0,
                        line.credit_amount.0,
                        line.line_memo,
                        line.sort_order,
                        now,
                    ],
                )
                .context("Failed to insert updated line")?;
        }
        Ok(())
    }

    /// Returns a journal entry and all its lines, ordered by sort_order then id.
    pub fn get_with_lines(
        &self,
        id: JournalEntryId,
    ) -> Result<(JournalEntry, Vec<JournalEntryLine>)> {
        let entry = self
            .conn
            .query_row(
                "SELECT id, je_number, entry_date, memo, status, is_reversed,
                         reversed_by_je_id, reversal_of_je_id, inter_entity_uuid,
                         source_entity_name, fiscal_period_id, created_at, updated_at, import_ref
                  FROM journal_entries WHERE id = ?1",
                params![i64::from(id)],
                row_to_entry,
            )
            .with_context(|| format!("Journal entry not found: {}", i64::from(id)))?;

        let mut stmt = self.conn.prepare(
            "SELECT id, journal_entry_id, account_id, debit_amount, credit_amount,
                    line_memo, reconcile_state, sort_order, created_at
             FROM journal_entry_lines
             WHERE journal_entry_id = ?1
             ORDER BY sort_order, id",
        )?;

        let lines = stmt
            .query_map(params![i64::from(id)], row_to_line)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect::<Result<Vec<_>>>()?;

        Ok((entry, lines))
    }

    /// Returns journal entries matching `filter`, ordered by entry_date DESC, je_number DESC.
    /// Uses dynamic SQL building (not sentinel patterns) for correct index usage.
    pub fn list(&self, filter: &JournalFilter) -> Result<Vec<JournalEntry>> {
        // Build WHERE conditions and a matching list of string params.
        let mut conditions: Vec<&'static str> = Vec::new();
        let mut param_strings: Vec<String> = Vec::new();

        if let Some(status) = &filter.status {
            conditions.push("status = ?");
            param_strings.push(status.to_string());
        }
        if let Some(from) = &filter.from_date {
            conditions.push("entry_date >= ?");
            param_strings.push(from.to_string());
        }
        if let Some(to) = &filter.to_date {
            conditions.push("entry_date <= ?");
            param_strings.push(to.to_string());
        }

        // Renumber placeholders to ?1, ?2, … required by rusqlite.
        let numbered: Vec<String> = conditions
            .iter()
            .enumerate()
            .map(|(i, c)| c.replace('?', &format!("?{}", i + 1)))
            .collect();

        let where_clause = if numbered.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", numbered.join(" AND "))
        };

        let sql = format!(
            "SELECT id, je_number, entry_date, memo, status, is_reversed,
                    reversed_by_je_id, reversal_of_je_id, inter_entity_uuid,
                    source_entity_name, fiscal_period_id, created_at, updated_at, import_ref
             FROM journal_entries
             {where_clause}
             ORDER BY entry_date DESC, je_number DESC"
        );

        let mut stmt = self.conn.prepare(&sql)?;
        stmt.query_map(
            rusqlite::params_from_iter(param_strings.iter()),
            row_to_entry,
        )?
        .map(|r| r.map_err(anyhow::Error::from))
        .collect()
    }

    /// Updates the status of a journal entry.
    pub fn update_status(&self, id: JournalEntryId, status: JournalEntryStatus) -> Result<()> {
        self.conn
            .execute(
                "UPDATE journal_entries SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![status.to_string(), now_str(), i64::from(id)],
            )
            .with_context(|| format!("Failed to update status for JE {}", i64::from(id)))?;
        Ok(())
    }

    /// Marks a journal entry as reversed, recording which JE reversed it.
    pub fn mark_reversed(&self, id: JournalEntryId, reversed_by: JournalEntryId) -> Result<()> {
        self.conn
            .execute(
                "UPDATE journal_entries
                 SET is_reversed = 1, reversed_by_je_id = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![i64::from(reversed_by), now_str(), i64::from(id)],
            )
            .with_context(|| format!("Failed to mark JE {} as reversed", i64::from(id)))?;
        Ok(())
    }

    /// Returns all posted lines for `account_id` in chronological order,
    /// with a cumulative running balance (debit-basis) pre-computed on each row.
    ///
    /// If `date_range` is `Some`, only lines whose JE date falls within the
    /// range are returned. The running balance starts at 0 from the first
    /// returned row; it does not include any pre-filter balance.
    pub fn list_lines_for_account(
        &self,
        account_id: AccountId,
        date_range: Option<DateRange>,
    ) -> Result<Vec<LedgerRow>> {
        let from_str: Option<String> = date_range.and_then(|dr| dr.from).map(|d| d.to_string());
        let to_str: Option<String> = date_range.and_then(|dr| dr.to).map(|d| d.to_string());

        let mut stmt = self.conn.prepare(
            "SELECT je.id, je.je_number, je.entry_date, je.memo,
                    jel.line_memo, jel.debit_amount, jel.credit_amount, jel.reconcile_state
             FROM journal_entry_lines jel
             JOIN journal_entries je ON je.id = jel.journal_entry_id
             WHERE jel.account_id = ?1
               AND je.status = 'Posted'
               AND (?2 IS NULL OR je.entry_date >= ?2)
               AND (?3 IS NULL OR je.entry_date <= ?3)
             ORDER BY je.entry_date ASC, je.je_number ASC, jel.sort_order ASC, jel.id ASC",
        )?;

        type RawRow = (
            JournalEntryId,
            String,
            NaiveDate,
            Option<String>,
            Money,
            Money,
            ReconcileState,
        );

        let raw: Vec<RawRow> = stmt
            .query_map(params![i64::from(account_id), from_str, to_str], |row| {
                let date_str: String = row.get(2)?;
                let entry_date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                let reconcile_str: String = row.get(7)?;
                let reconcile_state = reconcile_str.parse::<ReconcileState>().map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        7,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                // Prefer line-level memo; fall back to JE-level memo.
                let je_memo: Option<String> = row.get(3)?;
                let line_memo: Option<String> = row.get(4)?;
                let memo = line_memo.or(je_memo);
                Ok((
                    JournalEntryId::from(row.get::<_, i64>(0)?),
                    row.get::<_, String>(1)?,
                    entry_date,
                    memo,
                    Money(row.get::<_, i64>(5)?),
                    Money(row.get::<_, i64>(6)?),
                    reconcile_state,
                ))
            })?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect::<Result<Vec<_>>>()?;

        // Compute running balance (Σ debit − Σ credit), oldest-first.
        let mut balance = Money(0);
        let rows = raw
            .into_iter()
            .map(
                |(je_id, je_number, entry_date, memo, debit, credit, reconcile_state)| {
                    balance = Money(balance.0 + debit.0 - credit.0);
                    LedgerRow {
                        je_id,
                        je_number,
                        entry_date,
                        memo,
                        debit,
                        credit,
                        reconcile_state,
                        running_balance: balance,
                    }
                },
            )
            .collect();

        Ok(rows)
    }

    /// Updates the reconcile state of a single journal entry line.
    ///
    /// Rejects the update if the line is already `Reconciled` (terminal state)
    /// or if the line's journal entry is in a closed fiscal period.
    pub fn update_reconcile_state(
        &self,
        line_id: JournalEntryLineId,
        new_state: ReconcileState,
    ) -> Result<()> {
        // Fetch current reconcile state and the entry's period closure status in one query.
        let (current_state_str, is_closed): (String, i32) = self
            .conn
            .query_row(
                "SELECT jel.reconcile_state, fp.is_closed
                 FROM journal_entry_lines jel
                 JOIN journal_entries je ON je.id = jel.journal_entry_id
                 JOIN fiscal_periods fp ON fp.id = je.fiscal_period_id
                 WHERE jel.id = ?1",
                params![i64::from(line_id)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .with_context(|| {
                format!(
                    "Failed to look up reconcile state for line {}",
                    i64::from(line_id)
                )
            })?;

        let current_state = current_state_str
            .parse::<ReconcileState>()
            .with_context(|| {
                format!(
                    "Invalid reconcile_state '{}' for line {}",
                    current_state_str,
                    i64::from(line_id)
                )
            })?;

        if current_state == ReconcileState::Reconciled {
            anyhow::bail!(
                "Cannot modify reconciled entries. Reconciled state is permanent. (line {})",
                i64::from(line_id)
            );
        }

        if is_closed != 0 {
            anyhow::bail!(
                "Cannot modify reconcile state for line {} — fiscal period is closed",
                i64::from(line_id)
            );
        }

        self.conn
            .execute(
                "UPDATE journal_entry_lines SET reconcile_state = ?1 WHERE id = ?2",
                params![new_state.to_string(), i64::from(line_id)],
            )
            .with_context(|| {
                format!(
                    "Failed to update reconcile state for line {}",
                    i64::from(line_id)
                )
            })?;
        Ok(())
    }

    /// Returns all non-null import_ref values from journal entries created in the last `days` days.
    /// Used for duplicate detection before importing new CSV rows.
    pub fn get_recent_import_refs(&self, days: i64) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT import_ref FROM journal_entries
             WHERE import_ref IS NOT NULL
               AND created_at >= datetime('now', ?1)",
        )?;
        let modifier = format!("-{days} days");
        let refs = stmt
            .query_map(params![modifier], |row| row.get::<_, String>(0))?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect::<Result<HashSet<String>>>()?;
        Ok(refs)
    }

    /// Returns draft journal entries that have an import_ref but fewer than 2 lines
    /// with a non-null account_id. These are candidates for re-matching.
    pub fn get_incomplete_imports(&self) -> Result<Vec<JournalEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, je_number, entry_date, memo, status, is_reversed,
                    reversed_by_je_id, reversal_of_je_id, inter_entity_uuid,
                    source_entity_name, fiscal_period_id, created_at, updated_at,
                    import_ref
             FROM journal_entries je
             WHERE je.status = 'Draft'
               AND je.import_ref IS NOT NULL
               AND (
                   SELECT COUNT(*) FROM journal_entry_lines jel
                   WHERE jel.journal_entry_id = je.id
                     AND jel.account_id IS NOT NULL
               ) < 2
             ORDER BY je.entry_date, je.id",
        )?;
        stmt.query_map([], row_to_entry)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect()
    }
}

// ── Row mappers ───────────────────────────────────────────────────────────────

pub(crate) fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<JournalEntry> {
    let status_str: String = row.get(4)?;
    let status = status_str.parse::<JournalEntryStatus>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let date_str: String = row.get(2)?;
    let entry_date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(JournalEntry {
        id: JournalEntryId::from(row.get::<_, i64>(0)?),
        je_number: row.get(1)?,
        entry_date,
        memo: row.get(3)?,
        status,
        is_reversed: row.get::<_, i32>(5)? != 0,
        reversed_by_je_id: row.get::<_, Option<i64>>(6)?.map(JournalEntryId::from),
        reversal_of_je_id: row.get::<_, Option<i64>>(7)?.map(JournalEntryId::from),
        inter_entity_uuid: row.get(8)?,
        source_entity_name: row.get(9)?,
        fiscal_period_id: FiscalPeriodId::from(row.get::<_, i64>(10)?),
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        import_ref: row.get(13)?,
    })
}

fn row_to_line(row: &rusqlite::Row<'_>) -> rusqlite::Result<JournalEntryLine> {
    let reconcile_str: String = row.get(6)?;
    let reconcile_state = reconcile_str.parse::<ReconcileState>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(JournalEntryLine {
        id: JournalEntryLineId::from(row.get::<_, i64>(0)?),
        journal_entry_id: JournalEntryId::from(row.get::<_, i64>(1)?),
        account_id: AccountId::from(row.get::<_, i64>(2)?),
        debit_amount: Money(row.get::<_, i64>(3)?),
        credit_amount: Money(row.get::<_, i64>(4)?),
        line_memo: row.get(5)?,
        reconcile_state,
        sort_order: row.get(7)?,
        created_at: row.get(8)?,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::account_repo::{AccountRepo, NewAccount};
    use crate::db::fiscal_repo::FiscalRepo;
    use crate::db::schema::{initialize_schema, seed_default_accounts};
    use crate::types::{AccountType, JournalEntryStatus};
    use rusqlite::Connection;

    /// Sets up an in-memory DB with schema, seeded accounts, and a 2026 fiscal year.
    /// Returns the connection and the ID of the first open fiscal period (Jan 2026).
    fn db_with_fiscal_year() -> (Connection, FiscalPeriodId, AccountId, AccountId) {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema init");
        seed_default_accounts(&conn).expect("seed accounts");

        let fiscal = FiscalRepo::new(&conn);
        let fy_id = fiscal.create_fiscal_year(1, 2026).expect("create FY");
        let periods = fiscal.list_periods(fy_id).expect("list periods");
        let period_id = periods[0].id; // January 2026

        // Pick two postable (non-placeholder) accounts for test lines.
        let accounts = AccountRepo::new(&conn);
        let all = accounts.list_active().expect("list active");
        let non_placeholder: Vec<_> = all.iter().filter(|a| !a.is_placeholder).collect();
        assert!(
            non_placeholder.len() >= 2,
            "Need at least 2 non-placeholder accounts"
        );
        let acct1 = non_placeholder[0].id;
        let acct2 = non_placeholder[1].id;

        (conn, period_id, acct1, acct2)
    }

    /// Helper: creates a minimal NewAccount for tests that need custom accounts.
    fn make_account(conn: &Connection, number: &str, name: &str) -> AccountId {
        AccountRepo::new(conn)
            .create(&NewAccount {
                number: number.to_string(),
                name: name.to_string(),
                account_type: AccountType::Asset,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create account")
    }

    // ── create_draft + get_with_lines ─────────────────────────────────────────

    #[test]
    fn create_draft_and_retrieve_with_lines() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        let entry_date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let new_entry = NewJournalEntry {
            entry_date,
            memo: Some("Test entry".to_string()),
            fiscal_period_id: period_id,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: acct1,
                    debit_amount: Money(10_000_000_000), // $100.00
                    credit_amount: Money(0),
                    line_memo: Some("Debit line".to_string()),
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: acct2,
                    debit_amount: Money(0),
                    credit_amount: Money(10_000_000_000), // $100.00
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };

        let je_id = repo.create_draft(&new_entry).expect("create_draft failed");

        let (entry, lines) = repo.get_with_lines(je_id).expect("get_with_lines failed");

        assert_eq!(entry.id, je_id);
        assert_eq!(entry.entry_date, entry_date);
        assert_eq!(entry.memo.as_deref(), Some("Test entry"));
        assert_eq!(entry.status, JournalEntryStatus::Draft);
        assert!(!entry.is_reversed);
        assert!(entry.reversed_by_je_id.is_none());
        assert!(entry.reversal_of_je_id.is_none());
        assert_eq!(entry.fiscal_period_id, period_id);

        assert_eq!(lines.len(), 2, "Should have 2 lines");
        assert_eq!(lines[0].account_id, acct1);
        assert_eq!(lines[0].debit_amount, Money(10_000_000_000));
        assert_eq!(lines[0].credit_amount, Money(0));
        assert_eq!(lines[0].line_memo.as_deref(), Some("Debit line"));
        assert_eq!(lines[0].reconcile_state, ReconcileState::Uncleared);
        assert_eq!(lines[0].sort_order, 0);

        assert_eq!(lines[1].account_id, acct2);
        assert_eq!(lines[1].debit_amount, Money(0));
        assert_eq!(lines[1].credit_amount, Money(10_000_000_000));
        assert!(lines[1].line_memo.is_none());
        assert_eq!(lines[1].sort_order, 1);
    }

    #[test]
    fn get_with_lines_errors_on_nonexistent_id() {
        let (conn, _, _, _) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);
        let result = repo.get_with_lines(JournalEntryId::from(9999));
        assert!(result.is_err(), "Should error on nonexistent JE");
    }

    // ── JE number sequencing ──────────────────────────────────────────────────

    #[test]
    fn je_numbers_are_sequential_starting_at_0001() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        let make_entry = |date: NaiveDate| NewJournalEntry {
            entry_date: date,
            memo: None,
            fiscal_period_id: period_id,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: acct1,
                    debit_amount: Money(10_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: acct2,
                    debit_amount: Money(0),
                    credit_amount: Money(10_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };

        let d1 = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();

        let id1 = repo.create_draft(&make_entry(d1)).expect("first create");
        let id2 = repo.create_draft(&make_entry(d2)).expect("second create");

        let (e1, _) = repo.get_with_lines(id1).expect("get first");
        let (e2, _) = repo.get_with_lines(id2).expect("get second");

        assert_eq!(e1.je_number, "JE-0001");
        assert_eq!(e2.je_number, "JE-0002");
    }

    // ── update_draft ──────────────────────────────────────────────────────────

    #[test]
    fn update_draft_updates_header_and_lines() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        let orig_date = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let je_id = repo
            .create_draft(&NewJournalEntry {
                entry_date: orig_date,
                memo: Some("Original memo".to_string()),
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: acct1,
                        debit_amount: Money(10_000_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: acct2,
                        debit_amount: Money(0),
                        credit_amount: Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create draft");

        let new_date = NaiveDate::from_ymd_opt(2026, 1, 20).unwrap();
        let new_lines = vec![
            NewJournalEntryLine {
                account_id: acct2,
                debit_amount: Money(5_000_000_000),
                credit_amount: Money(0),
                line_memo: Some("Updated line".to_string()),
                sort_order: 0,
            },
            NewJournalEntryLine {
                account_id: acct1,
                debit_amount: Money(0),
                credit_amount: Money(5_000_000_000),
                line_memo: None,
                sort_order: 1,
            },
        ];

        repo.update_draft(
            je_id,
            new_date,
            Some("Updated memo".to_string()),
            period_id,
            &new_lines,
        )
        .expect("update_draft");

        let (entry, lines) = repo.get_with_lines(je_id).expect("get");
        assert_eq!(entry.entry_date, new_date);
        assert_eq!(entry.memo.as_deref(), Some("Updated memo"));
        assert_eq!(
            entry.status,
            JournalEntryStatus::Draft,
            "Status must remain Draft"
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].account_id, acct2);
        assert_eq!(lines[0].debit_amount, Money(5_000_000_000));
        assert_eq!(lines[0].line_memo.as_deref(), Some("Updated line"));
        assert_eq!(lines[1].account_id, acct1);
        assert_eq!(lines[1].credit_amount, Money(5_000_000_000));
    }

    #[test]
    fn update_draft_rejects_non_draft() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        let je_id = repo
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
                memo: None,
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: acct1,
                        debit_amount: Money(10_000_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: acct2,
                        debit_amount: Money(0),
                        credit_amount: Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create");
        repo.update_status(je_id, JournalEntryStatus::Posted)
            .expect("post");

        let result = repo.update_draft(
            je_id,
            NaiveDate::from_ymd_opt(2026, 1, 20).unwrap(),
            None,
            period_id,
            &[],
        );
        assert!(result.is_err(), "Should reject non-Draft entry");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Draft") || msg.contains("draft"),
            "Error should mention Draft: {msg}"
        );
    }

    #[test]
    fn update_draft_is_atomic() {
        // Enable FK enforcement so an invalid account_id triggers a line INSERT failure,
        // letting us verify the savepoint rolls back the DELETE + incomplete INSERT.
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .expect("fk on");
        initialize_schema(&conn).expect("schema init");
        seed_default_accounts(&conn).expect("seed");

        let fiscal = FiscalRepo::new(&conn);
        let fy_id = fiscal.create_fiscal_year(1, 2026).expect("fy");
        let periods = fiscal.list_periods(fy_id).expect("periods");
        let period_id = periods[0].id;

        let all = AccountRepo::new(&conn).list_active().expect("list");
        let non_placeholder: Vec<_> = all.iter().filter(|a| !a.is_placeholder).collect();
        let acct1 = non_placeholder[0].id;
        let acct2 = non_placeholder[1].id;
        let repo = JournalRepo::new(&conn);

        let je_id = repo
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
                memo: Some("Original".to_string()),
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: acct1,
                        debit_amount: Money(10_000_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: acct2,
                        debit_amount: Money(0),
                        credit_amount: Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create");

        // Attempt update with one valid line and one that has a bogus account_id.
        // With FK enforcement on, the second INSERT will fail.
        let bad_account = AccountId::from(999_999);
        let result = repo.update_draft(
            je_id,
            NaiveDate::from_ymd_opt(2026, 1, 20).unwrap(),
            Some("Should not persist".to_string()),
            period_id,
            &[
                NewJournalEntryLine {
                    account_id: acct1,
                    debit_amount: Money(5_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: bad_account,
                    debit_amount: Money(0),
                    credit_amount: Money(5_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        );
        assert!(result.is_err(), "Should fail due to invalid account FK");

        // Original data must be intact (savepoint rolled back all changes).
        let (entry, lines) = repo.get_with_lines(je_id).expect("get");
        assert_eq!(
            entry.memo.as_deref(),
            Some("Original"),
            "Memo must not have changed"
        );
        assert_eq!(
            entry.entry_date,
            NaiveDate::from_ymd_opt(2026, 1, 5).unwrap()
        );
        assert_eq!(lines.len(), 2, "Original 2 lines must still exist");
    }

    // ── list with filters ─────────────────────────────────────────────────────

    #[test]
    fn list_no_filter_returns_all() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        let line1 = NewJournalEntryLine {
            account_id: acct1,
            debit_amount: Money(10_000_000_000),
            credit_amount: Money(0),
            line_memo: None,
            sort_order: 0,
        };
        let line2 = NewJournalEntryLine {
            account_id: acct2,
            debit_amount: Money(0),
            credit_amount: Money(10_000_000_000),
            line_memo: None,
            sort_order: 1,
        };

        let d1 = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();

        repo.create_draft(&NewJournalEntry {
            entry_date: d1,
            memo: None,
            fiscal_period_id: period_id,
            reversal_of_je_id: None,
            lines: vec![line1.clone(), line2.clone()],
        })
        .expect("create 1");

        repo.create_draft(&NewJournalEntry {
            entry_date: d2,
            memo: None,
            fiscal_period_id: period_id,
            reversal_of_je_id: None,
            lines: vec![line1, line2],
        })
        .expect("create 2");

        let entries = repo.list(&JournalFilter::default()).expect("list all");
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn list_with_status_filter_returns_matching_only() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        let make_lines = || {
            vec![
                NewJournalEntryLine {
                    account_id: acct1,
                    debit_amount: Money(10_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: acct2,
                    debit_amount: Money(0),
                    credit_amount: Money(10_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ]
        };

        let date = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();

        let id1 = repo
            .create_draft(&NewJournalEntry {
                entry_date: date,
                memo: None,
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: make_lines(),
            })
            .expect("create draft 1");

        repo.create_draft(&NewJournalEntry {
            entry_date: date,
            memo: None,
            fiscal_period_id: period_id,
            reversal_of_je_id: None,
            lines: make_lines(),
        })
        .expect("create draft 2");

        // Promote one to Posted.
        repo.update_status(id1, JournalEntryStatus::Posted)
            .expect("update status");

        let drafts = repo
            .list(&JournalFilter {
                status: Some(JournalEntryStatus::Draft),
                ..Default::default()
            })
            .expect("list drafts");
        assert_eq!(drafts.len(), 1, "Only one Draft should remain");
        assert_eq!(drafts[0].status, JournalEntryStatus::Draft);

        let posted = repo
            .list(&JournalFilter {
                status: Some(JournalEntryStatus::Posted),
                ..Default::default()
            })
            .expect("list posted");
        assert_eq!(posted.len(), 1, "One Posted entry");
        assert_eq!(posted[0].status, JournalEntryStatus::Posted);
    }

    #[test]
    fn list_with_date_range_filter_works() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema init");

        // Two different fiscal years to cover different months.
        let fiscal = FiscalRepo::new(&conn);
        let fy_id = fiscal.create_fiscal_year(1, 2026).expect("create FY 2026");
        let periods = fiscal.list_periods(fy_id).expect("list periods");
        let jan_period = periods[0].id; // Jan 2026
        let mar_period = periods[2].id; // Mar 2026

        let acct1 = make_account(&conn, "9001", "Test Asset A");
        let acct2 = make_account(&conn, "9002", "Test Asset B");

        let repo = JournalRepo::new(&conn);

        let make_entry = |date: NaiveDate, period: FiscalPeriodId| NewJournalEntry {
            entry_date: date,
            memo: None,
            fiscal_period_id: period,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: acct1,
                    debit_amount: Money(10_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: acct2,
                    debit_amount: Money(0),
                    credit_amount: Money(10_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };

        let jan5 = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let mar10 = NaiveDate::from_ymd_opt(2026, 3, 10).unwrap();

        repo.create_draft(&make_entry(jan5, jan_period))
            .expect("create jan");
        repo.create_draft(&make_entry(mar10, mar_period))
            .expect("create mar");

        // Filter: from Feb 1 — should return only March entry.
        let from_feb = repo
            .list(&JournalFilter {
                from_date: Some(NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()),
                ..Default::default()
            })
            .expect("list from Feb");
        assert_eq!(from_feb.len(), 1);
        assert_eq!(from_feb[0].entry_date, mar10);

        // Filter: to Jan 31 — should return only January entry.
        let to_jan = repo
            .list(&JournalFilter {
                to_date: Some(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap()),
                ..Default::default()
            })
            .expect("list to Jan");
        assert_eq!(to_jan.len(), 1);
        assert_eq!(to_jan[0].entry_date, jan5);

        // Filter: both from and to narrowed to March.
        let mar_only = repo
            .list(&JournalFilter {
                from_date: Some(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()),
                to_date: Some(NaiveDate::from_ymd_opt(2026, 3, 31).unwrap()),
                ..Default::default()
            })
            .expect("list march only");
        assert_eq!(mar_only.len(), 1);
        assert_eq!(mar_only[0].entry_date, mar10);
    }

    // ── update_status ─────────────────────────────────────────────────────────

    #[test]
    fn update_status_changes_status_field() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let id = repo
            .create_draft(&NewJournalEntry {
                entry_date: date,
                memo: None,
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: acct1,
                        debit_amount: Money(10_000_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: acct2,
                        debit_amount: Money(0),
                        credit_amount: Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create");

        repo.update_status(id, JournalEntryStatus::Posted)
            .expect("update_status");

        let (entry, _) = repo.get_with_lines(id).expect("get");
        assert_eq!(entry.status, JournalEntryStatus::Posted);
    }

    // ── mark_reversed ─────────────────────────────────────────────────────────

    #[test]
    fn mark_reversed_sets_flag_and_link() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let make_lines = || {
            vec![
                NewJournalEntryLine {
                    account_id: acct1,
                    debit_amount: Money(10_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: acct2,
                    debit_amount: Money(0),
                    credit_amount: Money(10_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ]
        };

        let original_id = repo
            .create_draft(&NewJournalEntry {
                entry_date: date,
                memo: None,
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: make_lines(),
            })
            .expect("create original");

        let reversal_id = repo
            .create_draft(&NewJournalEntry {
                entry_date: date,
                memo: Some("Reversal".to_string()),
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: make_lines(),
            })
            .expect("create reversal");

        repo.mark_reversed(original_id, reversal_id)
            .expect("mark_reversed");

        let (original, _) = repo.get_with_lines(original_id).expect("get original");
        assert!(original.is_reversed, "is_reversed should be true");
        assert_eq!(
            original.reversed_by_je_id,
            Some(reversal_id),
            "reversed_by_je_id should point to reversal"
        );
    }

    // ── update_reconcile_state guard rails ────────────────────────────────────

    #[test]
    fn update_reconcile_state_rejects_reconciled_line() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let je_id = repo
            .create_draft(&NewJournalEntry {
                entry_date: date,
                memo: None,
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: acct1,
                        debit_amount: Money(10_000_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: acct2,
                        debit_amount: Money(0),
                        credit_amount: Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create");

        let (_, lines) = repo.get_with_lines(je_id).expect("get");
        let line_id = lines[0].id;

        // Force the line to Reconciled state directly in the DB.
        conn.execute(
            "UPDATE journal_entry_lines SET reconcile_state = 'Reconciled' WHERE id = ?1",
            params![i64::from(line_id)],
        )
        .expect("force reconciled");

        let result = repo.update_reconcile_state(line_id, ReconcileState::Cleared);
        assert!(result.is_err(), "Should reject mutation on Reconciled line");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Reconciled") && msg.contains("permanent"),
            "Error should mention Reconciled is permanent: {msg}"
        );
    }

    // ── list_lines_for_account ────────────────────────────────────────────────

    #[test]
    fn list_lines_for_account_omits_draft_entries() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        // Create a draft only — should NOT appear in GL.
        repo.create_draft(&NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
            memo: Some("Draft entry".to_string()),
            fiscal_period_id: period_id,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: acct1,
                    debit_amount: Money(10_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: acct2,
                    debit_amount: Money(0),
                    credit_amount: Money(10_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        })
        .expect("create draft");

        let rows = repo
            .list_lines_for_account(acct1, None)
            .expect("list_lines_for_account");
        assert!(rows.is_empty(), "Draft entries should not appear in GL");
    }

    #[test]
    fn list_lines_for_account_running_balance_correct() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        let make_entry = |date: NaiveDate, debit1: Money, credit1: Money| NewJournalEntry {
            entry_date: date,
            memo: None,
            fiscal_period_id: period_id,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: acct1,
                    debit_amount: debit1,
                    credit_amount: credit1,
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: acct2,
                    debit_amount: credit1,
                    credit_amount: debit1,
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };

        let d1 = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        let d3 = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        // JE1: debit acct1 $100, JE2: debit acct1 $50, JE3: credit acct1 $30.
        let id1 = repo
            .create_draft(&make_entry(d1, Money(10_000_000_000), Money(0)))
            .unwrap();
        let id2 = repo
            .create_draft(&make_entry(d2, Money(5_000_000_000), Money(0)))
            .unwrap();
        let id3 = repo
            .create_draft(&make_entry(d3, Money(0), Money(3_000_000_000)))
            .unwrap();

        repo.update_status(id1, JournalEntryStatus::Posted).unwrap();
        repo.update_status(id2, JournalEntryStatus::Posted).unwrap();
        repo.update_status(id3, JournalEntryStatus::Posted).unwrap();

        let rows = repo
            .list_lines_for_account(acct1, None)
            .expect("list_lines_for_account");
        assert_eq!(rows.len(), 3);

        // After JE1: +$100 → balance = 10_000_000_000
        assert_eq!(rows[0].debit, Money(10_000_000_000));
        assert_eq!(rows[0].credit, Money(0));
        assert_eq!(rows[0].running_balance, Money(10_000_000_000));
        assert_eq!(rows[0].entry_date, d1);

        // After JE2: +$50 → balance = 15_000_000_000
        assert_eq!(rows[1].debit, Money(5_000_000_000));
        assert_eq!(rows[1].running_balance, Money(15_000_000_000));

        // After JE3: -$30 → balance = 12_000_000_000
        assert_eq!(rows[2].credit, Money(3_000_000_000));
        assert_eq!(rows[2].running_balance, Money(12_000_000_000));
    }

    #[test]
    fn list_lines_for_account_date_filter_works() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema init");

        let fiscal = FiscalRepo::new(&conn);
        let fy_id = fiscal.create_fiscal_year(1, 2026).expect("create FY");
        let periods = fiscal.list_periods(fy_id).expect("list periods");
        let jan = periods[0].id;
        let mar = periods[2].id;

        let acct1 = make_account(&conn, "9001", "Test Asset A");
        let acct2 = make_account(&conn, "9002", "Test Asset B");
        let repo = JournalRepo::new(&conn);

        let make_simple = |date: NaiveDate, period: FiscalPeriodId| NewJournalEntry {
            entry_date: date,
            memo: None,
            fiscal_period_id: period,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: acct1,
                    debit_amount: Money(10_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: acct2,
                    debit_amount: Money(0),
                    credit_amount: Money(10_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };

        let jan15 = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let mar15 = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();

        let id1 = repo.create_draft(&make_simple(jan15, jan)).unwrap();
        let id2 = repo.create_draft(&make_simple(mar15, mar)).unwrap();
        repo.update_status(id1, JournalEntryStatus::Posted).unwrap();
        repo.update_status(id2, JournalEntryStatus::Posted).unwrap();

        // No filter: both rows returned.
        let all = repo.list_lines_for_account(acct1, None).unwrap();
        assert_eq!(all.len(), 2);

        // Filter from Feb 1: only March row.
        let from_feb = repo
            .list_lines_for_account(
                acct1,
                Some(DateRange {
                    from: Some(NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()),
                    to: None,
                }),
            )
            .unwrap();
        assert_eq!(from_feb.len(), 1);
        assert_eq!(from_feb[0].entry_date, mar15);

        // Filter to Jan 31: only January row.
        let to_jan = repo
            .list_lines_for_account(
                acct1,
                Some(DateRange {
                    from: None,
                    to: Some(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap()),
                }),
            )
            .unwrap();
        assert_eq!(to_jan.len(), 1);
        assert_eq!(to_jan[0].entry_date, jan15);

        // Running balance resets to 0 within the filtered window.
        assert_eq!(from_feb[0].running_balance, Money(10_000_000_000));
        assert_eq!(to_jan[0].running_balance, Money(10_000_000_000));
    }

    #[test]
    fn list_lines_for_account_memo_prefers_line_memo() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        let je_id = repo
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
                memo: Some("JE memo".to_string()),
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: acct1,
                        debit_amount: Money(10_000_000_000),
                        credit_amount: Money(0),
                        line_memo: Some("Line memo".to_string()),
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: acct2,
                        debit_amount: Money(0),
                        credit_amount: Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .unwrap();
        repo.update_status(je_id, JournalEntryStatus::Posted)
            .unwrap();

        let rows = repo.list_lines_for_account(acct1, None).unwrap();
        assert_eq!(rows.len(), 1);
        // Line memo takes priority over JE memo.
        assert_eq!(rows[0].memo.as_deref(), Some("Line memo"));

        // acct2 has no line memo, should fall back to JE memo.
        let rows2 = repo.list_lines_for_account(acct2, None).unwrap();
        assert_eq!(rows2[0].memo.as_deref(), Some("JE memo"));
    }

    #[test]
    fn update_reconcile_state_rejects_line_in_closed_period() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let je_id = repo
            .create_draft(&NewJournalEntry {
                entry_date: date,
                memo: None,
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: acct1,
                        debit_amount: Money(10_000_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: acct2,
                        debit_amount: Money(0),
                        credit_amount: Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create");

        let (_, lines) = repo.get_with_lines(je_id).expect("get");
        let line_id = lines[0].id;

        // Close the fiscal period.
        conn.execute(
            "UPDATE fiscal_periods SET is_closed = 1 WHERE id = ?1",
            params![i64::from(period_id)],
        )
        .expect("close period");

        let result = repo.update_reconcile_state(line_id, ReconcileState::Cleared);
        assert!(
            result.is_err(),
            "Should reject mutation on line in closed period"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("closed"),
            "Error should mention closed period: {msg}"
        );
    }

    #[test]
    fn create_draft_rejects_closed_period() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        let repo = JournalRepo::new(&conn);

        // Close the fiscal period.
        conn.execute(
            "UPDATE fiscal_periods SET is_closed = 1 WHERE id = ?1",
            params![i64::from(period_id)],
        )
        .expect("close period");

        let result = repo.create_draft(&NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            memo: None,
            fiscal_period_id: period_id,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: acct1,
                    debit_amount: Money(10_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: acct2,
                    debit_amount: Money(0),
                    credit_amount: Money(10_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        });

        assert!(result.is_err(), "create_draft should reject closed period");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("closed"), "Error should mention closed: {msg}");
    }

    // ── get_recent_import_refs ─────────────────────────────────────────────────

    fn make_entry_with_import_ref(
        conn: &Connection,
        period_id: FiscalPeriodId,
        acct1: AccountId,
        acct2: AccountId,
        import_ref: Option<&str>,
    ) -> JournalEntryId {
        let repo = JournalRepo::new(conn);
        let entry = NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            memo: None,
            fiscal_period_id: period_id,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: acct1,
                    debit_amount: Money(10_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: acct2,
                    debit_amount: Money(0),
                    credit_amount: Money(10_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };
        repo.create_draft_with_import_ref(&entry, import_ref)
            .expect("create_draft_with_import_ref")
    }

    #[test]
    fn get_recent_import_refs_returns_refs_from_recent_entries() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();

        make_entry_with_import_ref(
            &conn,
            period_id,
            acct1,
            acct2,
            Some("Bank|2026-01-15|DESC|-100.00"),
        );

        let refs = JournalRepo::new(&conn)
            .get_recent_import_refs(90)
            .expect("get_recent_import_refs");
        assert!(refs.contains("Bank|2026-01-15|DESC|-100.00"));
    }

    #[test]
    fn get_recent_import_refs_empty_when_no_imports() {
        let (conn, _period_id, _acct1, _acct2) = db_with_fiscal_year();
        let refs = JournalRepo::new(&conn)
            .get_recent_import_refs(90)
            .expect("get_recent_import_refs");
        assert!(refs.is_empty());
    }

    #[test]
    fn get_recent_import_refs_excludes_null_import_ref() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        // Entry with no import_ref
        make_entry_with_import_ref(&conn, period_id, acct1, acct2, None);
        let refs = JournalRepo::new(&conn)
            .get_recent_import_refs(90)
            .expect("get_recent_import_refs");
        assert!(refs.is_empty());
    }

    // ── get_incomplete_imports ─────────────────────────────────────────────────

    /// Inserts a single-line draft JE with an import_ref directly via SQL (bypasses the
    /// balance check in create_draft). This simulates an incomplete import where only
    /// one side has been matched.
    fn insert_incomplete_import(
        conn: &Connection,
        period_id: FiscalPeriodId,
        acct: AccountId,
        import_ref: Option<&str>,
    ) -> JournalEntryId {
        conn.execute(
            "INSERT INTO journal_entries
                 (je_number, entry_date, memo, status, is_reversed,
                  reversal_of_je_id, fiscal_period_id, created_at, updated_at, import_ref)
             VALUES ('JE-INCOMPLETE', '2026-01-15', NULL, 'Draft', 0,
                     NULL, ?1, '2026-01-15T00:00:00', '2026-01-15T00:00:00', ?2)",
            params![i64::from(period_id), import_ref],
        )
        .expect("insert je");
        let je_id = JournalEntryId::from(conn.last_insert_rowid());
        conn.execute(
            "INSERT INTO journal_entry_lines
                 (journal_entry_id, account_id, debit_amount, credit_amount,
                  line_memo, reconcile_state, sort_order, created_at)
             VALUES (?1, ?2, 10000000000, 0, NULL, 'None', 0, '2026-01-15T00:00:00')",
            params![i64::from(je_id), i64::from(acct)],
        )
        .expect("insert line");
        je_id
    }

    #[test]
    fn get_incomplete_imports_returns_drafts_with_one_line() {
        let (conn, period_id, acct1, _acct2) = db_with_fiscal_year();
        let je_id = insert_incomplete_import(
            &conn,
            period_id,
            acct1,
            Some("Bank|2026-01-15|DESC|-100.00"),
        );

        let incomplete = JournalRepo::new(&conn)
            .get_incomplete_imports()
            .expect("get_incomplete_imports");
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].id, je_id);
    }

    #[test]
    fn get_incomplete_imports_excludes_entries_without_import_ref() {
        let (conn, period_id, acct1, _acct2) = db_with_fiscal_year();
        insert_incomplete_import(&conn, period_id, acct1, None);

        let incomplete = JournalRepo::new(&conn)
            .get_incomplete_imports()
            .expect("get_incomplete_imports");
        assert!(incomplete.is_empty());
    }

    #[test]
    fn get_incomplete_imports_excludes_complete_drafts() {
        let (conn, period_id, acct1, acct2) = db_with_fiscal_year();
        // Both lines have accounts — not incomplete
        make_entry_with_import_ref(
            &conn,
            period_id,
            acct1,
            acct2,
            Some("Bank|2026-01-15|DESC|-100.00"),
        );

        let incomplete = JournalRepo::new(&conn)
            .get_incomplete_imports()
            .expect("get_incomplete_imports");
        assert!(incomplete.is_empty());
    }

    #[test]
    fn get_incomplete_imports_excludes_posted_entries() {
        let (conn, period_id, acct1, _acct2) = db_with_fiscal_year();
        let je_id = insert_incomplete_import(
            &conn,
            period_id,
            acct1,
            Some("Bank|2026-01-15|DESC|-100.00"),
        );

        conn.execute(
            "UPDATE journal_entries SET status = 'Posted' WHERE id = ?1",
            params![i64::from(je_id)],
        )
        .expect("post entry");

        let incomplete = JournalRepo::new(&conn)
            .get_incomplete_imports()
            .expect("get_incomplete_imports");
        assert!(incomplete.is_empty());
    }
}
