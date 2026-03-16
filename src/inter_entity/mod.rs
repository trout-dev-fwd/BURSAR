//! Inter-entity journal entry modal.
//!
//! Opens a temporary second `EntityDb` connection to allow posting a balanced pair
//! of journal entries to two different entity databases simultaneously.
//!
//! **Lifecycle**: open → (form in Task 2) → submit/cancel → drop (closes secondary connection).
//!
//! The primary entity's `EntityDb` is owned by `App::entity` and is NOT stored here.
//! All primary-entity data access is received as `&EntityDb` parameters (same pattern as tabs).

pub mod form;
pub mod recovery;
pub mod write_protocol;

use anyhow::Result;
use rusqlite::params;

use crate::db::EntityDb;
use crate::db::account_repo::{Account, NewAccount};
use crate::inter_entity::form::InterEntityForm;
use crate::types::AccountType;

// ── Intercompany account helpers ──────────────────────────────────────────────

/// Returns `true` if `db` already has both "Due From {other_name}" (Asset) and
/// "Due To {other_name}" (Liability) accounts.
pub fn has_intercompany_accounts(db: &EntityDb, other_name: &str) -> Result<bool> {
    let due_from = format!("Due From {other_name}");
    let due_to = format!("Due To {other_name}");
    let count: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM accounts WHERE name IN (?1, ?2)",
        params![due_from, due_to],
        |row| row.get(0),
    )?;
    Ok(count >= 2)
}

/// Creates "Due From {other_name}" (Asset) and "Due To {other_name}" (Liability) accounts.
///
/// "Due From" is a receivable from the other entity (Asset).
/// "Due To" is a payable to the other entity (Liability).
///
/// Looks up the top-level Assets (number "1000") and Liabilities (number "2000") placeholders
/// to use as parent accounts. Falls back to no parent if not found.
pub fn create_intercompany_accounts(db: &EntityDb, other_name: &str) -> Result<()> {
    let find_parent = |number: &str| -> Option<crate::types::AccountId> {
        db.conn()
            .query_row(
                "SELECT id FROM accounts WHERE number = ?1 AND is_placeholder = 1 LIMIT 1",
                params![number],
                |row| row.get::<_, i64>(0),
            )
            .ok()
            .map(crate::types::AccountId::from)
    };

    let assets_parent = find_parent("1000");
    let liabilities_parent = find_parent("2000");

    // Compute unique account numbers by appending to the parent's range.
    // Due From: 1900-series; Due To: 2900-series. Fall back to SQL MAX+1 if taken.
    let due_from_number = next_available_number(db, "19")?;
    let due_to_number = next_available_number(db, "29")?;

    let accounts = db.accounts();
    accounts.create(&NewAccount {
        number: due_from_number,
        name: format!("Due From {other_name}"),
        account_type: AccountType::Asset,
        parent_id: assets_parent,
        is_contra: false,
        is_placeholder: false,
    })?;
    accounts.create(&NewAccount {
        number: due_to_number,
        name: format!("Due To {other_name}"),
        account_type: AccountType::Liability,
        parent_id: liabilities_parent,
        is_contra: false,
        is_placeholder: false,
    })?;
    Ok(())
}

/// Returns the next available account number with the given prefix (e.g. "19" → "1900", "1901", ...).
fn next_available_number(db: &EntityDb, prefix: &str) -> Result<String> {
    let pattern = format!("{prefix}%");
    let max_num: Option<String> = db
        .conn()
        .query_row(
            "SELECT MAX(number) FROM accounts WHERE number LIKE ?1",
            params![pattern],
            |row| row.get(0),
        )
        .ok()
        .flatten();

    let base = format!("{prefix}00");
    let next = match max_num {
        None => base,
        Some(s) => {
            // Try to parse the last 2 digits and increment.
            if s.len() >= prefix.len() + 2 {
                let suffix = &s[prefix.len()..];
                let n: u32 = suffix.parse().unwrap_or(0);
                format!("{prefix}{:02}", n + 1)
            } else {
                base
            }
        }
    };
    Ok(next)
}

/// Error type for inter-entity operations.
#[derive(Debug, thiserror::Error)]
pub enum InterEntityError {
    #[error("Inter-entity mode requires at least two entities in the workspace config")]
    InsufficientEntities,
    #[error("Cannot open the same entity as both sides of an inter-entity transaction")]
    SameEntity,
    #[error("Entity '{0}' not found in workspace config")]
    EntityNotFound(String),
}

