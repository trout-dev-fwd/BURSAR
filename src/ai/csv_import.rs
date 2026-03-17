use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::NaiveDate;

use crate::ai::{ImportMatch, NormalizedTransaction};
use crate::config::BankAccountConfig;
use crate::db::account_repo::Account;
use crate::types::{AccountType, Money};

// ── Import Flow State ─────────────────────────────────────────────────────────

/// Tracks the current step in the CSV import wizard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportFlowStep {
    FilePathInput,
    BankSelection,
    NewBankName,
    NewBankDetection,
    NewBankConfirmation,
    NewBankAccountPicker,
    DuplicateWarning,
    Pass1Matching,
    Pass2AiMatching,
    Pass3Clarification,
    ReviewScreen,
    Creating,
    Complete,
    Failed(String),
}

/// Full wizard state persisted across steps.
pub struct ImportFlowState {
    pub step: ImportFlowStep,
    pub file_path: Option<PathBuf>,
    pub bank_config: Option<BankAccountConfig>,
    pub is_new_bank: bool,
    pub new_bank_name: Option<String>,
    pub detected_config: Option<BankAccountConfig>,
    pub transactions: Vec<NormalizedTransaction>,
    pub duplicates: Vec<NormalizedTransaction>,
    pub matches: Vec<ImportMatch>,
    pub input_buffer: String,
    pub selected_index: usize,
    pub scroll_offset: usize,
    /// Error message displayed within the current modal step.
    pub modal_error: Option<String>,
    /// Bank accounts from entity toml, populated when entering BankSelection step.
    pub available_banks: Vec<BankAccountConfig>,
    /// Accounts loaded from DB for the account picker step.
    pub picker_accounts: Vec<Account>,
    /// Account picker widget state for NewBankAccountPicker step.
    pub account_picker: crate::widgets::AccountPicker,
    /// Indices into `matches` of Low-confidence items awaiting Pass 3 clarification.
    pub clarification_queue: Vec<usize>,
    /// True once the Pass 3 prompt for the current item has been shown in the chat panel.
    pub clarification_prompted: bool,
    /// Which sections are expanded on the review screen (Local, Ai, UserConfirmed, Unmatched).
    pub review_section_expanded: [bool; 4],
}

