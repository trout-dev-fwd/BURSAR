use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
};

use chrono::NaiveDate;

use crate::db::{
    EntityDb,
    account_repo::{Account, AccountUpdate, NewAccount},
};
use crate::tabs::{RecordId, Tab, TabAction};
use crate::types::{AccountId, AccountType, AuditAction, Money};
use crate::widgets::account_picker::{AccountPicker, PickerAction};
use crate::widgets::centered_rect;
use crate::widgets::confirmation::{ConfirmAction, Confirmation};

// ── Data structures ───────────────────────────────────────────────────────────

/// One displayable row in the account list (normal or search mode).
#[derive(Debug, Clone)]
struct VisibleRow {
    account: Account,
    depth: usize,
    has_children: bool,
}

// ── Modal state machines ──────────────────────────────────────────────────────

struct AddFormState {
    number: String,
    name: String,
    account_type: AccountType,
    /// Display text for the parent field (set by AccountPicker on selection).
    parent_display: String,
    /// Resolved parent account ID (set by AccountPicker on selection).
    parent_id: Option<AccountId>,
    is_contra: bool,
    is_placeholder: bool,
    focused_field: usize,
    error: Option<String>,
}

impl AddFormState {
    fn new() -> Self {
        Self {
            number: String::new(),
            name: String::new(),
            account_type: AccountType::Asset,
            parent_display: String::new(),
            parent_id: None,
            is_contra: false,
            is_placeholder: false,
            focused_field: 0,
            error: None,
        }
    }

    fn field_count() -> usize {
        6
    }
}

#[derive(Debug)]
struct EditFormState {
    id: AccountId,
    original_number: String,
    name: String,
    number: String,
    focused_field: usize,
    error: Option<String>,
}

/// State for the deactivate/reactivate confirmation dialog.
struct ConfirmToggleState {
    id: AccountId,
    name: String,
    currently_active: bool,
    confirm: Confirmation,
}

/// State for the delete confirmation dialog.
struct ConfirmDeleteState {
    id: AccountId,
    number: String,
    name: String,
    confirm: Confirmation,
}

enum CoaModal {
    AddForm(AddFormState),
    /// The AccountPicker is open as a sub-overlay of the Add form.
    AddFormPickingParent(AddFormState, AccountPicker),
    EditForm(EditFormState),
    ConfirmToggle(ConfirmToggleState),
    ConfirmDelete(ConfirmDeleteState),
    PlaceInService(PlaceInServiceFormState),
    PlaceInServicePicking(PlaceInServiceFormState, AccountPicker, PisField),
}

// ── Place-in-Service form ─────────────────────────────────────────────────────

/// Which field is focused in the Place-in-Service form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PisField {
    Target,
    Accum,
    Expense,
    Date,
    Months,
}

impl PisField {
    const ALL: [PisField; 5] = [
        PisField::Target,
        PisField::Accum,
        PisField::Expense,
        PisField::Date,
        PisField::Months,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// Returns true for the picker-backed fields (Target, Accum, Expense).
    fn is_picker(self) -> bool {
        matches!(self, PisField::Target | PisField::Accum | PisField::Expense)
    }
}

struct PlaceInServiceFormState {
    cip_account_id: AccountId,
    cip_name: String,
    target_id: Option<AccountId>,
    target_name: String,
    accum_id: Option<AccountId>,
    accum_name: String,
    expense_id: Option<AccountId>,
    expense_name: String,
    date_input: String,
    months_input: String,
    focused_field: PisField,
    error: Option<String>,
}

// ── Tab struct ────────────────────────────────────────────────────────────────

pub struct ChartOfAccountsTab {
    entity_name: String,
    all_accounts: Vec<Account>,
    balances: HashMap<AccountId, Money>,
    /// Envelope ledger balances (earmarked amounts) for accounts with allocations.
    envelope_balances: HashMap<AccountId, Money>,
    /// Set of collapsed group accounts (has_children but currently folded).
    collapsed: HashSet<AccountId>,
    /// Flattened view for normal mode.
    visible: Vec<VisibleRow>,
    table_state: TableState,
    /// Whether `/` search mode is active.
    search_active: bool,
    search_query: String,
    /// Flattened, filtered view for search mode.
    filtered: Vec<VisibleRow>,
    filtered_state: TableState,
    /// Active modal overlay (Add/Edit form, or confirmation prompt).
    modal: Option<CoaModal>,
}

impl Default for ChartOfAccountsTab {
    fn default() -> Self {
        Self::new()
    }
}

impl ChartOfAccountsTab {
    pub fn new() -> Self {
        Self {
            entity_name: String::new(),
            all_accounts: Vec::new(),
            balances: HashMap::new(),
            envelope_balances: HashMap::new(),
            collapsed: HashSet::new(),
            visible: Vec::new(),
            table_state: {
                let mut s = TableState::default();
                s.select(Some(0));
                s
            },
            search_active: false,
            search_query: String::new(),
            filtered: Vec::new(),
            filtered_state: {
                let mut s = TableState::default();
                s.select(Some(0));
                s
            },
            modal: None,
        }
    }

    /// Sets the entity name used in audit log entries.
    pub fn set_entity_name(&mut self, name: &str) {
        self.entity_name = name.to_string();
    }

    // ── Internal navigation helpers ───────────────────────────────────────────

    fn build_visible(&mut self) {
        let is_parent: HashSet<AccountId> = self
            .all_accounts
            .iter()
            .filter_map(|a| a.parent_id)
            .collect();

        let mut children_map: HashMap<Option<AccountId>, Vec<&Account>> = HashMap::new();
        for acc in &self.all_accounts {
            children_map.entry(acc.parent_id).or_default().push(acc);
        }
        for list in children_map.values_mut() {
            list.sort_by(|a, b| a.number.cmp(&b.number));
        }

        let mut rows = Vec::new();
        flatten_tree(
            None,
            0,
            &children_map,
            &self.collapsed,
            &is_parent,
            &mut rows,
        );
        self.visible = rows;

        let len = self.visible.len();
        match self.table_state.selected() {
            Some(i) if i >= len && len > 0 => self.table_state.select(Some(len - 1)),
            None if len > 0 => self.table_state.select(Some(0)),
            _ => {}
        }
    }

