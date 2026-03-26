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

        // Two-tier envelope fills: primary fills (with cap gating) → overflow → secondary fills.
        if let Some(cash_received) = fill_cash {
            let env = EnvelopeRepo::new(&tx);
            let allocations = env.get_all_allocations()?;
            let mut overflow = Money(0);

            // Step 1: Primary fills (gated by cap).
            for alloc in &allocations {
                if alloc.percentage.0 <= 0 {
                    continue;
                }
                let primary_amount = cash_received.apply_percentage(alloc.percentage);
                let actual_fill = if let Some(cap) = alloc.cap_amount {
                    let current = env.get_balance(alloc.account_id)?;
                    let room = Money((cap - current).0.max(0));
                    let fill = Money(primary_amount.0.min(room.0));
                    overflow = overflow + (primary_amount - fill);
                    fill
                } else {
                    primary_amount
                };
                if actual_fill.0 > 0 {
                    env.record_fill(alloc.account_id, actual_fill, je_id)?;
                }
            }

            // Step 2: Secondary fills (from overflow; not gated by cap).
            if overflow.0 > 0 {
                for alloc in &allocations {
                    if alloc.secondary_percentage.0 <= 0 {
                        continue;
                    }
                    let secondary_amount = overflow.apply_percentage(alloc.secondary_percentage);
                    if secondary_amount.0 > 0 {
                        env.record_fill(alloc.account_id, secondary_amount, je_id)?;
                    }
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_v5;