/// Manages the temporary second database connection and shared state for
/// an inter-entity journal entry session.
///
/// Owns `secondary_db` — when this struct is dropped, the secondary connection is closed.
/// The primary entity's `EntityDb` lives in `App::entity.db` and is passed by reference
/// to methods that need it (consistent with the `Tab` pattern).
pub struct InterEntityMode {
    /// Temporary connection to Entity B's database.
    pub secondary_db: EntityDb,
    /// Display name of the primary entity (Entity A).
    pub primary_name: String,
    /// Display name of the secondary entity (Entity B).
    pub secondary_name: String,
    /// Active, non-placeholder accounts from Entity A (for the bottom-left pane).
    pub primary_accounts: Vec<Account>,
    /// Active, non-placeholder accounts from Entity B (for the bottom-right pane).
    pub secondary_accounts: Vec<Account>,
    /// The split-pane entry form.
    pub form: InterEntityForm,
    /// If true, primary entity is missing intercompany accounts for the secondary.
    pub primary_needs_accounts: bool,
    /// If true, secondary entity is missing intercompany accounts for the primary.
    pub secondary_needs_accounts: bool,
}

impl InterEntityMode {
    /// Opens inter-entity mode by connecting to the secondary entity's database,
    /// loading both entities' account lists, and returning the ready-to-use mode.
    ///
    /// `primary_db` — reference to the already-open primary entity database.
    /// `secondary_db` — freshly opened secondary entity database (caller opens it).
    /// `primary_name` / `secondary_name` — display names from workspace config.
    pub fn open(
        primary_db: &EntityDb,
        secondary_db: EntityDb,
        primary_name: String,
        secondary_name: String,
    ) -> Result<Self> {
        let primary_accounts = primary_db.accounts().list_active()?;
        let secondary_accounts = secondary_db.accounts().list_active()?;
        let primary_needs_accounts = !has_intercompany_accounts(primary_db, &secondary_name)?;
        let secondary_needs_accounts = !has_intercompany_accounts(&secondary_db, &primary_name)?;
        Ok(Self {
            secondary_db,
            primary_name,
            secondary_name,
            primary_accounts,
            secondary_accounts,
            form: InterEntityForm::new(),
            primary_needs_accounts,
            secondary_needs_accounts,
        })
    }

    /// Reloads account lists from both databases and re-checks intercompany account status.
    /// Call after any account mutation (e.g., auto-creating intercompany accounts).
    pub fn refresh_accounts(&mut self, primary_db: &EntityDb) -> Result<()> {
        self.primary_accounts = primary_db.accounts().list_active()?;
        self.secondary_accounts = self.secondary_db.accounts().list_active()?;
        self.primary_needs_accounts = !has_intercompany_accounts(primary_db, &self.secondary_name)?;
        self.secondary_needs_accounts =
            !has_intercompany_accounts(&self.secondary_db, &self.primary_name)?;
        Ok(())
    }

