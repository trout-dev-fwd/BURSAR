use anyhow::{Context, Result};
use chrono::NaiveDate;
use rusqlite::{Connection, params};

use super::now_str;
use crate::types::{JournalEntryId, Money, TaxFormTag, TaxReviewStatus};

// ── Data structs ──────────────────────────────────────────────────────────────

/// A tax tag row loaded from the database.
#[derive(Debug, Clone, PartialEq)]
pub struct TaxTag {
    pub id: i64,
    pub journal_entry_id: JournalEntryId,
    pub form_tag: Option<TaxFormTag>,
    pub status: TaxReviewStatus,
    pub ai_suggested_form: Option<TaxFormTag>,
    pub reason: Option<String>,
    pub reviewed_at: Option<String>,
}

/// A tax tag joined with key JE fields for list views and reports.
#[derive(Debug, Clone)]
pub struct TaxTagWithJe {
    pub tag: TaxTag,
    pub je_number: String,
    pub entry_date: NaiveDate,
    pub memo: Option<String>,
    /// Net amount: total debits for the JE (all lines summed).
    pub total_debits: Money,
}

// ── Repository ────────────────────────────────────────────────────────────────

pub struct TaxTagRepo<'conn> {
    conn: &'conn Connection,
}

impl<'conn> TaxTagRepo<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    /// Returns the tax tag for a journal entry, or `None` if not yet tagged.
    pub fn get_for_je(&self, je_id: JournalEntryId) -> Result<Option<TaxTag>> {
        let result = self.conn.query_row(
            "SELECT id, journal_entry_id, form_tag, status, ai_suggested_form, reason, reviewed_at
             FROM tax_tags WHERE journal_entry_id = ?1",
            params![i64::from(je_id)],
            row_to_tax_tag,
        );
        match result {
            Ok(tag) => Ok(Some(tag)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow::Error::from(e).context("Failed to fetch tax tag")),
        }
    }

    /// Sets a manual classification (user pressed `f`). Uses UPSERT so it works
    /// on any existing status — re-flagging always allowed.
    pub fn set_manual(
        &self,
        je_id: JournalEntryId,
        form_tag: TaxFormTag,
        reason: Option<&str>,
    ) -> Result<()> {
        let now = now_str();
        self.conn
            .execute(
                "INSERT INTO tax_tags (journal_entry_id, form_tag, status, reason, reviewed_at)
                 VALUES (?1, ?2, 'confirmed', ?3, ?4)
                 ON CONFLICT(journal_entry_id) DO UPDATE SET
                     form_tag = excluded.form_tag,
                     status = 'confirmed',
                     reason = excluded.reason,
                     reviewed_at = excluded.reviewed_at",
                params![i64::from(je_id), form_tag.to_string(), reason, now],
            )
            .context("Failed to set manual tax tag")?;
        Ok(())
    }

    /// Queues a JE for AI batch review (`a` key).
    pub fn set_ai_pending(&self, je_id: JournalEntryId) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO tax_tags (journal_entry_id, status)
                 VALUES (?1, 'ai_pending')
                 ON CONFLICT(journal_entry_id) DO UPDATE SET status = 'ai_pending'",
                params![i64::from(je_id)],
            )
            .context("Failed to set tax tag to ai_pending")?;
        Ok(())
    }

    /// Records an AI suggestion after batch review.
    pub fn set_ai_suggested(
        &self,
        je_id: JournalEntryId,
        form: TaxFormTag,
        reason: &str,
    ) -> Result<()> {
        let now = now_str();
        self.conn
            .execute(
                "INSERT INTO tax_tags (journal_entry_id, status, ai_suggested_form, reason, reviewed_at)
                 VALUES (?1, 'ai_suggested', ?2, ?3, ?4)
                 ON CONFLICT(journal_entry_id) DO UPDATE SET
                     status = 'ai_suggested',
                     ai_suggested_form = excluded.ai_suggested_form,
                     reason = excluded.reason,
                     reviewed_at = excluded.reviewed_at",
                params![i64::from(je_id), form.to_string(), reason, now],
            )
            .context("Failed to set AI suggested tax tag")?;
        Ok(())
    }

    /// Accepts the AI suggestion: copies `ai_suggested_form` → `form_tag`, sets status to confirmed.
    pub fn accept_suggestion(&self, je_id: JournalEntryId) -> Result<()> {
        let now = now_str();
        self.conn
            .execute(
                "UPDATE tax_tags
                 SET form_tag = ai_suggested_form, status = 'confirmed', reviewed_at = ?2
                 WHERE journal_entry_id = ?1 AND status = 'ai_suggested'",
                params![i64::from(je_id), now],
            )
            .context("Failed to accept AI suggestion")?;
        Ok(())
    }

    /// Marks a JE as non-deductible (`n` key). Uses UPSERT — works on any status.
    pub fn set_non_deductible(&self, je_id: JournalEntryId, reason: Option<&str>) -> Result<()> {
        let now = now_str();
        self.conn
            .execute(
                "INSERT INTO tax_tags (journal_entry_id, form_tag, status, reason, reviewed_at)
                 VALUES (?1, 'non_deductible', 'non_deductible', ?2, ?3)
                 ON CONFLICT(journal_entry_id) DO UPDATE SET
                     form_tag = 'non_deductible',
                     status = 'non_deductible',
                     reason = excluded.reason,
                     reviewed_at = excluded.reviewed_at",
                params![i64::from(je_id), reason, now],
            )
            .context("Failed to set non-deductible tax tag")?;
        Ok(())
    }

    /// Returns all JEs with `ai_pending` status (ready for batch review).
    pub fn get_pending(&self) -> Result<Vec<TaxTag>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, journal_entry_id, form_tag, status, ai_suggested_form, reason, reviewed_at
             FROM tax_tags WHERE status = 'ai_pending'
             ORDER BY journal_entry_id",
        )?;
        let tags = stmt
            .query_map([], row_to_tax_tag)?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect::<Result<Vec<_>>>()
            .context("Failed to list pending tax tags")?;
        Ok(tags)
    }

    /// Returns all tax tags joined with JE data for entries whose `entry_date` falls
    /// within `[start, end]`. Only posted JEs are included.
    pub fn list_for_date_range(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<TaxTagWithJe>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.journal_entry_id, t.form_tag, t.status, t.ai_suggested_form,
                    t.reason, t.reviewed_at,
                    j.je_number, j.entry_date, j.memo,
                    COALESCE((SELECT SUM(l.debit_amount) FROM journal_entry_lines l
                               WHERE l.journal_entry_id = j.id), 0) AS total_debits
             FROM tax_tags t
             JOIN journal_entries j ON j.id = t.journal_entry_id
             WHERE j.status = 'Posted'
               AND j.entry_date >= ?1
               AND j.entry_date <= ?2
             ORDER BY j.entry_date, j.je_number",
        )?;
        let rows = stmt
            .query_map(params![start.to_string(), end.to_string()], |row| {
                let tag = row_to_tax_tag(row)?;
                let je_number: String = row.get(7)?;
                let entry_date_str: String = row.get(8)?;
                let memo: Option<String> = row.get(9)?;
                let total_debits_raw: i64 = row.get(10)?;
                Ok((tag, je_number, entry_date_str, memo, total_debits_raw))
            })?
            .map(|r| r.map_err(anyhow::Error::from))
            .collect::<Result<Vec<_>>>()
            .context("Failed to list tax tags for date range")?;

        rows.into_iter()
            .map(|(tag, je_number, date_str, memo, debits_raw)| {
                let entry_date = date_str
                    .parse::<NaiveDate>()
                    .with_context(|| format!("Invalid date in journal_entries: {date_str}"))?;
                Ok(TaxTagWithJe {
                    tag,
                    je_number,
                    entry_date,
                    memo,
                    total_debits: Money(debits_raw),
                })
            })
            .collect()
    }
}

