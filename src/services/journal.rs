//! Orchestration functions for journal entry lifecycle transitions.
//!
//! This module sits above the repository layer: it calls multiple repos inside
//! SQLite transactions and enforces the business rules that span more than one table.
//!
//! **Data access only**: repositories in `src/db/` do not belong here.
//! **UI only**: tab code in `src/tabs/` does not belong here.

use anyhow::Result;
use chrono::NaiveDate;
use thiserror::Error;

use crate::db::EntityDb;
use crate::db::journal_repo::{NewJournalEntry, NewJournalEntryLine};
use crate::types::{AuditAction, JournalEntryId, JournalEntryStatus, Money};

// ── Domain errors ─────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("Journal entry {0} is not a Draft (current status does not allow posting)")]
    NotDraft(i64),

    #[error("Journal entry {0} is not Posted (only Posted entries can be reversed)")]
    NotPosted(i64),

    #[error("Entry does not balance: total debits {0}, total credits {1}")]
    Unbalanced(Money, Money),

    #[error("Journal entry must have at least 2 lines, got {0}")]
    TooFewLines(usize),

    #[error("Account {0} is inactive and cannot receive journal entry lines")]
    InactiveAccount(i64),

    #[error("Account {0} is a placeholder and cannot receive journal entry lines")]
    PlaceholderAccount(i64),

    #[error("Fiscal period {0} is closed")]
    PeriodClosed(i64),

    #[error("Journal entry {0} has already been reversed")]
    AlreadyReversed(i64),

    #[error("No fiscal period found for date {0}")]
    NoPeriodForDate(NaiveDate),
}

// ── post_journal_entry ────────────────────────────────────────────────────────

/// Posts a Draft journal entry, making it immutable and updating account balances.
///
/// Validations performed (all must pass):
/// - Status is `Draft`
/// - At least 2 lines
/// - `SUM(debit_amount) == SUM(credit_amount)` across all lines
/// - All referenced accounts are active and non-placeholder
/// - The fiscal period (`fiscal_period_id`) is open
///
/// On success: updates status to `Posted` and writes an audit log entry,
/// all within a single SQLite transaction.
///
/// `entity_name` is written to the audit log entry.
pub fn post_journal_entry(db: &EntityDb, je_id: JournalEntryId, entity_name: &str) -> Result<()> {
    let (entry, lines) = db.journals().get_with_lines(je_id)?;

    // Validate: must be Draft.
    if entry.status != JournalEntryStatus::Draft {
        return Err(JournalError::NotDraft(i64::from(je_id)).into());
    }

    // Validate: at least 2 lines.
    if lines.len() < 2 {
        return Err(JournalError::TooFewLines(lines.len()).into());
    }

    // Validate: balanced debits == credits.
    let total_debits: i64 = lines.iter().map(|l| l.debit_amount.0).sum();
    let total_credits: i64 = lines.iter().map(|l| l.credit_amount.0).sum();
    if total_debits != total_credits {
        return Err(JournalError::Unbalanced(Money(total_debits), Money(total_credits)).into());
    }

    // Validate: all accounts are active and non-placeholder.
    let accounts = db.accounts();
    for line in &lines {
        let acct = accounts.get_by_id(line.account_id)?;
        if !acct.is_active {
            return Err(JournalError::InactiveAccount(i64::from(line.account_id)).into());
        }
        if acct.is_placeholder {
            return Err(JournalError::PlaceholderAccount(i64::from(line.account_id)).into());
        }
    }

    // Validate: fiscal period is open.
    {
        let period_open: bool = db.conn().query_row(
            "SELECT is_closed FROM fiscal_periods WHERE id = ?1",
            rusqlite::params![i64::from(entry.fiscal_period_id)],
            |row| {
                let closed: i32 = row.get(0)?;
                Ok(closed == 0)
            },
        )?;
        if !period_open {
            return Err(JournalError::PeriodClosed(i64::from(entry.fiscal_period_id)).into());
        }
    }

    // Build audit description: summarise the first debit and credit lines.
    let description = build_post_description(&entry.je_number, &lines, db)?;

    // Execute the transition inside a single transaction.
    let tx = db.conn().unchecked_transaction()?;
    {
        use crate::db::audit_repo::AuditRepo;
        use crate::db::journal_repo::JournalRepo;

        JournalRepo::new(&tx).update_status(je_id, JournalEntryStatus::Posted)?;

        // TODO(Phase 4): Check for cash receipt and trigger envelope fills
        // If any debit line targets a Cash/Bank account (and this is not an
        // Owner's Draw entry), call envelope_repo.trigger_fills(je_id).

        AuditRepo::new(&tx).append(
            AuditAction::JournalEntryPosted,
            entity_name,
            Some("JournalEntry"),
            Some(i64::from(je_id)),
            &description,
        )?;
    }
    tx.commit()?;
    Ok(())
}

