//! Full lifecycle integration test covering the complete application workflow.
//!
//! This test exercises: entity creation → seed accounts → fiscal year → journal entries →
//! posting → GL view → AR with partial payment → envelope fills → CIP → place in service →
//! depreciation → period close → all 8 reports → year-end close → closing entries.

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::NaiveDate;
    use rusqlite::Connection;

    use crate::db::{
        ar_repo::NewArItem,
        entity_db_from_conn,
        journal_repo::{JournalFilter, NewJournalEntry, NewJournalEntryLine},
        schema::{initialize_schema, seed_default_accounts},
    };
    use crate::reports::{
        Report, ReportParams, account_detail::AccountDetail, ap_aging::ApAging, ar_aging::ArAging,
        balance_sheet::BalanceSheet, cash_flow::CashFlow, fixed_asset_schedule::FixedAssetSchedule,
        income_statement::IncomeStatement, trial_balance::TrialBalance, write_report,
    };
    use crate::services::{
        fiscal::{execute_year_end_close, generate_closing_entries},
        journal::post_journal_entry,
    };
    use crate::types::{AccountId, ArApStatus, JournalEntryStatus, Money, Percentage};

    fn account_id(db: &crate::db::EntityDb, number: &str) -> AccountId {
        let id: i64 = db
            .conn()
            .query_row(
                "SELECT id FROM accounts WHERE number = ?1",
                rusqlite::params![number],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| panic!("account {number} not found"));
        AccountId::from(id)
    }

    #[test]
    fn full_lifecycle_integration_test() {
        // ── Step 1: Create in-memory database ──────────────────────────────────
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema init");
        seed_default_accounts(&conn).expect("seed accounts");
        let db = entity_db_from_conn(conn);

        // ── Step 2: Create fiscal year 2026 (January start) ────────────────────
        let fy_id = db
            .fiscal()
            .create_fiscal_year(1, 2026)
            .expect("create FY 2026");
        let periods = db.fiscal().list_periods(fy_id).expect("list periods");
        assert_eq!(periods.len(), 12, "FY 2026 must have 12 periods");
        let jan_id = periods[0].id;
        let feb_id = periods[1].id;
        let mar_id = periods[2].id;

        // ── Step 3: Gather account IDs from seeded chart of accounts ───────────
        let checking_id = account_id(&db, "1110"); // Checking Account (cash)
        let ar_acct_id = account_id(&db, "1200"); // Accounts Receivable
        let cip_id = account_id(&db, "1400"); // Construction in Progress
        let buildings_id = account_id(&db, "1520"); // Buildings
        let accum_depr_id = account_id(&db, "1521"); // Accumulated Depreciation - Buildings
        let revenue_id = account_id(&db, "4100"); // Service Revenue
        let rent_id = account_id(&db, "5100"); // Rent (expense)
        let depr_exp_id = account_id(&db, "5700"); // Depreciation Expense
        let retained_id = account_id(&db, "3300"); // Retained Earnings

        // ── Step 4: Create and post a revenue JE ───────────────────────────────
        // Dr Checking $1,000 / Cr Service Revenue $1,000
        let revenue_je_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                memo: Some("Service revenue received".to_string()),
                fiscal_period_id: jan_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: checking_id,
                        debit_amount: Money(100_000_000_000), // $1,000
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: revenue_id,
                        debit_amount: Money(0),
                        credit_amount: Money(100_000_000_000), // $1,000
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create revenue draft");
        post_journal_entry(&db, revenue_je_id, "Test Entity").expect("post revenue JE");

        // ── Step 5: Create and post an expense JE ──────────────────────────────
        // Dr Rent $500 / Cr Checking $500
        let expense_je_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 20).unwrap(),
                memo: Some("January rent payment".to_string()),
                fiscal_period_id: jan_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: rent_id,
                        debit_amount: Money(50_000_000_000), // $500
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: checking_id,
                        debit_amount: Money(0),
                        credit_amount: Money(50_000_000_000), // $500
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create expense draft");
        post_journal_entry(&db, expense_je_id, "Test Entity").expect("post expense JE");
        let _ = expense_je_id; // used implicitly via post

        // ── Step 6: View GL for Checking Account ───────────────────────────────
        let gl_rows = db
            .journals()
            .list_lines_for_account(checking_id, None)
            .expect("list GL rows for Checking");
        assert_eq!(gl_rows.len(), 2, "Checking should have 2 GL rows");
        // Debit $1,000 then credit $500 → running balance = $500
        assert_eq!(
            gl_rows[1].running_balance,
            Money(50_000_000_000),
            "Checking running balance should be $500"
        );

        // ── Step 7: Create AR item and record partial payment ───────────────────
        let ar_item_id = db
            .ar()
            .create_item(&NewArItem {
                account_id: ar_acct_id,
                customer_name: "Acme Corp".to_string(),
                description: Some("Invoice #001".to_string()),
                amount: Money(100_000_000_000), // $1,000
                due_date: NaiveDate::from_ymd_opt(2026, 2, 15).unwrap(),
                originating_je_id: revenue_je_id,
            })
            .expect("create AR item");

        // Post a partial payment JE: Dr Checking $400, Cr AR $400
        let payment_je_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
                memo: Some("Partial payment from Acme Corp".to_string()),
                fiscal_period_id: feb_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: checking_id,
                        debit_amount: Money(40_000_000_000), // $400
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: ar_acct_id,
                        debit_amount: Money(0),
                        credit_amount: Money(40_000_000_000), // $400
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create payment draft");
        post_journal_entry(&db, payment_je_id, "Test Entity").expect("post payment JE");

        db.ar()
            .record_payment(
                ar_item_id,
                payment_je_id,
                Money(40_000_000_000),
                NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            )
            .expect("record AR partial payment");

        let (ar_item, payments) = db
            .ar()
            .get_with_payments(ar_item_id)
            .expect("get AR item with payments");
        assert_eq!(payments.len(), 1, "Should have one payment");
        assert_eq!(
            ar_item.status,
            ArApStatus::Partial,
            "AR item should be Partial after $400 of $1,000 paid"
        );

        // ── Step 8: Set envelope allocation and post cash receipt ───────────────
        // Allocate 20% of incoming cash to the Rent envelope
        db.envelopes()
            .set_allocation(rent_id, Percentage(20_000_000)) // 20%
            .expect("set envelope allocation for Rent");

        // Post a cash receipt: Dr Checking $2,000 / Cr Service Revenue $2,000
        // This triggers envelope fills because Checking is a cash account
        let receipt_je_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 2, 10).unwrap(),
                memo: Some("Cash receipt from client".to_string()),
                fiscal_period_id: feb_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: checking_id,
                        debit_amount: Money(200_000_000_000), // $2,000
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: revenue_id,
                        debit_amount: Money(0),
                        credit_amount: Money(200_000_000_000), // $2,000
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create cash receipt draft");
        post_journal_entry(&db, receipt_je_id, "Test Entity").expect("post cash receipt JE");

        // Verify envelope fills: 20% of $2,000 = $400 should be in the Rent envelope
        let fills = db
            .envelopes()
            .get_fills_for_je(receipt_je_id)
            .expect("get fills for receipt JE");
        assert!(
            !fills.is_empty(),
            "Envelope fills should have been created for cash receipt"
        );
        let rent_envelope_balance = db
            .envelopes()
            .get_balance(rent_id)
            .expect("get Rent envelope balance");
        assert_eq!(
            rent_envelope_balance,
            Money(40_000_000_000),
            "Rent envelope should hold $400 (20% of $2,000)"
        );

        // ── Step 9: Fund CIP account ────────────────────────────────────────────
        // Dr Construction in Progress $12,000 / Cr Checking $12,000
        let cip_je_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 2, 15).unwrap(),
                memo: Some("Construction costs capitalized".to_string()),
                fiscal_period_id: feb_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: cip_id,
                        debit_amount: Money(1_200_000_000_000), // $12,000
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: checking_id,
                        debit_amount: Money(0),
                        credit_amount: Money(1_200_000_000_000), // $12,000
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create CIP funding draft");
        post_journal_entry(&db, cip_je_id, "Test Entity").expect("post CIP funding JE");

        // ── Step 10: Place CIP in service ───────────────────────────────────────
        // Buildings placed in service Feb 28, 2026; 12-month useful life
        let _pis_je_id = db
            .assets()
            .place_in_service(
                cip_id,
                buildings_id,
                NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
                12,
                Some(accum_depr_id),
                Some(depr_exp_id),
            )
            .expect("place CIP in service");

        let assets = db.assets().list_assets().expect("list fixed assets");
        assert_eq!(
            assets.len(),
            1,
            "Should have one fixed asset after place-in-service"
        );
        assert_eq!(
            assets[0].detail.cost_basis,
            Money(1_200_000_000_000),
            "Buildings cost basis should be $12,000"
        );

        // ── Step 11: Generate and post depreciation entries ─────────────────────
        // as_of_period = Mar 2026 — Buildings placed in service Feb 28, first depr month is Mar 1
        let (depr_entries, depr_warn) = db
            .assets()
            .generate_pending_depreciation(mar_id)
            .expect("generate pending depreciation");
        assert!(
            depr_warn.is_none(),
            "No depreciation warning expected: {depr_warn:?}"
        );
        assert!(
            !depr_entries.is_empty(),
            "Should generate at least one depreciation entry"
        );

        for entry in &depr_entries {
            let je_id = db
                .journals()
                .create_draft(entry)
                .expect("create depreciation draft");
            post_journal_entry(&db, je_id, "Test Entity").expect("post depreciation JE");
        }

        let assets_after = db
            .assets()
            .list_assets()
            .expect("list assets after depreciation");
        assert!(
            assets_after[0].accumulated_depreciation.0 > 0,
            "Accumulated depreciation should be positive after posting depr entries"
        );

        // ── Step 12: Close January 2026 period ──────────────────────────────────
        db.fiscal()
            .close_period(jan_id, "Test Entity")
            .expect("close January 2026 period");

        let periods_after = db
            .fiscal()
            .list_periods(fy_id)
            .expect("list periods after close");
        assert!(periods_after[0].is_closed, "January 2026 should be closed");

        // ── Step 13: Generate all 8 reports and verify output files exist ───────
        let output_dir = std::env::temp_dir().join("bursar_integration_test_reports");
        let _ = std::fs::remove_dir_all(&output_dir); // clean slate

        let entity_name = "Test Entity".to_string();
        let fy_start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let fy_end = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();

        let params_as_of = ReportParams {
            entity_name: entity_name.clone(),
            as_of_date: Some(fy_end),
            date_range: None,
            account_id: None,
        };
        let params_range = ReportParams {
            entity_name: entity_name.clone(),
            as_of_date: None,
            date_range: Some((fy_start, fy_end)),
            account_id: None,
        };
        let params_acct_detail = ReportParams {
            entity_name: entity_name.clone(),
            as_of_date: None,
            date_range: Some((fy_start, fy_end)),
            account_id: Some(checking_id),
        };

        let reports: Vec<(&dyn Report, &ReportParams)> = vec![
            (&TrialBalance, &params_as_of),
            (&BalanceSheet, &params_as_of),
            (&IncomeStatement, &params_range),
            (&CashFlow, &params_range),
            (&ArAging, &params_as_of),
            (&ApAging, &params_as_of),
            (&FixedAssetSchedule, &params_as_of),
            (&AccountDetail, &params_acct_detail),
        ];

        let mut report_paths: Vec<PathBuf> = Vec::new();
        for (report, params) in &reports {
            let content = report
                .generate(&db, params)
                .unwrap_or_else(|e| panic!("generate '{}' failed: {e}", report.name()));
            let path = write_report(&content, report.name(), &output_dir)
                .unwrap_or_else(|e| panic!("write '{}' failed: {e}", report.name()));
            assert!(
                path.exists(),
                "Report file should exist: {}",
                path.display()
            );
            report_paths.push(path);
        }
        assert_eq!(
            report_paths.len(),
            8,
            "All 8 report files must be generated"
        );

        // ── Step 14: Year-end close ──────────────────────────────────────────────
        let closing_entries =
            generate_closing_entries(&db, fy_id).expect("generate closing entries");
        assert!(
            !closing_entries.is_empty(),
            "Should produce closing entries (revenue and expense activity exist)"
        );

        let closing_ids: Vec<_> = closing_entries
            .iter()
            .map(|entry| {
                db.journals()
                    .create_draft(entry)
                    .expect("create closing entry draft")
            })
            .collect();

        execute_year_end_close(&db, fy_id, &closing_ids, "Test Entity")
            .expect("execute year-end close");

        // Verify fiscal year is marked closed
        let is_closed: i32 = db
            .conn()
            .query_row(
                "SELECT is_closed FROM fiscal_years WHERE id = ?1",
                rusqlite::params![i64::from(fy_id)],
                |row| row.get(0),
            )
            .expect("query is_closed");
        assert_eq!(
            is_closed, 1,
            "Fiscal year 2026 should be closed after year-end close"
        );

        // Verify closing entries were posted
        let all_posted = db
            .journals()
            .list(&JournalFilter {
                status: Some(JournalEntryStatus::Posted),
                from_date: None,
                to_date: None,
            })
            .expect("list posted JEs");
        assert!(
            all_posted.len() >= closing_ids.len() + 5,
            "Should have several posted JEs including closing entries"
        );

        // Verify Retained Earnings has a credit balance (net income > 0):
        // Revenue = $1,000 + $2,000 = $3,000; Expenses = $500 + $1,000 depr = $1,500 → profit
        let re_balance = db
            .accounts()
            .get_balance(retained_id)
            .expect("get Retained Earnings balance");
        assert!(
            re_balance.0 < 0,
            "Retained Earnings should have a credit (negative debit-net) balance after year-end close; got {re_balance}"
        );

        // ── Cleanup ──────────────────────────────────────────────────────────────
        let _ = std::fs::remove_dir_all(&output_dir);
    }

    // ── Transfer match integration tests ─────────────────────────────────────

    /// Helper: create a minimal in-memory DB with fiscal year 2026.
    fn setup_transfer_test_db() -> (crate::db::EntityDb, crate::types::FiscalPeriodId) {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        initialize_schema(&conn).expect("schema");
        seed_default_accounts(&conn).expect("seed");
        let db = entity_db_from_conn(conn);
        let fy_id = db
            .fiscal()
            .create_fiscal_year(1, 2026)
            .expect("fiscal year");
        let periods = db.fiscal().list_periods(fy_id).expect("list periods");
        let jan_id = periods[0].id;
        (db, jan_id)
    }

    #[test]
    fn confirmed_transfer_match_links_second_import_ref() {
        // Simulates: Bank A JE has import_ref stored, Bank B confirms the match →
        // second import_ref is added to the junction table, no new JE created.
        let (db, jan_id) = setup_transfer_test_db();
        let jan15 = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        // Step 1: Bank A import creates a draft JE with import_ref.
        let bank_a_ref = "BankA|2026-01-15|Transfer|50000000000";
        let je_id = db
            .journals()
            .create_draft_with_import_ref(
                &NewJournalEntry {
                    entry_date: jan15,
                    memo: Some("Import: Transfer".to_string()),
                    fiscal_period_id: jan_id,
                    reversal_of_je_id: None,
                    lines: vec![NewJournalEntryLine {
                        account_id: account_id(&db, "1110"),
                        debit_amount: Money(0),
                        credit_amount: Money(50_000_000_000),
                        line_memo: None,
                        sort_order: 0,
                    }],
                },
                Some(bank_a_ref),
            )
            .expect("create Bank A draft");

        // Step 2: Bank B confirms the match — stores its import_ref on the same JE.
        let bank_b_ref = "BankB|2026-01-16|ACH Deposit|50000000000";
        db.import_refs()
            .insert(je_id, bank_b_ref)
            .expect("link Bank B import_ref");

        // Verify: junction table has both refs for the same JE.
        let refs = db.import_refs().get_for_je(je_id).expect("get_for_je");
        assert_eq!(refs.len(), 2);
        assert!(refs.contains(&bank_a_ref.to_string()));
        assert!(refs.contains(&bank_b_ref.to_string()));

        // Verify: no new JE was created (still only one JE exists).
        let all_jes = db
            .journals()
            .list(&JournalFilter::default())
            .expect("list JEs");
        assert_eq!(
            all_jes.len(),
            1,
            "only the original Bank A draft should exist"
        );
    }

    #[test]
    fn confirmed_match_import_ref_detected_as_duplicate_on_reimport() {
        // After confirming a transfer match, both import_refs should be in the
        // junction table and detected as duplicates on subsequent imports.
        let (db, jan_id) = setup_transfer_test_db();
        let jan15 = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        let bank_a_ref = "BankA|2026-01-15|Transfer Out|50000000000";
        let bank_b_ref = "BankB|2026-01-16|Transfer In|50000000000";

        // Bank A import.
        let je_id = db
            .journals()
            .create_draft_with_import_ref(
                &NewJournalEntry {
                    entry_date: jan15,
                    memo: None,
                    fiscal_period_id: jan_id,
                    reversal_of_je_id: None,
                    lines: vec![NewJournalEntryLine {
                        account_id: account_id(&db, "1110"),
                        debit_amount: Money(0),
                        credit_amount: Money(50_000_000_000),
                        line_memo: None,
                        sort_order: 0,
                    }],
                },
                Some(bank_a_ref),
            )
            .expect("Bank A draft");

        // Confirm transfer: link Bank B ref to the same JE.
        db.import_refs()
            .insert(je_id, bank_b_ref)
            .expect("link Bank B ref");

        // Re-import Bank B: import_ref exists → duplicate detected.
        assert!(
            db.import_refs().exists(bank_b_ref).expect("exists check"),
            "Bank B ref should be detected as duplicate"
        );

        // Re-import Bank A: original import_ref exists → duplicate detected.
        assert!(
            db.import_refs().exists(bank_a_ref).expect("exists check"),
            "Bank A ref should still be detected as duplicate"
        );
    }

    #[test]
    fn rejected_transfer_match_creates_new_draft() {
        // Simulates: Bank B transaction was flagged as transfer match but user rejected it.
        // A new draft JE is created with the bank line only (no contra account).
        let (db, jan_id) = setup_transfer_test_db();
        let jan16 = NaiveDate::from_ymd_opt(2026, 1, 16).unwrap();

        let bank_b_ref = "BankB|2026-01-16|ACH Deposit|50000000000";

        // Rejected match → create a draft with just the bank line.
        let checking_id = account_id(&db, "1110");
        let je_id = db
            .journals()
            .create_draft_with_import_ref(
                &NewJournalEntry {
                    entry_date: jan16,
                    memo: Some("Import: ACH Deposit".to_string()),
                    fiscal_period_id: jan_id,
                    reversal_of_je_id: None,
                    lines: vec![NewJournalEntryLine {
                        account_id: checking_id,
                        debit_amount: Money(50_000_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    }],
                },
                Some(bank_b_ref),
            )
            .expect("rejected transfer draft");

        // Verify: a new JE was created.
        let all_jes = db
            .journals()
            .list(&JournalFilter::default())
            .expect("list JEs");
        assert_eq!(all_jes.len(), 1, "one new draft for the rejected match");

        // Verify: import_ref is stored so future re-imports are detected as duplicates.
        assert!(
            db.import_refs().exists(bank_b_ref).expect("exists"),
            "rejected match import_ref should be stored"
        );

        // Verify: JE has only one line (no contra account).
        let (_je, lines) = db
            .journals()
            .get_with_lines(je_id)
            .expect("get JE with lines");
        assert_eq!(lines.len(), 1, "rejected match draft has only bank line");
    }

    #[test]
    fn mix_of_confirmed_and_rejected_transfer_matches() {
        // Confirmed match: second import_ref linked to existing JE.
        // Rejected match: new draft JE created.
        let (db, jan_id) = setup_transfer_test_db();
        let jan15 = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let jan16 = NaiveDate::from_ymd_opt(2026, 1, 16).unwrap();

        let bank_a_ref = "BankA|2026-01-15|Wire Out|100000000000";
        let confirmed_b_ref = "BankB|2026-01-15|Wire In|100000000000";
        let rejected_b_ref = "BankB|2026-01-16|Random Deposit|20000000000";

        // Bank A JE (will be confirmed match target).
        let je_a = db
            .journals()
            .create_draft_with_import_ref(
                &NewJournalEntry {
                    entry_date: jan15,
                    memo: None,
                    fiscal_period_id: jan_id,
                    reversal_of_je_id: None,
                    lines: vec![NewJournalEntryLine {
                        account_id: account_id(&db, "1110"),
                        debit_amount: Money(0),
                        credit_amount: Money(100_000_000_000),
                        line_memo: None,
                        sort_order: 0,
                    }],
                },
                Some(bank_a_ref),
            )
            .expect("Bank A draft");

        // Confirmed match: link confirmed_b_ref to je_a.
        db.import_refs()
            .insert(je_a, confirmed_b_ref)
            .expect("link confirmed ref");

        // Rejected match: create a new draft.
        db.journals()
            .create_draft_with_import_ref(
                &NewJournalEntry {
                    entry_date: jan16,
                    memo: Some("Import: Random Deposit".to_string()),
                    fiscal_period_id: jan_id,
                    reversal_of_je_id: None,
                    lines: vec![NewJournalEntryLine {
                        account_id: account_id(&db, "1110"),
                        debit_amount: Money(20_000_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    }],
                },
                Some(rejected_b_ref),
            )
            .expect("rejected draft");

        // Verify: je_a has both import_refs.
        let refs_a = db.import_refs().get_for_je(je_a).expect("get_for_je");
        assert_eq!(refs_a.len(), 2);
        assert!(refs_a.contains(&bank_a_ref.to_string()));
        assert!(refs_a.contains(&confirmed_b_ref.to_string()));

        // Verify: two JEs total (je_a + rejected draft).
        let all_jes = db.journals().list(&JournalFilter::default()).expect("list");
        assert_eq!(all_jes.len(), 2, "je_a + rejected draft = 2 JEs");

        // Verify: all three import_refs detected as duplicates.
        assert!(db.import_refs().exists(bank_a_ref).expect("exists"));
        assert!(db.import_refs().exists(confirmed_b_ref).expect("exists"));
        assert!(db.import_refs().exists(rejected_b_ref).expect("exists"));
    }

    // ── End-to-end transfer detection tests ──────────────────────────────────
    // These tests use find_transfer_matches directly to verify the detection
    // logic (tolerance, date range, multiple matches) and check_duplicates to
    // verify that confirmed import_refs prevent re-import.

    // $3 tolerance = 300_000_000 internal units. Day range = 3.
    const TOLERANCE: Money = Money(300_000_000);
    const DAY_RANGE: i64 = 3;

    /// Creates a single-line draft JE with an import_ref for transfer detection tests.
    fn make_transfer_draft_je(
        db: &crate::db::EntityDb,
        period_id: crate::types::FiscalPeriodId,
        date: NaiveDate,
        signed_amount: Money, // positive = debit, negative = credit on bank line
        import_ref: &str,
    ) -> crate::types::JournalEntryId {
        let (debit, credit) = if signed_amount.0 >= 0 {
            (signed_amount, Money(0))
        } else {
            (Money(0), Money(-signed_amount.0))
        };
        db.journals()
            .create_draft_with_import_ref(
                &NewJournalEntry {
                    entry_date: date,
                    memo: None,
                    fiscal_period_id: period_id,
                    reversal_of_je_id: None,
                    lines: vec![NewJournalEntryLine {
                        account_id: account_id(db, "1110"),
                        debit_amount: debit,
                        credit_amount: credit,
                        line_memo: None,
                        sort_order: 0,
                    }],
                },
                Some(import_ref),
            )
            .expect("transfer draft JE")
    }

    #[test]
    fn end_to_end_cross_bank_transfer_scenario() {
        // Full scenario:
        // 1. Import Bank A with -$500 on Jan 15 → draft JE created
        // 2. Import Bank B with +$500 on Jan 16 → find_transfer_matches detects Bank A draft
        // 3. Confirm match → link Bank B import_ref to the Bank A JE
        // 4. Re-import Bank B → detected as duplicate
        // 5. Re-import Bank A → detected as duplicate
        let (db, jan_id) = setup_transfer_test_db();
        let jan15 = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let jan16 = NaiveDate::from_ymd_opt(2026, 1, 16).unwrap();

        let bank_a_ref = "BankA|2026-01-15|Transfer|50000000000";
        let bank_b_ref = "BankB|2026-01-16|ACH Deposit Chase|50000000000";

        // Step 1: Bank A import — -$500 withdrawal (credit on checking line).
        let je_a = make_transfer_draft_je(&db, jan_id, jan15, Money(-50_000_000_000), bank_a_ref);

        // Step 2: Bank B +$500 transaction — find_transfer_matches detects Bank A draft.
        // Bank B amount is +$500; negated = -$500; Bank A line signed amount = -$500. Match!
        let matches = db
            .journals()
            .find_transfer_matches(Money(50_000_000_000), jan16, TOLERANCE, DAY_RANGE)
            .expect("find_transfer_matches");
        assert_eq!(
            matches.len(),
            1,
            "Bank B +$500 should match Bank A -$500 draft"
        );
        assert_eq!(matches[0].je_id, je_a);

        // Step 3: Confirm — link Bank B ref to Bank A JE.
        db.import_refs()
            .insert(je_a, bank_b_ref)
            .expect("link Bank B ref");

        // Step 4: Re-import Bank B — import_ref exists → duplicate.
        let existing_refs = db.journals().get_recent_import_refs(90).expect("refs");
        use crate::ai::csv_import::check_duplicates;
        let bank_b_txn = vec![crate::ai::NormalizedTransaction {
            date: jan16,
            description: "ACH Deposit Chase".to_string(),
            amount: Money(50_000_000_000),
            import_ref: bank_b_ref.to_string(),
            raw_row: String::new(),
        }];
        let (unique, dups) = check_duplicates(&bank_b_txn, &existing_refs);
        assert_eq!(
            dups.len(),
            1,
            "Bank B re-import should be detected as duplicate"
        );
        assert!(unique.is_empty());

        // Step 5: Re-import Bank A — original ref exists → duplicate.
        let bank_a_txn = vec![crate::ai::NormalizedTransaction {
            date: jan15,
            description: "Transfer".to_string(),
            amount: Money(-50_000_000_000),
            import_ref: bank_a_ref.to_string(),
            raw_row: String::new(),
        }];
        let (unique_a, dups_a) = check_duplicates(&bank_a_txn, &existing_refs);
        assert_eq!(
            dups_a.len(),
            1,
            "Bank A re-import should be detected as duplicate"
        );
        assert!(unique_a.is_empty());
    }

    #[test]
    fn transfer_detection_amount_within_tolerance_matches() {
        // -$502 vs +$500: difference = $2, within $3 tolerance → should match.
        let (db, jan_id) = setup_transfer_test_db();
        let jan15 = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        // Bank A draft: -$502 (50_200_000_000 internal units credit).
        make_transfer_draft_je(
            &db,
            jan_id,
            jan15,
            Money(-50_200_000_000),
            "BankA|2026-01-15|Wire Out|50200000000",
        );

        // Bank B: +$500 → negated = -$500. Tolerance ±$3 → range [-$503, -$497].
        // Bank A line = -$502, which falls in [-$503, -$497]. Should match.
        let matches = db
            .journals()
            .find_transfer_matches(Money(50_000_000_000), jan15, TOLERANCE, DAY_RANGE)
            .expect("find_transfer_matches");
        assert_eq!(
            matches.len(),
            1,
            "-$502 vs +$500 within $3 tolerance should match"
        );
    }

    #[test]
    fn transfer_detection_amount_outside_tolerance_does_not_match() {
        // -$510 vs +$500: difference = $10, outside $3 tolerance → no match.
        let (db, jan_id) = setup_transfer_test_db();
        let jan15 = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        make_transfer_draft_je(
            &db,
            jan_id,
            jan15,
            Money(-51_000_000_000), // -$510
            "BankA|2026-01-15|Wire Out|51000000000",
        );

        // Bank B: +$500 → tolerance range [-$503, -$497]. -$510 is outside. No match.
        let matches = db
            .journals()
            .find_transfer_matches(Money(50_000_000_000), jan15, TOLERANCE, DAY_RANGE)
            .expect("find_transfer_matches");
        assert!(
            matches.is_empty(),
            "-$510 vs +$500 exceeds $3 tolerance, should not match"
        );
    }

    #[test]
    fn transfer_detection_date_within_3_days_matches() {
        // Bank A on Jan 12; Bank B on Jan 15 (3-day gap, exactly at boundary).
        let (db, jan_id) = setup_transfer_test_db();
        let jan12 = NaiveDate::from_ymd_opt(2026, 1, 12).unwrap();
        let jan15 = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        make_transfer_draft_je(
            &db,
            jan_id,
            jan12,
            Money(-50_000_000_000),
            "BankA|2026-01-12|Transfer|50000000000",
        );

        // Jan 15 query with day_range=3 → window is Jan 12 to Jan 18. Jan 12 is in window.
        let matches = db
            .journals()
            .find_transfer_matches(Money(50_000_000_000), jan15, TOLERANCE, DAY_RANGE)
            .expect("find_transfer_matches");
        assert_eq!(matches.len(), 1, "3-day gap is within range, should match");
    }

    #[test]
    fn transfer_detection_date_4_days_does_not_match() {
        // Bank A on Jan 11; Bank B on Jan 15 (4-day gap, outside boundary).
        let (db, jan_id) = setup_transfer_test_db();
        let jan11 = NaiveDate::from_ymd_opt(2026, 1, 11).unwrap();
        let jan15 = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        make_transfer_draft_je(
            &db,
            jan_id,
            jan11,
            Money(-50_000_000_000),
            "BankA|2026-01-11|Transfer|50000000000",
        );

        // Jan 15 query with day_range=3 → window is Jan 12 to Jan 18. Jan 11 is outside.
        let matches = db
            .journals()
            .find_transfer_matches(Money(50_000_000_000), jan15, TOLERANCE, DAY_RANGE)
            .expect("find_transfer_matches");
        assert!(
            matches.is_empty(),
            "4-day gap exceeds range, should not match"
        );
    }

    #[test]
    fn transfer_detection_multiple_matches_returns_all() {
        // Two drafts both match → find_transfer_matches returns 2.
        // The caller (run_pass1) skips single-match flagging and sends to Pass 2.
        let (db, jan_id) = setup_transfer_test_db();
        let jan15 = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();

        make_transfer_draft_je(
            &db,
            jan_id,
            jan15,
            Money(-50_000_000_000),
            "BankA|2026-01-15|Wire 1|50000000000",
        );
        make_transfer_draft_je(
            &db,
            jan_id,
            jan15,
            Money(-50_000_000_000),
            "BankB|2026-01-15|Wire 2|50000000000",
        );

        let matches = db
            .journals()
            .find_transfer_matches(Money(50_000_000_000), jan15, TOLERANCE, DAY_RANGE)
            .expect("find_transfer_matches");
        assert_eq!(
            matches.len(),
            2,
            "multiple matching drafts should all be returned"
        );
    }
}
