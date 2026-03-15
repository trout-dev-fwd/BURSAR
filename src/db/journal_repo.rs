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
                let num: u32 = s.strip_prefix("JE-").unwrap_or("0").parse().unwrap_or(0);
                num + 1
            }
        };

        Ok(format!("JE-{next:04}"))
    }

    /// Creates a draft journal entry with its lines in a single operation.
    /// Returns the new JE's ID.
    pub fn create_draft(&self, entry: &NewJournalEntry) -> Result<JournalEntryId> {
        let je_number = self.get_next_je_number()?;
        let now = now_str();
        let date_str = entry.entry_date.to_string();

        self.conn
            .execute(
                "INSERT INTO journal_entries
                    (je_number, entry_date, memo, status, is_reversed,
                     fiscal_period_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'Draft', 0, ?4, ?5, ?6)",
                params![
                    je_number,
                    date_str,
                    entry.memo,
                    i64::from(entry.fiscal_period_id),
                    now,
                    now,
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
                         source_entity_name, fiscal_period_id, created_at, updated_at
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
                    source_entity_name, fiscal_period_id, created_at, updated_at
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
}

// ── Row mappers ───────────────────────────────────────────────────────────────

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<JournalEntry> {
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
            lines: vec![line1.clone(), line2.clone()],
        })
        .expect("create 1");

        repo.create_draft(&NewJournalEntry {
            entry_date: d2,
            memo: None,
            fiscal_period_id: period_id,
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
                lines: make_lines(),
            })
            .expect("create draft 1");

        repo.create_draft(&NewJournalEntry {
            entry_date: date,
            memo: None,
            fiscal_period_id: period_id,
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
                lines: make_lines(),
            })
            .expect("create original");

        let reversal_id = repo
            .create_draft(&NewJournalEntry {
                entry_date: date,
                memo: Some("Reversal".to_string()),
                fiscal_period_id: period_id,
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
}
