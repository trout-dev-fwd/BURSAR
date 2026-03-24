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
    JournalEntryDeleted,
    AccountCreated,
    AccountModified,
    AccountDeactivated,
    AccountReactivated,
    AccountDeleted,
    PeriodClosed,
    PeriodReopened,
    YearEndClose,
    EnvelopeAllocationChanged,
    EnvelopeTransfer,
    PlaceInService,
    InterEntityEntryPosted,
    ArItemCreated,
    ArPaymentRecorded,
    ApItemCreated,
    ApPaymentRecorded,
    // V2: AI interaction audit entries
    AiPrompt,
    AiResponse,
    AiToolUse,
    CsvImport,
    MappingLearned,
}

// ── ImportMatchType ───────────────────────────────────────────────────────────

/// How a description pattern matches a bank transaction description.
/// Stored as TEXT in `import_mappings.match_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMatchType {
    Exact,
    Substring,
}

impl fmt::Display for ImportMatchType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportMatchType::Exact => write!(f, "exact"),
            ImportMatchType::Substring => write!(f, "substring"),
        }
    }
}

#[derive(Debug, Error)]
#[error("Unknown import match type: '{0}'")]
pub struct UnknownImportMatchType(String);

impl FromStr for ImportMatchType {
    type Err = UnknownImportMatchType;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "exact" => Ok(ImportMatchType::Exact),
            "substring" => Ok(ImportMatchType::Substring),
            _ => Err(UnknownImportMatchType(s.to_owned())),
        }
    }
}

// ── ImportMatchSource ─────────────────────────────────────────────────────────

/// How an import mapping was established.
/// Stored as TEXT in `import_mappings.source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMatchSource {
    Confirmed,
    AiSuggested,
}

impl fmt::Display for ImportMatchSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportMatchSource::Confirmed => write!(f, "confirmed"),
            ImportMatchSource::AiSuggested => write!(f, "ai_suggested"),
        }
    }
}

#[derive(Debug, Error)]
#[error("Unknown import match source: '{0}'")]
pub struct UnknownImportMatchSource(String);

impl FromStr for ImportMatchSource {
    type Err = UnknownImportMatchSource;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "confirmed" => Ok(ImportMatchSource::Confirmed),
            "ai_suggested" => Ok(ImportMatchSource::AiSuggested),
            _ => Err(UnknownImportMatchSource(s.to_owned())),
        }
    }
}

// ── AiRequestState ────────────────────────────────────────────────────────────

/// Tracks the current state of an AI API interaction for UI display.
/// In-memory only — not persisted to the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiRequestState {
    Idle,
    CallingApi,
    FulfillingTools,
}

// ── ChatRole ──────────────────────────────────────────────────────────────────

/// Identifies the sender of a chat message.
/// In-memory only — not persisted to the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

// ── FocusTarget ───────────────────────────────────────────────────────────────

/// Tracks which UI element has keyboard focus when the chat panel is open.
/// In-memory only — not persisted to the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    MainTab,
    ChatPanel,
}

// ── MatchSource ───────────────────────────────────────────────────────────────

/// How a transaction-to-account match was determined during CSV import.
/// In-memory only — not persisted to the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchSource {
    Local,
    Ai,
    UserConfirmed,
    Unmatched,
    /// The transaction was identified as the other side of an existing draft JE
    /// (a cross-bank transfer). Details are stored in `ImportMatch::transfer_match`.
    TransferMatch,
}

// ── MatchConfidence ───────────────────────────────────────────────────────────

/// Confidence level for AI-suggested import matches.
/// In-memory only — not persisted to the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchConfidence {
    High,
    Medium,
    Low,
}

// ── TaxReviewStatus ───────────────────────────────────────────────────────────

/// The review lifecycle state of a single journal entry in the Tax tab.
/// Stored as TEXT in `tax_tags.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaxReviewStatus {
    Unreviewed,
    AiPending,
    AiSuggested,
    Confirmed,
    NonDeductible,
}

impl fmt::Display for TaxReviewStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaxReviewStatus::Unreviewed => write!(f, "unreviewed"),
            TaxReviewStatus::AiPending => write!(f, "ai_pending"),
            TaxReviewStatus::AiSuggested => write!(f, "ai_suggested"),
            TaxReviewStatus::Confirmed => write!(f, "confirmed"),
            TaxReviewStatus::NonDeductible => write!(f, "non_deductible"),
        }
    }
}

