//! Year-end close service.
//!
//! The workflow:
//! 1. Call `generate_closing_entries` → get `Vec<NewJournalEntry>` for user review.
//! 2. Create each entry in the DB via `JournalRepo::create_draft` → collect the IDs.
//! 3. Present the draft entries to the user. If approved:
//! 4. Call `execute_year_end_close` with the collected IDs → posts entries, marks year closed.

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;

use crate::db::{
    EntityDb,
    audit_repo::AuditRepo,
    journal_repo::{NewJournalEntry, NewJournalEntryLine},
    now_str,
};
use crate::services::journal::post_journal_entry;
use crate::types::{AccountId, AuditAction, FiscalYearId, JournalEntryId, Money};

// ── generate_closing_entries ─────────────────────────────────────────────────

/// Calculates closing journal entries for the given fiscal year.
///
/// Returns a `Vec<NewJournalEntry>` — typically one combined closing entry —
/// for user review. The caller must create these drafts via `JournalRepo::create_draft`
/// and then call `execute_year_end_close` to post them and mark the year closed.
///
/// Returns an empty `Vec` if no Revenue or Expense accounts have non-zero balances.
///
/// Errors:
/// - Fiscal year not found or already closed.
/// - Retained Earnings account (3300) not found.
/// - No fiscal periods for the year (unlikely but guarded).
pub fn generate_closing_entries(
    db: &EntityDb,
    fiscal_year_id: FiscalYearId,
) -> Result<Vec<NewJournalEntry>> {
    // Check fiscal year exists and is not already closed.
    let fy = db
        .conn()
        .query_row(
            "SELECT id, is_closed, end_date FROM fiscal_years WHERE id = ?1",
            rusqlite::params![i64::from(fiscal_year_id)],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i32>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .with_context(|| format!("Fiscal year {} not found", i64::from(fiscal_year_id)))?;

    if fy.1 != 0 {
        bail!(
            "Fiscal year {} is already closed",
            i64::from(fiscal_year_id)
        );
    }

    let fy_end_date = NaiveDate::parse_from_str(&fy.2, "%Y-%m-%d")
        .context("Invalid fiscal year end_date in database")?;

    // Get the last period of the fiscal year (for the closing entry's fiscal_period_id).
    let last_period = db
        .fiscal()
        .list_periods(fiscal_year_id)?
        .into_iter()
        .max_by_key(|p| p.period_number)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No fiscal periods found for year {}",
                i64::from(fiscal_year_id)
            )
        })?;

    // Query net balances for Revenue and Expense accounts over all posted JEs in this FY.
    let mut stmt = db.conn().prepare(
        "SELECT a.id, a.account_type,
                COALESCE(SUM(jel.debit_amount - jel.credit_amount), 0) AS net
         FROM accounts a
         JOIN journal_entry_lines jel ON jel.account_id = a.id
         JOIN journal_entries je ON je.id = jel.journal_entry_id
         WHERE je.status = 'Posted'
           AND je.fiscal_period_id IN (
               SELECT id FROM fiscal_periods WHERE fiscal_year_id = ?1
           )
           AND a.account_type IN ('Revenue', 'Expense')
           AND a.is_placeholder = 0
         GROUP BY a.id, a.account_type
         HAVING net != 0
         ORDER BY a.account_type, a.id",
    )?;

    struct AccountBalance {
        id: AccountId,
        account_type: String,
        net: i64,
    }

    let balances: Vec<AccountBalance> = stmt
        .query_map(rusqlite::params![i64::from(fiscal_year_id)], |row| {
            Ok(AccountBalance {
                id: AccountId::from(row.get::<_, i64>(0)?),
                account_type: row.get(1)?,
                net: row.get(2)?,
            })
        })?
        .map(|r| r.map_err(anyhow::Error::from))
        .collect::<Result<Vec<_>>>()?;

    if balances.is_empty() {
        return Ok(Vec::new());
    }

    // Find the Retained Earnings account (account number 3300).
    let retained_earnings_id = db
        .conn()
        .query_row(
            "SELECT id FROM accounts WHERE number = '3300' AND is_placeholder = 0 LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .context("Retained Earnings account (3300) not found — create it in Chart of Accounts")?;
    let retained_earnings_id = AccountId::from(retained_earnings_id);

    // Build closing JE lines: for each account, add a line to zero its balance.
    // Revenue (credit-normal, net < 0): Dr Revenue to close. debit = -net, credit = 0.
    // Expense (debit-normal, net > 0): Cr Expense to close. debit = 0, credit = net.
    // Handle unusual cases where sign is flipped.
    let mut lines: Vec<NewJournalEntryLine> = Vec::new();
    let mut total_debits: i64 = 0;
    let mut total_credits: i64 = 0;

    for (sort, bal) in balances.iter().enumerate() {
        let debit = if bal.net < 0 { -bal.net } else { 0 };
        let credit = if bal.net > 0 { bal.net } else { 0 };
        total_debits += debit;
        total_credits += credit;
        lines.push(NewJournalEntryLine {
            account_id: bal.id,
            debit_amount: Money(debit),
            credit_amount: Money(credit),
            line_memo: Some(format!("Close {}", bal.account_type)),
            sort_order: sort as i32,
        });
    }

    // Retained Earnings balancing line.
    let re_sort = lines.len() as i32;
    let net_income = total_debits - total_credits;
    let (re_debit, re_credit) = if net_income > 0 {
        // More debits than credits so far (profit): Cr Retained Earnings.
        (0, net_income)
    } else if net_income < 0 {
        // More credits than debits (loss): Dr Retained Earnings.
        (-net_income, 0)
    } else {
        // Exactly balanced — no Retained Earnings line needed.
        (0, 0)
    };

    if re_debit > 0 || re_credit > 0 {
        lines.push(NewJournalEntryLine {
            account_id: retained_earnings_id,
            debit_amount: Money(re_debit),
            credit_amount: Money(re_credit),
            line_memo: Some("Net income to Retained Earnings".to_string()),
            sort_order: re_sort,
        });
    }

    let closing_je = NewJournalEntry {
        entry_date: fy_end_date,
        memo: Some(format!(
            "Year-end closing entries — FY {}",
            i64::from(fiscal_year_id)
        )),
        fiscal_period_id: last_period.id,
        reversal_of_je_id: None,
        lines,
    };

    Ok(vec![closing_je])
}

