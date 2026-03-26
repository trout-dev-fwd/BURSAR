//! V5 Phase 3 integration tests for the two-tier envelope fill algorithm.
//!
//! Covers scenarios that require 4 accounts or multi-step interactions
//! (full two-tier distribution + resume-after-spend) that would push
//! `tests.rs` beyond the 1,500-line file size limit.

use super::*;
use crate::db::EntityDb;
use crate::db::journal_repo::{NewJournalEntry, NewJournalEntryLine};
use crate::db::schema::{initialize_schema, seed_default_accounts};
use crate::types::{FiscalPeriodId, Money, Percentage};
use chrono::NaiveDate;
use rusqlite::Connection;

// ── Helpers ───────────────────────────────────────────────────────────────

fn make_entity_db() -> EntityDb {
    let conn = Connection::open_in_memory().expect("in-memory db");
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .expect("fk on");
    initialize_schema(&conn).expect("schema init");
    seed_default_accounts(&conn).expect("seed accounts");
    crate::db::entity_db_from_conn(conn)
}

fn setup_fiscal_year(db: &EntityDb) -> FiscalPeriodId {
    let fy_id = db.fiscal().create_fiscal_year(1, 2026).expect("create FY");
    let periods = db.fiscal().list_periods(fy_id).expect("list periods");
    periods[0].id
}

fn find_account_id(db: &EntityDb, number: &str) -> crate::types::AccountId {
    db.accounts()
        .list_all()
        .expect("list_all")
        .into_iter()
        .find(|a| a.number == number)
        .unwrap_or_else(|| panic!("Account {number} not found in seeded chart"))
        .id
}

/// Post a cash receipt (debit Checking 1110, credit Revenue 4100).
fn post_cash_receipt(db: &EntityDb, period_id: FiscalPeriodId, amount: Money) -> JournalEntryId {
    let checking = find_account_id(db, "1110");
    let revenue = find_account_id(db, "4100");
    let je_id = db
        .journals()
        .create_draft(&NewJournalEntry {
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
        })
        .expect("create cash receipt");
    post_journal_entry(db, je_id, "Test Entity").expect("post cash receipt");
    je_id
}

/// Seed an envelope balance without triggering the fill algorithm.
fn seed_envelope_balance(
    db: &EntityDb,
    period_id: FiscalPeriodId,
    account_id: crate::types::AccountId,
    amount: Money,
) {
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

// ── Tests ─────────────────────────────────────────────────────────────────

/// Full two-tier scenario with 4 accounts:
///   A (5100): primary 10%, cap $500, currently at cap
///   B (5200): primary 30%, no cap
///   C (5300): secondary 60%
///   D (5400): secondary 40%
///
/// Post $2,000 receipt:
///   - A primary $200 blocked (room=0), overflow += $200
///   - B primary $600 fills normally
///   - C secondary 60% of $200 overflow = $120
///   - D secondary 40% of $200 overflow = $80
#[test]
fn two_tier_full_scenario_distributes_overflow_to_secondaries() {
    let db = make_entity_db();
    let period = setup_fiscal_year(&db);
    let acct_a = find_account_id(&db, "5100");
    let acct_b = find_account_id(&db, "5200");
    let acct_c = find_account_id(&db, "5300");
    let acct_d = find_account_id(&db, "5400");

    // Seed A at its $500 cap.
    seed_envelope_balance(&db, period, acct_a, Money(50_000_000_000)); // $500

    // A: 10% primary, $500 cap.
    db.envelopes()
        .set_allocation(
            acct_a,
            Percentage(10_000_000),
            Percentage(0),
            Some(Money(50_000_000_000)),
        )
        .expect("set A");
    // B: 30% primary, no cap.
    db.envelopes()
        .set_allocation(acct_b, Percentage(30_000_000), Percentage(0), None)
        .expect("set B");
    // C: 60% secondary only.
    db.envelopes()
        .set_allocation(acct_c, Percentage(0), Percentage(60_000_000), None)
        .expect("set C");
    // D: 40% secondary only.
    db.envelopes()
        .set_allocation(acct_d, Percentage(0), Percentage(40_000_000), None)
        .expect("set D");

    // Post $2,000 receipt.
    post_cash_receipt(&db, period, Money(200_000_000_000));

    let bal_a = db.envelopes().get_balance(acct_a).expect("bal A");
    let bal_b = db.envelopes().get_balance(acct_b).expect("bal B");
    let bal_c = db.envelopes().get_balance(acct_c).expect("bal C");
    let bal_d = db.envelopes().get_balance(acct_d).expect("bal D");

    assert_eq!(
        bal_a,
        Money(50_000_000_000),
        "A: stays at $500 (cap hit, no secondary)"
    );
    assert_eq!(
        bal_b,
        Money(60_000_000_000),
        "B: $600 (30% of $2,000, uncapped)"
    );
    assert_eq!(
        bal_c,
        Money(12_000_000_000),
        "C: $120 (60% of $200 overflow)"
    );
    assert_eq!(bal_d, Money(8_000_000_000), "D: $80 (40% of $200 overflow)");
}

/// Resume-after-spend: primary fills resume once balance drops below cap.
///
///   A (5100): primary 10%, cap $500, seeded at $500 (at cap)
///   Transfer $100 out of A → balance $400, room = $100
///   Post $2,000 receipt → primary would be $200, but room = $100 → fill $100
///   Expected: A = $500 again (back at cap).
#[test]
fn two_tier_resume_after_spend_fills_to_cap() {
    let db = make_entity_db();
    let period = setup_fiscal_year(&db);
    let acct_a = find_account_id(&db, "5100");
    let acct_b = find_account_id(&db, "5200"); // transfer destination (no allocation)

    // Seed A at $500 cap.
    seed_envelope_balance(&db, period, acct_a, Money(50_000_000_000)); // $500

    // Seed B enough to be a valid envelope (record_transfer checks source balance).
    // (B is just the destination; we don't set an allocation on it.)

    // A: 10% primary, $500 cap.
    db.envelopes()
        .set_allocation(
            acct_a,
            Percentage(10_000_000),
            Percentage(0),
            Some(Money(50_000_000_000)),
        )
        .expect("set A");

    // Transfer $100 out of A (simulate spending from this envelope).
    db.envelopes()
        .record_transfer(acct_a, acct_b, Money(10_000_000_000)) // $100
        .expect("transfer out");

    // A now has $400; room under $500 cap = $100.
    let bal_before = db.envelopes().get_balance(acct_a).expect("before");
    assert_eq!(
        bal_before,
        Money(40_000_000_000),
        "A should be $400 after transfer"
    );

    // Post $2,000 receipt: primary = $200, room = $100 → fills $100.
    post_cash_receipt(&db, period, Money(200_000_000_000));

    let bal_after = db.envelopes().get_balance(acct_a).expect("after");
    assert_eq!(
        bal_after,
        Money(50_000_000_000),
        "A should be back at $500 after partial fill"
    );
}