// ── reverse_journal_entry ─────────────────────────────────────────────────────

/// Creates a reversal of a Posted journal entry.
///
/// Validations:
/// - Status is `Posted`
/// - Not already reversed (`is_reversed == false`)
/// - The fiscal period containing `reversal_date` is open
///
/// On success: creates a new Posted JE with swapped debit/credit amounts,
/// marks the original as reversed, and writes an audit log entry — all in
/// a single SQLite transaction.
///
/// Returns the ID of the newly created reversal entry.
pub fn reverse_journal_entry(
    db: &EntityDb,
    je_id: JournalEntryId,
    reversal_date: NaiveDate,
    entity_name: &str,
) -> Result<JournalEntryId> {
    let (entry, lines) = db.journals().get_with_lines(je_id)?;

    // Validate: must be Posted.
    if entry.status != JournalEntryStatus::Posted {
        return Err(JournalError::NotPosted(i64::from(je_id)).into());
    }

    // Validate: not already reversed.
    if entry.is_reversed {
        return Err(JournalError::AlreadyReversed(i64::from(je_id)).into());
    }

    // Validate: fiscal period for reversal_date exists and is open.
    let reversal_period = db
        .fiscal()
        .get_period_for_date(reversal_date)
        .map_err(|_| JournalError::NoPeriodForDate(reversal_date))?;
    if reversal_period.is_closed {
        return Err(JournalError::PeriodClosed(i64::from(reversal_period.id)).into());
    }

    // Build the reversed lines (swap debit/credit amounts).
    let reversed_lines: Vec<NewJournalEntryLine> = lines
        .iter()
        .map(|l| NewJournalEntryLine {
            account_id: l.account_id,
            debit_amount: l.credit_amount,
            credit_amount: l.debit_amount,
            line_memo: l.line_memo.clone(),
            sort_order: l.sort_order,
        })
        .collect();

    let reversal_memo = format!(
        "Reversal of {}: {}",
        entry.je_number,
        entry.memo.as_deref().unwrap_or("")
    );

    let reversal_entry = NewJournalEntry {
        entry_date: reversal_date,
        memo: Some(reversal_memo),
        fiscal_period_id: reversal_period.id,
        reversal_of_je_id: Some(je_id),
        lines: reversed_lines,
    };

    let tx = db.conn().unchecked_transaction()?;
    let reversal_id = {
        use crate::db::audit_repo::AuditRepo;
        use crate::db::journal_repo::JournalRepo;

        let journal = JournalRepo::new(&tx);

        // Create the reversal as a draft, then immediately post it.
        let rev_id = journal.create_draft(&reversal_entry)?;
        journal.update_status(rev_id, JournalEntryStatus::Posted)?;

        // Mark the original as reversed.
        journal.mark_reversed(je_id, rev_id)?;

        let description = format!(
            "Reversed {}: created reversal {}",
            entry.je_number,
            journal.get_with_lines(rev_id)?.0.je_number,
        );

        AuditRepo::new(&tx).append(
            AuditAction::JournalEntryReversed,
            entity_name,
            Some("JournalEntry"),
            Some(i64::from(je_id)),
            &description,
        )?;

        rev_id
    };
    tx.commit()?;
    Ok(reversal_id)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Builds a human-readable description for the post audit log entry.
/// Shows at most the first debit and first credit line with account names.
fn build_post_description(
    je_number: &str,
    lines: &[crate::db::journal_repo::JournalEntryLine],
    db: &EntityDb,
) -> Result<String> {
    let accounts = db.accounts();
    let mut parts: Vec<String> = Vec::new();

    for line in lines {
        let acct = accounts.get_by_id(line.account_id)?;
        if !line.debit_amount.is_zero() {
            parts.push(format!("Dr {} {}", acct.name, line.debit_amount));
        } else if !line.credit_amount.is_zero() {
            parts.push(format!("Cr {} {}", acct.name, line.credit_amount));
        }
    }

    Ok(format!("Posted {}: {}", je_number, parts.join(", ")))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::EntityDb;
    use crate::db::journal_repo::{NewJournalEntry, NewJournalEntryLine};
    use crate::db::schema::{initialize_schema, seed_default_accounts};
    use crate::types::{FiscalPeriodId, JournalEntryStatus, Money};
    use rusqlite::Connection;

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn make_entity_db() -> EntityDb {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .expect("fk on");
        initialize_schema(&conn).expect("schema init");
        seed_default_accounts(&conn).expect("seed accounts");
        crate::db::entity_db_from_conn(conn)
    }

    fn setup_fiscal_year(db: &EntityDb) -> (FiscalPeriodId, FiscalPeriodId) {
        let fy_id = db.fiscal().create_fiscal_year(1, 2026).expect("create FY");
        let periods = db.fiscal().list_periods(fy_id).expect("list periods");
        // Return Jan 2026 and Feb 2026 period IDs.
        (periods[0].id, periods[1].id)
    }

    fn get_two_postable_accounts(db: &EntityDb) -> (i64, i64) {
        let all = db.accounts().list_active().expect("list active");
        let postable: Vec<_> = all.iter().filter(|a| !a.is_placeholder).collect();
        assert!(
            postable.len() >= 2,
            "Need at least 2 non-placeholder accounts"
        );
        (i64::from(postable[0].id), i64::from(postable[1].id))
    }

    fn make_balanced_entry(
        period_id: FiscalPeriodId,
        acct1: i64,
        acct2: i64,
        date: NaiveDate,
    ) -> NewJournalEntry {
        use crate::types::AccountId;
        NewJournalEntry {
            entry_date: date,
            memo: Some("Test JE".to_string()),
            fiscal_period_id: period_id,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: AccountId::from(acct1),
                    debit_amount: Money(10_000_000_000), // $100.00
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
        }
    }

    fn close_period(db: &EntityDb, period_id: FiscalPeriodId) {
        db.conn()
            .execute(
                "UPDATE fiscal_periods SET is_closed = 1 WHERE id = ?1",
                rusqlite::params![i64::from(period_id)],
            )
            .expect("close period");
    }

    // ── post_journal_entry ────────────────────────────────────────────────────

    #[test]
    fn post_valid_entry_changes_status_and_logs_audit() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);
        let (acct1, acct2) = get_two_postable_accounts(&db);
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        let entry = make_balanced_entry(jan_period, acct1, acct2, date);
        let je_id = db.journals().create_draft(&entry).expect("create_draft");

        post_journal_entry(&db, je_id, "Test Entity").expect("post");

        // Status should now be Posted.
        let (je, _) = db.journals().get_with_lines(je_id).expect("get");
        assert_eq!(je.status, JournalEntryStatus::Posted);

        // Audit log should have one JournalEntryPosted entry.
        use crate::db::audit_repo::{AuditFilter, AuditRepo};
        use crate::types::AuditAction;
        let audit = AuditRepo::new(db.conn());
        let entries = audit
            .list(&AuditFilter {
                action_type: Some(AuditAction::JournalEntryPosted),
                ..Default::default()
            })
            .expect("list audit");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].record_id, Some(i64::from(je_id)));
    }

    #[test]
    fn post_unbalanced_entry_returns_error() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);
        let (acct1, acct2) = get_two_postable_accounts(&db);
        use crate::types::AccountId;

        // Create an unbalanced entry: debit $100, credit $50.
        let entry = NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            memo: None,
            fiscal_period_id: jan_period,
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
                    credit_amount: Money(5_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };

        let je_id = db.journals().create_draft(&entry).expect("create_draft");
        let result = post_journal_entry(&db, je_id, "Test Entity");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("balance") || msg.contains("Unbalanced"),
            "Error message should mention balance: {msg}"
        );
    }

    #[test]
    fn post_to_placeholder_account_returns_error() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);

        // Find the placeholder "Assets" (1000) account.
        let all = db.accounts().list_all().expect("list all");
        let placeholder = all
            .iter()
            .find(|a| a.is_placeholder)
            .expect("placeholder account exists");
        let non_placeholder = all
            .iter()
            .find(|a| !a.is_placeholder && a.is_active)
            .expect("non-placeholder account");

        let entry = NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            memo: None,
            fiscal_period_id: jan_period,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: placeholder.id,
                    debit_amount: Money(10_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: non_placeholder.id,
                    debit_amount: Money(0),
                    credit_amount: Money(10_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };

        let je_id = db.journals().create_draft(&entry).expect("create");
        let result = post_journal_entry(&db, je_id, "Test Entity");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("placeholder"),
            "Error should mention placeholder: {msg}"
        );
    }

    #[test]
    fn post_to_inactive_account_returns_error() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);

        // Find two postable accounts; deactivate one.
        let (acct1_raw, acct2_raw) = get_two_postable_accounts(&db);
        use crate::types::AccountId;
        let acct1 = AccountId::from(acct1_raw);

        db.accounts().deactivate(acct1).expect("deactivate acct1");

        let entry = make_balanced_entry(
            jan_period,
            acct1_raw,
            acct2_raw,
            NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
        );
        let je_id = db.journals().create_draft(&entry).expect("create");
        let result = post_journal_entry(&db, je_id, "Test Entity");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("inactive") || msg.contains("Inactive"),
            "Error should mention inactive: {msg}"
        );
    }

    #[test]
    fn post_to_closed_period_returns_error() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);
        let (acct1, acct2) = get_two_postable_accounts(&db);

        let entry = make_balanced_entry(
            jan_period,
            acct1,
            acct2,
            NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
        );
        let je_id = db.journals().create_draft(&entry).expect("create");

        // Close the period.
        close_period(&db, jan_period);

        let result = post_journal_entry(&db, je_id, "Test Entity");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("closed") || msg.contains("Closed"),
            "Error should mention closed period: {msg}"
        );
    }

    #[test]
    fn post_draft_with_one_line_returns_error() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);
        let (acct1, _acct2) = get_two_postable_accounts(&db);
        use crate::types::AccountId;

        // Single line (debit only, which is also unbalanced — but TooFewLines triggers first).
        let entry = NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
            memo: None,
            fiscal_period_id: jan_period,
            reversal_of_je_id: None,
            lines: vec![NewJournalEntryLine {
                account_id: AccountId::from(acct1),
                debit_amount: Money(10_000_000_000),
                credit_amount: Money(0),
                line_memo: None,
                sort_order: 0,
            }],
        };

        let je_id = db.journals().create_draft(&entry).expect("create");
        let result = post_journal_entry(&db, je_id, "Test Entity");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("2 line") || msg.contains("line"),
            "Error should mention line count: {msg}"
        );
    }

    // ── reverse_journal_entry ─────────────────────────────────────────────────

    #[test]
    fn reverse_posted_entry_creates_mirror_and_marks_original() {
        let db = make_entity_db();
        let (jan_period, _feb_period) = setup_fiscal_year(&db);
        let (acct1, acct2) = get_two_postable_accounts(&db);

        let entry = make_balanced_entry(
            jan_period,
            acct1,
            acct2,
            NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
        );
        let je_id = db.journals().create_draft(&entry).expect("create");
        post_journal_entry(&db, je_id, "Test Entity").expect("post");

        let reversal_date = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let rev_id =
            reverse_journal_entry(&db, je_id, reversal_date, "Test Entity").expect("reverse");

        // Original should be marked reversed.
        let (original, orig_lines) = db.journals().get_with_lines(je_id).expect("get original");
        assert!(original.is_reversed, "Original should be marked reversed");
        assert_eq!(
            original.reversed_by_je_id,
            Some(rev_id),
            "reversed_by_je_id should point to reversal"
        );

        // Reversal entry should be Posted and link back to original.
        let (reversal, rev_lines) = db.journals().get_with_lines(rev_id).expect("get reversal");
        assert_eq!(reversal.status, JournalEntryStatus::Posted);
        assert_eq!(
            reversal.reversal_of_je_id,
            Some(je_id),
            "reversal_of_je_id should point to original"
        );
        assert_eq!(reversal.entry_date, reversal_date);
        assert!(
            reversal
                .memo
                .as_deref()
                .unwrap_or("")
                .contains("Reversal of"),
            "Memo should be prefixed with 'Reversal of'"
        );

        // Reversal lines should have swapped debit/credit.
        assert_eq!(rev_lines.len(), orig_lines.len());
        for (orig_line, rev_line) in orig_lines.iter().zip(rev_lines.iter()) {
            assert_eq!(
                rev_line.debit_amount, orig_line.credit_amount,
                "Reversal debit should equal original credit"
            );
            assert_eq!(
                rev_line.credit_amount, orig_line.debit_amount,
                "Reversal credit should equal original debit"
            );
        }

        // Audit log should have JournalEntryReversed.
        use crate::db::audit_repo::{AuditFilter, AuditRepo};
        use crate::types::AuditAction;
        let entries = AuditRepo::new(db.conn())
            .list(&AuditFilter {
                action_type: Some(AuditAction::JournalEntryReversed),
                ..Default::default()
            })
            .expect("list audit");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].record_id, Some(i64::from(je_id)));
    }

    #[test]
    fn reverse_already_reversed_entry_returns_error() {
        let db = make_entity_db();
        let (jan_period, _feb_period) = setup_fiscal_year(&db);
        let (acct1, acct2) = get_two_postable_accounts(&db);

        let entry = make_balanced_entry(
            jan_period,
            acct1,
            acct2,
            NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
        );
        let je_id = db.journals().create_draft(&entry).expect("create");
        post_journal_entry(&db, je_id, "Test Entity").expect("post");
        reverse_journal_entry(
            &db,
            je_id,
            NaiveDate::from_ymd_opt(2026, 1, 20).unwrap(),
            "Test Entity",
        )
        .expect("first reverse");

        // Try to reverse again.
        let result = reverse_journal_entry(
            &db,
            je_id,
            NaiveDate::from_ymd_opt(2026, 1, 21).unwrap(),
            "Test Entity",
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("reversed") || msg.contains("Reversed"),
            "Error should mention already reversed: {msg}"
        );
    }

    #[test]
    fn reverse_to_closed_period_returns_error() {
        let db = make_entity_db();
        let (jan_period, feb_period) = setup_fiscal_year(&db);
        let (acct1, acct2) = get_two_postable_accounts(&db);

        let entry = make_balanced_entry(
            jan_period,
            acct1,
            acct2,
            NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
        );
        let je_id = db.journals().create_draft(&entry).expect("create");
        post_journal_entry(&db, je_id, "Test Entity").expect("post");

        // Close February so the reversal date's period is closed.
        close_period(&db, feb_period);

        let result = reverse_journal_entry(
            &db,
            je_id,
            NaiveDate::from_ymd_opt(2026, 2, 5).unwrap(),
            "Test Entity",
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("closed") || msg.contains("Closed"),
            "Error should mention closed period: {msg}"
        );
    }
}