// ── execute_year_end_close ────────────────────────────────────────────────────

/// Posts the closing entries and marks the fiscal year as closed.
///
/// `closing_je_ids` must be Draft journal entries previously created from
/// `generate_closing_entries`. Each entry is posted via `post_journal_entry`.
/// The fiscal year is then marked `is_closed = 1`.
///
/// Errors:
/// - Fiscal year not found or already closed.
/// - Any closing JE fails to post (e.g., accounts inactive, period closed).
pub fn execute_year_end_close(
    db: &EntityDb,
    fiscal_year_id: FiscalYearId,
    closing_je_ids: &[JournalEntryId],
    entity_name: &str,
) -> Result<()> {
    // Check fiscal year exists and is not already closed.
    let is_closed: i32 = db
        .conn()
        .query_row(
            "SELECT is_closed FROM fiscal_years WHERE id = ?1",
            rusqlite::params![i64::from(fiscal_year_id)],
            |row| row.get(0),
        )
        .with_context(|| format!("Fiscal year {} not found", i64::from(fiscal_year_id)))?;

    if is_closed != 0 {
        bail!(
            "Fiscal year {} is already closed",
            i64::from(fiscal_year_id)
        );
    }

    // Post each closing entry.
    for &je_id in closing_je_ids {
        post_journal_entry(db, je_id, entity_name)
            .with_context(|| format!("Failed to post closing JE {}", i64::from(je_id)))?;
    }

    // Mark the fiscal year as closed.
    let now = now_str();
    db.conn()
        .execute(
            "UPDATE fiscal_years SET is_closed = 1, closed_at = ?2 WHERE id = ?1",
            rusqlite::params![i64::from(fiscal_year_id), now],
        )
        .context("Failed to mark fiscal year as closed")?;

    AuditRepo::new(db.conn()).append(
        AuditAction::YearEndClose,
        entity_name,
        Some("FiscalYear"),
        Some(i64::from(fiscal_year_id)),
        &format!(
            "Year-end close completed for fiscal year {}",
            i64::from(fiscal_year_id)
        ),
    )?;

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{
        account_repo::{AccountRepo, NewAccount},
        entity_db_from_conn,
        journal_repo::{NewJournalEntry, NewJournalEntryLine},
        schema::{initialize_schema, seed_default_accounts},
    };
    use crate::types::{AccountType, FiscalPeriodId};
    use rusqlite::Connection;

    fn setup() -> (EntityDb, FiscalYearId, FiscalPeriodId) {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema");
        seed_default_accounts(&conn).expect("seed");
        let db = entity_db_from_conn(conn);
        let fy_id = db.fiscal().create_fiscal_year(1, 2026).expect("create FY");
        let periods = db.fiscal().list_periods(fy_id).expect("list periods");
        let jan = periods[0].id;
        (db, fy_id, jan)
    }

    /// Post a balanced JE: debit acct1, credit acct2 for the given amount.
    fn post_balanced_je(
        db: &EntityDb,
        acct1: AccountId,
        acct2: AccountId,
        amount: i64,
        period: FiscalPeriodId,
        date: NaiveDate,
    ) -> JournalEntryId {
        let je_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: date,
                memo: None,
                fiscal_period_id: period,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: acct1,
                        debit_amount: Money(amount),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: acct2,
                        debit_amount: Money(0),
                        credit_amount: Money(amount),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create draft");
        post_journal_entry(db, je_id, "Test Entity").expect("post");
        je_id
    }

    /// Returns the net balance for an account in the current FY (all posted JEs).
    fn fy_balance(db: &EntityDb, account_id: AccountId, fy_id: FiscalYearId) -> Money {
        let net: i64 = db
            .conn()
            .query_row(
                "SELECT COALESCE(SUM(jel.debit_amount - jel.credit_amount), 0)
                 FROM journal_entry_lines jel
                 JOIN journal_entries je ON je.id = jel.journal_entry_id
                 WHERE jel.account_id = ?1
                   AND je.status = 'Posted'
                   AND je.fiscal_period_id IN (
                       SELECT id FROM fiscal_periods WHERE fiscal_year_id = ?2
                   )",
                rusqlite::params![i64::from(account_id), i64::from(fy_id)],
                |row| row.get(0),
            )
            .expect("balance query");
        Money(net)
    }

    // ── find account helpers ──────────────────────────────────────────────────

    fn find_account_by_number(db: &EntityDb, number: &str) -> AccountId {
        db.accounts()
            .list_all()
            .expect("list all")
            .into_iter()
            .find(|a| a.number == number)
            .unwrap_or_else(|| panic!("account {number} not found"))
            .id
    }

    fn create_revenue_account(db: &EntityDb) -> AccountId {
        let revenue_parent = find_account_by_number(db, "4000");
        AccountRepo::new(db.conn())
            .create(&NewAccount {
                number: "4100".to_string(),
                name: "Test Revenue".to_string(),
                account_type: AccountType::Revenue,
                parent_id: Some(revenue_parent),
                is_contra: false,
                is_placeholder: false,
            })
            .unwrap_or_else(|_| find_account_by_number(db, "4100"))
    }

    fn create_expense_account(db: &EntityDb) -> AccountId {
        let expense_parent = find_account_by_number(db, "5000");
        AccountRepo::new(db.conn())
            .create(&NewAccount {
                number: "5100".to_string(),
                name: "Test Expense".to_string(),
                account_type: AccountType::Expense,
                parent_id: Some(expense_parent),
                is_contra: false,
                is_placeholder: false,
            })
            .unwrap_or_else(|_| find_account_by_number(db, "5100"))
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn generate_closing_entries_returns_empty_if_no_activity() {
        let (db, fy_id, _) = setup();
        let entries = generate_closing_entries(&db, fy_id).expect("generate");
        assert!(entries.is_empty(), "No activity → no closing entries");
    }

    #[test]
    fn generate_closing_entries_zero_out_revenue_and_expense() {
        let (db, fy_id, jan) = setup();
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        let cash = find_account_by_number(&db, "1110"); // Checking Account (non-placeholder)
        let revenue = create_revenue_account(&db);
        let expense = create_expense_account(&db);

        // Post: Dr Cash $1000, Cr Revenue $1000
        post_balanced_je(&db, cash, revenue, 100_000_000_000, jan, date);

        // Post: Dr Expense $400, Cr Cash $400
        post_balanced_je(&db, expense, cash, 40_000_000_000, jan, date);

        // Revenue balance = -$1000 (net credit), Expense balance = +$400 (net debit)
        assert_eq!(fy_balance(&db, revenue, fy_id), Money(-100_000_000_000));
        assert_eq!(fy_balance(&db, expense, fy_id), Money(40_000_000_000));

        let entries = generate_closing_entries(&db, fy_id).expect("generate");
        assert_eq!(entries.len(), 1, "Should generate one closing JE");

        let je = &entries[0];
        // Should have lines for revenue, expense, and retained earnings.
        assert!(je.lines.len() >= 3);

        // Revenue closing line: Dr Revenue
        let rev_line = je
            .lines
            .iter()
            .find(|l| l.account_id == revenue)
            .expect("revenue line");
        assert_eq!(rev_line.debit_amount, Money(100_000_000_000));
        assert_eq!(rev_line.credit_amount, Money(0));

        // Expense closing line: Cr Expense
        let exp_line = je
            .lines
            .iter()
            .find(|l| l.account_id == expense)
            .expect("expense line");
        assert_eq!(exp_line.credit_amount, Money(40_000_000_000));
        assert_eq!(exp_line.debit_amount, Money(0));

        // JE is balanced.
        let total_debit: i64 = je.lines.iter().map(|l| l.debit_amount.0).sum();
        let total_credit: i64 = je.lines.iter().map(|l| l.credit_amount.0).sum();
        assert_eq!(total_debit, total_credit, "Closing JE must be balanced");
    }

    #[test]
    fn retained_earnings_receives_net_income() {
        let (db, fy_id, jan) = setup();
        let date = NaiveDate::from_ymd_opt(2026, 1, 20).unwrap();

        let cash = find_account_by_number(&db, "1110"); // Checking Account (non-placeholder)
        let revenue = create_revenue_account(&db);
        let expense = create_expense_account(&db);
        let re = find_account_by_number(&db, "3300"); // Retained Earnings

        // Revenue $1000, Expense $300 → net income $700
        post_balanced_je(&db, cash, revenue, 100_000_000_000, jan, date);
        post_balanced_je(&db, expense, cash, 30_000_000_000, jan, date);

        let entries = generate_closing_entries(&db, fy_id).expect("generate");
        let je = &entries[0];

        // Retained Earnings line: Cr $700
        let re_line = je
            .lines
            .iter()
            .find(|l| l.account_id == re)
            .expect("retained earnings line");
        assert_eq!(re_line.credit_amount, Money(70_000_000_000)); // $700
        assert_eq!(re_line.debit_amount, Money(0));
    }

    #[test]
    fn retained_earnings_debited_on_net_loss() {
        let (db, fy_id, jan) = setup();
        let date = NaiveDate::from_ymd_opt(2026, 1, 20).unwrap();

        let cash = find_account_by_number(&db, "1110"); // Checking Account (non-placeholder)
        let revenue = create_revenue_account(&db);
        let expense = create_expense_account(&db);
        let re = find_account_by_number(&db, "3300");

        // Revenue $200, Expense $500 → net loss $300
        post_balanced_je(&db, cash, revenue, 20_000_000_000, jan, date);
        post_balanced_je(&db, expense, cash, 50_000_000_000, jan, date);

        let entries = generate_closing_entries(&db, fy_id).expect("generate");
        let je = &entries[0];

        let re_line = je
            .lines
            .iter()
            .find(|l| l.account_id == re)
            .expect("retained earnings line");
        assert_eq!(re_line.debit_amount, Money(30_000_000_000)); // Dr $300 (loss)
        assert_eq!(re_line.credit_amount, Money(0));
    }

    #[test]
    fn execute_year_end_close_posts_entries_and_marks_year_closed() {
        let (db, fy_id, jan) = setup();
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        let cash = find_account_by_number(&db, "1110"); // Checking Account (non-placeholder)
        let revenue = create_revenue_account(&db);
        let expense = create_expense_account(&db);

        post_balanced_je(&db, cash, revenue, 100_000_000_000, jan, date);
        post_balanced_je(&db, expense, cash, 60_000_000_000, jan, date);

        // Generate and create the closing drafts.
        let closing_jes = generate_closing_entries(&db, fy_id).expect("generate");
        assert_eq!(closing_jes.len(), 1);

        let closing_ids: Vec<JournalEntryId> = closing_jes
            .iter()
            .map(|je| db.journals().create_draft(je).expect("create draft"))
            .collect();

        execute_year_end_close(&db, fy_id, &closing_ids, "Test Entity").expect("year-end close");

        // Fiscal year should now be closed.
        let is_closed: i32 = db
            .conn()
            .query_row(
                "SELECT is_closed FROM fiscal_years WHERE id = ?1",
                rusqlite::params![i64::from(fy_id)],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(is_closed, 1, "Fiscal year must be marked closed");

        // Revenue and Expense should have zero balance after closing.
        assert_eq!(
            fy_balance(&db, revenue, fy_id),
            Money(0),
            "Revenue balance must be zero after close"
        );
        assert_eq!(
            fy_balance(&db, expense, fy_id),
            Money(0),
            "Expense balance must be zero after close"
        );
    }

    #[test]
    fn execute_year_end_close_already_closed_returns_error() {
        let (db, fy_id, jan) = setup();
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        let cash = find_account_by_number(&db, "1110"); // Checking Account (non-placeholder)
        let revenue = create_revenue_account(&db);

        post_balanced_je(&db, cash, revenue, 10_000_000_000, jan, date);

        let closing_jes = generate_closing_entries(&db, fy_id).expect("generate");
        let closing_ids: Vec<JournalEntryId> = closing_jes
            .iter()
            .map(|je| db.journals().create_draft(je).expect("create draft"))
            .collect();

        execute_year_end_close(&db, fy_id, &closing_ids, "Test Entity").expect("first close");

        // Second close attempt.
        let result = execute_year_end_close(&db, fy_id, &[], "Test Entity");
        assert!(result.is_err(), "Double close should fail");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("already closed"),
            "Error should mention already closed: {msg}"
        );
    }

    #[test]
    fn generate_closing_entries_rejects_already_closed_year() {
        let (db, fy_id, jan) = setup();
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        let cash = find_account_by_number(&db, "1110"); // Checking Account (non-placeholder)
        let revenue = create_revenue_account(&db);

        post_balanced_je(&db, cash, revenue, 10_000_000_000, jan, date);

        let closing_jes = generate_closing_entries(&db, fy_id).expect("generate");
        let closing_ids: Vec<JournalEntryId> = closing_jes
            .iter()
            .map(|je| db.journals().create_draft(je).expect("create draft"))
            .collect();

        execute_year_end_close(&db, fy_id, &closing_ids, "Test Entity").expect("close");

        let result = generate_closing_entries(&db, fy_id);
        assert!(result.is_err(), "Generate on closed year should fail");
    }

    #[test]
    fn envelope_balances_persist_after_year_end_close() {
        // Envelope allocations are budgetary and must NOT be cleared by year-end close.
        let (db, fy_id, jan) = setup();
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        let cash = find_account_by_number(&db, "1110"); // Checking Account (Cash)
        let revenue = create_revenue_account(&db);
        let expense = create_expense_account(&db);

        // Set a 10% envelope allocation on the expense account.
        db.envelopes()
            .set_allocation(
                expense,
                crate::types::Percentage(10_000_000),
                crate::types::Percentage(0),
                None,
            )
            .expect("set allocation");

        // Post: cash receipt $1000 (triggers a fill of 10% = $100).
        post_balanced_je(&db, cash, revenue, 100_000_000_000, jan, date);

        // Verify fill was created ($100).
        let balance_before = db.envelopes().get_balance(expense).expect("balance");
        assert_eq!(
            balance_before,
            Money(10_000_000_000),
            "$100 earmarked before close"
        );

        // Post an expense too so closing entries are non-trivial.
        post_balanced_je(&db, expense, cash, 20_000_000_000, jan, date);

        // Execute year-end close.
        let closing_jes = generate_closing_entries(&db, fy_id).expect("generate");
        let closing_ids: Vec<JournalEntryId> = closing_jes
            .iter()
            .map(|je| db.journals().create_draft(je).expect("draft"))
            .collect();
        execute_year_end_close(&db, fy_id, &closing_ids, "Test Entity").expect("close");

        // Verify GL balances for expense zeroed out by year-end close.
        assert_eq!(
            fy_balance(&db, expense, fy_id),
            Money(0),
            "Year-end close should zero out expense GL balance"
        );

        // Envelope balance must still be $100 — it is independent of the GL close.
        let balance_after = db.envelopes().get_balance(expense).expect("balance");
        assert_eq!(
            balance_after,
            Money(10_000_000_000),
            "Envelope earmark must persist after year-end close"
        );
    }

    #[test]
    fn execute_year_end_close_writes_audit_entry() {
        use crate::db::audit_repo::{AuditFilter, AuditRepo};

        let (db, fy_id, jan) = setup();
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let cash = find_account_by_number(&db, "1110"); // Checking Account (non-placeholder)
        let revenue = create_revenue_account(&db);

        post_balanced_je(&db, cash, revenue, 5_000_000_000, jan, date);

        let closing_jes = generate_closing_entries(&db, fy_id).expect("generate");
        let closing_ids: Vec<JournalEntryId> = closing_jes
            .iter()
            .map(|je| db.journals().create_draft(je).expect("draft"))
            .collect();

        execute_year_end_close(&db, fy_id, &closing_ids, "Acme LLC").expect("close");

        let entries = AuditRepo::new(db.conn())
            .list(&AuditFilter {
                action_type: Some(AuditAction::YearEndClose),
                ..Default::default()
            })
            .expect("list audit");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entity_name, "Acme LLC");
    }
}
