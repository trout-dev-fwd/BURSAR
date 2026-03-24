pub mod accounts_payable;
pub mod accounts_receivable;
pub mod audit_log;
pub mod chart_of_accounts;
pub mod envelopes;
pub mod fixed_assets;
pub mod general_ledger;
pub mod journal_entries;
pub mod reports;
pub mod tax;

pub use accounts_payable::AccountsPayableTab;
pub use accounts_receivable::AccountsReceivableTab;
pub use audit_log::AuditLogTab;
pub use chart_of_accounts::ChartOfAccountsTab;
pub use envelopes::EnvelopesTab;
pub use fixed_assets::FixedAssetsTab;
pub use general_ledger::GeneralLedgerTab;
pub use journal_entries::JournalEntriesTab;
pub use reports::ReportsTab;
pub use tax::TaxTab;

use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect};

use crate::db::EntityDb;
use crate::types::{AccountId, ApItemId, ArItemId, JournalEntryId};

/// All 10 application tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabId {
    AuditLog,           // index 0
    ChartOfAccounts,    // index 1
    GeneralLedger,      // index 2
    JournalEntries,     // index 3
    AccountsReceivable, // index 4
    AccountsPayable,    // index 5
    Envelopes,          // index 6
    FixedAssets,        // index 7
    Reports,            // index 8
    Tax,                // index 9
}

impl TabId {
    /// Returns all tab IDs in display order.
    pub fn all() -> [TabId; 10] {
        [
            TabId::AuditLog,
            TabId::ChartOfAccounts,
            TabId::GeneralLedger,
            TabId::JournalEntries,
            TabId::AccountsReceivable,
            TabId::AccountsPayable,
            TabId::Envelopes,
            TabId::FixedAssets,
            TabId::Reports,
            TabId::Tax,
        ]
    }
}

/// Used for cross-tab navigation. Wraps the relevant typed ID.
#[derive(Debug, Clone, Copy)]
pub enum RecordId {
    Account(AccountId),
    JournalEntry(JournalEntryId),
    ArItem(ArItemId),
    ApItem(ApItemId),
}

/// Actions a tab returns to the App after handling a key event.
/// Tabs never mutate App state directly — they return an action and App processes it.
#[derive(Debug)]
pub enum TabAction {
    /// Nothing happened; no state change.
    None,
    /// Switch to another tab by ID.
    SwitchTab(TabId),
    /// Switch to another tab and focus a specific record.
    NavigateTo(TabId, RecordId),
    /// Display a message in the status bar.
    ShowMessage(String),
    /// Data was mutated. App should call refresh() on all tabs.
    RefreshData,
    /// Enter inter-entity journal entry mode (requires 2nd entity in config).
    StartInterEntityMode,
    /// Open the CSV import wizard.
    StartImport,
    /// Start re-matching incomplete import drafts (Shift+U).
    StartRematch,
    /// Save tax form configuration to entity TOML.
    /// Carries the list of enabled form tag strings.
    SaveTaxFormConfig(Vec<String>),
    /// Start ingestion of IRS tax reference publications (`u` key in Tax tab).
    StartTaxIngestion,
    /// Quit the application.
    Quit,
}

/// The contract every tab implements.
pub trait Tab {
    /// Display name shown in the tab bar.
    fn title(&self) -> &str;

    /// Handle a key press. Receives a read reference to the database for queries.
    /// For mutations, the tab calls repo methods and returns `TabAction::RefreshData`.
    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction;

    /// Render this tab's content into the given area.
    fn render(&self, frame: &mut Frame, area: Rect);

    /// Called by App after any data mutation (RefreshData action).
    /// The tab re-queries whatever data it displays.
    fn refresh(&mut self, db: &EntityDb);

    /// Returns true when the tab has an active form, modal, or search field
    /// that should capture all keypresses (suppressing global hotkeys like 1-9).
    fn wants_input(&self) -> bool {
        false
    }

    /// Called when navigating to this tab with a specific record to focus.
    /// Default implementation is a no-op; tabs that support it override this.
    fn navigate_to(&mut self, record_id: RecordId, db: &EntityDb) {
        let _ = (record_id, db);
    }

    /// Returns true when the tab has unsaved in-progress content (e.g. a partially filled
    /// new-entry form). Used by App to drive the `[*]` indicator in the status bar.
    fn has_unsaved_changes(&self) -> bool {
        false
    }

    /// Returns (key, description) pairs for this tab's context-specific hotkeys.
    /// Shown in the `?` help overlay. Default returns an empty list.
    fn hotkey_help(&self) -> Vec<(&'static str, &'static str)> {
        vec![]
    }

    /// Returns the import_ref of the currently selected Draft JE (if it has one).
    /// Used by the `/match` slash command to get context for re-matching.
    fn selected_draft_import_ref(&self) -> Option<String> {
        None
    }
}
