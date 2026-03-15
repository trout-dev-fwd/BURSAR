use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect};

use crate::db::EntityDb;
use crate::types::{AccountId, ApItemId, ArItemId, JournalEntryId};

/// All 9 application tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabId {
    ChartOfAccounts,    // index 0
    GeneralLedger,      // index 1
    JournalEntries,     // index 2
    AccountsReceivable, // index 3
    AccountsPayable,    // index 4
    Envelopes,          // index 5
    FixedAssets,        // index 6
    Reports,            // index 7
    AuditLog,           // index 8
}

impl TabId {
    /// Returns all tab IDs in display order.
    pub fn all() -> [TabId; 9] {
        [
            TabId::ChartOfAccounts,
            TabId::GeneralLedger,
            TabId::JournalEntries,
            TabId::AccountsReceivable,
            TabId::AccountsPayable,
            TabId::Envelopes,
            TabId::FixedAssets,
            TabId::Reports,
            TabId::AuditLog,
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

    /// Called when navigating to this tab with a specific record to focus.
    /// Default implementation is a no-op; tabs that support it override this.
    fn navigate_to(&mut self, record_id: RecordId, db: &EntityDb) {
        let _ = (record_id, db);
    }
}