    fn update_filter(&mut self) {
        let q = self.search_query.to_lowercase();
        self.filtered = self
            .all_accounts
            .iter()
            .filter(|a| {
                q.is_empty()
                    || a.name.to_lowercase().contains(&q)
                    || a.number.to_lowercase().contains(&q)
            })
            .map(|a| VisibleRow {
                account: a.clone(),
                depth: 0,
                has_children: false,
            })
            .collect();

        let len = self.filtered.len();
        match self.filtered_state.selected() {
            Some(i) if i >= len && len > 0 => self.filtered_state.select(Some(len - 1)),
            None if len > 0 => self.filtered_state.select(Some(0)),
            _ => {}
        }
    }

    fn current_rows(&self) -> &[VisibleRow] {
        if self.search_active {
            &self.filtered
        } else {
            &self.visible
        }
    }

    fn selected_idx(&self) -> Option<usize> {
        if self.search_active {
            self.filtered_state.selected()
        } else {
            self.table_state.selected()
        }
    }

    fn scroll_up(&mut self) {
        if self.search_active {
            let cur = self.filtered_state.selected().unwrap_or(0);
            if cur > 0 {
                self.filtered_state.select(Some(cur - 1));
            }
        } else {
            let cur = self.table_state.selected().unwrap_or(0);
            if cur > 0 {
                self.table_state.select(Some(cur - 1));
            }
        }
    }

    fn scroll_down(&mut self) {
        let len = self.current_rows().len();
        if len == 0 {
            return;
        }
        if self.search_active {
            let cur = self.filtered_state.selected().unwrap_or(0);
            if cur + 1 < len {
                self.filtered_state.select(Some(cur + 1));
            }
        } else {
            let cur = self.table_state.selected().unwrap_or(0);
            if cur + 1 < len {
                self.table_state.select(Some(cur + 1));
            }
        }
    }

    fn toggle_expand(&mut self) {
        if self.search_active {
            return;
        }
        let idx = match self.table_state.selected() {
            Some(i) => i,
            None => return,
        };
        let row = match self.visible.get(idx) {
            Some(r) => r.clone(),
            None => return,
        };
        if !row.has_children {
            return;
        }
        if self.collapsed.contains(&row.account.id) {
            self.collapsed.remove(&row.account.id);
        } else {
            self.collapsed.insert(row.account.id);
        }
        self.build_visible();
    }

    fn selected_account(&self) -> Option<&Account> {
        let idx = self.selected_idx()?;
        let row = self.current_rows().get(idx)?;
        Some(&row.account)
    }

    // ── Modal openers ─────────────────────────────────────────────────────────

    fn open_add_form(&mut self) {
        self.search_active = false;
        self.modal = Some(CoaModal::AddForm(AddFormState::new()));
    }

    fn open_edit_form(&mut self) {
        let acc = match self.selected_account() {
            Some(a) => a.clone(),
            None => return,
        };
        self.search_active = false;
        self.modal = Some(CoaModal::EditForm(EditFormState {
            id: acc.id,
            original_number: acc.number.clone(),
            name: acc.name.clone(),
            number: acc.number,
            focused_field: 0,
            error: None,
        }));
    }

    fn open_confirm_delete(&mut self) {
        let acc = match self.selected_account() {
            Some(a) => a.clone(),
            None => return,
        };
        let msg = format!(
            "Delete account {} {}? This cannot be undone.",
            acc.number, acc.name
        );
        self.search_active = false;
        self.modal = Some(CoaModal::ConfirmDelete(ConfirmDeleteState {
            id: acc.id,
            number: acc.number.clone(),
            name: acc.name.clone(),
            confirm: Confirmation::new(msg),
        }));
    }

    fn open_confirm_toggle(&mut self) {
        let acc = match self.selected_account() {
            Some(a) => a.clone(),
            None => return,
        };
        let action = if acc.is_active {
            "Deactivate"
        } else {
            "Reactivate"
        };
        let msg = format!("{} '{}'?", action, acc.name);
        self.search_active = false;
        self.modal = Some(CoaModal::ConfirmToggle(ConfirmToggleState {
            id: acc.id,
            name: acc.name.clone(),
            currently_active: acc.is_active,
            confirm: Confirmation::new(msg),
        }));
    }

    fn open_place_in_service(&mut self) {
        let acc = match self.selected_account() {
            Some(a) => a.clone(),
            None => return,
        };
        // Only applicable to CIP accounts (name contains "construction").
        if !acc.name.to_lowercase().contains("construction") {
            return;
        }
        self.search_active = false;
        self.modal = Some(CoaModal::PlaceInService(PlaceInServiceFormState {
            cip_account_id: acc.id,
            cip_name: acc.name.clone(),
            target_id: None,
            target_name: String::new(),
            accum_id: None,
            accum_name: String::new(),
            expense_id: None,
            expense_name: String::new(),
            date_input: String::new(),
            months_input: String::new(),
            focused_field: PisField::Target,
            error: None,
        }));
    }

    // ── Modal key handlers ────────────────────────────────────────────────────

    fn handle_add_form_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        let form = match &mut self.modal {
            Some(CoaModal::AddForm(f)) => f,
            _ => return TabAction::None,
        };