#[derive(Debug, Error)]
#[error("Unknown tax review status: '{0}'")]
pub struct UnknownTaxReviewStatus(String);

impl FromStr for TaxReviewStatus {
    type Err = UnknownTaxReviewStatus;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "unreviewed" => Ok(TaxReviewStatus::Unreviewed),
            "ai_pending" => Ok(TaxReviewStatus::AiPending),
            "ai_suggested" => Ok(TaxReviewStatus::AiSuggested),
            "confirmed" => Ok(TaxReviewStatus::Confirmed),
            "non_deductible" => Ok(TaxReviewStatus::NonDeductible),
            _ => Err(UnknownTaxReviewStatus(s.to_owned())),
        }
    }
}

// ── TaxFormTag ────────────────────────────────────────────────────────────────

/// A tax form classification for a journal entry.
/// Stored as TEXT in `tax_tags.form_tag` and `tax_tags.ai_suggested_form`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaxFormTag {
    ScheduleC,
    ScheduleAMedical,
    ScheduleATaxes,
    ScheduleAInterest,
    ScheduleACharity,
    ScheduleD,
    ScheduleE,
    ScheduleSe,
    Form4562,
    Form8829,
    Form4797,
    Form1120s,
    EstimatedPayment,
    NonDeductible,
}

impl TaxFormTag {
    /// Returns all variants in display order.
    pub fn all() -> Vec<TaxFormTag> {
        vec![
            TaxFormTag::ScheduleC,
            TaxFormTag::ScheduleAMedical,
            TaxFormTag::ScheduleATaxes,
            TaxFormTag::ScheduleAInterest,
            TaxFormTag::ScheduleACharity,
            TaxFormTag::ScheduleD,
            TaxFormTag::ScheduleE,
            TaxFormTag::ScheduleSe,
            TaxFormTag::Form4562,
            TaxFormTag::Form8829,
            TaxFormTag::Form4797,
            TaxFormTag::Form1120s,
            TaxFormTag::EstimatedPayment,
            TaxFormTag::NonDeductible,
        ]
    }

    /// Human-readable display name (e.g. "Schedule C").
    pub fn display_name(&self) -> &str {
        match self {
            TaxFormTag::ScheduleC => "Schedule C",
            TaxFormTag::ScheduleAMedical => "Schedule A — Medical",
            TaxFormTag::ScheduleATaxes => "Schedule A — Taxes",
            TaxFormTag::ScheduleAInterest => "Schedule A — Interest",
            TaxFormTag::ScheduleACharity => "Schedule A — Charity",
            TaxFormTag::ScheduleD => "Schedule D",
            TaxFormTag::ScheduleE => "Schedule E",
            TaxFormTag::ScheduleSe => "Schedule SE",
            TaxFormTag::Form4562 => "Form 4562",
            TaxFormTag::Form8829 => "Form 8829",
            TaxFormTag::Form4797 => "Form 4797",
            TaxFormTag::Form1120s => "Form 1120-S",
            TaxFormTag::EstimatedPayment => "Form 1040-ES",
            TaxFormTag::NonDeductible => "Non-Deductible",
        }
    }

    /// Short description shown in the form picker.
    pub fn description(&self) -> &str {
        match self {
            TaxFormTag::ScheduleC => "Business income & expenses",
            TaxFormTag::ScheduleAMedical => "Medical & dental expenses",
            TaxFormTag::ScheduleATaxes => "State & local taxes paid",
            TaxFormTag::ScheduleAInterest => "Mortgage & investment interest",
            TaxFormTag::ScheduleACharity => "Charitable contributions",
            TaxFormTag::ScheduleD => "Capital gains & losses",
            TaxFormTag::ScheduleE => "Rental income & expenses",
            TaxFormTag::ScheduleSe => "Self-employment tax",
            TaxFormTag::Form4562 => "Depreciation & amortization",
            TaxFormTag::Form8829 => "Home office deduction",
            TaxFormTag::Form4797 => "Sale of business property",
            TaxFormTag::Form1120s => "S-Corporation return",
            TaxFormTag::EstimatedPayment => "Estimated tax payments",
            TaxFormTag::NonDeductible => "No deduction applies",
        }
    }
}

