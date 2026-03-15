use anyhow::{Result, bail};
use chrono::{Datelike, NaiveDate};
use rusqlite::{Connection, params};

use crate::types::{FiscalPeriodId, FiscalYearId};

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
}
