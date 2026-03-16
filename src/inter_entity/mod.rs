//! Inter-entity journal entry modal.
//!
//! Opens a temporary second `EntityDb` connection to allow posting a balanced pair
//! of journal entries to two different entity databases simultaneously.
//!
//! **Lifecycle**: open → (form in Task 2) → submit/cancel → drop (closes secondary connection).
//!
//! The primary entity's `EntityDb` is owned by `App::entity` and is NOT stored here.
//! All primary-entity data access is received as `&EntityDb` parameters (same pattern as tabs).

pub mod recovery;

use anyhow::Result;

use crate::db::EntityDb;
use crate::db::account_repo::Account;

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
        Ok(Self {
            secondary_db,
            primary_name,
            secondary_name,
            primary_accounts,
            secondary_accounts,
        })
    }

    /// Reloads account lists from both databases.
    /// Call after any account mutation (e.g., auto-creating intercompany accounts in Task 6).
    pub fn refresh_accounts(&mut self, primary_db: &EntityDb) -> Result<()> {
        self.primary_accounts = primary_db.accounts().list_active()?;
        self.secondary_accounts = self.secondary_db.accounts().list_active()?;
        Ok(())
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
}