        match key.code {
            KeyCode::Esc => {
                self.modal = None;
                return TabAction::None;
            }
            KeyCode::Tab | KeyCode::Down => {
                form.focused_field = (form.focused_field + 1) % AddFormState::field_count();
                return TabAction::None;
            }
            KeyCode::BackTab | KeyCode::Up => {
                form.focused_field = (form.focused_field + AddFormState::field_count() - 1)
                    % AddFormState::field_count();
                return TabAction::None;
            }
            KeyCode::Enter => {
                // Field 3 (Parent): open AccountPicker instead of advancing.
                if form.focused_field == 3 {
                    let mut picker = AccountPicker::with_placeholders();
                    picker.refresh(&self.all_accounts);
                    // Transition: move the form into the AddFormPickingParent variant.
                    let form_state = match self.modal.take() {
                        Some(CoaModal::AddForm(f)) => f,
                        _ => return TabAction::None,
                    };
                    self.modal = Some(CoaModal::AddFormPickingParent(form_state, picker));
                    return TabAction::None;
                }
                // Move forward through fields or submit on last field.
                if form.focused_field < AddFormState::field_count() - 1 {
                    form.focused_field += 1;
                    return TabAction::None;
                }
                // Submit the form.
            }
            KeyCode::Backspace => {
                match form.focused_field {
                    0 => {
                        form.number.pop();
                    }
                    1 => {
                        form.name.pop();
                    }
                    3 => {
                        // Clear parent selection.
                        form.parent_display.clear();
                        form.parent_id = None;
                    }
                    _ => {}
                }
                return TabAction::None;
            }
            KeyCode::Char(c) => {
                match form.focused_field {
                    0 => form.number.push(c),
                    1 => form.name.push(c),
                    2 => {
                        // Cycle account type on any char (or Space)
                        form.account_type = cycle_account_type(form.account_type, true);
                    }
                    3 => {
                        // Any char on parent field opens the picker.
                        let mut picker = AccountPicker::with_placeholders();
                        picker.refresh(&self.all_accounts);
                        // Pre-seed the picker with the typed character.
                        let seed_key =
                            KeyEvent::new(KeyCode::Char(c), crossterm::event::KeyModifiers::NONE);
                        picker.handle_key(seed_key, &self.all_accounts);
                        let form_state = match self.modal.take() {
                            Some(CoaModal::AddForm(f)) => f,
                            _ => return TabAction::None,
                        };
                        self.modal = Some(CoaModal::AddFormPickingParent(form_state, picker));
                    }
                    4 => form.is_contra = !form.is_contra,
                    5 => form.is_placeholder = !form.is_placeholder,
                    _ => {}
                }
                return TabAction::None;
            }
            KeyCode::Left => {
                if form.focused_field == 2 {
                    form.account_type = cycle_account_type(form.account_type, false);
                }
                return TabAction::None;
            }
            KeyCode::Right => {
                if form.focused_field == 2 {
                    form.account_type = cycle_account_type(form.account_type, true);
                }
                return TabAction::None;
            }
            _ => return TabAction::None,
        }

        // Clone the form data for submission (to avoid borrow conflict).
        let (number, name, account_type, parent_id, is_contra, is_placeholder) = {
            let f = match &self.modal {
                Some(CoaModal::AddForm(f)) => f,
                _ => return TabAction::None,
            };
            (
                f.number.trim().to_string(),
                f.name.trim().to_string(),
                f.account_type,
                f.parent_id,
                f.is_contra,
                f.is_placeholder,
            )
        };

        // Validate.
        if number.is_empty() {
            if let Some(CoaModal::AddForm(f)) = &mut self.modal {
                f.error = Some("Account number is required.".to_string());
                f.focused_field = 0;
            }
            return TabAction::None;
        }
        if name.is_empty() {
            if let Some(CoaModal::AddForm(f)) = &mut self.modal {
                f.error = Some("Account name is required.".to_string());
                f.focused_field = 1;
            }
            return TabAction::None;
        }

        let new_account = NewAccount {
            number: number.clone(),
            name: name.clone(),
            account_type,
            parent_id,
            is_contra,
            is_placeholder,
        };

        match db.accounts().create(&new_account) {
            Err(e) => {
                let msg = format!("Failed to create account: {e}");
                if let Some(CoaModal::AddForm(f)) = &mut self.modal {
                    f.error = Some(msg);
                    f.focused_field = 0;
                }
                TabAction::None
            }
            Ok(new_id) => {
                let desc = format!("Created account {number} {name}");
                if let Err(e) = db.audit().append(
                    crate::types::AuditAction::AccountCreated,
                    &self.entity_name,
                    Some("Account"),
                    Some(i64::from(new_id)),
                    &desc,
                ) {
                    tracing::error!("Failed to write audit log: {e}");
                }
                self.modal = None;
                TabAction::RefreshData
            }
        }
    }

    fn handle_add_form_picker_key(&mut self, key: KeyEvent) -> TabAction {
        let (form, picker) = match &mut self.modal {
            Some(CoaModal::AddFormPickingParent(f, p)) => (f, p),
            _ => return TabAction::None,
        };

        match picker.handle_key(key, &self.all_accounts) {
            PickerAction::Selected(id) => {
                // Look up the account to get its display number.
                if let Some(acc) = self.all_accounts.iter().find(|a| a.id == id) {
                    form.parent_display = format!("{} {}", acc.number, acc.name);
                    form.parent_id = Some(id);
                }
                // Transition back to AddForm.
                let form_state = match self.modal.take() {
                    Some(CoaModal::AddFormPickingParent(f, _)) => f,
                    _ => return TabAction::None,
                };
                self.modal = Some(CoaModal::AddForm(form_state));
            }
            PickerAction::Cancelled => {
                // Return to the add form without changing parent.
                let form_state = match self.modal.take() {
                    Some(CoaModal::AddFormPickingParent(f, _)) => f,
                    _ => return TabAction::None,
                };
                self.modal = Some(CoaModal::AddForm(form_state));
            }
            PickerAction::Pending => {}
        }
        TabAction::None
    }