impl fmt::Display for TaxFormTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaxFormTag::ScheduleC => write!(f, "schedule_c"),
            TaxFormTag::ScheduleAMedical => write!(f, "schedule_a_medical"),
            TaxFormTag::ScheduleATaxes => write!(f, "schedule_a_taxes"),
            TaxFormTag::ScheduleAInterest => write!(f, "schedule_a_interest"),
            TaxFormTag::ScheduleACharity => write!(f, "schedule_a_charity"),
            TaxFormTag::ScheduleD => write!(f, "schedule_d"),
            TaxFormTag::ScheduleE => write!(f, "schedule_e"),
            TaxFormTag::ScheduleSe => write!(f, "schedule_se"),
            TaxFormTag::Form4562 => write!(f, "form_4562"),
            TaxFormTag::Form8829 => write!(f, "form_8829"),
            TaxFormTag::Form4797 => write!(f, "form_4797"),
            TaxFormTag::Form1120s => write!(f, "form_1120s"),
            TaxFormTag::EstimatedPayment => write!(f, "estimated_payment"),
            TaxFormTag::NonDeductible => write!(f, "non_deductible"),
        }
    }
}

#[derive(Debug, Error)]
#[error("Unknown tax form tag: '{0}'")]
pub struct UnknownTaxFormTag(String);