impl Default for ImportFlowState {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportFlowState {
    /// Creates a new flow starting at the file path input step.
    pub fn new() -> Self {
        Self {
            step: ImportFlowStep::FilePathInput,
            file_path: None,
            bank_config: None,
            is_new_bank: false,
            new_bank_name: None,
            detected_config: None,
            transactions: Vec::new(),
            duplicates: Vec::new(),
            matches: Vec::new(),
            input_buffer: String::new(),
            selected_index: 0,
            scroll_offset: 0,
            modal_error: None,
            available_banks: Vec::new(),
            picker_accounts: Vec::new(),
            account_picker: crate::widgets::AccountPicker::new(),
            clarification_queue: Vec::new(),
            clarification_prompted: false,
            // Local expanded=false (dimmed/collapsed by default), others expanded.
            review_section_expanded: [false, true, true, true],
        }
    }
}

// ── Pass 1: Local Matching ────────────────────────────────────────────────────

/// Runs Pass 1: local deterministic matching against the `import_mappings` table.
///
/// For each transaction, tries exact match first, then longest-substring match.
/// Records use on matched mappings. Returns one `ImportMatch` per transaction.
pub fn run_pass1(
    transactions: &[NormalizedTransaction],
    bank_name: &str,
    db: &crate::db::EntityDb,
) -> Vec<crate::ai::ImportMatch> {
    use crate::types::MatchSource;

    let repo = db.import_mappings();
    // Load all accounts for display names.
    let accounts: Vec<crate::db::account_repo::Account> =
        db.accounts().list_all().unwrap_or_default();

    transactions
        .iter()
        .map(|txn| {
            // Try exact match.
            let result = repo.find_exact_match(bank_name, &txn.description);
            let matched = match result {
                Ok(Some((id, account_id))) => {
                    let _ = repo.record_use(id);
                    Some(account_id)
                }
                Ok(None) => {
                    // Try substring match.
                    match repo.find_substring_match(bank_name, &txn.description) {
                        Ok(Some((id, account_id))) => {
                            let _ = repo.record_use(id);
                            Some(account_id)
                        }
                        _ => None,
                    }
                }
                Err(_) => None,
            };

            match matched {
                Some(account_id) => {
                    let display = accounts
                        .iter()
                        .find(|a| a.id == account_id)
                        .map(|a| format!("{} - {}", a.number, a.name));
                    crate::ai::ImportMatch {
                        transaction: txn.clone(),
                        matched_account_id: Some(account_id),
                        matched_account_display: display,
                        match_source: MatchSource::Local,
                        confidence: None,
                        reasoning: None,
                        rejected: false,
                    }
                }
                None => crate::ai::ImportMatch {
                    transaction: txn.clone(),
                    matched_account_id: None,
                    matched_account_display: None,
                    match_source: MatchSource::Unmatched,
                    confidence: None,
                    reasoning: None,
                    rejected: false,
                },
            }
        })
        .collect()
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Parses a CSV bank statement into normalized transactions using the bank config
/// to identify columns and date format.
///
/// Malformed rows are skipped with a `tracing::warn!` — they do not cause an error.
/// Returns an empty vec for an empty CSV (no error).
pub fn parse_csv(
    file_path: &Path,
    bank_config: &BankAccountConfig,
) -> Result<Vec<NormalizedTransaction>> {
    let mut reader = csv::Reader::from_path(file_path)
        .with_context(|| format!("Failed to open CSV: {}", file_path.display()))?;

    let headers = reader
        .headers()
        .context("Failed to read CSV headers")?
        .clone();

    let date_idx = col_index(&headers, &bank_config.date_column)
        .with_context(|| format!("Date column '{}' not found in CSV", bank_config.date_column))?;
    let desc_idx = col_index(&headers, &bank_config.description_column).with_context(|| {
        format!(
            "Description column '{}' not found in CSV",
            bank_config.description_column
        )
    })?;

    // Determine column mode: single-amount or split debit/credit.
    let amount_mode = AmountMode::from_config(bank_config, &headers)?;

    let mut results = Vec::new();
    for record in reader.records() {
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Skipping malformed CSV row: {e}");
                continue;
            }
        };

        let raw_row = record.iter().collect::<Vec<_>>().join(",");

        let date_str = field(&record, date_idx);
        let date = match NaiveDate::parse_from_str(date_str, &bank_config.date_format) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Skipping row with unparseable date '{date_str}': {e}");
                continue;
            }
        };

        let description = field(&record, desc_idx).to_string();

        let amount = match amount_mode.parse_amount(&record) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Skipping row with unparseable amount: {e}");
                continue;
            }
        };

        let import_ref = build_import_ref(&bank_config.name, date, &description, amount);

        results.push(NormalizedTransaction {
            date,
            description,
            amount,
            import_ref,
            raw_row,
        });
    }

    Ok(results)
}

/// Separates transactions into unique (not seen before) and duplicates (already imported).
/// Returns `(unique, duplicates)`.
pub fn check_duplicates(
    transactions: &[NormalizedTransaction],
    existing_refs: &HashSet<String>,
) -> (Vec<NormalizedTransaction>, Vec<NormalizedTransaction>) {
    let mut unique = Vec::new();
    let mut duplicates = Vec::new();
    for txn in transactions {
        if existing_refs.contains(&txn.import_ref) {
            duplicates.push(txn.clone());
        } else {
            unique.push(txn.clone());
        }
    }
    (unique, duplicates)
}

