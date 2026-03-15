use std::fmt;
use std::str::FromStr;

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountType {
    Asset,
    Liability,
    Equity,
    Revenue,
    Expense,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalanceDirection {
    Debit,
    Credit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileState {
    Uncleared,
    Cleared,
    Reconciled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalEntryStatus {
    Draft,
    Posted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArApStatus {
    Open,
    Partial,
    Paid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryFrequency {
    Monthly,
    Quarterly,
    Annually,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeEntryType {
    Fill,
    Transfer,
    Reversal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAction {
    JournalEntryCreated,
    JournalEntryPosted,
    JournalEntryReversed,
    AccountCreated,
    AccountModified,
    AccountDeactivated,
    AccountReactivated,
    PeriodClosed,
    PeriodReopened,
    YearEndClose,
    EnvelopeAllocationChanged,
    EnvelopeTransfer,
    PlaceInService,
    InterEntityEntryPosted,
}

// ── AccountType ──────────────────────────────────────────────────────────────

impl AccountType {
    /// Returns the normal balance direction for this account type.
    pub fn normal_balance(self) -> BalanceDirection {
        match self {
            AccountType::Asset | AccountType::Expense => BalanceDirection::Debit,
            AccountType::Liability | AccountType::Equity | AccountType::Revenue => {
                BalanceDirection::Credit
            }
        }
    }
}

impl fmt::Display for AccountType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccountType::Asset => write!(f, "Asset"),
            AccountType::Liability => write!(f, "Liability"),
            AccountType::Equity => write!(f, "Equity"),
            AccountType::Revenue => write!(f, "Revenue"),
            AccountType::Expense => write!(f, "Expense"),
        }
    }
}

#[derive(Debug, Error)]
#[error("Unknown account type: '{0}'")]
pub struct UnknownAccountType(String);

impl FromStr for AccountType {
    type Err = UnknownAccountType;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Asset" => Ok(AccountType::Asset),
            "Liability" => Ok(AccountType::Liability),
            "Equity" => Ok(AccountType::Equity),
            "Revenue" => Ok(AccountType::Revenue),
            "Expense" => Ok(AccountType::Expense),
            _ => Err(UnknownAccountType(s.to_owned())),
        }
    }
}

// ── BalanceDirection ─────────────────────────────────────────────────────────

impl fmt::Display for BalanceDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BalanceDirection::Debit => write!(f, "Debit"),
            BalanceDirection::Credit => write!(f, "Credit"),
        }
    }
}

#[derive(Debug, Error)]
#[error("Unknown balance direction: '{0}'")]
pub struct UnknownBalanceDirection(String);

impl FromStr for BalanceDirection {
    type Err = UnknownBalanceDirection;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Debit" => Ok(BalanceDirection::Debit),
            "Credit" => Ok(BalanceDirection::Credit),
            _ => Err(UnknownBalanceDirection(s.to_owned())),
        }
    }
}

// ── ReconcileState ───────────────────────────────────────────────────────────

impl fmt::Display for ReconcileState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReconcileState::Uncleared => write!(f, "Uncleared"),
            ReconcileState::Cleared => write!(f, "Cleared"),
            ReconcileState::Reconciled => write!(f, "Reconciled"),
        }
    }
}

#[derive(Debug, Error)]
#[error("Unknown reconcile state: '{0}'")]
pub struct UnknownReconcileState(String);

impl FromStr for ReconcileState {
    type Err = UnknownReconcileState;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Uncleared" => Ok(ReconcileState::Uncleared),
            "Cleared" => Ok(ReconcileState::Cleared),
            "Reconciled" => Ok(ReconcileState::Reconciled),
            _ => Err(UnknownReconcileState(s.to_owned())),
        }
    }
}

// ── JournalEntryStatus ───────────────────────────────────────────────────────

impl fmt::Display for JournalEntryStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JournalEntryStatus::Draft => write!(f, "Draft"),
            JournalEntryStatus::Posted => write!(f, "Posted"),
        }
    }
}

#[derive(Debug, Error)]
#[error("Unknown journal entry status: '{0}'")]
pub struct UnknownJournalEntryStatus(String);