impl FromStr for TaxFormTag {
    type Err = UnknownTaxFormTag;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "schedule_c" => Ok(TaxFormTag::ScheduleC),
            "schedule_a_medical" => Ok(TaxFormTag::ScheduleAMedical),
            "schedule_a_taxes" => Ok(TaxFormTag::ScheduleATaxes),
            "schedule_a_interest" => Ok(TaxFormTag::ScheduleAInterest),
            "schedule_a_charity" => Ok(TaxFormTag::ScheduleACharity),
            "schedule_d" => Ok(TaxFormTag::ScheduleD),
            "schedule_e" => Ok(TaxFormTag::ScheduleE),
            "schedule_se" => Ok(TaxFormTag::ScheduleSe),
            "form_4562" => Ok(TaxFormTag::Form4562),
            "form_8829" => Ok(TaxFormTag::Form8829),
            "form_4797" => Ok(TaxFormTag::Form4797),
            "form_1120s" => Ok(TaxFormTag::Form1120s),
            "estimated_payment" => Ok(TaxFormTag::EstimatedPayment),
            "non_deductible" => Ok(TaxFormTag::NonDeductible),
            _ => Err(UnknownTaxFormTag(s.to_owned())),
        }
    }
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
            AuditAction::JournalEntryDeleted => write!(f, "JournalEntryDeleted"),
            AuditAction::AccountCreated => write!(f, "AccountCreated"),
            AuditAction::AccountModified => write!(f, "AccountModified"),
            AuditAction::AccountDeactivated => write!(f, "AccountDeactivated"),
            AuditAction::AccountReactivated => write!(f, "AccountReactivated"),
            AuditAction::AccountDeleted => write!(f, "AccountDeleted"),
            AuditAction::PeriodClosed => write!(f, "PeriodClosed"),
            AuditAction::PeriodReopened => write!(f, "PeriodReopened"),
            AuditAction::YearEndClose => write!(f, "YearEndClose"),
            AuditAction::EnvelopeAllocationChanged => write!(f, "EnvelopeAllocationChanged"),
            AuditAction::EnvelopeTransfer => write!(f, "EnvelopeTransfer"),
            AuditAction::PlaceInService => write!(f, "PlaceInService"),
            AuditAction::InterEntityEntryPosted => write!(f, "InterEntityEntryPosted"),
            AuditAction::ArItemCreated => write!(f, "ArItemCreated"),
            AuditAction::ArPaymentRecorded => write!(f, "ArPaymentRecorded"),
            AuditAction::ApItemCreated => write!(f, "ApItemCreated"),
            AuditAction::ApPaymentRecorded => write!(f, "ApPaymentRecorded"),
            AuditAction::AiPrompt => write!(f, "AiPrompt"),
            AuditAction::AiResponse => write!(f, "AiResponse"),
            AuditAction::AiToolUse => write!(f, "AiToolUse"),
            AuditAction::CsvImport => write!(f, "CsvImport"),
            AuditAction::MappingLearned => write!(f, "MappingLearned"),
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
            "JournalEntryDeleted" => Ok(AuditAction::JournalEntryDeleted),
            "AccountCreated" => Ok(AuditAction::AccountCreated),
            "AccountModified" => Ok(AuditAction::AccountModified),
            "AccountDeactivated" => Ok(AuditAction::AccountDeactivated),
            "AccountReactivated" => Ok(AuditAction::AccountReactivated),
            "AccountDeleted" => Ok(AuditAction::AccountDeleted),
            "PeriodClosed" => Ok(AuditAction::PeriodClosed),
            "PeriodReopened" => Ok(AuditAction::PeriodReopened),
            "YearEndClose" => Ok(AuditAction::YearEndClose),
            "EnvelopeAllocationChanged" => Ok(AuditAction::EnvelopeAllocationChanged),
            "EnvelopeTransfer" => Ok(AuditAction::EnvelopeTransfer),
            "PlaceInService" => Ok(AuditAction::PlaceInService),
            "InterEntityEntryPosted" => Ok(AuditAction::InterEntityEntryPosted),
            "ArItemCreated" => Ok(AuditAction::ArItemCreated),
            "ArPaymentRecorded" => Ok(AuditAction::ArPaymentRecorded),
            "ApItemCreated" => Ok(AuditAction::ApItemCreated),
            "ApPaymentRecorded" => Ok(AuditAction::ApPaymentRecorded),
            "AiPrompt" => Ok(AuditAction::AiPrompt),
            "AiResponse" => Ok(AuditAction::AiResponse),
            "AiToolUse" => Ok(AuditAction::AiToolUse),
            "CsvImport" => Ok(AuditAction::CsvImport),
            "MappingLearned" => Ok(AuditAction::MappingLearned),
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
            JournalEntryDeleted,
            AccountCreated,
            AccountModified,
            AccountDeactivated,
            AccountReactivated,
            AccountDeleted,
            PeriodClosed,
            PeriodReopened,
            YearEndClose,
            EnvelopeAllocationChanged,
            EnvelopeTransfer,
            PlaceInService,
            InterEntityEntryPosted,
            ArItemCreated,
            ArPaymentRecorded,
            ApItemCreated,
            ApPaymentRecorded,
            AiPrompt,
            AiResponse,
            AiToolUse,
            CsvImport,
            MappingLearned
        );
    }

    #[test]
    fn import_match_type_round_trips() {
        assert_eq!(ImportMatchType::Exact.to_string(), "exact");
        assert_eq!(ImportMatchType::Substring.to_string(), "substring");
        assert_eq!(
            "exact".parse::<ImportMatchType>().unwrap(),
            ImportMatchType::Exact
        );
        assert_eq!(
            "substring".parse::<ImportMatchType>().unwrap(),
            ImportMatchType::Substring
        );
        assert!("Exact".parse::<ImportMatchType>().is_err());
    }

    #[test]
    fn import_match_source_round_trips() {
        assert_eq!(ImportMatchSource::Confirmed.to_string(), "confirmed");
        assert_eq!(ImportMatchSource::AiSuggested.to_string(), "ai_suggested");
        assert_eq!(
            "confirmed".parse::<ImportMatchSource>().unwrap(),
            ImportMatchSource::Confirmed
        );
        assert_eq!(
            "ai_suggested".parse::<ImportMatchSource>().unwrap(),
            ImportMatchSource::AiSuggested
        );
        assert!("Confirmed".parse::<ImportMatchSource>().is_err());
    }

    #[test]
    fn in_memory_enums_compile() {
        // Verify in-memory-only enums exist and are usable
        let _ = AiRequestState::Idle;
        let _ = AiRequestState::CallingApi;
        let _ = AiRequestState::FulfillingTools;

        let _ = ChatRole::User;
        let _ = ChatRole::Assistant;
        let _ = ChatRole::System;

        let _ = FocusTarget::MainTab;
        let _ = FocusTarget::ChatPanel;

        let _ = MatchSource::Local;
        let _ = MatchSource::Ai;
        let _ = MatchSource::UserConfirmed;
        let _ = MatchSource::Unmatched;

        let _ = MatchConfidence::High;
        let _ = MatchConfidence::Medium;
        let _ = MatchConfidence::Low;
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