/// Determines the debit/credit amounts for the bank account line of a journal entry.
///
/// Returns `(debit_amount, credit_amount, bank_side_is_debit)`.
/// All returned Money values are non-negative.
///
/// **Algorithm:**
/// - Asset account + positive amount (deposit): debit bank → `(abs, 0, true)`
/// - Asset account + negative amount (withdrawal): credit bank → `(0, abs, false)`
/// - Liability account + positive amount (purchase): credit bank → `(0, abs, false)`
/// - Liability account + negative amount (payment): debit bank → `(abs, 0, true)`
pub fn determine_debit_credit(
    amount: Money,
    linked_account_type: AccountType,
) -> (Money, Money, bool) {
    let abs_amount = amount.abs();
    let bank_side_is_debit = match linked_account_type {
        AccountType::Asset => amount > Money(0),
        AccountType::Liability => amount < Money(0),
        // For other account types, treat like Asset (debit on positive inflow).
        _ => amount > Money(0),
    };
    if bank_side_is_debit {
        (abs_amount, Money(0), true)
    } else {
        (Money(0), abs_amount, false)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Builds the composite import reference string.
/// Format: `"{bank_name}|{date}|{description}|{amount_raw}"` where
/// `amount_raw` is the Money internal representation as a string.
pub fn build_import_ref(
    bank_name: &str,
    date: NaiveDate,
    description: &str,
    amount: Money,
) -> String {
    format!(
        "{bank_name}|{}|{description}|{}",
        date.format("%Y-%m-%d"),
        amount.0
    )
}

/// Returns the column index for a given header name, or an error.
fn col_index(headers: &csv::StringRecord, name: &str) -> Result<usize> {
    headers
        .iter()
        .position(|h| h.trim() == name.trim())
        .ok_or_else(|| anyhow::anyhow!("Column '{}' not found", name))
}

/// Returns the field at `idx` from a record, or "" if out of bounds.
fn field(record: &csv::StringRecord, idx: usize) -> &str {
    record.get(idx).unwrap_or("").trim()
}

/// Parses a dollar amount string to `Money` without using f64 as an intermediate.
///
/// Handles:
/// - Standard: `"1247.32"`, `"-1247.32"`
/// - Commas: `"1,247.32"`
/// - Parentheses: `"(1247.32)"` → negative
/// - Empty string → `Money(0)`
fn parse_money_str(s: &str) -> Result<Money> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(Money(0));
    }

    // Strip commas and spaces
    let s = s.replace([',', ' '], "");

    let (negative, s): (bool, &str) = if s.starts_with('(') && s.ends_with(')') {
        (true, &s[1..s.len() - 1])
    } else if let Some(stripped) = s.strip_prefix('-') {
        (true, stripped)
    } else {
        (false, &s)
    };

    let (int_str, dec_str) = if let Some(dot) = s.find('.') {
        (&s[..dot], &s[dot + 1..])
    } else {
        (s, "")
    };

    let int_part: i64 = int_str
        .parse()
        .with_context(|| format!("Cannot parse integer part '{int_str}'"))?;

    // Normalize decimal to exactly 8 places (right-pad with zeros, truncate if longer).
    let dec_8 = if dec_str.len() >= 8 {
        dec_str[..8].to_string()
    } else {
        format!("{dec_str:0<8}")
    };
    let dec_part: i64 = dec_8
        .parse()
        .with_context(|| format!("Cannot parse decimal part '{dec_8}'"))?;

    let raw = int_part * 100_000_000 + dec_part;
    Ok(Money(if negative { -raw } else { raw }))
}

// ── Amount Mode ───────────────────────────────────────────────────────────────

/// Encodes whether the bank CSV uses a single amount column or split debit/credit columns.
enum AmountMode {
    /// Single column: positive = deposit, negative = withdrawal if `debit_is_negative == true`.
    /// If `debit_is_negative == false`, sign is inverted.
    Single { idx: usize, debit_is_negative: bool },
    /// Debit column (withdrawal) and credit column (deposit). One is blank per row.
    Split { debit_idx: usize, credit_idx: usize },
}