    fn handle_edit_form_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        let form = match &mut self.modal {
            Some(CoaModal::EditForm(f)) => f,
            _ => return TabAction::None,
        };

        match key.code {
            KeyCode::Esc => {
                self.modal = None;
                return TabAction::None;
            }
            KeyCode::Tab | KeyCode::Down => {
                form.focused_field = (form.focused_field + 1) % 2;
                return TabAction::None;
            }
            KeyCode::BackTab | KeyCode::Up => {
                form.focused_field = (form.focused_field + 1) % 2;
                return TabAction::None;
            }
            KeyCode::Backspace => {
                match form.focused_field {
                    0 => {
                        form.name.pop();
                    }
                    1 => {
                        form.number.pop();
                    }
                    _ => {}
                }
                return TabAction::None;
            }
            KeyCode::Char(c) => {
                match form.focused_field {
                    0 => form.name.push(c),
                    1 => form.number.push(c),
                    _ => {}
                }
                return TabAction::None;
            }
            KeyCode::Enter => {
                // Advance through fields; submit only on the last field.
                if form.focused_field < 1 {
                    form.focused_field += 1;
                    return TabAction::None;
                }
            }
            _ => return TabAction::None,
        }

        // Clone form data for submission.
        let (id, original_number, name, number) = {
            let f = match &self.modal {
                Some(CoaModal::EditForm(f)) => f,
                _ => return TabAction::None,
            };
            (
                f.id,
                f.original_number.clone(),
                f.name.trim().to_string(),
                f.number.trim().to_string(),
            )
        };

        if name.is_empty() {
            if let Some(CoaModal::EditForm(f)) = &mut self.modal {
                f.error = Some("Account name is required.".to_string());
                f.focused_field = 0;
            }
            return TabAction::None;
        }
        if number.is_empty() {
            if let Some(CoaModal::EditForm(f)) = &mut self.modal {
                f.error = Some("Account number is required.".to_string());
                f.focused_field = 1;
            }
            return TabAction::None;
        }

        let changes = AccountUpdate {
            name: Some(name.clone()),
            number: Some(number.clone()),
        };