impl FromStr for JournalEntryStatus {
    type Err = UnknownJournalEntryStatus;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Draft" => Ok(JournalEntryStatus::Draft),
            "Posted" => Ok(JournalEntryStatus::Posted),
            _ => Err(UnknownJournalEntryStatus(s.to_owned())),
        }
    }
}

// ── ArApStatus ───────────────────────────────────────────────────────────────

impl fmt::Display for ArApStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArApStatus::Open => write!(f, "Open"),
            ArApStatus::Partial => write!(f, "Partial"),
            ArApStatus::Paid => write!(f, "Paid"),
        }
    }
}

#[derive(Debug, Error)]
#[error("Unknown AR/AP status: '{0}'")]
pub struct UnknownArApStatus(String);

impl FromStr for ArApStatus {
    type Err = UnknownArApStatus;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Open" => Ok(ArApStatus::Open),
            "Partial" => Ok(ArApStatus::Partial),
            "Paid" => Ok(ArApStatus::Paid),
            _ => Err(UnknownArApStatus(s.to_owned())),
        }
    }
}

// ── EntryFrequency ───────────────────────────────────────────────────────────

impl fmt::Display for EntryFrequency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntryFrequency::Monthly => write!(f, "Monthly"),
            EntryFrequency::Quarterly => write!(f, "Quarterly"),
            EntryFrequency::Annually => write!(f, "Annually"),
        }
    }
}

#[derive(Debug, Error)]
#[error("Unknown entry frequency: '{0}'")]
pub struct UnknownEntryFrequency(String);

impl FromStr for EntryFrequency {
    type Err = UnknownEntryFrequency;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Monthly" => Ok(EntryFrequency::Monthly),
            "Quarterly" => Ok(EntryFrequency::Quarterly),
            "Annually" => Ok(EntryFrequency::Annually),
            _ => Err(UnknownEntryFrequency(s.to_owned())),
        }
    }
}

// ── EnvelopeEntryType ────────────────────────────────────────────────────────

impl fmt::Display for EnvelopeEntryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvelopeEntryType::Fill => write!(f, "Fill"),
            EnvelopeEntryType::Transfer => write!(f, "Transfer"),
            EnvelopeEntryType::Reversal => write!(f, "Reversal"),
        }
    }
}

#[derive(Debug, Error)]
#[error("Unknown envelope entry type: '{0}'")]
pub struct UnknownEnvelopeEntryType(String);

impl FromStr for EnvelopeEntryType {
    type Err = UnknownEnvelopeEntryType;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Fill" => Ok(EnvelopeEntryType::Fill),
            "Transfer" => Ok(EnvelopeEntryType::Transfer),
            "Reversal" => Ok(EnvelopeEntryType::Reversal),
            _ => Err(UnknownEnvelopeEntryType(s.to_owned())),
        }
    }
}

// ── AuditAction ──────────────────────────────────────────────────────────────

impl fmt::Display for AuditAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuditAction::JournalEntryCreated => write!(f, "JournalEntryCreated"),
            AuditAction::JournalEntryPosted => write!(f, "JournalEntryPosted"),
            AuditAction::JournalEntryReversed => write!(f, "JournalEntryReversed"),
            AuditAction::AccountCreated => write!(f, "AccountCreated"),
            AuditAction::AccountModified => write!(f, "AccountModified"),
            AuditAction::AccountDeactivated => write!(f, "AccountDeactivated"),
            AuditAction::AccountReactivated => write!(f, "AccountReactivated"),
            AuditAction::PeriodClosed => write!(f, "PeriodClosed"),
            AuditAction::PeriodReopened => write!(f, "PeriodReopened"),
            AuditAction::YearEndClose => write!(f, "YearEndClose"),
            AuditAction::EnvelopeAllocationChanged => write!(f, "EnvelopeAllocationChanged"),
            AuditAction::EnvelopeTransfer => write!(f, "EnvelopeTransfer"),
            AuditAction::PlaceInService => write!(f, "PlaceInService"),
            AuditAction::InterEntityEntryPosted => write!(f, "InterEntityEntryPosted"),
        }
    }
}

#[derive(Debug, Error)]
#[error("Unknown audit action: '{0}'")]
pub struct UnknownAuditAction(String);

