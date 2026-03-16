use anyhow::{Context, Result, bail};
use chrono::{Datelike, NaiveDate};
use rusqlite::{Connection, params};

use crate::db::audit_repo::AuditRepo;
use crate::db::now_str;
use crate::types::{AuditAction, FiscalPeriodId, FiscalYearId};

/// A fiscal period row loaded from the database.
#[derive(Debug, Clone, PartialEq)]
pub struct FiscalPeriod {
    pub id: FiscalPeriodId,
    pub fiscal_year_id: FiscalYearId,
    pub period_number: i32,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub is_closed: bool,
}

/// Repository for fiscal years and periods.
pub struct FiscalRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> FiscalRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    /// Creates a fiscal year starting on `start_month`/1 of `year` and all 12 monthly periods.
    /// Returns the new fiscal year's ID.
    pub fn create_fiscal_year(&self, start_month: u32, year: i32) -> Result<FiscalYearId> {
        if !(1..=12).contains(&start_month) {
            bail!("start_month must be 1–12, got {start_month}");
        }

        let fy_start = NaiveDate::from_ymd_opt(year, start_month, 1)
            .ok_or_else(|| anyhow::anyhow!("Invalid fiscal year start date"))?;

        // The fiscal year ends on the last day of the 12th month.
        let fy_end = month_end(advance_months(fy_start, 11));

        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();

        self.conn.execute(
            "INSERT INTO fiscal_years (start_date, end_date, is_closed, created_at)
             VALUES (?1, ?2, 0, ?3)",
            params![fy_start.to_string(), fy_end.to_string(), now],
        )?;
        let fy_id = FiscalYearId::from(self.conn.last_insert_rowid());

        // Create the 12 monthly periods.
        for period_num in 1_u32..=12 {
            let period_start = advance_months(fy_start, period_num - 1);
            let period_end = month_end(period_start);
            self.conn.execute(
                "INSERT INTO fiscal_periods
                    (fiscal_year_id, period_number, start_date, end_date, is_closed, created_at)
                 VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                params![
                    i64::from(fy_id),
                    period_num,
                    period_start.to_string(),
                    period_end.to_string(),
                    now
                ],
            )?;
        }

        Ok(fy_id)
    }

    /// Returns the fiscal period that contains `date`, or an error if none is found.
    pub fn get_period_for_date(&self, date: NaiveDate) -> Result<FiscalPeriod> {
        let date_str = date.to_string();
        let row = self
            .conn
            .query_row(
                "SELECT id, fiscal_year_id, period_number, start_date, end_date, is_closed
                 FROM fiscal_periods
                 WHERE start_date <= ?1 AND end_date >= ?1
                 LIMIT 1",
                params![date_str],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i32>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i32>(5)?,
                    ))
                },
            )
            .map_err(|e| anyhow::anyhow!("No fiscal period found for date {date}: {e}"))?;

        Ok(FiscalPeriod {
            id: FiscalPeriodId::from(row.0),
            fiscal_year_id: FiscalYearId::from(row.1),
            period_number: row.2,
            start_date: NaiveDate::parse_from_str(&row.3, "%Y-%m-%d")?,
            end_date: NaiveDate::parse_from_str(&row.4, "%Y-%m-%d")?,
            is_closed: row.5 != 0,
        })
    }

    /// Returns the fiscal period with the given ID, or an error if not found.
    pub fn get_period_by_id(&self, id: FiscalPeriodId) -> Result<FiscalPeriod> {
        let row = self
            .conn
            .query_row(
                "SELECT id, fiscal_year_id, period_number, start_date, end_date, is_closed
                 FROM fiscal_periods WHERE id = ?1",
                params![i64::from(id)],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i32>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i32>(5)?,
                    ))
                },
            )
            .map_err(|e| anyhow::anyhow!("Fiscal period {} not found: {e}", i64::from(id)))?;

        Ok(FiscalPeriod {
            id: FiscalPeriodId::from(row.0),
            fiscal_year_id: FiscalYearId::from(row.1),
            period_number: row.2,
            start_date: NaiveDate::parse_from_str(&row.3, "%Y-%m-%d")?,
            end_date: NaiveDate::parse_from_str(&row.4, "%Y-%m-%d")?,
            is_closed: row.5 != 0,
        })
    }

    /// Returns `true` if the fiscal period is open (not closed), `false` if closed.
    /// Returns an error if the period ID does not exist.
    pub fn is_period_open(&self, id: FiscalPeriodId) -> Result<bool> {
        let is_closed: i32 = self
            .conn
            .query_row(
                "SELECT is_closed FROM fiscal_periods WHERE id = ?1",
                params![i64::from(id)],
                |row| row.get(0),
            )
            .with_context(|| format!("Fiscal period {} not found", i64::from(id)))?;
        Ok(is_closed == 0)
    }

    /// Closes a fiscal period.
    ///
    /// - Rejects if there are any Draft journal entries in the period.
    /// - Sets `is_closed = 1` and `closed_at = now`.
    /// - Writes an audit log entry.
    pub fn close_period(&self, id: FiscalPeriodId, entity_name: &str) -> Result<()> {
        let period = self.get_period_by_id(id)?;
        if period.is_closed {
            bail!("Fiscal period {} is already closed", i64::from(id));
        }

        // Reject if any Draft JEs exist in this period.
        let draft_count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM journal_entries
                 WHERE fiscal_period_id = ?1 AND status = 'Draft'",
                params![i64::from(id)],
                |row| row.get(0),
            )
            .context("Failed to count draft entries in period")?;

        if draft_count > 0 {
            bail!(
                "Cannot close fiscal period {}: {} draft journal entr{} must be posted or deleted first",
                i64::from(id),
                draft_count,
                if draft_count == 1 { "y" } else { "ies" }
            );
        }

        let now = now_str();
        self.conn
            .execute(
                "UPDATE fiscal_periods SET is_closed = 1, closed_at = ?2 WHERE id = ?1",
                params![i64::from(id), now],
            )
            .context("Failed to close fiscal period")?;

        AuditRepo::new(self.conn).append(
            AuditAction::PeriodClosed,
            entity_name,
            Some("FiscalPeriod"),
            Some(i64::from(id)),
            &format!(
                "Period {} ({} – {}) closed",
                period.period_number, period.start_date, period.end_date
            ),
        )?;

        Ok(())
    }

    /// Reopens a closed fiscal period.
    ///
    /// - Rejects if the period is already open.
    /// - Sets `is_closed = 0` and `reopened_at = now`.
    /// - Writes an audit log entry.
    pub fn reopen_period(&self, id: FiscalPeriodId, entity_name: &str) -> Result<()> {
        let period = self.get_period_by_id(id)?;
        if !period.is_closed {
            bail!("Fiscal period {} is already open", i64::from(id));
        }

        let now = now_str();
        self.conn
            .execute(
                "UPDATE fiscal_periods SET is_closed = 0, reopened_at = ?2 WHERE id = ?1",
                params![i64::from(id), now],
            )
            .context("Failed to reopen fiscal period")?;

        AuditRepo::new(self.conn).append(
            AuditAction::PeriodReopened,
            entity_name,
            Some("FiscalPeriod"),
            Some(i64::from(id)),
            &format!(
                "Period {} ({} – {}) reopened",
                period.period_number, period.start_date, period.end_date
            ),
        )?;

        Ok(())
    }

    /// Returns all open (not closed) fiscal periods across all fiscal years,
    /// ordered by start_date ascending.
    pub fn get_open_periods(&self) -> Result<Vec<FiscalPeriod>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, fiscal_year_id, period_number, start_date, end_date, is_closed
             FROM fiscal_periods
             WHERE is_closed = 0
             ORDER BY start_date ASC",
        )?;
        let periods = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i32>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i32>(5)?,
                ))
            })?
            .map(|r| {
                r.map_err(anyhow::Error::from).and_then(|row| {
                    Ok(FiscalPeriod {
                        id: FiscalPeriodId::from(row.0),
                        fiscal_year_id: FiscalYearId::from(row.1),
                        period_number: row.2,
                        start_date: NaiveDate::parse_from_str(&row.3, "%Y-%m-%d")?,
                        end_date: NaiveDate::parse_from_str(&row.4, "%Y-%m-%d")?,
                        is_closed: row.5 != 0,
                    })
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(periods)
    }

    /// Returns all 12 periods for the given fiscal year, ordered by period number.
    pub fn list_periods(&self, fiscal_year_id: FiscalYearId) -> Result<Vec<FiscalPeriod>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, fiscal_year_id, period_number, start_date, end_date, is_closed
             FROM fiscal_periods
             WHERE fiscal_year_id = ?1
             ORDER BY period_number",
        )?;
        let periods = stmt
            .query_map(params![i64::from(fiscal_year_id)], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i32>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i32>(5)?,
                ))
            })?
            .map(|r| {
                r.map_err(anyhow::Error::from).and_then(|row| {
                    Ok(FiscalPeriod {
                        id: FiscalPeriodId::from(row.0),
                        fiscal_year_id: FiscalYearId::from(row.1),
                        period_number: row.2,
                        start_date: NaiveDate::parse_from_str(&row.3, "%Y-%m-%d")?,
                        end_date: NaiveDate::parse_from_str(&row.4, "%Y-%m-%d")?,
                        is_closed: row.5 != 0,
                    })
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(periods)
    }
}

/// Returns the date advanced by `months` calendar months (always day 1 of that month).
fn advance_months(date: NaiveDate, months: u32) -> NaiveDate {
    let total_months = (date.month0() + months) as i32;
    let year_offset = total_months / 12;
    let new_month = (total_months % 12) as u32 + 1;
    let new_year = date.year() + year_offset;
    NaiveDate::from_ymd_opt(new_year, new_month, 1)
        .expect("advance_months: constructed date is always valid (day 1)")
}

/// Returns the last day of the month containing `date`.
fn month_end(date: NaiveDate) -> NaiveDate {
    // First day of next month minus one day.
    let next_month = advance_months(date, 1);
    next_month
        .pred_opt()
        .expect("month_end: always has a predecessor")
}

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

    #[test]
    fn creates_12_periods_for_january_start() {
        let conn = db();
        let repo = FiscalRepo::new(&conn);
        let fy_id = repo.create_fiscal_year(1, 2025).expect("create failed");

        let periods = repo.list_periods(fy_id).expect("list failed");
        assert_eq!(periods.len(), 12);

        // Period 1 starts Jan 1, ends Jan 31
        assert_eq!(periods[0].period_number, 1);
        assert_eq!(periods[0].start_date.to_string(), "2025-01-01");
        assert_eq!(periods[0].end_date.to_string(), "2025-01-31");

        // Period 3 (March) starts Mar 1, ends Mar 31
        assert_eq!(periods[2].period_number, 3);
        assert_eq!(periods[2].start_date.to_string(), "2025-03-01");
        assert_eq!(periods[2].end_date.to_string(), "2025-03-31");

        // Period 12 ends Dec 31
        assert_eq!(periods[11].period_number, 12);
        assert_eq!(periods[11].end_date.to_string(), "2025-12-31");
    }

    #[test]
    fn get_period_for_date_returns_period_3() {
        let conn = db();
        let repo = FiscalRepo::new(&conn);
        let _ = repo.create_fiscal_year(1, 2025).expect("create failed");

        let date = NaiveDate::from_ymd_opt(2025, 3, 15).unwrap();
        let period = repo.get_period_for_date(date).expect("not found");
        assert_eq!(period.period_number, 3);
        assert_eq!(period.start_date.to_string(), "2025-03-01");
        assert_eq!(period.end_date.to_string(), "2025-03-31");
    }

    #[test]
    fn periods_are_contiguous_no_gaps() {
        let conn = db();
        let repo = FiscalRepo::new(&conn);
        let fy_id = repo.create_fiscal_year(1, 2025).expect("create failed");
        let periods = repo.list_periods(fy_id).expect("list failed");

        for window in periods.windows(2) {
            let prev_end = window[0].end_date;
            let next_start = window[1].start_date;
            assert_eq!(
                prev_end.succ_opt().unwrap(),
                next_start,
                "Gap between period {} and {}",
                window[0].period_number,
                window[1].period_number
            );
        }
    }

    #[test]
    fn fiscal_year_not_starting_january() {
        let conn = db();
        let repo = FiscalRepo::new(&conn);
        // Fiscal year starts April 2025 → ends March 2026
        let fy_id = repo.create_fiscal_year(4, 2025).expect("create failed");
        let periods = repo.list_periods(fy_id).expect("list failed");
        assert_eq!(periods.len(), 12);
        assert_eq!(periods[0].start_date.to_string(), "2025-04-01");
        assert_eq!(periods[0].end_date.to_string(), "2025-04-30");
        assert_eq!(periods[11].end_date.to_string(), "2026-03-31");
    }

    #[test]
    fn invalid_start_month_returns_error() {
        let conn = db();
        let repo = FiscalRepo::new(&conn);
        assert!(repo.create_fiscal_year(0, 2025).is_err());
        assert!(repo.create_fiscal_year(13, 2025).is_err());
    }

    #[test]
    fn february_end_is_correct() {
        let conn = db();
        let repo = FiscalRepo::new(&conn);
        let fy_id = repo.create_fiscal_year(1, 2025).expect("create");
        let periods = repo.list_periods(fy_id).expect("list");
        // Feb 2025 (non-leap year) ends on the 28th
        assert_eq!(periods[1].end_date.to_string(), "2025-02-28");
    }

    // ── close_period / reopen_period ─────────────────────────────────────────

    #[test]
    fn close_period_sets_is_closed() {
        let conn = db();
        let repo = FiscalRepo::new(&conn);
        let fy_id = repo.create_fiscal_year(1, 2026).expect("create FY");
        let periods = repo.list_periods(fy_id).expect("list");
        let jan = periods[0].id;

        assert!(!periods[0].is_closed, "should start open");
        repo.close_period(jan, "Test Entity").expect("close failed");

        assert!(
            !repo.is_period_open(jan).expect("is_period_open"),
            "should be closed"
        );
    }

    #[test]
    fn close_period_rejects_if_already_closed() {
        let conn = db();
        let repo = FiscalRepo::new(&conn);
        let fy_id = repo.create_fiscal_year(1, 2026).expect("create FY");
        let periods = repo.list_periods(fy_id).expect("list");
        let jan = periods[0].id;

        repo.close_period(jan, "Test Entity").expect("first close");
        let result = repo.close_period(jan, "Test Entity");
        assert!(result.is_err(), "double-close should fail");
    }

    #[test]
    fn close_period_rejects_draft_entries() {
        use crate::db::account_repo::AccountRepo;
        use crate::db::journal_repo::{JournalRepo, NewJournalEntry, NewJournalEntryLine};
        use crate::db::schema::seed_default_accounts;

        let conn = db();
        seed_default_accounts(&conn).expect("seed");
        let repo = FiscalRepo::new(&conn);
        let fy_id = repo.create_fiscal_year(1, 2026).expect("create FY");
        let periods = repo.list_periods(fy_id).expect("list");
        let jan = periods[0].id;

        // Get two non-placeholder accounts for a JE.
        let accounts = AccountRepo::new(&conn);
        let all = accounts.list_active().expect("list");
        let non_ph: Vec<_> = all.iter().filter(|a| !a.is_placeholder).collect();
        let (a1, a2) = (non_ph[0].id, non_ph[1].id);

        // Create a Draft JE in January.
        JournalRepo::new(&conn)
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                memo: None,
                fiscal_period_id: jan,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: a1,
                        debit_amount: crate::types::Money(10_000_000_000),
                        credit_amount: crate::types::Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: a2,
                        debit_amount: crate::types::Money(0),
                        credit_amount: crate::types::Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create draft");

        let result = repo.close_period(jan, "Test Entity");
        assert!(result.is_err(), "close with drafts should fail");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("draft") || msg.contains("Draft"),
            "error should mention drafts: {msg}"
        );
    }

    #[test]
    fn reopen_period_clears_is_closed() {
        let conn = db();
        let repo = FiscalRepo::new(&conn);
        let fy_id = repo.create_fiscal_year(1, 2026).expect("create FY");
        let periods = repo.list_periods(fy_id).expect("list");
        let jan = periods[0].id;

        repo.close_period(jan, "Test Entity").expect("close");
        assert!(!repo.is_period_open(jan).expect("open check"), "closed");

        repo.reopen_period(jan, "Test Entity").expect("reopen");
        assert!(
            repo.is_period_open(jan).expect("open check"),
            "should be open again"
        );
    }

    #[test]
    fn reopen_period_rejects_if_already_open() {
        let conn = db();
        let repo = FiscalRepo::new(&conn);
        let fy_id = repo.create_fiscal_year(1, 2026).expect("create FY");
        let periods = repo.list_periods(fy_id).expect("list");
        let jan = periods[0].id;

        let result = repo.reopen_period(jan, "Test Entity");
        assert!(result.is_err(), "reopen of open period should fail");
    }

    #[test]
    fn get_open_periods_returns_only_open() {
        let conn = db();
        let repo = FiscalRepo::new(&conn);
        let fy_id = repo.create_fiscal_year(1, 2026).expect("create FY");
        let periods = repo.list_periods(fy_id).expect("list");

        // Close January.
        repo.close_period(periods[0].id, "Test Entity")
            .expect("close jan");

        let open = repo.get_open_periods().expect("get_open_periods");
        // 11 periods remain open.
        assert_eq!(open.len(), 11);
        assert!(
            open.iter().all(|p| !p.is_closed),
            "all returned periods must be open"
        );
        assert!(
            !open.iter().any(|p| p.id == periods[0].id),
            "January should not appear in open periods"
        );
    }

    #[test]
    fn close_period_writes_audit_entry() {
        use crate::db::audit_repo::{AuditFilter, AuditRepo};
        use crate::types::AuditAction;

        let conn = db();
        let repo = FiscalRepo::new(&conn);
        let fy_id = repo.create_fiscal_year(1, 2026).expect("create FY");
        let periods = repo.list_periods(fy_id).expect("list");
        let jan = periods[0].id;

        repo.close_period(jan, "Acme LLC").expect("close");

        let entries = AuditRepo::new(&conn)
            .list(&AuditFilter {
                action_type: Some(AuditAction::PeriodClosed),
                ..Default::default()
            })
            .expect("list audit");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entity_name, "Acme LLC");
    }

    #[test]
    fn reopen_period_writes_audit_entry() {
        use crate::db::audit_repo::{AuditFilter, AuditRepo};
        use crate::types::AuditAction;

        let conn = db();
        let repo = FiscalRepo::new(&conn);
        let fy_id = repo.create_fiscal_year(1, 2026).expect("create FY");
        let periods = repo.list_periods(fy_id).expect("list");
        let jan = periods[0].id;

        repo.close_period(jan, "Test Co").expect("close");
        repo.reopen_period(jan, "Test Co").expect("reopen");

        let entries = AuditRepo::new(&conn)
            .list(&AuditFilter {
                action_type: Some(AuditAction::PeriodReopened),
                ..Default::default()
            })
            .expect("list audit");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entity_name, "Test Co");
    }
}