        match db.accounts().update(id, &changes) {
            Err(e) => {
                let msg = format!("Failed to update account: {e}");
                if let Some(CoaModal::EditForm(f)) = &mut self.modal {
                    f.error = Some(msg);
                }
                TabAction::None
            }
            Ok(()) => {
                let desc =
                    format!("Updated account {original_number}: name='{name}', number='{number}'");
                if let Err(e) = db.audit().append(
                    crate::types::AuditAction::AccountModified,
                    &self.entity_name,
                    Some("Account"),
                    Some(i64::from(id)),
                    &desc,
                ) {
                    tracing::error!("Failed to write audit log: {e}");
                }
                self.modal = None;
                TabAction::RefreshData
            }
        }
    }

    fn handle_confirm_toggle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        let state = match &mut self.modal {
            Some(CoaModal::ConfirmToggle(s)) => s,
            _ => return TabAction::None,
        };

        match state.confirm.handle_key(key) {
            ConfirmAction::Confirmed => {
                let id = state.id;
                let name = state.name.clone();
                let currently_active = state.currently_active;

                let result = if currently_active {
                    db.accounts().deactivate(id)
                } else {
                    db.accounts().activate(id)
                };
                match result {
                    Err(e) => {
                        tracing::error!("Failed to toggle account active state: {e}");
                        self.modal = None;
                        TabAction::None
                    }
                    Ok(()) => {
                        let (audit_action, action_word) = if currently_active {
                            (crate::types::AuditAction::AccountDeactivated, "Deactivated")
                        } else {
                            (crate::types::AuditAction::AccountReactivated, "Reactivated")
                        };
                        let desc = format!("{action_word} account {}", name);
                        if let Err(e) = db.audit().append(
                            audit_action,
                            &self.entity_name,
                            Some("Account"),
                            Some(i64::from(id)),
                            &desc,
                        ) {
                            tracing::error!("Failed to write audit log: {e}");
                        }
                        self.modal = None;
                        TabAction::RefreshData
                    }
                }
            }
            ConfirmAction::Cancelled => {
                self.modal = None;
                TabAction::None
            }
            ConfirmAction::Pending => TabAction::None,
        }
    }

    fn handle_confirm_delete_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        let state = match &mut self.modal {
            Some(CoaModal::ConfirmDelete(s)) => s,
            _ => return TabAction::None,
        };

        match state.confirm.handle_key(key) {
            ConfirmAction::Confirmed => {
                let id = state.id;
                let number = state.number.clone();
                let name = state.name.clone();

                match db.accounts().delete(id) {
                    Err(e) => {
                        self.modal = None;
                        TabAction::ShowMessage(format!("{e}"))
                    }
                    Ok(()) => {
                        let desc = format!("Deleted account {number} {name}");
                        if let Err(e) = db.audit().append(
                            crate::types::AuditAction::AccountDeleted,
                            &self.entity_name,
                            Some("Account"),
                            Some(i64::from(id)),
                            &desc,
                        ) {
                            tracing::error!("Failed to write audit log: {e}");
                        }
                        self.modal = None;
                        TabAction::RefreshData
                    }
                }
            }
            ConfirmAction::Cancelled => {
                self.modal = None;
                TabAction::None
            }
            ConfirmAction::Pending => TabAction::None,
        }
    }

    fn handle_place_in_service_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        // Navigate: Tab/Down/Up/BackTab move between fields.
        // Picker fields: Enter or any printable char opens picker.
        // Text fields: chars append, Backspace pops.
        // Enter on Months (last field): submit.

        match key.code {
            KeyCode::Esc => {
                self.modal = None;
                TabAction::None
            }
            KeyCode::Tab | KeyCode::Down => {
                if let Some(CoaModal::PlaceInService(f)) = &mut self.modal {
                    f.focused_field = f.focused_field.next();
                    f.error = None;
                }
                TabAction::None
            }
            KeyCode::BackTab | KeyCode::Up => {
                if let Some(CoaModal::PlaceInService(f)) = &mut self.modal {
                    f.focused_field = f.focused_field.prev();
                    f.error = None;
                }
                TabAction::None
            }
            KeyCode::Enter => {
                let field = match &self.modal {
                    Some(CoaModal::PlaceInService(f)) => f.focused_field,
                    _ => return TabAction::None,
                };
                if field.is_picker() {
                    // Open the account picker for this field.
                    let mut picker = AccountPicker::new();
                    picker.refresh(&self.all_accounts);
                    let form_state = match self.modal.take() {
                        Some(CoaModal::PlaceInService(f)) => f,
                        _ => return TabAction::None,
                    };
                    self.modal = Some(CoaModal::PlaceInServicePicking(form_state, picker, field));
                    return TabAction::None;
                }
                if field == PisField::Months {
                    // Submit.
                    return self.submit_place_in_service(db);
                }
                // Advance.
                if let Some(CoaModal::PlaceInService(f)) = &mut self.modal {
                    f.focused_field = f.focused_field.next();
                }
                TabAction::None
            }
            KeyCode::Backspace => {
                if let Some(CoaModal::PlaceInService(f)) = &mut self.modal {
                    match f.focused_field {
                        PisField::Target => {
                            f.target_id = None;
                            f.target_name.clear();
                        }
                        PisField::Accum => {
                            f.accum_id = None;
                            f.accum_name.clear();
                        }
                        PisField::Expense => {
                            f.expense_id = None;
                            f.expense_name.clear();
                        }
                        PisField::Date => {
                            f.date_input.pop();
                        }
                        PisField::Months => {
                            f.months_input.pop();
                        }
                    }
                }
                TabAction::None
            }
            KeyCode::Char(c) => {
                let field = match &self.modal {
                    Some(CoaModal::PlaceInService(f)) => f.focused_field,
                    _ => return TabAction::None,
                };
                if field.is_picker() {
                    // Any char on a picker field opens the picker pre-seeded.
                    let mut picker = AccountPicker::new();
                    picker.refresh(&self.all_accounts);
                    let seed_key =
                        KeyEvent::new(KeyCode::Char(c), crossterm::event::KeyModifiers::NONE);
                    picker.handle_key(seed_key, &self.all_accounts);
                    let form_state = match self.modal.take() {
                        Some(CoaModal::PlaceInService(f)) => f,
                        _ => return TabAction::None,
                    };
                    self.modal = Some(CoaModal::PlaceInServicePicking(form_state, picker, field));
                    return TabAction::None;
                }
                if let Some(CoaModal::PlaceInService(f)) = &mut self.modal {
                    match f.focused_field {
                        PisField::Date => f.date_input.push(c),
                        PisField::Months => f.months_input.push(c),
                        _ => {}
                    }
                }
                TabAction::None
            }
            _ => TabAction::None,
        }
    }

    fn handle_place_in_service_picker_key(&mut self, key: KeyEvent) -> TabAction {
        let (form, picker, picking_field) = match &mut self.modal {
            Some(CoaModal::PlaceInServicePicking(f, p, field)) => (f, p, *field),
            _ => return TabAction::None,
        };

        match picker.handle_key(key, &self.all_accounts) {
            PickerAction::Selected(id) => {
                if let Some(acc) = self.all_accounts.iter().find(|a| a.id == id) {
                    let display = format!("{} {}", acc.number, acc.name);
                    match picking_field {
                        PisField::Target => {
                            form.target_id = Some(id);
                            form.target_name = display;
                        }
                        PisField::Accum => {
                            form.accum_id = Some(id);
                            form.accum_name = display;
                        }
                        PisField::Expense => {
                            form.expense_id = Some(id);
                            form.expense_name = display;
                        }
                        _ => {}
                    }
                }
                let form_state = match self.modal.take() {
                    Some(CoaModal::PlaceInServicePicking(f, _, _)) => f,
                    _ => return TabAction::None,
                };
                self.modal = Some(CoaModal::PlaceInService(form_state));
            }
            PickerAction::Cancelled => {
                let form_state = match self.modal.take() {
                    Some(CoaModal::PlaceInServicePicking(f, _, _)) => f,
                    _ => return TabAction::None,
                };
                self.modal = Some(CoaModal::PlaceInService(form_state));
            }
            PickerAction::Pending => {}
        }
        TabAction::None
    }

    fn submit_place_in_service(&mut self, db: &EntityDb) -> TabAction {
        let (cip_id, cip_name, target_id, date_str, months_str, accum_id, expense_id) = {
            let f = match &self.modal {
                Some(CoaModal::PlaceInService(f)) => f,
                _ => return TabAction::None,
            };
            (
                f.cip_account_id,
                f.cip_name.clone(),
                f.target_id,
                f.date_input.trim().to_string(),
                f.months_input.trim().to_string(),
                f.accum_id,
                f.expense_id,
            )
        };

        let target_id = match target_id {
            Some(id) => id,
            None => {
                if let Some(CoaModal::PlaceInService(f)) = &mut self.modal {
                    f.error = Some("Target fixed asset account is required.".to_string());
                    f.focused_field = PisField::Target;
                }
                return TabAction::None;
            }
        };

        let in_service_date = match NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => {
                if let Some(CoaModal::PlaceInService(f)) = &mut self.modal {
                    f.error = Some("Date must be YYYY-MM-DD.".to_string());
                    f.focused_field = PisField::Date;
                }
                return TabAction::None;
            }
        };

        let useful_life_months: u32 = match months_str.parse::<u32>() {
            Ok(m) if m > 0 => m,
            _ => {
                if let Some(CoaModal::PlaceInService(f)) = &mut self.modal {
                    f.error = Some("Useful life must be a positive integer (months).".to_string());
                    f.focused_field = PisField::Months;
                }
                return TabAction::None;
            }
        };

        match db.assets().place_in_service(
            cip_id,
            target_id,
            in_service_date,
            useful_life_months,
            accum_id,
            expense_id,
        ) {
            Err(e) => {
                if let Some(CoaModal::PlaceInService(f)) = &mut self.modal {
                    f.error = Some(format!("Failed: {e}"));
                }
                TabAction::None
            }
            Ok(je_id) => {
                let desc = format!(
                    "Placed {} in service → account id {}; life {} mo; JE #{}",
                    cip_name,
                    i64::from(target_id),
                    useful_life_months,
                    i64::from(je_id),
                );
                if let Err(e) = db.audit().append(
                    AuditAction::PlaceInService,
                    &self.entity_name,
                    Some("FixedAsset"),
                    Some(i64::from(target_id)),
                    &desc,
                ) {
                    tracing::error!("Failed to write audit log: {e}");
                }
                self.modal = None;
                TabAction::RefreshData
            }
        }
    }

    // ── Render helpers ────────────────────────────────────────────────────────

    fn render_table(&self, frame: &mut Frame, area: Rect) {
        let rows = self.current_rows();
        let table = make_account_table(
            rows,
            &self.balances,
            &self.envelope_balances,
            &self.collapsed,
        );
        let mut state = if self.search_active {
            self.filtered_state.clone()
        } else {
            self.table_state.clone()
        };
        frame.render_stateful_widget(table, area, &mut state);
    }

    fn render_add_form(&self, frame: &mut Frame, area: Rect, form: &AddFormState) {
        let modal_area = centered_rect(60, 60, area);
        frame.render_widget(Clear, modal_area);

        let field_labels = ["Number", "Name", "Type", "Parent", "Contra", "Placeholder"];
        let type_str = form.account_type.to_string();
        let parent_str = if form.parent_display.is_empty() {
            "(none — Enter to pick)".to_string()
        } else {
            form.parent_display.clone()
        };
        let values: [&str; 6] = [
            &form.number,
            &form.name,
            &type_str,
            &parent_str,
            if form.is_contra { "Yes" } else { "No" },
            if form.is_placeholder { "Yes" } else { "No" },
        ];

        let lines: Vec<Line> = (0..AddFormState::field_count())
            .map(|i| {
                let label = field_labels[i];
                let value = values[i];
                let cursor = if i == form.focused_field { "█" } else { "" };
                let style = if i == form.focused_field {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                Line::from(vec![
                    Span::styled(format!("  {:<12} ", label), style),
                    Span::raw(value),
                    Span::styled(cursor, Style::default().fg(Color::Yellow)),
                ])
            })
            .collect();

        let mut all_lines = vec![Line::from(Span::raw(""))];
        all_lines.extend(lines);
        if let Some(err) = &form.error {
            all_lines.push(Line::from(Span::raw("")));
            all_lines.push(Line::from(Span::styled(
                format!("  {err}"),
                Style::default().fg(Color::Red),
            )));
        }
        all_lines.push(Line::from(Span::raw("")));
        all_lines.push(Line::from(vec![
            Span::styled("  Tab/↑↓: navigate  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Space/←→: toggle/cycle  ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("Enter: save  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc: cancel", Style::default().fg(Color::DarkGray)),
        ]));

        frame.render_widget(
            Paragraph::new(all_lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Add Account ")
                    .style(Style::default().fg(Color::Cyan)),
            ),
            modal_area,
        );
    }

    fn render_edit_form(&self, frame: &mut Frame, area: Rect, form: &EditFormState) {
        let modal_area = centered_rect(60, 40, area);
        frame.render_widget(Clear, modal_area);

        let fields = [
            ("Name", form.name.as_str()),
            ("Number", form.number.as_str()),
        ];
        let mut lines = vec![Line::from(Span::raw(""))];
        for (i, (label, value)) in fields.iter().enumerate() {
            let cursor = if i == form.focused_field { "█" } else { "" };
            let style = if i == form.focused_field {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<12} ", label), style),
                Span::raw(*value),
                Span::styled(cursor, Style::default().fg(Color::Yellow)),
            ]));
        }
        if let Some(err) = &form.error {
            lines.push(Line::from(Span::raw("")));
            lines.push(Line::from(Span::styled(
                format!("  {err}"),
                Style::default().fg(Color::Red),
            )));
        }
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "  Tab: next field  Enter: save  Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )));

        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Edit Account {} ", form.original_number))
                    .style(Style::default().fg(Color::Cyan)),
            ),
            modal_area,
        );
    }

    fn render_confirm_toggle(&self, frame: &mut Frame, area: Rect, state: &ConfirmToggleState) {
        state.confirm.render(frame, area);
    }

    fn render_place_in_service(
        &self,
        frame: &mut Frame,
        area: Rect,
        form: &PlaceInServiceFormState,
    ) {
        let modal_area = centered_rect(65, 65, area);
        frame.render_widget(Clear, modal_area);

        let field_defs: &[(&str, PisField, &str)] = &[
            ("Target Account", PisField::Target, &form.target_name),
            ("Accum. Dep. Acct", PisField::Accum, &form.accum_name),
            ("Dep. Expense Acct", PisField::Expense, &form.expense_name),
            ("In-Service Date", PisField::Date, &form.date_input),
            ("Useful Life (mo)", PisField::Months, &form.months_input),
        ];

        let mut lines = vec![
            Line::from(Span::raw("")),
            Line::from(vec![
                Span::styled("  CIP Account: ", Style::default().fg(Color::DarkGray)),
                Span::raw(form.cip_name.clone()),
            ]),
            Line::from(Span::raw("")),
        ];

        for (label, field, value) in field_defs {
            let focused = form.focused_field == *field;
            let is_optional = matches!(field, PisField::Accum | PisField::Expense);
            let cursor = if focused { "█" } else { "" };
            let label_style = if focused {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            let hint = if is_optional && value.is_empty() {
                "(optional — Enter to pick)"
            } else if field.is_picker() && value.is_empty() {
                "(Enter to pick)"
            } else {
                ""
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<20} ", label), label_style),
                Span::raw((*value).to_string()),
                Span::styled(hint, Style::default().fg(Color::DarkGray)),
                Span::styled(cursor, Style::default().fg(Color::Yellow)),
            ]));
        }

        if let Some(err) = &form.error {
            lines.push(Line::from(Span::raw("")));
            lines.push(Line::from(Span::styled(
                format!("  {err}"),
                Style::default().fg(Color::Red),
            )));
        }
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "  Tab/↑↓: navigate  Enter: pick/next  Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )));

        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Place in Service ")
                    .style(Style::default().fg(Color::Green)),
            ),
            modal_area,
        );
    }
}