impl FromStr for AuditAction {
    type Err = UnknownAuditAction;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "JournalEntryCreated" => Ok(AuditAction::JournalEntryCreated),
            "JournalEntryPosted" => Ok(AuditAction::JournalEntryPosted),
            "JournalEntryReversed" => Ok(AuditAction::JournalEntryReversed),
            "AccountCreated" => Ok(AuditAction::AccountCreated),
            "AccountModified" => Ok(AuditAction::AccountModified),
            "AccountDeactivated" => Ok(AuditAction::AccountDeactivated),
            "AccountReactivated" => Ok(AuditAction::AccountReactivated),
            "PeriodClosed" => Ok(AuditAction::PeriodClosed),
            "PeriodReopened" => Ok(AuditAction::PeriodReopened),
            "YearEndClose" => Ok(AuditAction::YearEndClose),
            "EnvelopeAllocationChanged" => Ok(AuditAction::EnvelopeAllocationChanged),
            "EnvelopeTransfer" => Ok(AuditAction::EnvelopeTransfer),
            "PlaceInService" => Ok(AuditAction::PlaceInService),
            "InterEntityEntryPosted" => Ok(AuditAction::InterEntityEntryPosted),
            _ => Err(UnknownAuditAction(s.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! round_trip_tests {
        ($enum_type:ident, $($variant:ident),+) => {
            $(
                {
                    let s = $enum_type::$variant.to_string();
                    let parsed: $enum_type = s.parse().expect("round-trip parse failed");
                    assert_eq!(parsed, $enum_type::$variant);
                }
            )+
        };
    }

    #[test]
    fn account_type_round_trips() {
        round_trip_tests!(AccountType, Asset, Liability, Equity, Revenue, Expense);
    }

    #[test]
    fn balance_direction_round_trips() {
        round_trip_tests!(BalanceDirection, Debit, Credit);
    }

    #[test]
    fn reconcile_state_round_trips() {
        round_trip_tests!(ReconcileState, Uncleared, Cleared, Reconciled);
    }

    #[test]
    fn journal_entry_status_round_trips() {
        round_trip_tests!(JournalEntryStatus, Draft, Posted);
    }

    #[test]
    fn ar_ap_status_round_trips() {
        round_trip_tests!(ArApStatus, Open, Partial, Paid);
    }

    #[test]
    fn entry_frequency_round_trips() {
        round_trip_tests!(EntryFrequency, Monthly, Quarterly, Annually);
    }

    #[test]
    fn envelope_entry_type_round_trips() {
        round_trip_tests!(EnvelopeEntryType, Fill, Transfer, Reversal);
    }

    #[test]
    fn audit_action_round_trips() {
        round_trip_tests!(
            AuditAction,
            JournalEntryCreated,
            JournalEntryPosted,
            JournalEntryReversed,
            AccountCreated,
            AccountModified,
            AccountDeactivated,
            AccountReactivated,
            PeriodClosed,
            PeriodReopened,
            YearEndClose,
            EnvelopeAllocationChanged,
            EnvelopeTransfer,
            PlaceInService,
            InterEntityEntryPosted
        );
    }

    #[test]
    fn asset_normal_balance_is_debit() {
        assert_eq!(AccountType::Asset.normal_balance(), BalanceDirection::Debit);
    }

    #[test]
    fn expense_normal_balance_is_debit() {
        assert_eq!(
            AccountType::Expense.normal_balance(),
            BalanceDirection::Debit
        );
    }

    #[test]
    fn liability_normal_balance_is_credit() {
        assert_eq!(
            AccountType::Liability.normal_balance(),
            BalanceDirection::Credit
        );
    }

    #[test]
    fn equity_normal_balance_is_credit() {
        assert_eq!(
            AccountType::Equity.normal_balance(),
            BalanceDirection::Credit
        );
    }

    #[test]
    fn revenue_normal_balance_is_credit() {
        assert_eq!(
            AccountType::Revenue.normal_balance(),
            BalanceDirection::Credit
        );
    }

    #[test]
    fn unknown_variant_returns_error() {
        assert!("Bogus".parse::<AccountType>().is_err());
        assert!("bogus".parse::<ReconcileState>().is_err());
    }
}