// ── Row mapping helper ────────────────────────────────────────────────────────

fn row_to_tax_tag(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaxTag> {
    let id: i64 = row.get(0)?;
    let je_id_raw: i64 = row.get(1)?;
    let form_tag_str: Option<String> = row.get(2)?;
    let status_str: String = row.get(3)?;
    let ai_form_str: Option<String> = row.get(4)?;
    let reason: Option<String> = row.get(5)?;
    let reviewed_at: Option<String> = row.get(6)?;

    let form_tag = form_tag_str
        .as_deref()
        .map(|s| {
            s.parse::<TaxFormTag>().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })
        })
        .transpose()?;

    let ai_suggested_form = ai_form_str
        .as_deref()
        .map(|s| {
            s.parse::<TaxFormTag>().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })
        })
        .transpose()?;

    let status = status_str.parse::<TaxReviewStatus>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;

    Ok(TaxTag {
        id,
        journal_entry_id: JournalEntryId::from(je_id_raw),
        form_tag,
        status,
        ai_suggested_form,
        reason,
        reviewed_at,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::fiscal_repo::FiscalRepo;
    use crate::db::journal_repo::{JournalRepo, NewJournalEntry, NewJournalEntryLine};
    use crate::db::schema::{initialize_schema, seed_default_accounts};
    use crate::types::{AccountId, FiscalPeriodId, JournalEntryStatus};
    use rusqlite::Connection;

    fn setup_db() -> (Connection, FiscalPeriodId) {
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
        let entry = NewJournalEntry {
            entry_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            memo: Some("Test entry".to_string()),
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

    fn post_je(conn: &Connection, je_id: JournalEntryId) {
        conn.execute(
            "UPDATE journal_entries SET status = 'Posted' WHERE id = ?1",
            params![i64::from(je_id)],
        )
        .expect("post je");
    }

    #[test]
    fn get_for_je_returns_none_when_untagged() {
        let (conn, period_id) = setup_db();
        let je_id = make_je(&conn, period_id);
        let repo = TaxTagRepo::new(&conn);
        assert!(repo.get_for_je(je_id).expect("get").is_none());
    }

    #[test]
    fn set_manual_creates_confirmed_tag() {
        let (conn, period_id) = setup_db();
        let je_id = make_je(&conn, period_id);
        let repo = TaxTagRepo::new(&conn);

        repo.set_manual(je_id, TaxFormTag::ScheduleC, Some("Office supplies"))
            .expect("set_manual");

        let tag = repo.get_for_je(je_id).expect("get").expect("should exist");
        assert_eq!(tag.status, TaxReviewStatus::Confirmed);
        assert_eq!(tag.form_tag, Some(TaxFormTag::ScheduleC));
        assert_eq!(tag.reason.as_deref(), Some("Office supplies"));
    }

    #[test]
    fn set_manual_without_reason() {
        let (conn, period_id) = setup_db();
        let je_id = make_je(&conn, period_id);
        let repo = TaxTagRepo::new(&conn);

        repo.set_manual(je_id, TaxFormTag::ScheduleD, None)
            .expect("set_manual");

        let tag = repo.get_for_je(je_id).expect("get").expect("should exist");
        assert_eq!(tag.form_tag, Some(TaxFormTag::ScheduleD));
        assert!(tag.reason.is_none());
    }

    #[test]
    fn set_manual_overwrites_existing_tag() {
        let (conn, period_id) = setup_db();
        let je_id = make_je(&conn, period_id);
        let repo = TaxTagRepo::new(&conn);

        repo.set_manual(je_id, TaxFormTag::ScheduleC, Some("First"))
            .expect("first");
        repo.set_manual(je_id, TaxFormTag::ScheduleE, Some("Second"))
            .expect("second");

        let tag = repo.get_for_je(je_id).expect("get").expect("should exist");
        assert_eq!(tag.form_tag, Some(TaxFormTag::ScheduleE));
        assert_eq!(tag.reason.as_deref(), Some("Second"));
    }

    #[test]
    fn set_ai_pending_and_retrieve() {
        let (conn, period_id) = setup_db();
        let je_id = make_je(&conn, period_id);
        let repo = TaxTagRepo::new(&conn);

        repo.set_ai_pending(je_id).expect("set_ai_pending");

        let tag = repo.get_for_je(je_id).expect("get").expect("should exist");
        assert_eq!(tag.status, TaxReviewStatus::AiPending);
    }

    #[test]
    fn get_pending_returns_only_ai_pending() {
        let (conn, period_id) = setup_db();
        let je_a = make_je(&conn, period_id);
        let je_b = make_je(&conn, period_id);
        let repo = TaxTagRepo::new(&conn);

        repo.set_ai_pending(je_a).expect("pending a");
        repo.set_manual(je_b, TaxFormTag::ScheduleC, None)
            .expect("confirmed b");

        let pending = repo.get_pending().expect("get_pending");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].journal_entry_id, je_a);
    }

    #[test]
    fn set_ai_suggested_and_accept() {
        let (conn, period_id) = setup_db();
        let je_id = make_je(&conn, period_id);
        let repo = TaxTagRepo::new(&conn);

        repo.set_ai_pending(je_id).expect("pending");
        repo.set_ai_suggested(je_id, TaxFormTag::Form4562, "Depreciation expense")
            .expect("ai_suggested");

        let tag = repo.get_for_je(je_id).expect("get").expect("exists");
        assert_eq!(tag.status, TaxReviewStatus::AiSuggested);
        assert_eq!(tag.ai_suggested_form, Some(TaxFormTag::Form4562));
        assert_eq!(tag.reason.as_deref(), Some("Depreciation expense"));

        repo.accept_suggestion(je_id).expect("accept");

        let tag = repo.get_for_je(je_id).expect("get").expect("exists");
        assert_eq!(tag.status, TaxReviewStatus::Confirmed);
        assert_eq!(tag.form_tag, Some(TaxFormTag::Form4562));
        assert_eq!(tag.ai_suggested_form, Some(TaxFormTag::Form4562));
    }

    #[test]
    fn set_non_deductible_creates_tag() {
        let (conn, period_id) = setup_db();
        let je_id = make_je(&conn, period_id);
        let repo = TaxTagRepo::new(&conn);

        repo.set_non_deductible(je_id, Some("Personal transfer"))
            .expect("set_non_deductible");

        let tag = repo.get_for_je(je_id).expect("get").expect("exists");
        assert_eq!(tag.status, TaxReviewStatus::NonDeductible);
        assert_eq!(tag.form_tag, Some(TaxFormTag::NonDeductible));
        assert_eq!(tag.reason.as_deref(), Some("Personal transfer"));
    }

    #[test]
    fn re_flagging_from_confirmed_to_non_deductible() {
        let (conn, period_id) = setup_db();
        let je_id = make_je(&conn, period_id);
        let repo = TaxTagRepo::new(&conn);

        repo.set_manual(je_id, TaxFormTag::ScheduleC, None)
            .expect("confirmed");
        repo.set_non_deductible(je_id, None).expect("re-flag");

        let tag = repo.get_for_je(je_id).expect("get").expect("exists");
        assert_eq!(tag.status, TaxReviewStatus::NonDeductible);
    }

    #[test]
    fn re_flagging_from_non_deductible_to_form() {
        let (conn, period_id) = setup_db();
        let je_id = make_je(&conn, period_id);
        let repo = TaxTagRepo::new(&conn);

        repo.set_non_deductible(je_id, None).expect("non_ded");
        repo.set_manual(je_id, TaxFormTag::ScheduleACharity, Some("Donation"))
            .expect("re-flag");

        let tag = repo.get_for_je(je_id).expect("get").expect("exists");
        assert_eq!(tag.status, TaxReviewStatus::Confirmed);
        assert_eq!(tag.form_tag, Some(TaxFormTag::ScheduleACharity));
    }

    #[test]
    fn re_flagging_from_ai_suggested() {
        let (conn, period_id) = setup_db();
        let je_id = make_je(&conn, period_id);
        let repo = TaxTagRepo::new(&conn);

        repo.set_ai_pending(je_id).expect("pending");
        repo.set_ai_suggested(je_id, TaxFormTag::ScheduleC, "AI reason")
            .expect("suggested");
        repo.set_manual(je_id, TaxFormTag::ScheduleE, Some("Override"))
            .expect("override");

        let tag = repo.get_for_je(je_id).expect("get").expect("exists");
        assert_eq!(tag.status, TaxReviewStatus::Confirmed);
        assert_eq!(tag.form_tag, Some(TaxFormTag::ScheduleE));
        assert_eq!(tag.reason.as_deref(), Some("Override"));
    }

    #[test]
    fn tax_form_tag_round_trip() {
        for tag in TaxFormTag::all() {
            let s = tag.to_string();
            let parsed: TaxFormTag = s.parse().expect("parse");
            assert_eq!(parsed, tag, "round-trip failed for {s}");
        }
    }

    #[test]
    fn tax_review_status_round_trip() {
        let statuses = [
            TaxReviewStatus::Unreviewed,
            TaxReviewStatus::AiPending,
            TaxReviewStatus::AiSuggested,
            TaxReviewStatus::Confirmed,
            TaxReviewStatus::NonDeductible,
        ];
        for status in &statuses {
            let s = status.to_string();
            let parsed: TaxReviewStatus = s.parse().expect("parse");
            assert_eq!(parsed, *status, "round-trip failed for {s}");
        }
    }

    #[test]
    fn list_for_date_range_returns_posted_jes_with_tags() {
        let (conn, period_id) = setup_db();
        let je_id = make_je(&conn, period_id);
        post_je(&conn, je_id);
        let repo = TaxTagRepo::new(&conn);

        repo.set_manual(je_id, TaxFormTag::ScheduleC, Some("Test"))
            .expect("tag");

        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let results = repo.list_for_date_range(start, end).expect("list");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tag.journal_entry_id, je_id);
        assert_eq!(results[0].tag.form_tag, Some(TaxFormTag::ScheduleC));
    }

    #[test]
    fn list_for_date_range_excludes_draft_jes() {
        let (conn, period_id) = setup_db();
        let je_id = make_je(&conn, period_id);
        // Not posted — stays Draft
        let repo = TaxTagRepo::new(&conn);
        repo.set_manual(je_id, TaxFormTag::ScheduleC, None)
            .expect("tag");

        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let results = repo.list_for_date_range(start, end).expect("list");
        assert!(
            results.is_empty(),
            "draft JEs should not appear in tax list"
        );
    }

    #[test]
    fn list_for_date_range_excludes_out_of_range() {
        let (conn, period_id) = setup_db();
        let je_id = make_je(&conn, period_id);
        post_je(&conn, je_id);
        let repo = TaxTagRepo::new(&conn);
        repo.set_manual(je_id, TaxFormTag::ScheduleC, None)
            .expect("tag");

        // Query February — JE is in January
        let start = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();
        let results = repo.list_for_date_range(start, end).expect("list");
        assert!(results.is_empty());
    }

    #[test]
    fn tax_form_tag_all_has_expected_count() {
        assert_eq!(TaxFormTag::all().len(), 14);
    }

    #[test]
    fn tax_form_tag_display_names_are_nonempty() {
        for tag in TaxFormTag::all() {
            assert!(
                !tag.display_name().is_empty(),
                "display_name empty for {tag}"
            );
            assert!(!tag.description().is_empty(), "description empty for {tag}");
        }
    }

    #[test]
    fn journal_entry_status_check() {
        // Verify JournalEntryStatus is accessible (used in list_for_date_range filter).
        let _ = JournalEntryStatus::Posted;
        let _ = JournalEntryStatus::Draft;
    }
}