// ── Tab trait ─────────────────────────────────────────────────────────────────

impl Tab for ChartOfAccountsTab {
    fn title(&self) -> &str {
        "Chart of Accounts"
    }

    fn handle_key(&mut self, key: KeyEvent, db: &EntityDb) -> TabAction {
        // Modal takes priority.
        match &self.modal {
            Some(CoaModal::AddForm(_)) => {
                return self.handle_add_form_key(key, db);
            }
            Some(CoaModal::AddFormPickingParent(_, _)) => {
                return self.handle_add_form_picker_key(key);
            }
            Some(CoaModal::EditForm(_)) => {
                return self.handle_edit_form_key(key, db);
            }
            Some(CoaModal::ConfirmToggle(_)) => {
                return self.handle_confirm_toggle_key(key, db);
            }
            Some(CoaModal::ConfirmDelete(_)) => {
                return self.handle_confirm_delete_key(key, db);
            }
            Some(CoaModal::PlaceInService(_)) => {
                return self.handle_place_in_service_key(key, db);
            }
            Some(CoaModal::PlaceInServicePicking(_, _, _)) => {
                return self.handle_place_in_service_picker_key(key);
            }
            None => {}
        }

        if key.modifiers != KeyModifiers::NONE && key.modifiers != KeyModifiers::SHIFT {
            return TabAction::None;
        }

        if self.search_active {
            match key.code {
                KeyCode::Esc => {
                    self.search_active = false;
                    self.search_query.clear();
                    self.update_filter();
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.update_filter();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.update_filter();
                }
                KeyCode::Up => self.scroll_up(),
                KeyCode::Down => self.scroll_down(),
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.scroll_up(),
                KeyCode::Down | KeyCode::Char('j') => self.scroll_down(),
                KeyCode::Enter => {
                    // Group accounts (has children): toggle expand/collapse.
                    // Leaf accounts: navigate to the General Ledger for that account.
                    let has_children = self
                        .selected_account()
                        .and_then(|acc| {
                            self.current_rows()
                                .iter()
                                .find(|r| r.account.id == acc.id)
                                .map(|r| r.has_children)
                        })
                        .unwrap_or(false);
                    if has_children {
                        self.toggle_expand();
                    } else if let Some(acc) = self.selected_account() {
                        return TabAction::NavigateTo(
                            crate::tabs::TabId::GeneralLedger,
                            crate::tabs::RecordId::Account(acc.id),
                        );
                    }
                }
                KeyCode::Char('/') => {
                    self.search_active = true;
                    self.search_query.clear();
                    self.update_filter();
                }
                KeyCode::Char('a') => self.open_add_form(),
                KeyCode::Char('e') => self.open_edit_form(),
                KeyCode::Char('d') => self.open_confirm_toggle(),
                KeyCode::Char('x') => self.open_confirm_delete(),
                KeyCode::Char('s') => self.open_place_in_service(),
                _ => {}
            }
        }
        TabAction::None
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let hint_height = if self.search_active { 2 } else { 1 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(hint_height)])
            .split(area);