impl AmountMode {
    fn from_config(config: &BankAccountConfig, headers: &csv::StringRecord) -> Result<Self> {
        if let Some(ref col) = config.amount_column {
            let idx = col_index(headers, col)
                .with_context(|| format!("Amount column '{col}' not found in CSV"))?;
            return Ok(AmountMode::Single {
                idx,
                debit_is_negative: config.debit_is_negative,
            });
        }

        let debit_col = config.debit_column.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Bank config has neither amount_column nor debit_column")
        })?;
        let credit_col = config.credit_column.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Bank config has neither amount_column nor credit_column")
        })?;

        let debit_idx = col_index(headers, debit_col)
            .with_context(|| format!("Debit column '{debit_col}' not found in CSV"))?;
        let credit_idx = col_index(headers, credit_col)
            .with_context(|| format!("Credit column '{credit_col}' not found in CSV"))?;

        Ok(AmountMode::Split {
            debit_idx,
            credit_idx,
        })
    }

    fn parse_amount(&self, record: &csv::StringRecord) -> Result<Money> {
        match self {
            AmountMode::Single {
                idx,
                debit_is_negative,
            } => {
                let raw = parse_money_str(field(record, *idx))?;
                // If debit_is_negative == true: CSV sign directly maps to normalized sign.
                // If debit_is_negative == false: positive = withdrawal = negative normalized.
                if *debit_is_negative {
                    Ok(raw)
                } else {
                    Ok(Money(-raw.0))
                }
            }
            AmountMode::Split {
                debit_idx,
                credit_idx,
            } => {
                let debit_str = field(record, *debit_idx);
                let credit_str = field(record, *credit_idx);
                if !debit_str.is_empty() {
                    // Withdrawal: debit column has the amount → negative normalized
                    let amt = parse_money_str(debit_str)?;
                    Ok(Money(-amt.abs().0))
                } else {
                    // Deposit: credit column has the amount → positive normalized
                    let amt = parse_money_str(credit_str)?;
                    Ok(amt.abs())
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn single_amount_config(date_format: &str) -> BankAccountConfig {
        BankAccountConfig {
            name: "TestBank".to_string(),
            linked_account: "1010".to_string(),
            date_column: "Date".to_string(),
            description_column: "Description".to_string(),
            amount_column: Some("Amount".to_string()),
            debit_column: None,
            credit_column: None,
            debit_is_negative: true,
            date_format: date_format.to_string(),
        }
    }

    fn split_column_config() -> BankAccountConfig {
        BankAccountConfig {
            name: "ChaseBank".to_string(),
            linked_account: "1010".to_string(),
            date_column: "Date".to_string(),
            description_column: "Description".to_string(),
            amount_column: None,
            debit_column: Some("Debit".to_string()),
            credit_column: Some("Credit".to_string()),
            debit_is_negative: true,
            date_format: "%Y-%m-%d".to_string(),
        }
    }

    /// Writes CSV content to a temp file and returns the path.
    /// The caller must not delete the returned path before use.
    fn write_csv(test_name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("accounting_csv_test_{test_name}.csv"));
        fs::write(&path, content).expect("write test csv");
        path
    }

    // ── parse_csv: single-amount format ───────────────────────────────────────

    #[test]
    fn parse_single_amount_csv_negative_is_withdrawal() {
        let csv = "Date,Description,Amount\n03/15/2026,WELLS FARGO MORTGAGE,-1247.32\n03/16/2026,PAYROLL,3500.00\n";
        let path = write_csv("single_amount", csv);
        let config = single_amount_config("%m/%d/%Y");

        let txns = parse_csv(&path, &config).expect("parse");
        assert_eq!(txns.len(), 2);

        // Withdrawal: negative amount
        assert_eq!(txns[0].description, "WELLS FARGO MORTGAGE");
        assert!(txns[0].amount < Money(0), "withdrawal should be negative");

        // Deposit: positive amount
        assert_eq!(txns[1].description, "PAYROLL");
        assert!(txns[1].amount > Money(0), "deposit should be positive");
    }

    #[test]
    fn parse_single_amount_csv_with_iso_date() {
        let csv = "Date,Description,Amount\n2026-01-15,INSURANCE PAYMENT,-450.00\n";
        let path = write_csv("iso_date", csv);
        let config = single_amount_config("%Y-%m-%d");

        let txns = parse_csv(&path, &config).expect("parse");
        assert_eq!(txns.len(), 1);
        assert_eq!(txns[0].date, NaiveDate::from_ymd_opt(2026, 1, 15).unwrap());
    }

    #[test]
    fn parse_split_column_csv() {
        let csv = "Date,Description,Debit,Credit\n2026-01-10,RENT PAYMENT,1200.00,\n2026-01-15,PAYROLL,,3500.00\n";
        let path = write_csv("split_col", csv);
        let config = split_column_config();

        let txns = parse_csv(&path, &config).expect("parse");
        assert_eq!(txns.len(), 2);

        // Debit (withdrawal) → negative
        assert_eq!(txns[0].description, "RENT PAYMENT");
        assert!(txns[0].amount < Money(0), "debit column → negative amount");

        // Credit (deposit) → positive
        assert_eq!(txns[1].description, "PAYROLL");
        assert!(txns[1].amount > Money(0), "credit column → positive amount");
    }

    #[test]
    fn parse_empty_csv_returns_empty_vec() {
        let csv = "Date,Description,Amount\n";
        let path = write_csv("empty", csv);
        let config = single_amount_config("%m/%d/%Y");

        let txns = parse_csv(&path, &config).expect("parse");
        assert!(txns.is_empty());
    }

    #[test]
    fn parse_skips_malformed_row_and_continues() {
        let csv = "Date,Description,Amount\n03/15/2026,VALID ROW,-100.00\nBAD_DATE,SKIP ME,50.00\n03/17/2026,ALSO VALID,200.00\n";
        let path = write_csv("malformed", csv);
        let config = single_amount_config("%m/%d/%Y");

        let txns = parse_csv(&path, &config).expect("parse");
        // Bad date row skipped; valid rows returned
        assert_eq!(txns.len(), 2);
        assert_eq!(txns[0].description, "VALID ROW");
        assert_eq!(txns[1].description, "ALSO VALID");
    }

    #[test]
    fn import_ref_format_is_correct() {
        let csv = "Date,Description,Amount\n03/15/2026,WELLS FARGO MORTGAGE,-1247.32\n";
        let path = write_csv("import_ref", csv);
        let config = single_amount_config("%m/%d/%Y");

        let txns = parse_csv(&path, &config).expect("parse");
        assert_eq!(txns.len(), 1);

        // import_ref format: "{bank_name}|{date}|{description}|{amount_raw}"
        let expected_amount = Money(parse_money_str("-1247.32").unwrap().0).0;
        let expected_ref = format!("TestBank|2026-03-15|WELLS FARGO MORTGAGE|{expected_amount}");
        assert_eq!(txns[0].import_ref, expected_ref);
    }

    // ── check_duplicates ──────────────────────────────────────────────────────

    fn make_txn(import_ref: &str) -> NormalizedTransaction {
        NormalizedTransaction {
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            description: "TEST".to_string(),
            amount: Money(0),
            import_ref: import_ref.to_string(),
            raw_row: String::new(),
        }
    }

    #[test]
    fn check_duplicates_identifies_existing_refs() {
        let txns = vec![
            make_txn("Bank|2026-01-01|DESC|-100"),
            make_txn("Bank|2026-01-02|DESC2|200"),
        ];
        let existing: HashSet<String> = ["Bank|2026-01-01|DESC|-100".to_string()]
            .into_iter()
            .collect();

        let (unique, dupes) = check_duplicates(&txns, &existing);
        assert_eq!(unique.len(), 1);
        assert_eq!(dupes.len(), 1);
        assert_eq!(unique[0].import_ref, "Bank|2026-01-02|DESC2|200");
        assert_eq!(dupes[0].import_ref, "Bank|2026-01-01|DESC|-100");
    }

    #[test]
    fn check_duplicates_no_duplicates_all_unique() {
        let txns = vec![
            make_txn("Bank|2026-01-01|A|100"),
            make_txn("Bank|2026-01-02|B|200"),
        ];
        let existing: HashSet<String> = HashSet::new();

        let (unique, dupes) = check_duplicates(&txns, &existing);
        assert_eq!(unique.len(), 2);
        assert!(dupes.is_empty());
    }

    // ── determine_debit_credit ────────────────────────────────────────────────

    #[test]
    fn asset_positive_amount_debits_bank() {
        let (debit, credit, bank_is_debit) =
            determine_debit_credit(Money(10_000_000_000), AccountType::Asset);
        assert_eq!(debit, Money(10_000_000_000));
        assert_eq!(credit, Money(0));
        assert!(bank_is_debit);
    }

    #[test]
    fn asset_negative_amount_credits_bank() {
        let (debit, credit, bank_is_debit) =
            determine_debit_credit(Money(-10_000_000_000), AccountType::Asset);
        assert_eq!(debit, Money(0));
        assert_eq!(credit, Money(10_000_000_000));
        assert!(!bank_is_debit);
    }

    #[test]
    fn liability_positive_amount_credits_bank() {
        let (debit, credit, bank_is_debit) =
            determine_debit_credit(Money(10_000_000_000), AccountType::Liability);
        assert_eq!(debit, Money(0));
        assert_eq!(credit, Money(10_000_000_000));
        assert!(!bank_is_debit);
    }

    #[test]
    fn liability_negative_amount_debits_bank() {
        let (debit, credit, bank_is_debit) =
            determine_debit_credit(Money(-10_000_000_000), AccountType::Liability);
        assert_eq!(debit, Money(10_000_000_000));
        assert_eq!(credit, Money(0));
        assert!(bank_is_debit);
    }

    #[test]
    fn amounts_always_positive_in_output() {
        // Even with a negative input, returned amounts are non-negative.
        let (debit, credit, _) = determine_debit_credit(Money(-10_000_000_000), AccountType::Asset);
        assert!(debit >= Money(0));
        assert!(credit >= Money(0));
    }

    // ── parse_money_str (internal) ────────────────────────────────────────────

    #[test]
    fn parse_money_handles_commas() {
        let m = parse_money_str("1,247.32").expect("parse");
        assert_eq!(m, parse_money_str("1247.32").unwrap());
    }

    #[test]
    fn parse_money_handles_parentheses_negative() {
        let m = parse_money_str("(1247.32)").expect("parse");
        assert!(m < Money(0));
        assert_eq!(m.abs(), parse_money_str("1247.32").unwrap());
    }

    #[test]
    fn parse_money_empty_string_is_zero() {
        let m = parse_money_str("").expect("parse");
        assert_eq!(m, Money(0));
    }

    #[test]
    fn parse_money_no_decimal() {
        let m = parse_money_str("100").expect("parse");
        assert_eq!(m, Money(10_000_000_000));
    }
}
