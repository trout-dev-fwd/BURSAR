use anyhow::Result;
use rusqlite::Connection;

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
}
