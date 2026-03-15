use anyhow::Result;
use rusqlite::{Connection, params};

/// Creates all 14 tables in a single transaction and enables WAL mode and foreign keys.
/// Safe to call on a new database; uses `CREATE TABLE IF NOT EXISTS`.
pub fn initialize_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    conn.execute_batch(
        "
        BEGIN;

        CREATE TABLE IF NOT EXISTS accounts (
            id              INTEGER PRIMARY KEY,
            number          TEXT    NOT NULL UNIQUE,
            name            TEXT    NOT NULL,
            account_type    TEXT    NOT NULL,
            parent_id       INTEGER REFERENCES accounts(id),
            is_active       INTEGER NOT NULL DEFAULT 1,
            is_contra       INTEGER NOT NULL DEFAULT 0,
            is_placeholder  INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT    NOT NULL,
            updated_at      TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS fixed_asset_details (
            id                    INTEGER PRIMARY KEY,
            account_id            INTEGER NOT NULL UNIQUE REFERENCES accounts(id),
            cost_basis            INTEGER NOT NULL,
            in_service_date       TEXT,
            useful_life_months    INTEGER,
            is_depreciable        INTEGER NOT NULL DEFAULT 1,
            source_cip_account_id INTEGER REFERENCES accounts(id),
            created_at            TEXT    NOT NULL,
            updated_at            TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS fiscal_years (
            id              INTEGER PRIMARY KEY,
            start_date      TEXT    NOT NULL,
            end_date        TEXT    NOT NULL,
            is_closed       INTEGER NOT NULL DEFAULT 0,
            closed_at       TEXT,
            created_at      TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS fiscal_periods (
            id              INTEGER PRIMARY KEY,
            fiscal_year_id  INTEGER NOT NULL REFERENCES fiscal_years(id),
            period_number   INTEGER NOT NULL,
            start_date      TEXT    NOT NULL,
            end_date        TEXT    NOT NULL,
            is_closed       INTEGER NOT NULL DEFAULT 0,
            closed_at       TEXT,
            reopened_at     TEXT,
            created_at      TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS journal_entries (
            id                  INTEGER PRIMARY KEY,
            je_number           TEXT    NOT NULL UNIQUE,
            entry_date          TEXT    NOT NULL,
            memo                TEXT,
            status              TEXT    NOT NULL DEFAULT 'Draft',
            is_reversed         INTEGER NOT NULL DEFAULT 0,
            reversed_by_je_id   INTEGER REFERENCES journal_entries(id),
            reversal_of_je_id   INTEGER REFERENCES journal_entries(id),
            inter_entity_uuid   TEXT,
            source_entity_name  TEXT,
            fiscal_period_id    INTEGER NOT NULL REFERENCES fiscal_periods(id),
            created_at          TEXT    NOT NULL,
            updated_at          TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS journal_entry_lines (
            id                INTEGER PRIMARY KEY,
            journal_entry_id  INTEGER NOT NULL REFERENCES journal_entries(id),
            account_id        INTEGER NOT NULL REFERENCES accounts(id),
            debit_amount      INTEGER NOT NULL DEFAULT 0,
            credit_amount     INTEGER NOT NULL DEFAULT 0,
            line_memo         TEXT,
            reconcile_state   TEXT    NOT NULL DEFAULT 'Uncleared',
            sort_order        INTEGER NOT NULL DEFAULT 0,
            created_at        TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS ar_items (
            id                  INTEGER PRIMARY KEY,
            account_id          INTEGER NOT NULL REFERENCES accounts(id),
            customer_name       TEXT    NOT NULL,
            description         TEXT,
            amount              INTEGER NOT NULL,
            due_date            TEXT    NOT NULL,
            status              TEXT    NOT NULL DEFAULT 'Open',
            originating_je_id   INTEGER NOT NULL REFERENCES journal_entries(id),
            created_at          TEXT    NOT NULL,
            updated_at          TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS ar_payments (
            id              INTEGER PRIMARY KEY,
            ar_item_id      INTEGER NOT NULL REFERENCES ar_items(id),
            je_id           INTEGER NOT NULL REFERENCES journal_entries(id),
            amount          INTEGER NOT NULL,
            payment_date    TEXT    NOT NULL,
            created_at      TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS ap_items (
            id                  INTEGER PRIMARY KEY,
            account_id          INTEGER NOT NULL REFERENCES accounts(id),
            vendor_name         TEXT    NOT NULL,
            description         TEXT,
            amount              INTEGER NOT NULL,
            due_date            TEXT    NOT NULL,
            status              TEXT    NOT NULL DEFAULT 'Open',
            originating_je_id   INTEGER NOT NULL REFERENCES journal_entries(id),
            created_at          TEXT    NOT NULL,
            updated_at          TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS ap_payments (
            id              INTEGER PRIMARY KEY,
            ap_item_id      INTEGER NOT NULL REFERENCES ap_items(id),
            je_id           INTEGER NOT NULL REFERENCES journal_entries(id),
            amount          INTEGER NOT NULL,
            payment_date    TEXT    NOT NULL,
            created_at      TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS envelope_allocations (
            id              INTEGER PRIMARY KEY,
            account_id      INTEGER NOT NULL UNIQUE REFERENCES accounts(id),
            percentage      INTEGER NOT NULL,
            created_at      TEXT    NOT NULL,
            updated_at      TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS envelope_ledger (
            id                  INTEGER PRIMARY KEY,
            account_id          INTEGER NOT NULL REFERENCES accounts(id),
            entry_type          TEXT    NOT NULL,
            amount              INTEGER NOT NULL,
            source_je_id        INTEGER REFERENCES journal_entries(id),
            related_account_id  INTEGER REFERENCES accounts(id),
            transfer_group_id   TEXT,
            memo                TEXT,
            created_at          TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS recurring_entry_templates (
            id                    INTEGER PRIMARY KEY,
            source_je_id          INTEGER NOT NULL REFERENCES journal_entries(id),
            frequency             TEXT    NOT NULL,
            next_due_date         TEXT    NOT NULL,
            is_active             INTEGER NOT NULL DEFAULT 1,
            last_generated_date   TEXT,
            created_at            TEXT    NOT NULL,
            updated_at            TEXT    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS audit_log (
            id              INTEGER PRIMARY KEY,
            action_type     TEXT    NOT NULL,
            entity_name     TEXT    NOT NULL,
            record_type     TEXT,
            record_id       INTEGER,
            description     TEXT    NOT NULL,
            created_at      TEXT    NOT NULL
        );

        COMMIT;
        ",
    )?;

    Ok(())
}

/// Seeds the default chart of accounts for a new entity.
///
/// Hierarchy (all top-level are placeholders):
/// - 1000 Assets
///   - 1100 Cash & Bank Accounts (placeholder)
///     - 1110 Checking Account
///     - 1120 Savings Account
///   - 1200 Accounts Receivable
///   - 1300 Prepaid Expenses
///   - 1400 Construction in Progress
///   - 1500 Fixed Assets (placeholder)
///     - 1510 Land
///     - 1520 Buildings
///     - 1521 Accumulated Depreciation - Buildings (contra)
/// - 2000 Liabilities
///   - 2100 Accounts Payable
///   - 2200 Credit Cards
///   - 2300 Mortgage Payable
///   - 2400 Security Deposits Held
/// - 3000 Equity
///   - 3100 Owner's Capital
///   - 3200 Owner's Draw (contra)
///   - 3300 Retained Earnings
/// - 4000 Revenue
///   - 4100 Rental Income
///   - 4200 Other Income
/// - 5000 Expenses
///   - 5100 Repairs & Maintenance
///   - 5200 Property Management Fees
///   - 5300 Insurance
///   - 5400 Property Taxes
///   - 5500 Utilities
///   - 5600 Depreciation Expense
///   - 5700 Interest Expense
///   - 5800 Professional Fees
///
/// NOTE: "feature spec Section 2.2 (Standard Built-in Account Categories)" referenced in
/// phase-1.md is not present in the current spec files. This hierarchy is a judgment call
/// appropriate for a small real estate LLC. Documented in progress.md.
pub fn seed_default_accounts(conn: &Connection) -> Result<()> {
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();

    // Helper: insert an account, return its rowid.
    let insert = |number: &str,
                  name: &str,
                  account_type: &str,
                  parent_id: Option<i64>,
                  is_placeholder: i64,
                  is_contra: i64|
     -> Result<i64> {
        conn.execute(
            "INSERT INTO accounts
                (number, name, account_type, parent_id, is_active, is_contra, is_placeholder, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8)",
            params![
                number,
                name,
                account_type,
                parent_id,
                is_contra,
                is_placeholder,
                now,
                now
            ],
        )?;
        Ok(conn.last_insert_rowid())
    };

    // Top-level placeholders
    let assets = insert("1000", "Assets", "Asset", None, 1, 0)?;
    let liabilities = insert("2000", "Liabilities", "Liability", None, 1, 0)?;
    let equity = insert("3000", "Equity", "Equity", None, 1, 0)?;
    let revenue = insert("4000", "Revenue", "Revenue", None, 1, 0)?;
    let expenses = insert("5000", "Expenses", "Expense", None, 1, 0)?;

    // Assets sub-accounts
    let cash_bank = insert("1100", "Cash & Bank Accounts", "Asset", Some(assets), 1, 0)?;
    insert("1110", "Checking Account", "Asset", Some(cash_bank), 0, 0)?;
    insert("1120", "Savings Account", "Asset", Some(cash_bank), 0, 0)?;
    insert("1200", "Accounts Receivable", "Asset", Some(assets), 0, 0)?;
    insert("1300", "Prepaid Expenses", "Asset", Some(assets), 0, 0)?;
    insert(
        "1400",
        "Construction in Progress",
        "Asset",
        Some(assets),
        0,
        0,
    )?;
    let fixed_assets = insert("1500", "Fixed Assets", "Asset", Some(assets), 1, 0)?;
    insert("1510", "Land", "Asset", Some(fixed_assets), 0, 0)?;
    insert("1520", "Buildings", "Asset", Some(fixed_assets), 0, 0)?;
    insert(
        "1521",
        "Accumulated Depreciation - Buildings",
        "Asset",
        Some(fixed_assets),
        0,
        1, // contra
    )?;

    // Liabilities sub-accounts
    insert(
        "2100",
        "Accounts Payable",
        "Liability",
        Some(liabilities),
        0,
        0,
    )?;
    insert("2200", "Credit Cards", "Liability", Some(liabilities), 0, 0)?;
    insert(
        "2300",
        "Mortgage Payable",
        "Liability",
        Some(liabilities),
        0,
        0,
    )?;
    insert(
        "2400",
        "Security Deposits Held",
        "Liability",
        Some(liabilities),
        0,
        0,
    )?;

    // Equity sub-accounts
    insert("3100", "Owner's Capital", "Equity", Some(equity), 0, 0)?;
    insert(
        "3200",
        "Owner's Draw",
        "Equity",
        Some(equity),
        0,
        1, // contra
    )?;
    insert("3300", "Retained Earnings", "Equity", Some(equity), 0, 0)?;

    // Revenue sub-accounts
    insert("4100", "Rental Income", "Revenue", Some(revenue), 0, 0)?;
    insert("4200", "Other Income", "Revenue", Some(revenue), 0, 0)?;

    // Expense sub-accounts
    insert(
        "5100",
        "Repairs & Maintenance",
        "Expense",
        Some(expenses),
        0,
        0,
    )?;
    insert(
        "5200",
        "Property Management Fees",
        "Expense",
        Some(expenses),
        0,
        0,
    )?;
    insert("5300", "Insurance", "Expense", Some(expenses), 0, 0)?;
    insert("5400", "Property Taxes", "Expense", Some(expenses), 0, 0)?;
    insert("5500", "Utilities", "Expense", Some(expenses), 0, 0)?;
    insert(
        "5600",
        "Depreciation Expense",
        "Expense",
        Some(expenses),
        0,
        0,
    )?;
    insert("5700", "Interest Expense", "Expense", Some(expenses), 0, 0)?;
    insert("5800", "Professional Fees", "Expense", Some(expenses), 0, 0)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_TABLES: &[&str] = &[
        "accounts",
        "fixed_asset_details",
        "fiscal_years",
        "fiscal_periods",
        "journal_entries",
        "journal_entry_lines",
        "ar_items",
        "ar_payments",
        "ap_items",
        "ap_payments",
        "envelope_allocations",
        "envelope_ledger",
        "recurring_entry_templates",
        "audit_log",
    ];

    #[test]
    fn all_14_tables_exist_after_init() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("initialize_schema failed");

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .expect("prepare failed");
        let table_names: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .expect("query failed")
            .map(|r| r.expect("row error"))
            .collect();

        for expected in EXPECTED_TABLES {
            assert!(
                table_names.contains(&expected.to_string()),
                "Missing table: {expected}"
            );
        }
        assert_eq!(
            table_names.len(),
            14,
            "Expected 14 tables, found {}",
            table_names.len()
        );
    }

    #[test]
    fn foreign_keys_pragma_enabled() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("initialize_schema failed");

        let fk_enabled: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("pragma query failed");
        assert_eq!(fk_enabled, 1, "foreign_keys pragma should be ON");
    }

    #[test]
    fn idempotent_double_init() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("first init");
        initialize_schema(&conn).expect("second init should succeed (IF NOT EXISTS)");
    }

    fn seeded_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("initialize_schema");
        seed_default_accounts(&conn).expect("seed_default_accounts");
        conn
    }

    #[test]
    fn five_top_level_placeholders_exist() {
        let conn = seeded_conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE parent_id IS NULL AND is_placeholder = 1",
                [],
                |row| row.get(0),
            )
            .expect("query failed");
        assert_eq!(count, 5, "Expected 5 top-level placeholder accounts");
    }

    #[test]
    fn sub_accounts_have_parent_ids() {
        let conn = seeded_conn();
        let orphans: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts
                 WHERE parent_id IS NOT NULL
                 AND parent_id NOT IN (SELECT id FROM accounts)",
                [],
                |row| row.get(0),
            )
            .expect("query failed");
        assert_eq!(
            orphans, 0,
            "All parent_id references must point to valid accounts"
        );
    }

    #[test]
    fn contra_accounts_flagged() {
        let conn = seeded_conn();
        let contra_names: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM accounts WHERE is_contra = 1 ORDER BY number")
                .expect("prepare");
            stmt.query_map([], |row| row.get(0))
                .expect("query")
                .map(|r| r.expect("row"))
                .collect()
        };
        assert!(
            contra_names
                .iter()
                .any(|n| n.contains("Accumulated Depreciation")),
            "Accumulated Depreciation should be contra"
        );
        assert!(
            contra_names.iter().any(|n| n.contains("Owner's Draw")),
            "Owner's Draw should be contra"
        );
    }

    #[test]
    fn account_types_correct() {
        let conn = seeded_conn();
        // Assets parent is Asset type
        let asset_type: String = conn
            .query_row(
                "SELECT account_type FROM accounts WHERE number = '1000'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(asset_type, "Asset");

        // Revenue parent is Revenue type
        let rev_type: String = conn
            .query_row(
                "SELECT account_type FROM accounts WHERE number = '4000'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(rev_type, "Revenue");

        // Expense parent is Expense type
        let exp_type: String = conn
            .query_row(
                "SELECT account_type FROM accounts WHERE number = '5000'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(exp_type, "Expense");
    }
}