        // ── Account table ──────────────────────────────────────────────────────
        self.render_table(frame, chunks[0]);

        // ── Bottom hint bar ────────────────────────────────────────────────────
        if self.search_active {
            let bottom = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Length(1)])
                .split(chunks[1]);

            let search_line = Line::from(vec![
                Span::styled(" Search: ", Style::default().fg(Color::Yellow)),
                Span::raw(self.search_query.clone()),
                Span::styled("█", Style::default().fg(Color::Yellow)),
            ]);
            frame.render_widget(Paragraph::new(search_line), bottom[0]);

            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    " Esc: cancel search  ↑↓: navigate",
                    Style::default().fg(Color::DarkGray),
                ))),
                bottom[1],
            );
        } else {
            let count = self.visible.len();
            let selected = self.selected_idx().map(|i| i + 1).unwrap_or(0);
            let is_cip = self
                .selected_account()
                .map(|a| a.name.to_lowercase().contains("construction"))
                .unwrap_or(false);
            let hint = if is_cip {
                " ↑↓/jk: navigate  Enter: expand/GL  /: search  a: add  e: edit  d: toggle active  x: delete  s: place in service"
            } else {
                " ↑↓/jk: navigate  Enter: expand/GL  /: search  a: add  e: edit  d: toggle active  x: delete"
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(hint, Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("  [{}/{}]", selected, count),
                        Style::default().fg(Color::Gray),
                    ),
                ])),
                chunks[1],
            );
        }

        // ── Modal overlay ──────────────────────────────────────────────────────
        if let Some(ref modal) = self.modal {
            match modal {
                CoaModal::AddForm(f) => self.render_add_form(frame, area, f),
                CoaModal::AddFormPickingParent(_, picker) => {
                    picker.render(frame, area, &self.all_accounts);
                }
                CoaModal::EditForm(f) => self.render_edit_form(frame, area, f),
                CoaModal::ConfirmToggle(s) => self.render_confirm_toggle(frame, area, s),
                CoaModal::ConfirmDelete(s) => s.confirm.render(frame, area),
                CoaModal::PlaceInService(f) => self.render_place_in_service(frame, area, f),
                CoaModal::PlaceInServicePicking(_, picker, _) => {
                    picker.render(frame, area, &self.all_accounts);
                }
            }
        }
    }

    fn wants_input(&self) -> bool {
        self.modal.is_some() || self.search_active
    }

    fn refresh(&mut self, db: &EntityDb) {
        let repo = db.accounts();
        match repo.list_all() {
            Err(e) => {
                tracing::error!("CoA tab: failed to load accounts: {e}");
                return;
            }
            Ok(accounts) => {
                self.all_accounts = accounts;
            }
        }

        match repo.get_all_balances() {
            Ok(balances) => self.balances = balances,
            Err(e) => {
                tracing::error!("CoA tab: failed to load balances: {e}");
                self.balances.clear();
            }
        }

        // Load envelope earmarked amounts for accounts with allocations.
        let mut env_bals = HashMap::new();
        if let Ok(allocs) = db.envelopes().get_all_allocations() {
            for alloc in allocs {
                if let Ok(bal) = db.envelopes().get_balance(alloc.account_id)
                    && bal.0 != 0
                {
                    env_bals.insert(alloc.account_id, bal);
                }
            }
        }
        self.envelope_balances = env_bals;

        self.build_visible();
        if self.search_active {
            self.update_filter();
        }
    }

    fn navigate_to(&mut self, record_id: RecordId, _db: &EntityDb) {
        if let RecordId::Account(aid) = record_id
            && let Some(idx) = self.visible.iter().position(|r| r.account.id == aid)
        {
            self.table_state.select(Some(idx));
        }
    }
}