    /// Returns true if either entity needs intercompany accounts created.
    pub fn needs_account_setup(&self) -> bool {
        self.primary_needs_accounts || self.secondary_needs_accounts
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::initialize_schema;
    use rusqlite::Connection;

    fn make_in_memory_entity_db() -> EntityDb {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema");
        crate::db::entity_db_from_conn(conn)
    }

    #[test]
    fn open_creates_mode_with_both_account_lists() {
        let primary = make_in_memory_entity_db();
        // Seed default accounts for primary.
        crate::db::schema::seed_default_accounts(primary.conn()).expect("seed primary");

        let secondary = make_in_memory_entity_db();
        // Seed default accounts for secondary.
        crate::db::schema::seed_default_accounts(secondary.conn()).expect("seed secondary");

        let mode = InterEntityMode::open(
            &primary,
            secondary,
            "Entity A".to_owned(),
            "Entity B".to_owned(),
        )
        .expect("open failed");

        assert_eq!(mode.primary_name, "Entity A");
        assert_eq!(mode.secondary_name, "Entity B");
        // Both should have accounts (seeded defaults include non-placeholder accounts).
        assert!(
            !mode.primary_accounts.is_empty(),
            "primary accounts should be non-empty"
        );
        assert!(
            !mode.secondary_accounts.is_empty(),
            "secondary accounts should be non-empty"
        );
        // list_active filters out inactive but includes placeholders — verify no inactive.
        assert!(mode.primary_accounts.iter().all(|a| a.is_active));
        assert!(mode.secondary_accounts.iter().all(|a| a.is_active));
    }

    #[test]
    fn open_with_empty_databases_produces_empty_account_lists() {
        let primary = make_in_memory_entity_db();
        let secondary = make_in_memory_entity_db();

        let mode = InterEntityMode::open(
            &primary,
            secondary,
            "Entity A".to_owned(),
            "Entity B".to_owned(),
        )
        .expect("open failed");

        assert!(mode.primary_accounts.is_empty());
        assert!(mode.secondary_accounts.is_empty());
    }

    #[test]
    fn drop_closes_secondary_connection() {
        // When InterEntityMode is dropped, the secondary EntityDb (and its Connection) is dropped.
        // We verify this by creating a temp file, opening it as secondary, dropping the mode,
        // and verifying the file can be opened again (no lock held).
        let dir = std::env::temp_dir().join("inter_entity_drop_test");
        std::fs::create_dir_all(&dir).expect("dir");
        let secondary_path = dir.join("secondary.sqlite");
        let _ = std::fs::remove_file(&secondary_path);

        // Create the secondary DB file.
        let secondary_for_setup =
            EntityDb::create(&secondary_path, "Secondary", 1).expect("create secondary");
        drop(secondary_for_setup);

        // Open secondary in InterEntityMode.
        let primary = make_in_memory_entity_db();
        let secondary = EntityDb::open(&secondary_path).expect("open secondary");

        let mode = InterEntityMode::open(
            &primary,
            secondary,
            "Primary".to_owned(),
            "Secondary".to_owned(),
        )
        .expect("open mode");

        // Drop the mode — secondary connection should be released.
        drop(mode);

        // Should be able to re-open the file without "database is locked" error.
        let reopen = EntityDb::open(&secondary_path);
        assert!(
            reopen.is_ok(),
            "secondary DB should be openable after InterEntityMode drop"
        );

        // Cleanup.
        let _ = std::fs::remove_file(&secondary_path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn refresh_accounts_reloads_from_both_databases() {
        use crate::db::account_repo::NewAccount;
        use crate::types::AccountType;

        let primary = make_in_memory_entity_db();
        let secondary = make_in_memory_entity_db();

        let mut mode = InterEntityMode::open(&primary, secondary, "P".to_owned(), "S".to_owned())
            .expect("open");

        assert!(mode.primary_accounts.is_empty());
        assert!(mode.secondary_accounts.is_empty());

        // Add an account to primary.
        primary
            .accounts()
            .create(&NewAccount {
                number: "1110".to_owned(),
                name: "Cash".to_owned(),
                account_type: AccountType::Asset,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create account");

        // Add an account to secondary.
        mode.secondary_db
            .accounts()
            .create(&NewAccount {
                number: "2100".to_owned(),
                name: "Payables".to_owned(),
                account_type: AccountType::Liability,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create secondary account");

        mode.refresh_accounts(&primary).expect("refresh");

        assert_eq!(mode.primary_accounts.len(), 1);
        assert_eq!(mode.secondary_accounts.len(), 1);
        assert_eq!(mode.primary_accounts[0].name, "Cash");
        assert_eq!(mode.secondary_accounts[0].name, "Payables");
    }

    // ── Intercompany account helpers ──────────────────────────────────────────

    #[test]
    fn has_intercompany_accounts_returns_false_when_absent() {
        let db = make_in_memory_entity_db();
        assert!(!has_intercompany_accounts(&db, "Entity B").expect("query"));
    }

    #[test]
    fn create_intercompany_accounts_creates_due_from_and_due_to() {
        let db = make_in_memory_entity_db();
        create_intercompany_accounts(&db, "Entity B").expect("create");

        let accounts = db.accounts().list_active().expect("list");
        let names: Vec<&str> = accounts.iter().map(|a| a.name.as_str()).collect();
        assert!(
            names.contains(&"Due From Entity B"),
            "Due From account missing; accounts: {names:?}"
        );
        assert!(
            names.contains(&"Due To Entity B"),
            "Due To account missing; accounts: {names:?}"
        );
    }

    #[test]
    fn has_intercompany_accounts_returns_true_after_creation() {
        let db = make_in_memory_entity_db();
        assert!(!has_intercompany_accounts(&db, "Entity B").expect("before"));
        create_intercompany_accounts(&db, "Entity B").expect("create");
        assert!(has_intercompany_accounts(&db, "Entity B").expect("after"));
    }

    #[test]
    fn open_detects_missing_accounts_on_both_sides() {
        let primary = make_in_memory_entity_db();
        let secondary = make_in_memory_entity_db();

        let mode = InterEntityMode::open(
            &primary,
            secondary,
            "Entity A".to_owned(),
            "Entity B".to_owned(),
        )
        .expect("open");

        assert!(mode.primary_needs_accounts, "primary should need accounts");
        assert!(
            mode.secondary_needs_accounts,
            "secondary should need accounts"
        );
        assert!(mode.needs_account_setup());
    }

    #[test]
    fn open_no_account_setup_needed_when_accounts_exist() {
        let primary = make_in_memory_entity_db();
        let secondary = make_in_memory_entity_db();

        // Pre-create intercompany accounts on both sides.
        create_intercompany_accounts(&primary, "Entity B").expect("create primary");
        create_intercompany_accounts(&secondary, "Entity A").expect("create secondary");

        let mode = InterEntityMode::open(
            &primary,
            secondary,
            "Entity A".to_owned(),
            "Entity B".to_owned(),
        )
        .expect("open");

        assert!(!mode.primary_needs_accounts);
        assert!(!mode.secondary_needs_accounts);
        assert!(!mode.needs_account_setup());
    }
}
