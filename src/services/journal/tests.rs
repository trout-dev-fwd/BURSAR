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
    let rev_id = reverse_journal_entry(&db, je_id, reversal_date, "Test Entity").expect("reverse");

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

    let checking = find_account_id(&db, "1110");
    let owners_draw = find_account_id(&db, "3200"); // Owner's Draw (contra-equity)
    let envelope_acct = find_account_id(&db, "5100");

    set_ten_pct_allocation(&db, envelope_acct);

    // Owner's Draw: credit Checking, debit Owner's Draw.
    // (or any direction — the presence of Owner's Draw suppresses fills)
    let amount = Money(50_000_000_000);
    let entry = NewJournalEntry {
        entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
        memo: None,
        fiscal_period_id: jan_period,
        reversal_of_je_id: None,
        lines: vec![
            NewJournalEntryLine {
                account_id: owners_draw,
                debit_amount: amount,
                credit_amount: Money(0),
                line_memo: None,
                sort_order: 0,
            },
            NewJournalEntryLine {
                account_id: checking,
                debit_amount: Money(0),
                credit_amount: amount,
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

    let checking = find_account_id(&db, "1110");
    let owners_equity = find_account_id(&db, "3100"); // Owner's Capital (not contra)
    let envelope_acct = find_account_id(&db, "5100");

    set_ten_pct_allocation(&db, envelope_acct);

    // Capital contribution: debit Checking, credit Owner's Capital.
    let amount = Money(100_000_000_000); // $1000
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
                account_id: owners_equity,
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
    assert_eq!(
        balance,
        Money(10_000_000_000), // $100 = 10% of $1000
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

    // Post $1000 cash receipt.
    let amount = Money(100_000_000_000);
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
        Money(10_000_000_000), // $100
        "Balance should be $100 after fill"
    );

    // Reverse the JE (use the same period for simplicity).
    reverse_journal_entry(
        &db,
        je_id,
        NaiveDate::from_ymd_opt(2026, 1, 20).unwrap(),
        "Test Entity",
    )
    .expect("reverse");

    // Net envelope balance should be zero.
    let balance_after_reversal = db.envelopes().get_balance(envelope_acct).expect("balance");
    assert_eq!(
        balance_after_reversal,
        Money(0),
        "Net envelope balance should be zero after reversal"
    );

    let ledger = db.envelopes().get_ledger(envelope_acct).expect("ledger");
    assert_eq!(ledger.len(), 2, "Should have Fill + Reversal entries");
}

#[test]
fn reverse_non_cash_je_creates_no_envelope_entries() {
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
    reverse_journal_entry(
        &db,
        je_id,
        NaiveDate::from_ymd_opt(2026, 1, 20).unwrap(),
        "Test Entity",
    )
    .expect("reverse");

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
                debit_amount: Money(100_000_000_000),
                credit_amount: Money(0),
                line_memo: None,
                sort_order: 0,
            },
            NewJournalEntryLine {
                account_id: revenue,
                debit_amount: Money(0),
                credit_amount: Money(100_000_000_000),
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

// ── Two-tier fill algorithm tests (V5 Phase 2, Task 1) ───────────────────

/// Helper: post a cash receipt of `amount` (debit Checking, credit Revenue).
/// Returns the JournalEntryId of the posted entry.
fn post_cash_receipt(
    db: &EntityDb,
    period_id: FiscalPeriodId,
    amount: Money,
) -> crate::types::JournalEntryId {
    let checking = find_account_id(db, "1110");
    let revenue = find_account_id(db, "4100");
    let entry = NewJournalEntry {
        entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
        memo: None,
        fiscal_period_id: period_id,
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
    let je_id = db
        .journals()
        .create_draft(&entry)
        .expect("create cash receipt");
    post_journal_entry(db, je_id, "Test Entity").expect("post cash receipt");
    je_id
}

/// Helper: seed the envelope ledger with `amount` for `account_id` without
/// posting a cash receipt (avoids re-triggering the fill algorithm).
fn seed_envelope_balance(
    db: &EntityDb,
    period_id: FiscalPeriodId,
    account_id: crate::types::AccountId,
    amount: Money,
) {
    // Create a draft JE (don't post it) just to get a valid JE ID for the FK.
    let checking = find_account_id(db, "1110");
    let revenue = find_account_id(db, "4100");
    let entry = NewJournalEntry {
        entry_date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
        memo: Some("seed envelope balance".into()),
        fiscal_period_id: period_id,
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
    let je_id = db.journals().create_draft(&entry).expect("draft for seed");
    db.envelopes()
        .record_fill(account_id, amount, je_id)
        .expect("seed fill");
}

/// [TEST-FIRST] Primary fill with cap — capped to room, overflow accumulated.
/// Old code ignores cap → balance would be $100 instead of $50. This test FAILS
/// with the old single-tier algorithm.
#[test]
fn two_tier_primary_fill_respects_cap() {
    let db = make_entity_db();
    let (jan_period, _) = setup_fiscal_year(&db);
    let envelope_acct = find_account_id(&db, "5100");

    // 10% primary, cap $50, no secondary. Account starts empty.
    db.envelopes()
        .set_allocation(
            envelope_acct,
            crate::types::Percentage(10_000_000), // 10%
            crate::types::Percentage(0),
            Some(Money(5_000_000_000)), // $50 cap
        )
        .expect("set alloc");

    // Post $1000 cash receipt → primary = $100, but capped at $50.
    post_cash_receipt(&db, jan_period, Money(100_000_000_000));

    let balance = db.envelopes().get_balance(envelope_acct).expect("balance");
    assert_eq!(
        balance,
        Money(5_000_000_000), // $50
        "Primary fill should be capped at $50, not $100"
    );
}

/// [TEST-FIRST] Secondary fill receives overflow from capped primary.
/// Old code: no secondary fills, account B = $0. This test FAILS with old code.
#[test]
fn two_tier_secondary_fill_receives_overflow() {
    let db = make_entity_db();
    let (jan_period, _) = setup_fiscal_year(&db);
    let acct_a = find_account_id(&db, "5100");
    let acct_b = find_account_id(&db, "5200");

    // A: 10% primary, $50 cap. B: 50% secondary, no primary.
    db.envelopes()
        .set_allocation(
            acct_a,
            crate::types::Percentage(10_000_000),
            crate::types::Percentage(0),
            Some(Money(5_000_000_000)),
        )
        .expect("set A");
    db.envelopes()
        .set_allocation(
            acct_b,
            crate::types::Percentage(0),
            crate::types::Percentage(50_000_000), // 50% secondary
            None,
        )
        .expect("set B");

    // Post $1000: A primary = $100, cap hit → fills $50, overflow = $50.
    // B secondary = 50% of $50 overflow = $25.
    post_cash_receipt(&db, jan_period, Money(100_000_000_000));

    let bal_a = db.envelopes().get_balance(acct_a).expect("bal A");
    let bal_b = db.envelopes().get_balance(acct_b).expect("bal B");
    assert_eq!(bal_a, Money(5_000_000_000), "A should be $50 (capped)");
    assert_eq!(
        bal_b,
        Money(2_500_000_000),
        "B should be $25 (50% of $50 overflow)"
    );
}

/// [TEST-FIRST] Dual-allocation account: primary blocked by cap, secondary still fills.
/// Old code: primary fills $100 ignoring cap; no secondary. This test FAILS with old code.
#[test]
fn two_tier_dual_allocation_receives_secondary_when_primary_capped() {
    let db = make_entity_db();
    let (jan_period, _) = setup_fiscal_year(&db);
    let acct_a = find_account_id(&db, "5100");

    // A starts at $100 (at cap).
    seed_envelope_balance(&db, jan_period, acct_a, Money(10_000_000_000));

    // A: 10% primary, $100 cap, 5% secondary.
    db.envelopes()
        .set_allocation(
            acct_a,
            crate::types::Percentage(10_000_000), // 10%
            crate::types::Percentage(5_000_000),  // 5% secondary
            Some(Money(10_000_000_000)),          // $100 cap
        )
        .expect("set alloc");

    // Post $1000: primary = $100, cap hit (room=0) → overflow = $100.
    // Secondary: 5% of $100 = $5. A goes from $100 → $105.
    post_cash_receipt(&db, jan_period, Money(100_000_000_000));

    let balance = db.envelopes().get_balance(acct_a).expect("balance");
    assert_eq!(
        balance,
        Money(10_500_000_000), // $105
        "A should be $105: $100 seed + $5 secondary (primary blocked by cap)"
    );
}

/// [TEST-FIRST] Cap already reached → primary fill = $0, entire primary goes to overflow.
/// Old code: fills $100 ignoring cap. This test FAILS with old code.
#[test]
fn two_tier_cap_already_reached_no_primary_fill() {
    let db = make_entity_db();
    let (jan_period, _) = setup_fiscal_year(&db);
    let acct_a = find_account_id(&db, "5100");
    let acct_b = find_account_id(&db, "5200");

    // A at cap already.
    seed_envelope_balance(&db, jan_period, acct_a, Money(10_000_000_000)); // $100

    // A: 10% primary, $100 cap. B: 100% secondary.
    db.envelopes()
        .set_allocation(
            acct_a,
            crate::types::Percentage(10_000_000),
            crate::types::Percentage(0),
            Some(Money(10_000_000_000)),
        )
        .expect("set A");
    db.envelopes()
        .set_allocation(
            acct_b,
            crate::types::Percentage(0),
            crate::types::Percentage(100_000_000), // 100% secondary
            None,
        )
        .expect("set B");

    // Post $1000: A primary = $100, cap hit (room=0) → overflow = $100.
    // B secondary = 100% of $100 = $100.
    post_cash_receipt(&db, jan_period, Money(100_000_000_000));

    let bal_a = db.envelopes().get_balance(acct_a).expect("bal A");
    let bal_b = db.envelopes().get_balance(acct_b).expect("bal B");
    // A stays at $100 (no primary fill added).
    assert_eq!(
        bal_a,
        Money(10_000_000_000),
        "A should remain at $100 (cap reached)"
    );
    assert_eq!(
        bal_b,
        Money(10_000_000_000),
        "B should receive $100 (100% of overflow)"
    );
}

/// [TEST-FIRST] Partially capped: room < primary amount → partial fill + overflow.
/// Old code: fills full $100 ignoring cap. This test FAILS with old code.
#[test]
fn two_tier_partially_capped_fills_to_room() {
    let db = make_entity_db();
    let (jan_period, _) = setup_fiscal_year(&db);
    let acct_a = find_account_id(&db, "5100");
    let acct_b = find_account_id(&db, "5200");

    // A has $80 earmarked, cap is $100 → room = $20.
    seed_envelope_balance(&db, jan_period, acct_a, Money(8_000_000_000)); // $80

    // A: 10% primary, $100 cap. B: 100% secondary.
    db.envelopes()
        .set_allocation(
            acct_a,
            crate::types::Percentage(10_000_000),
            crate::types::Percentage(0),
            Some(Money(10_000_000_000)),
        )
        .expect("set A");
    db.envelopes()
        .set_allocation(
            acct_b,
            crate::types::Percentage(0),
            crate::types::Percentage(100_000_000),
            None,
        )
        .expect("set B");

    // Post $1000: primary = $100, room = $20 → fill $20, overflow = $80.
    // B secondary = 100% of $80 = $80.
    post_cash_receipt(&db, jan_period, Money(100_000_000_000));

    let bal_a = db.envelopes().get_balance(acct_a).expect("bal A");
    let bal_b = db.envelopes().get_balance(acct_b).expect("bal B");
    assert_eq!(
        bal_a,
        Money(10_000_000_000),
        "A: $80 seed + $20 fill = $100 (at cap)"
    );
    assert_eq!(
        bal_b,
        Money(8_000_000_000),
        "B: $80 secondary from overflow"
    );
}

/// [TEST-FIRST] No secondary allocations: overflow stays unearmarked.
/// Old code: doesn't generate overflow at all but fills $100. This test FAILS with old code.
#[test]
fn two_tier_overflow_stays_unearmarked_without_secondary() {
    let db = make_entity_db();
    let (jan_period, _) = setup_fiscal_year(&db);
    let acct_a = find_account_id(&db, "5100");

    // A: 10% primary, $50 cap, no secondary. Account starts empty.
    db.envelopes()
        .set_allocation(
            acct_a,
            crate::types::Percentage(10_000_000),
            crate::types::Percentage(0),
            Some(Money(5_000_000_000)),
        )
        .expect("set A");

    // Post $1000: primary = $100, cap hit → fill $50, overflow = $50 (goes nowhere).
    let je_id = post_cash_receipt(&db, jan_period, Money(100_000_000_000));

    let bal_a = db.envelopes().get_balance(acct_a).expect("bal A");
    assert_eq!(bal_a, Money(5_000_000_000), "A should be capped at $50");

    // Only one fill entry (the $50 primary fill; overflow is unearmarked).
    let fills = db.envelopes().get_fills_for_je(je_id).expect("fills");
    assert_eq!(
        fills.len(),
        1,
        "Only one fill (capped primary, no secondary)"
    );
    assert_eq!(fills[0].amount, Money(5_000_000_000), "Fill is $50");
}

/// [TEST-FIRST] Secondary < 100%: portion of overflow is unearmarked.
/// Old code: no secondary fills at all. This test FAILS with old code.
#[test]
fn two_tier_secondary_partial_leaves_remainder_unearmarked() {
    let db = make_entity_db();
    let (jan_period, _) = setup_fiscal_year(&db);
    let acct_a = find_account_id(&db, "5100");
    let acct_b = find_account_id(&db, "5200");

    // A: 10% primary, $50 cap. B: 40% secondary.
    // Overflow = $50. B gets 40% = $20. $30 stays unearmarked.
    db.envelopes()
        .set_allocation(
            acct_a,
            crate::types::Percentage(10_000_000),
            crate::types::Percentage(0),
            Some(Money(5_000_000_000)),
        )
        .expect("set A");
    db.envelopes()
        .set_allocation(
            acct_b,
            crate::types::Percentage(0),
            crate::types::Percentage(40_000_000),
            None,
        )
        .expect("set B");

    post_cash_receipt(&db, jan_period, Money(100_000_000_000));

    let bal_a = db.envelopes().get_balance(acct_a).expect("bal A");
    let bal_b = db.envelopes().get_balance(acct_b).expect("bal B");
    assert_eq!(bal_a, Money(5_000_000_000), "A capped at $50");
    assert_eq!(
        bal_b,
        Money(2_000_000_000),
        "B gets 40% of $50 overflow = $20"
    );
}

/// [TEST-FIRST] Zero cash receipt: no fills at all.
#[test]
fn two_tier_zero_cash_receipt_no_fills() {
    let db = make_entity_db();
    let (jan_period, _) = setup_fiscal_year(&db);
    let envelope_acct = find_account_id(&db, "5100");
    let expense = find_account_id(&db, "5200");
    let ap = find_account_id(&db, "2100");

    db.envelopes()
        .set_allocation(
            envelope_acct,
            crate::types::Percentage(10_000_000),
            crate::types::Percentage(5_000_000),
            Some(Money(10_000_000_000)),
        )
        .expect("set alloc");

    // Non-cash JE (expense + AP, no cash debit).
    let entry = NewJournalEntry {
        entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
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
    assert_eq!(balance, Money(0), "No fills for non-cash JE");
}

/// [TEST-FIRST] Reversal undoes all two-tier fills (primary + secondary).
/// This works because reversal reads fills from `get_fills_for_je` regardless
/// of how they were created. Should pass even with old code once tests 1-7 pass.
#[test]
fn two_tier_reversal_undoes_all_fills() {
    let db = make_entity_db();
    let (jan_period, _) = setup_fiscal_year(&db);
    let acct_a = find_account_id(&db, "5100");
    let acct_b = find_account_id(&db, "5200");

    // A: 10% primary, $50 cap. B: 100% secondary.
    db.envelopes()
        .set_allocation(
            acct_a,
            crate::types::Percentage(10_000_000),
            crate::types::Percentage(0),
            Some(Money(5_000_000_000)),
        )
        .expect("set A");
    db.envelopes()
        .set_allocation(
            acct_b,
            crate::types::Percentage(0),
            crate::types::Percentage(100_000_000),
            None,
        )
        .expect("set B");

    // Post $1000: A gets $50 (capped), B gets $50 (100% of overflow).
    let je_id = post_cash_receipt(&db, jan_period, Money(100_000_000_000));

    // Verify fills were created.
    let bal_a = db
        .envelopes()
        .get_balance(acct_a)
        .expect("bal A before reversal");
    let bal_b = db
        .envelopes()
        .get_balance(acct_b)
        .expect("bal B before reversal");
    assert_eq!(
        bal_a,
        Money(5_000_000_000),
        "A should be $50 before reversal"
    );
    assert_eq!(
        bal_b,
        Money(5_000_000_000),
        "B should be $50 before reversal"
    );

    // Reverse the JE.
    reverse_journal_entry(
        &db,
        je_id,
        NaiveDate::from_ymd_opt(2026, 1, 20).unwrap(),
        "Test Entity",
    )
    .expect("reverse");

    // Both balances should return to zero.
    let bal_a_after = db.envelopes().get_balance(acct_a).expect("bal A after");
    let bal_b_after = db.envelopes().get_balance(acct_b).expect("bal B after");
    assert_eq!(bal_a_after, Money(0), "A should be $0 after reversal");
    assert_eq!(bal_b_after, Money(0), "B should be $0 after reversal");
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