// ── Free functions ────────────────────────────────────────────────────────────

fn flatten_tree(
    parent: Option<AccountId>,
    depth: usize,
    children_map: &HashMap<Option<AccountId>, Vec<&Account>>,
    collapsed: &HashSet<AccountId>,
    is_parent: &HashSet<AccountId>,
    result: &mut Vec<VisibleRow>,
) {
    let Some(children) = children_map.get(&parent) else {
        return;
    };
    for acc in children {
        let has_children = is_parent.contains(&acc.id);
        result.push(VisibleRow {
            account: (*acc).clone(),
            depth,
            has_children,
        });
        if has_children && !collapsed.contains(&acc.id) {
            flatten_tree(
                Some(acc.id),
                depth + 1,
                children_map,
                collapsed,
                is_parent,
                result,
            );
        }
    }
}

fn make_account_table<'a>(
    rows: &'a [VisibleRow],
    balances: &'a HashMap<AccountId, Money>,
    envelope_balances: &'a HashMap<AccountId, Money>,
    collapsed: &'a HashSet<AccountId>,
) -> Table<'a> {
    let show_earmarked = !envelope_balances.is_empty();

    let mut header_cells = vec![
        Cell::from("Number").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Name").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Type").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Balance").style(Style::default().add_modifier(Modifier::BOLD)),
    ];
    if show_earmarked {
        header_cells
            .push(Cell::from("Earmarked").style(Style::default().add_modifier(Modifier::BOLD)));
    }
    header_cells.push(Cell::from("Flags").style(Style::default().add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).style(Style::default().bg(Color::DarkGray));

    let table_rows: Vec<Row> = rows
        .iter()
        .map(|vr| {
            let acc = &vr.account;

            let indent = "  ".repeat(vr.depth);
            let indicator = if vr.has_children {
                if collapsed.contains(&acc.id) {
                    "▶ "
                } else {
                    "▼ "
                }
            } else {
                "  "
            };
            let name_cell = format!("{}{}{}", indent, indicator, acc.name);

            let type_str = match acc.account_type {
                AccountType::Asset => "Asset",
                AccountType::Liability => "Liab",
                AccountType::Equity => "Equity",
                AccountType::Revenue => "Rev",
                AccountType::Expense => "Exp",
            };

            let balance = balances.get(&acc.id).copied().unwrap_or(Money(0));

            let mut flags = String::new();
            if acc.is_placeholder {
                flags.push('P');
            }
            if acc.is_contra {
                flags.push('C');
            }
            if !acc.is_active {
                flags.push('x');
            }

            let row_style = if !acc.is_active {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };

            let mut cells = vec![
                Cell::from(acc.number.clone()),
                Cell::from(name_cell),
                Cell::from(type_str),
                Cell::from(balance.to_string()),
            ];
            if show_earmarked {
                let earmark_cell = match envelope_balances.get(&acc.id) {
                    Some(&amt) => Cell::from(Span::styled(
                        amt.to_string(),
                        Style::default().fg(Color::Cyan),
                    )),
                    None => Cell::from("—"),
                };
                cells.push(earmark_cell);
            }
            cells.push(Cell::from(flags));

            Row::new(cells).style(row_style)
        })
        .collect();

    let mut widths = vec![
        Constraint::Length(8),  // Number
        Constraint::Min(10),    // Name (gets remaining space)
        Constraint::Length(7),  // Type
        Constraint::Length(14), // Balance
    ];
    if show_earmarked {
        widths.push(Constraint::Length(14)); // Earmarked
    }
    widths.push(Constraint::Length(5)); // Flags

    Table::new(table_rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Chart of Accounts "),
        )
        .row_highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("» ")
}

fn cycle_account_type(current: AccountType, forward: bool) -> AccountType {
    let types = [
        AccountType::Asset,
        AccountType::Liability,
        AccountType::Equity,
        AccountType::Revenue,
        AccountType::Expense,
    ];
    let pos = types.iter().position(|t| *t == current).unwrap_or(0);
    let next = if forward {
        (pos + 1) % types.len()
    } else {
        (pos + types.len() - 1) % types.len()
    };
    types[next]
}
