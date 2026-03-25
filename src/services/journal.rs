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
use crate::db::account_repo::Account;
use crate::db::journal_repo::{JournalEntryLine, NewJournalEntry, NewJournalEntryLine};
use crate::types::{
    AccountId, AccountType, AuditAction, JournalEntryId, JournalEntryStatus, Money,
};

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

// ── Cash-receipt detection helpers ────────────────────────────────────────────

/// Returns `true` if the account is a Cash/Bank account eligible for envelope fills.
///
/// Identification rule: type = Asset, not a placeholder, and name contains one
/// of "cash", "bank", "checking", or "savings" (case-insensitive).
/// This matches the seeded hierarchy (1110 Checking Account, 1120 Savings Account,
/// 1100 Cash & Bank Accounts). Custom cash accounts should follow similar naming.
///
/// Decision [Phase 4, Task 2]: name-based detection chosen over a new `is_cash_account`
/// DB flag to avoid a schema change. Developer may revisit if finer control is needed.
fn is_cash_account(acct: &Account) -> bool {
    if acct.account_type != AccountType::Asset || acct.is_placeholder {
        return false;
    }
    let lower = acct.name.to_lowercase();
    lower.contains("cash")
        || lower.contains("bank")
        || lower.contains("checking")
        || lower.contains("savings")
}

/// Returns `true` if the account is Owner's Draw (contra-Equity).
/// Draws and draw reversals suppress envelope fills.
fn is_owners_draw(acct: &Account) -> bool {
    acct.account_type == AccountType::Equity && acct.is_contra
}

