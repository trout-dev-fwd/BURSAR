//! Recurring journal entry templates.
//!
//! Each template references a posted JE whose line items are copied on generation.
//! Generation creates a Draft JE dated at `next_due_date` and advances the schedule.

use anyhow::{Context, Result, bail};
use chrono::{Datelike, NaiveDate};
use rusqlite::{Connection, params};

use super::now_str;
use crate::db::journal_repo::{NewJournalEntry, NewJournalEntryLine};
use crate::types::{EntryFrequency, JournalEntryId, Money, RecurringTemplateId};

// ── Data structs ──────────────────────────────────────────────────────────────

/// A loaded recurring entry template row.
#[derive(Debug, Clone, PartialEq)]
pub struct RecurringTemplate {
    pub id: RecurringTemplateId,
    pub source_je_id: JournalEntryId,
    pub frequency: EntryFrequency,
    pub next_due_date: NaiveDate,
    pub is_active: bool,
    pub last_generated_date: Option<NaiveDate>,
}

// ── Repo ──────────────────────────────────────────────────────────────────────

pub struct RecurringRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> RecurringRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    /// Creates a new recurring template from a posted JE.
    ///
    /// `start_date` is both the first `next_due_date` and the logical start of the schedule.
    pub fn create_template(
        &self,
        source_je_id: JournalEntryId,
        frequency: EntryFrequency,
        start_date: NaiveDate,
    ) -> Result<RecurringTemplateId> {
        // Verify the source JE exists and is Posted.
        let status: String = self
            .conn
            .query_row(
                "SELECT status FROM journal_entries WHERE id = ?1",
                params![i64::from(source_je_id)],
                |row| row.get(0),
            )
            .with_context(|| format!("Source JE {source_je_id:?} not found"))?;
        if status != "Posted" {
            bail!("Source JE must be Posted to create a recurring template (got {status})");
        }

        let now = now_str();
        self.conn.execute(
            "INSERT INTO recurring_entry_templates
                 (source_je_id, frequency, next_due_date, is_active, last_generated_date,
                  created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, NULL, ?4, ?5)",
            params![
                i64::from(source_je_id),
                frequency.to_string(),
                start_date.format("%Y-%m-%d").to_string(),
                now,
                now,
            ],
        )?;
        Ok(RecurringTemplateId::from(self.conn.last_insert_rowid()))
    }

    /// Returns all active templates ordered by next_due_date ascending.
    pub fn list_upcoming(&self) -> Result<Vec<RecurringTemplate>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source_je_id, frequency, next_due_date, is_active, last_generated_date
             FROM recurring_entry_templates
             WHERE is_active = 1
             ORDER BY next_due_date ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        rows.map(|r| {
            let (id, source_je_id, frequency_str, next_due_str, is_active, last_gen_str) = r?;
            let frequency = frequency_str
                .parse::<EntryFrequency>()
                .map_err(|e| rusqlite::Error::InvalidColumnName(e.to_string()))?;
            let next_due_date = NaiveDate::parse_from_str(&next_due_str, "%Y-%m-%d")
                .map_err(|e| rusqlite::Error::InvalidColumnName(e.to_string()))?;
            let last_generated_date = last_gen_str
                .map(|s| {
                    NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                        .map_err(|e| rusqlite::Error::InvalidColumnName(e.to_string()))
                })
                .transpose()?;
            Ok(RecurringTemplate {
                id: RecurringTemplateId::from(id),
                source_je_id: JournalEntryId::from(source_je_id),
                frequency,
                next_due_date,
                is_active,
                last_generated_date,
            })
        })
        .collect()
    }

    /// Generates Draft JEs for all active templates whose `next_due_date <= as_of`.
    ///
    /// For each qualifying template:
    /// 1. Looks up the fiscal period for `next_due_date`.
    /// 2. Copies line items from the source JE.
    /// 3. Creates a Draft JE dated at `next_due_date`.
    /// 4. Advances `next_due_date` by the template's frequency.
    /// 5. Updates `last_generated_date`.
    ///
    /// Entries are always created as Draft — never auto-posted.
    /// Returns the list of generated JE IDs.
    pub fn generate_entries(&self, as_of: NaiveDate) -> Result<Vec<JournalEntryId>> {
        let templates = self.list_upcoming()?;
        let due: Vec<RecurringTemplate> = templates
            .into_iter()
            .filter(|t| t.next_due_date <= as_of)
            .collect();

        let mut generated = Vec::new();
        for template in due {
            let je_id = self.generate_one(&template)?;
            generated.push(je_id);
        }
        Ok(generated)
    }

    /// Deactivates a recurring template so it no longer generates entries.
    pub fn deactivate(&self, id: RecurringTemplateId) -> Result<()> {
        let now = now_str();
        let changed = self.conn.execute(
            "UPDATE recurring_entry_templates SET is_active = 0, updated_at = ?1 WHERE id = ?2",
            params![now, i64::from(id)],
        )?;
        if changed == 0 {
            bail!("Recurring template {id:?} not found");
        }
        Ok(())
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Generates one Draft JE from a template and advances its schedule.
    fn generate_one(&self, template: &RecurringTemplate) -> Result<JournalEntryId> {
        let date = template.next_due_date;

        // Find the fiscal period for this date.
        let period_id: i64 = self
            .conn
            .query_row(
                "SELECT id FROM fiscal_periods
                 WHERE start_date <= ?1 AND end_date >= ?1
                 LIMIT 1",
                params![date.format("%Y-%m-%d").to_string()],
                |row| row.get(0),
            )
            .with_context(|| format!("No fiscal period found for date {date}"))?;

        // Load source JE memo and line items.
        let memo: Option<String> = self
            .conn
            .query_row(
                "SELECT memo FROM journal_entries WHERE id = ?1",
                params![i64::from(template.source_je_id)],
                |row| row.get(0),
            )
            .with_context(|| "Source JE not found during generation")?;

        let mut line_stmt = self.conn.prepare(
            "SELECT account_id, debit_amount, credit_amount, line_memo, sort_order
             FROM journal_entry_lines
             WHERE journal_entry_id = ?1
             ORDER BY sort_order ASC",
        )?;
        let lines: Vec<NewJournalEntryLine> = line_stmt
            .query_map(params![i64::from(template.source_je_id)], |row| {
                Ok(NewJournalEntryLine {
                    account_id: crate::types::AccountId::from(row.get::<_, i64>(0)?),
                    debit_amount: Money(row.get::<_, i64>(1)?),
                    credit_amount: Money(row.get::<_, i64>(2)?),
                    line_memo: row.get(3)?,
                    sort_order: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        // Create the Draft JE.
        let new_je = NewJournalEntry {
            entry_date: date,
            memo,
            fiscal_period_id: crate::types::FiscalPeriodId::from(period_id),
            reversal_of_je_id: None,
            lines,
        };

        let je_repo = crate::db::journal_repo::JournalRepo::new(self.conn);
        let new_je_id = je_repo.create_draft(&new_je)?;

        // Advance the schedule.
        let next = advance_date(date, template.frequency);
        let now = now_str();
        self.conn.execute(
            "UPDATE recurring_entry_templates
             SET next_due_date = ?1, last_generated_date = ?2, updated_at = ?3
             WHERE id = ?4",
            params![
                next.format("%Y-%m-%d").to_string(),
                date.format("%Y-%m-%d").to_string(),
                now,
                i64::from(template.id),
            ],
        )?;

        Ok(new_je_id)
    }
}

/// Advances a date by one frequency period.
fn advance_date(date: NaiveDate, frequency: EntryFrequency) -> NaiveDate {
    match frequency {
        EntryFrequency::Monthly => {
            // Advance by 1 month, clamping to the last day of the target month.
            let month = date.month();
            let year = date.year();
            let (new_year, new_month) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            let last_day = days_in_month(new_year, new_month);
            NaiveDate::from_ymd_opt(new_year, new_month, date.day().min(last_day))
                .expect("advance_date monthly: valid date")
        }
        EntryFrequency::Quarterly => {
            // Advance by 3 months.
            let total_months = date.year() * 12 + (date.month() as i32 - 1) + 3;
            let new_year = total_months / 12;
            let new_month = (total_months % 12 + 1) as u32;
            let last_day = days_in_month(new_year, new_month);
            NaiveDate::from_ymd_opt(new_year, new_month, date.day().min(last_day))
                .expect("advance_date quarterly: valid date")
        }
        EntryFrequency::Annually => {
            // Same day next year (handle Feb 29 → Feb 28 on non-leap years).
            let new_year = date.year() + 1;
            let last_day = days_in_month(new_year, date.month());
            NaiveDate::from_ymd_opt(new_year, date.month(), date.day().min(last_day))
                .expect("advance_date annually: valid date")
        }
    }
}

/// Returns the number of days in a given month.
fn days_in_month(year: i32, month: u32) -> u32 {
    // The first day of the next month minus one day.
    let (y, m) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(y, m, 1)
        .expect("days_in_month: valid")
        .pred_opt()
        .expect("days_in_month: pred")
        .day()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::account_repo::NewAccount;
    use crate::db::entity_db_from_conn;
    use crate::db::journal_repo::{NewJournalEntry, NewJournalEntryLine};
    use crate::db::schema::initialize_schema;
    use crate::services::journal::post_journal_entry;
    use crate::types::{AccountId, AccountType, FiscalPeriodId};
    use rusqlite::Connection;

    fn make_db() -> (crate::db::EntityDb, FiscalPeriodId) {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema");
        let db = entity_db_from_conn(conn);
        let fy = db.fiscal().create_fiscal_year(1, 2026).expect("fy");
        let periods = db.fiscal().list_periods(fy).expect("periods");
        (db, periods[0].id)
    }

    fn create_account(
        db: &crate::db::EntityDb,
        number: &str,
        name: &str,
        account_type: AccountType,
    ) -> AccountId {
        db.accounts()
            .create(&NewAccount {
                number: number.to_owned(),
                name: name.to_owned(),
                account_type,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create account")
    }

    fn post_je(
        db: &crate::db::EntityDb,
        period_id: FiscalPeriodId,
        date: NaiveDate,
        debit_id: AccountId,
        credit_id: AccountId,
        amount: Money,
    ) -> JournalEntryId {
        let je_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: date,
                memo: Some("Test recurring".to_owned()),
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: debit_id,
                        debit_amount: amount,
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: credit_id,
                        debit_amount: Money(0),
                        credit_amount: amount,
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create draft");
        post_journal_entry(db, je_id, "Test Entity").expect("post");
        je_id
    }

    #[test]
    fn create_template_requires_posted_je() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Cash", AccountType::Asset);
        let revenue = create_account(&db, "4100", "Revenue", AccountType::Revenue);
        // Create a Draft JE (not posted).
        let draft_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                memo: None,
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: cash,
                        debit_amount: Money::from_dollars(100.0),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: revenue,
                        debit_amount: Money(0),
                        credit_amount: Money::from_dollars(100.0),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("draft");
        let result = db.recurring().create_template(
            draft_id,
            EntryFrequency::Monthly,
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        );
        assert!(result.is_err(), "should reject draft JE");
    }

    #[test]
    fn create_template_succeeds_for_posted_je() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Cash", AccountType::Asset);
        let revenue = create_account(&db, "4100", "Revenue", AccountType::Revenue);
        let je_id = post_je(
            &db,
            period_id,
            NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            cash,
            revenue,
            Money::from_dollars(500.0),
        );
        let template_id = db
            .recurring()
            .create_template(
                je_id,
                EntryFrequency::Monthly,
                NaiveDate::from_ymd_opt(2026, 2, 15).unwrap(),
            )
            .expect("create template");
        // Verify it shows up in list_upcoming.
        let upcoming = db.recurring().list_upcoming().expect("list");
        assert_eq!(upcoming.len(), 1);
        assert_eq!(upcoming[0].id, template_id);
        assert_eq!(upcoming[0].source_je_id, je_id);
        assert_eq!(upcoming[0].frequency, EntryFrequency::Monthly);
    }

    #[test]
    fn generate_entries_creates_draft_je_and_advances_schedule() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Cash", AccountType::Asset);
        let revenue = create_account(&db, "4100", "Revenue", AccountType::Revenue);
        let je_id = post_je(
            &db,
            period_id,
            NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            cash,
            revenue,
            Money::from_dollars(200.0),
        );
        db.recurring()
            .create_template(
                je_id,
                EntryFrequency::Monthly,
                NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            )
            .expect("create template");

        // Generate as of 2026-01-31: template is due.
        let generated = db
            .recurring()
            .generate_entries(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap())
            .expect("generate");
        assert_eq!(generated.len(), 1);

        // Generated JE should be Draft.
        let (entry, lines) = db
            .journals()
            .get_with_lines(generated[0])
            .expect("get_with_lines");
        assert_eq!(entry.status, crate::types::JournalEntryStatus::Draft);
        assert_eq!(lines.len(), 2);

        // Template next_due_date should have advanced by one month.
        let upcoming = db.recurring().list_upcoming().expect("list");
        assert_eq!(upcoming.len(), 1);
        assert_eq!(
            upcoming[0].next_due_date,
            NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
        );
        assert_eq!(
            upcoming[0].last_generated_date,
            Some(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap())
        );
    }

    #[test]
    fn generate_entries_same_date_no_duplicates() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Cash", AccountType::Asset);
        let revenue = create_account(&db, "4100", "Revenue", AccountType::Revenue);
        let je_id = post_je(
            &db,
            period_id,
            NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            cash,
            revenue,
            Money::from_dollars(100.0),
        );
        db.recurring()
            .create_template(
                je_id,
                EntryFrequency::Monthly,
                NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            )
            .expect("create");
        // First generation.
        db.recurring()
            .generate_entries(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap())
            .expect("first generate");
        // Second generation on same date: next_due_date advanced, so nothing generated.
        let second = db
            .recurring()
            .generate_entries(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap())
            .expect("second generate");
        assert_eq!(second.len(), 0, "no duplicates on same date");
    }

    #[test]
    fn deactivate_stops_generation() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Cash", AccountType::Asset);
        let revenue = create_account(&db, "4100", "Revenue", AccountType::Revenue);
        let je_id = post_je(
            &db,
            period_id,
            NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            cash,
            revenue,
            Money::from_dollars(100.0),
        );
        let template_id = db
            .recurring()
            .create_template(
                je_id,
                EntryFrequency::Monthly,
                NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            )
            .expect("create");
        db.recurring().deactivate(template_id).expect("deactivate");

        // After deactivation, list_upcoming returns nothing.
        let upcoming = db.recurring().list_upcoming().expect("list");
        assert!(upcoming.is_empty());

        // generate_entries does nothing.
        let generated = db
            .recurring()
            .generate_entries(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap())
            .expect("generate");
        assert!(generated.is_empty());
    }

    #[test]
    fn advance_date_monthly_end_of_month() {
        // Jan 31 → Feb 28 (not Feb 31).
        let d = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let next = advance_date(d, EntryFrequency::Monthly);
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 2, 28).unwrap());
    }

    #[test]
    fn advance_date_quarterly() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let next = advance_date(d, EntryFrequency::Quarterly);
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 4, 15).unwrap());
    }

    #[test]
    fn advance_date_annually() {
        let d = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        let next = advance_date(d, EntryFrequency::Annually);
        assert_eq!(next, NaiveDate::from_ymd_opt(2027, 3, 15).unwrap());
    }
}