/// Computes the total cash-received amount for a journal entry.
///
/// Returns `Some(cash_received)` when envelope fills should be created,
/// or `None` when fills should be suppressed (no cash debit, or Owner's Draw present).
fn cash_receipt_amount(lines: &[JournalEntryLine], accounts: &[Account]) -> Option<Money> {
    let get_acct = |id: AccountId| accounts.iter().find(|a| a.id == id);

    let cash_received: i64 = lines
        .iter()
        .filter(|l| {
            get_acct(l.account_id)
                .map(|a| is_cash_account(a) && !l.debit_amount.is_zero())
                .unwrap_or(false)
        })
        .map(|l| l.debit_amount.0)
        .sum();

    if cash_received == 0 {
        return None;
    }

    // Suppress fills if Owner's Draw appears on either side of the entry.
    let has_owners_draw = lines
        .iter()
        .any(|l| get_acct(l.account_id).map(is_owners_draw).unwrap_or(false));
    if has_owners_draw {
        return None;
    }

    Some(Money(cash_received))
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
    // Collect accounts for later envelope fill detection.
    let account_repo = db.accounts();
    let mut line_accounts: Vec<Account> = Vec::with_capacity(lines.len());
    for line in &lines {
        let acct = account_repo.get_by_id(line.account_id)?;
        if !acct.is_active {
            return Err(JournalError::InactiveAccount(i64::from(line.account_id)).into());
        }
        if acct.is_placeholder {
            return Err(JournalError::PlaceholderAccount(i64::from(line.account_id)).into());
        }
        line_accounts.push(acct);
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

    // Pre-compute envelope fill amount (None = no fills needed).
    let fill_cash = cash_receipt_amount(&lines, &line_accounts);

    // Build audit description: summarise the first debit and credit lines.
    let description = build_post_description(&entry.je_number, &lines, &line_accounts);

    // Execute the transition inside a single transaction.
    let tx = db.conn().unchecked_transaction()?;
    {
        use crate::db::audit_repo::AuditRepo;
        use crate::db::envelope_repo::EnvelopeRepo;
        use crate::db::journal_repo::JournalRepo;

        JournalRepo::new(&tx).update_status(je_id, JournalEntryStatus::Posted)?;

        // Envelope fills: if a cash receipt was detected, apply each allocation.
        if let Some(cash_received) = fill_cash {
            let env = EnvelopeRepo::new(&tx);
            for alloc in env.get_all_allocations()? {
                let fill = cash_received.apply_percentage(alloc.percentage);
                if !fill.is_zero() {
                    env.record_fill(alloc.account_id, fill, je_id)?;
                }
            }
        }

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

    // Pre-fetch fills for the original JE so we can reverse them inside the transaction.
    let original_fills = db.envelopes().get_fills_for_je(je_id)?;

    let tx = db.conn().unchecked_transaction()?;
    let reversal_id = {
        use crate::db::audit_repo::AuditRepo;
        use crate::db::envelope_repo::EnvelopeRepo;
        use crate::db::journal_repo::JournalRepo;

        let journal = JournalRepo::new(&tx);

        // Create the reversal as a draft, then immediately post it.
        let rev_id = journal.create_draft(&reversal_entry)?;
        journal.update_status(rev_id, JournalEntryStatus::Posted)?;

        // Mark the original as reversed.
        journal.mark_reversed(je_id, rev_id)?;

        // Reverse any envelope fills that were created when the original JE was posted.
        if !original_fills.is_empty() {
            let env = EnvelopeRepo::new(&tx);
            for fill in &original_fills {
                // record_reversal stores -amount, effectively undoing the fill.
                env.record_reversal(fill.account_id, fill.amount, je_id)?;
            }
        }

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

// ── create_payment_je ─────────────────────────────────────────────────────────

/// Creates and immediately posts a two-line payment journal entry.
///
/// - `debit_account_id` receives the debit line.
/// - `credit_account_id` receives the credit line.
///
/// Caller decides the direction:
/// - AR payment received: debit = cash account, credit = AR account.
/// - AP payment made:     debit = AP account,   credit = cash account.
///
/// Returns the `JournalEntryId` of the posted entry.
pub fn create_payment_je(
    db: &EntityDb,
    entity_name: &str,
    debit_account_id: AccountId,
    credit_account_id: AccountId,
    amount: Money,
    payment_date: NaiveDate,
    memo: Option<String>,
) -> Result<JournalEntryId> {
    let period = db
        .fiscal()
        .get_period_for_date(payment_date)
        .map_err(|_| JournalError::NoPeriodForDate(payment_date))?;

    let je_id = db.journals().create_draft(&NewJournalEntry {
        entry_date: payment_date,
        memo,
        fiscal_period_id: period.id,
        reversal_of_je_id: None,
        lines: vec![
            NewJournalEntryLine {
                account_id: debit_account_id,
                debit_amount: amount,
                credit_amount: Money(0),
                line_memo: None,
                sort_order: 0,
            },
            NewJournalEntryLine {
                account_id: credit_account_id,
                debit_amount: Money(0),
                credit_amount: amount,
                line_memo: None,
                sort_order: 1,
            },
        ],
    })?;

    post_journal_entry(db, je_id, entity_name)?;
    Ok(je_id)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Builds a human-readable description for the post audit log entry.
/// Shows all debit and credit lines with account names.
fn build_post_description(
    je_number: &str,
    lines: &[JournalEntryLine],
    accounts: &[Account],
) -> String {
    let get_name = |id: AccountId| {
        accounts
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.name.as_str())
            .unwrap_or("Unknown")
    };

    let parts: Vec<String> = lines
        .iter()
        .filter_map(|l| {
            if !l.debit_amount.is_zero() {
                Some(format!("Dr {} {}", get_name(l.account_id), l.debit_amount))
            } else if !l.credit_amount.is_zero() {
                Some(format!("Cr {} {}", get_name(l.account_id), l.credit_amount))
            } else {
                None
            }
        })
        .collect();

    format!("Posted {}: {}", je_number, parts.join(", "))
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

    #[test]
    fn post_already_posted_entry_returns_not_draft_error() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);
        let (acct1, acct2) = get_two_postable_accounts(&db);
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        let entry = make_balanced_entry(jan_period, acct1, acct2, date);
        let je_id = db.journals().create_draft(&entry).expect("create");
        post_journal_entry(&db, je_id, "Test Entity").expect("first post");

        let result = post_journal_entry(&db, je_id, "Test Entity");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not a Draft"),
            "Error should mention not a Draft: {msg}"
        );
    }

    // ── reverse_journal_entry ─────────────────────────────────────────────────

    #[test]
    fn reverse_draft_entry_returns_not_posted_error() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);
        let (acct1, acct2) = get_two_postable_accounts(&db);
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        let entry = make_balanced_entry(jan_period, acct1, acct2, date);
        let je_id = db.journals().create_draft(&entry).expect("create");

        let result = reverse_journal_entry(
            &db,
            je_id,
            NaiveDate::from_ymd_opt(2026, 1, 20).unwrap(),
            "Test Entity",
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not Posted"),
            "Error should mention not Posted: {msg}"
        );
    }

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

    // ── Envelope fill tests ───────────────────────────────────────────────────

    /// Helper: find an account by number in the seeded chart.
    fn find_account_id(db: &EntityDb, number: &str) -> crate::types::AccountId {
        db.accounts()
            .list_all()
            .expect("list_all")
            .into_iter()
            .find(|a| a.number == number)
            .unwrap_or_else(|| panic!("Account {number} not found in seeded chart"))
            .id
    }

    /// Helper: set a 10% allocation on `account_id`.
    fn set_ten_pct_allocation(db: &EntityDb, account_id: crate::types::AccountId) {
        db.envelopes()
            .set_allocation(
                account_id,
                crate::types::Percentage(10_000_000),
                crate::types::Percentage(0),
                None,
            )
            .expect("set allocation");
    }

    #[test]
    fn post_cash_receipt_fills_envelopes() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);

        let checking = find_account_id(&db, "1110"); // Checking Account (Cash)
        let revenue = find_account_id(&db, "4100"); // Service Revenue
        let envelope_acct = find_account_id(&db, "5100"); // some account to earmark

        // Configure 10% allocation.
        set_ten_pct_allocation(&db, envelope_acct);

        // Post: debit Checking $1000, credit Revenue $1000.
        let amount = Money(100_000_000_000); // $1000.00
        let entry = NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            memo: None,
            fiscal_period_id: jan_period,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: checking,
                    debit_amount: amount,
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: revenue,
                    debit_amount: Money(0),
                    credit_amount: amount,
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };
        let je_id = db.journals().create_draft(&entry).expect("create");
        post_journal_entry(&db, je_id, "Test Entity").expect("post");

        // Envelope balance should be 10% of $1000 = $100.
        let balance = db.envelopes().get_balance(envelope_acct).expect("balance");
        let expected = Money(10_000_000_000); // $100.00
        assert_eq!(balance, expected, "10% of $1000 should be earmarked");

        // Fill entry should reference the JE.
        let fills = db.envelopes().get_fills_for_je(je_id).expect("fills");
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].source_je_id, Some(je_id));
    }

    #[test]
    fn post_non_cash_je_does_not_fill_envelopes() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);

        let expense = find_account_id(&db, "5100"); // Rent (Expense)
        let ap = find_account_id(&db, "2100"); // Accounts Payable
        let envelope_acct = find_account_id(&db, "5200"); // different account

        set_ten_pct_allocation(&db, envelope_acct);

        // Post: debit Rent Expense, credit AP (no cash debit).
        let entry = NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            memo: None,
            fiscal_period_id: jan_period,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: expense,
                    debit_amount: Money(50_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: ap,
                    debit_amount: Money(0),
                    credit_amount: Money(50_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };
        let je_id = db.journals().create_draft(&entry).expect("create");
        post_journal_entry(&db, je_id, "Test Entity").expect("post");

        let balance = db.envelopes().get_balance(envelope_acct).expect("balance");
        assert_eq!(balance, Money(0), "Non-cash JE should not trigger fills");
    }

    #[test]
    fn post_owners_draw_je_does_not_fill_envelopes() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);

        // Owner's Draw (3200, is_contra) debits Owner's Draw and credits Checking.
        let draw = find_account_id(&db, "3200"); // Owner's Draw (contra-equity)
        let checking = find_account_id(&db, "1110"); // Checking Account
        let envelope_acct = find_account_id(&db, "5100");

        set_ten_pct_allocation(&db, envelope_acct);

        // Entry: debit Owner's Draw, credit Checking (draw taken out).
        let entry = NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 20).unwrap(),
            memo: None,
            fiscal_period_id: jan_period,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: draw,
                    debit_amount: Money(20_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: checking,
                    debit_amount: Money(0),
                    credit_amount: Money(20_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };
        let je_id = db.journals().create_draft(&entry).expect("create");
        post_journal_entry(&db, je_id, "Test Entity").expect("post");

        // No fills: cash was credited (not debited), and Owner's Draw is present.
        let balance = db.envelopes().get_balance(envelope_acct).expect("balance");
        assert_eq!(
            balance,
            Money(0),
            "Owner's Draw JE should not trigger fills"
        );
    }

    #[test]
    fn post_capital_contribution_fills_envelopes() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);

        let checking = find_account_id(&db, "1110"); // Checking (cash)
        let capital = find_account_id(&db, "3100"); // Owner's Capital
        let envelope_acct = find_account_id(&db, "5100");

        set_ten_pct_allocation(&db, envelope_acct);

        // Capital contribution: debit Checking, credit Owner's Capital.
        let amount = Money(200_000_000_000); // $2000.00
        let entry = NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
            memo: None,
            fiscal_period_id: jan_period,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: checking,
                    debit_amount: amount,
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: capital,
                    debit_amount: Money(0),
                    credit_amount: amount,
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };
        let je_id = db.journals().create_draft(&entry).expect("create");
        post_journal_entry(&db, je_id, "Test Entity").expect("post");

        let balance = db.envelopes().get_balance(envelope_acct).expect("balance");
        let expected = Money(20_000_000_000); // 10% of $2000 = $200
        assert_eq!(
            balance, expected,
            "Capital contribution should trigger fills"
        );
    }

    #[test]
    fn reverse_cash_receipt_creates_reversal_entries_net_zero() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);

        let checking = find_account_id(&db, "1110");
        let revenue = find_account_id(&db, "4100");
        let envelope_acct = find_account_id(&db, "5100");

        set_ten_pct_allocation(&db, envelope_acct);

        // Post cash receipt: debit Checking $1000, credit Revenue $1000.
        let amount = Money(100_000_000_000); // $1000.00
        let entry = NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            memo: None,
            fiscal_period_id: jan_period,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: checking,
                    debit_amount: amount,
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: revenue,
                    debit_amount: Money(0),
                    credit_amount: amount,
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };
        let je_id = db.journals().create_draft(&entry).expect("create");
        post_journal_entry(&db, je_id, "Test Entity").expect("post");

        // Confirm fill was created ($100 earmarked).
        let balance_after_fill = db.envelopes().get_balance(envelope_acct).expect("balance");
        assert_eq!(
            balance_after_fill,
            Money(10_000_000_000),
            "Balance should be $100 after fill"
        );

        // Reverse the entry.
        let reversal_date = NaiveDate::from_ymd_opt(2026, 1, 20).unwrap();
        reverse_journal_entry(&db, je_id, reversal_date, "Test Entity").expect("reverse");

        // Net envelope balance should be zero.
        let balance_after_reversal = db.envelopes().get_balance(envelope_acct).expect("balance");
        assert_eq!(
            balance_after_reversal,
            Money(0),
            "Net envelope balance should be zero after reversal"
        );

        // Ledger should have 1 Fill and 1 Reversal.
        let ledger = db.envelopes().get_ledger(envelope_acct).expect("ledger");
        assert_eq!(ledger.len(), 2, "Should have Fill + Reversal entries");
        let types: Vec<_> = ledger.iter().map(|e| e.entry_type).collect();
        assert!(
            types.contains(&crate::types::EnvelopeEntryType::Fill),
            "Fill entry should exist"
        );
        assert!(
            types.contains(&crate::types::EnvelopeEntryType::Reversal),
            "Reversal entry should exist"
        );
    }

    #[test]
    fn reverse_non_cash_je_creates_no_reversal_entries() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);

        let expense = find_account_id(&db, "5100");
        let ap = find_account_id(&db, "2100");
        let envelope_acct = find_account_id(&db, "5200");

        set_ten_pct_allocation(&db, envelope_acct);

        // Post non-cash JE (no fills created).
        let entry = NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            memo: None,
            fiscal_period_id: jan_period,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: expense,
                    debit_amount: Money(50_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: ap,
                    debit_amount: Money(0),
                    credit_amount: Money(50_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };
        let je_id = db.journals().create_draft(&entry).expect("create");
        post_journal_entry(&db, je_id, "Test Entity").expect("post");

        // Reverse it.
        let reversal_date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        reverse_journal_entry(&db, je_id, reversal_date, "Test Entity").expect("reverse");

        // No envelope entries at all.
        let ledger = db.envelopes().get_ledger(envelope_acct).expect("ledger");
        assert_eq!(
            ledger.len(),
            0,
            "Reversing a non-cash JE should create no envelope entries"
        );
    }

    #[test]
    fn post_cash_receipt_with_no_allocations_creates_no_fills() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);

        let checking = find_account_id(&db, "1110");
        let revenue = find_account_id(&db, "4100");

        // No allocations configured.
        let entry = NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            memo: None,
            fiscal_period_id: jan_period,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: checking,
                    debit_amount: Money(50_000_000_000),
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: revenue,
                    debit_amount: Money(0),
                    credit_amount: Money(50_000_000_000),
                    line_memo: None,
                    sort_order: 1,
                },
            ],
        };
        let je_id = db.journals().create_draft(&entry).expect("create");
        post_journal_entry(&db, je_id, "Test Entity").expect("post");

        let fills = db.envelopes().get_fills_for_je(je_id).expect("fills");
        assert_eq!(fills.len(), 0, "No allocations → no fills");
    }

    // ── Multiple cash accounts ────────────────────────────────────────────────

    #[test]
    fn post_cash_receipt_with_two_cash_accounts_sums_both_for_fill() {
        let db = make_entity_db();
        let (jan_period, _) = setup_fiscal_year(&db);

        let checking = find_account_id(&db, "1110"); // Checking Account (Cash)
        let savings = find_account_id(&db, "1120"); // Savings Account (Cash)
        let revenue = find_account_id(&db, "4100"); // Service Revenue
        let envelope_acct = find_account_id(&db, "5100");

        // 10% allocation.
        set_ten_pct_allocation(&db, envelope_acct);

        // Post JE: Debit Checking $600 + Debit Savings $400 = $1000 cash total;
        // Credit Revenue $1000. This verifies fills sum ALL cash lines.
        let entry = NewJournalEntry {
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            memo: None,
            fiscal_period_id: jan_period,
            reversal_of_je_id: None,
            lines: vec![
                NewJournalEntryLine {
                    account_id: checking,
                    debit_amount: Money(60_000_000_000), // $600
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 0,
                },
                NewJournalEntryLine {
                    account_id: savings,
                    debit_amount: Money(40_000_000_000), // $400
                    credit_amount: Money(0),
                    line_memo: None,
                    sort_order: 1,
                },
                NewJournalEntryLine {
                    account_id: revenue,
                    debit_amount: Money(0),
                    credit_amount: Money(100_000_000_000), // $1000
                    line_memo: None,
                    sort_order: 2,
                },
            ],
        };
        let je_id = db.journals().create_draft(&entry).expect("create");
        post_journal_entry(&db, je_id, "Test Entity").expect("post");

        // Fill should be 10% of ($600 + $400) = 10% of $1000 = $100.
        let balance = db.envelopes().get_balance(envelope_acct).expect("balance");
        assert_eq!(
            balance,
            Money(10_000_000_000), // $100
            "Fill must sum both cash debit lines: 10% of $600+$400 = $100"
        );
    }

    // ── create_payment_je ─────────────────────────────────────────────────────

    #[test]
    fn create_payment_je_posts_and_returns_id() {
        let db = make_entity_db();
        setup_fiscal_year(&db);

        let all = db.accounts().list_active().expect("list");
        let postable: Vec<_> = all.iter().filter(|a| !a.is_placeholder).collect();
        let cash_id = postable[0].id;
        let ar_id = postable[1].id;

        let payment_date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let amount = Money(10_000_000_000); // $100
        let je_id = create_payment_je(
            &db,
            "Test Entity",
            cash_id,
            ar_id,
            amount,
            payment_date,
            Some("Test payment".to_string()),
        )
        .expect("create_payment_je");

        let (je, lines) = db.journals().get_with_lines(je_id).expect("get");
        assert_eq!(je.status, JournalEntryStatus::Posted);
        assert_eq!(lines.len(), 2);
        // First line: debit cash.
        assert_eq!(lines[0].account_id, cash_id);
        assert_eq!(lines[0].debit_amount, amount);
        assert!(lines[0].credit_amount.is_zero());
        // Second line: credit AR.
        assert_eq!(lines[1].account_id, ar_id);
        assert!(lines[1].debit_amount.is_zero());
        assert_eq!(lines[1].credit_amount, amount);
    }

    #[test]
    fn create_payment_je_fails_without_fiscal_period() {
        let db = make_entity_db();
        // No fiscal year created.

        let all = db.accounts().list_active().expect("list");
        let postable: Vec<_> = all.iter().filter(|a| !a.is_placeholder).collect();
        let cash_id = postable[0].id;
        let ar_id = postable[1].id;

        let result = create_payment_je(
            &db,
            "Test Entity",
            cash_id,
            ar_id,
            Money(10_000_000_000),
            NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            None,
        );
        assert!(result.is_err(), "Should fail without a fiscal period");
    }
}
