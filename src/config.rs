use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

// ── Default value helpers for serde ──────────────────────────────────────────

fn default_ai_persona() -> String {
    "Professional Tax Accountant".to_string()
}

fn default_ai_model() -> String {
    "claude-sonnet-4-20250514".to_string()
}

fn default_debit_is_negative() -> bool {
    true
}

fn default_report_output_dir() -> PathBuf {
    PathBuf::from("~/bursar/reports")
}

// ── Workspace configuration ───────────────────────────────────────────────────

/// Parsed from `workspace.toml`. Shared across all entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default = "default_report_output_dir")]
    pub report_output_dir: PathBuf,
    #[serde(default)]
    pub entities: Vec<EntityConfig>,
    /// Optional AI configuration section `[ai]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<WorkspaceAiConfig>,
    /// Directory where entity context `.md` files are stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_dir: Option<String>,
    /// Name of the entity that was most recently opened. Used to pre-select it on next launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_opened_entity: Option<String>,
    /// Optional update-check configuration.
    #[serde(default)]
    pub updates: UpdatesConfig,
}

/// The `[ai]` section of `workspace.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceAiConfig {
    #[serde(default = "default_ai_persona")]
    pub persona: String,
    #[serde(default = "default_ai_model")]
    pub model: String,
}

impl Default for WorkspaceAiConfig {
    fn default() -> Self {
        Self {
            persona: default_ai_persona(),
            model: default_ai_model(),
        }
    }
}

/// The `[updates]` section of `workspace.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UpdatesConfig {
    /// GitHub repository slug in `owner/repo` form, e.g. `"owner/bursar"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_repo: Option<String>,
}

/// One entry per entity database in the workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityConfig {
    pub name: String,
    pub db_path: PathBuf,
    /// Path to the per-entity TOML file (relative to workspace dir or absolute).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            report_output_dir: PathBuf::from("~/bursar/reports"),
            entities: Vec::new(),
            ai: None,
            context_dir: None,
            last_opened_entity: None,
            updates: UpdatesConfig::default(),
        }
    }
}

impl WorkspaceConfig {
    /// Returns the GitHub repo slug from `[updates]` if configured.
    pub fn updates_github_repo(&self) -> Option<&str> {
        self.updates.github_repo.as_deref()
    }
}

// ── Per-entity TOML configuration ─────────────────────────────────────────────

/// Contents of a per-entity `.toml` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct EntityTomlConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_persona: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_import_dir: Option<String>,
    #[serde(default)]
    pub bank_accounts: Vec<BankAccountConfig>,
}

/// A single `[[bank_accounts]]` entry in the entity TOML file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BankAccountConfig {
    pub name: String,
    /// Chart-of-accounts account number (TEXT) linked to this bank account.
    pub linked_account: String,
    pub date_column: String,
    pub description_column: String,
    /// Single-amount format column name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_column: Option<String>,
    /// Split-format debit column name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debit_column: Option<String>,
    /// Split-format credit column name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_column: Option<String>,
    /// For single-amount format: whether a negative value means a debit from the account.
    #[serde(default = "default_debit_is_negative")]
    pub debit_is_negative: bool,
    /// Chrono format string for parsing dates (e.g. `"%m/%d/%Y"`).
    pub date_format: String,
}

impl BankAccountConfig {
    /// Returns `true` if the column configuration is valid:
    /// either `amount_column` is `Some`, or both `debit_column` and `credit_column` are `Some`.
    pub fn is_valid(&self) -> bool {
        match &self.amount_column {
            Some(_) => true,
            None => self.debit_column.is_some() && self.credit_column.is_some(),
        }
    }
}

// ── Secrets configuration ─────────────────────────────────────────────────────

/// Loaded from `~/.config/bookkeeper/secrets.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct SecretsConfig {
    pub anthropic_api_key: String,
}

/// Returns the canonical path to the secrets file.
pub fn secrets_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("bookkeeper")
        .join("secrets.toml")
}

/// Loads the API key from `~/.config/bookkeeper/secrets.toml`.
/// Auto-creates the directory if it does not exist.
/// Returns an error if the file is missing or the key is empty.
pub fn load_secrets() -> Result<SecretsConfig> {
    let path = secrets_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create secrets directory: {}", parent.display()))?;
    }
    if !path.exists() {
        bail!(
            "No API key configured. Create {} with:\n\
             anthropic_api_key = \"sk-ant-...\"",
            path.display()
        );
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read secrets file: {}", path.display()))?;
    let secrets: SecretsConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse secrets file: {}", path.display()))?;
    if secrets.anthropic_api_key.is_empty() {
        bail!("anthropic_api_key is empty in {}", path.display());
    }
    Ok(secrets)
}

// ── Entity TOML I/O ───────────────────────────────────────────────────────────

/// Resolves a config path (possibly relative or tilde-prefixed) against the workspace directory.
fn resolve_config_path(config_path: &str, workspace_dir: &Path) -> PathBuf {
    let expanded = expand_tilde(Path::new(config_path));
    if expanded.is_absolute() {
        expanded
    } else {
        workspace_dir.join(expanded)
    }
}

/// Loads an entity TOML from `config_path` (relative to `workspace_dir` or absolute).
/// Returns an empty `EntityTomlConfig` if the file does not exist.
pub fn load_entity_toml(config_path: &str, workspace_dir: &Path) -> Result<EntityTomlConfig> {
    let path = resolve_config_path(config_path, workspace_dir);
    if !path.exists() {
        return Ok(EntityTomlConfig::default());
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read entity config: {}", path.display()))?;
    toml::from_str(&contents)
        .with_context(|| format!("Failed to parse entity config: {}", path.display()))
}

/// Serializes `config` and writes it to `config_path` (relative to `workspace_dir` or absolute).
/// Creates parent directories if needed.
pub fn save_entity_toml(
    config_path: &str,
    workspace_dir: &Path,
    config: &EntityTomlConfig,
) -> Result<()> {
    let path = resolve_config_path(config_path, workspace_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
    }
    let contents = toml::to_string_pretty(config).context("Failed to serialize entity config")?;
    std::fs::write(&path, contents)
        .with_context(|| format!("Failed to write entity config: {}", path.display()))
}

// ── Workspace config I/O ──────────────────────────────────────────────────────

/// Expands a leading `~` in a path to the user's home directory.
fn expand_tilde(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if (s.starts_with("~/") || s == "~")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(s.strip_prefix("~/").unwrap_or(""));
    }
    p.to_path_buf()
}

/// Loads `WorkspaceConfig` from `path`. Creates a default config if the file does not exist.
/// Expands leading `~` and resolves relative paths against the workspace.toml directory.
pub fn load_config(path: &Path) -> Result<WorkspaceConfig> {
    let workspace_dir = path.parent().unwrap_or(Path::new("."));
    if !path.exists() {
        let default = WorkspaceConfig::default();
        save_config(path, &default)
            .with_context(|| format!("Failed to create default config at {}", path.display()))?;
        return Ok(expand_config_paths(default, workspace_dir));
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;
    let config: WorkspaceConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
    Ok(expand_config_paths(config, workspace_dir))
}

/// Applies tilde expansion to all path fields in the config, then resolves
/// any remaining relative paths against `workspace_dir` (the directory
/// containing `workspace.toml`).
fn expand_config_paths(mut config: WorkspaceConfig, workspace_dir: &Path) -> WorkspaceConfig {
    config.report_output_dir =
        resolve_relative(expand_tilde(&config.report_output_dir), workspace_dir);
    for entity in &mut config.entities {
        entity.db_path = resolve_relative(expand_tilde(&entity.db_path), workspace_dir);
    }
    config
}

/// If `path` is relative, joins it with `base` to make it absolute.
fn resolve_relative(path: PathBuf, base: &Path) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

/// Serializes `config` to TOML and writes it to `path`.
/// Creates parent directories if they do not exist.
pub fn save_config(path: &Path, config: &WorkspaceConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
    }
    let contents =
        toml::to_string_pretty(config).context("Failed to serialize workspace config")?;
    std::fs::write(path, contents)
        .with_context(|| format!("Failed to write config file: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── Existing workspace config tests ──────────────────────────────────────

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn round_trip_with_two_entities() {
        let dir = std::env::temp_dir().join("bursar_test_config");
        let path = dir.join("workspace.toml");

        let config = WorkspaceConfig {
            report_output_dir: PathBuf::from("/tmp/reports"),
            entities: vec![
                EntityConfig {
                    name: "Entity One".to_owned(),
                    db_path: PathBuf::from("/tmp/entity_one.sqlite"),
                    config_path: None,
                },
                EntityConfig {
                    name: "Entity Two".to_owned(),
                    db_path: PathBuf::from("/tmp/entity_two.sqlite"),
                    config_path: None,
                },
            ],
            ai: None,
            context_dir: None,
            last_opened_entity: None,
            updates: UpdatesConfig::default(),
        };

        save_config(&path, &config).expect("save_config failed");
        let loaded = load_config(&path).expect("load_config failed");

        assert_eq!(config, loaded);

        // Cleanup
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn load_creates_default_when_missing() {
        let dir = std::env::temp_dir().join("bursar_test_config_missing");
        let path = dir.join("workspace.toml");

        // Ensure file does not exist
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);

        let loaded = load_config(&path).expect("load_config should create default");
        assert!(loaded.entities.is_empty());
        assert!(path.exists(), "config file should have been created");
        // report_output_dir should have ~ expanded
        let dir_str = loaded.report_output_dir.to_string_lossy();
        assert!(
            !dir_str.starts_with("~/"),
            "tilde should be expanded: {}",
            dir_str
        );

        // Cleanup
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn expand_tilde_replaces_home() {
        let home = std::env::var("HOME").expect("HOME must be set");
        let expanded = expand_tilde(Path::new("~/bursar/reports"));
        assert_eq!(expanded, PathBuf::from(format!("{}/bursar/reports", home)));
    }

    #[test]
    fn expand_tilde_leaves_absolute_paths_unchanged() {
        let p = Path::new("/tmp/reports");
        assert_eq!(expand_tilde(p), PathBuf::from("/tmp/reports"));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn load_config_expands_tilde_in_entity_db_path() {
        let dir = std::env::temp_dir().join("bursar_test_tilde_entity");
        let path = dir.join("workspace.toml");

        let config = WorkspaceConfig {
            report_output_dir: PathBuf::from("~/reports"),
            entities: vec![EntityConfig {
                name: "Test".to_owned(),
                db_path: PathBuf::from("~/data/test.sqlite"),
                config_path: None,
            }],
            ai: None,
            context_dir: None,
            last_opened_entity: None,
            updates: UpdatesConfig::default(),
        };
        save_config(&path, &config).expect("save");
        let loaded = load_config(&path).expect("load");

        let home = std::env::var("HOME").expect("HOME");
        assert_eq!(
            loaded.report_output_dir,
            PathBuf::from(format!("{}/reports", home))
        );
        assert_eq!(
            loaded.entities[0].db_path,
            PathBuf::from(format!("{}/data/test.sqlite", home))
        );

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn toml_format_has_report_output_dir() {
        let config = WorkspaceConfig {
            report_output_dir: PathBuf::from("~/bursar/reports"),
            entities: vec![EntityConfig {
                name: "Acme LLC".to_owned(),
                db_path: PathBuf::from("~/bursar/database/acme.sqlite"),
                config_path: None,
            }],
            ai: None,
            context_dir: None,
            last_opened_entity: None,
            updates: UpdatesConfig::default(),
        };
        let toml_str = toml::to_string_pretty(&config).expect("serialization failed");
        assert!(toml_str.contains("report_output_dir"));
        assert!(toml_str.contains("[[entities]]"));
        assert!(toml_str.contains("Acme LLC"));
    }

    // ── New V2 workspace config tests ─────────────────────────────────────────

    #[test]
    fn workspace_config_with_ai_section_round_trips() {
        let dir = std::env::temp_dir().join("bursar_test_ai_config");
        let path = dir.join("workspace.toml");

        let config = WorkspaceConfig {
            report_output_dir: PathBuf::from("/tmp/reports"),
            entities: vec![],
            ai: Some(WorkspaceAiConfig {
                persona: "Senior CPA".to_string(),
                model: "claude-sonnet-4-20250514".to_string(),
            }),
            context_dir: Some("~/context".to_string()),
            last_opened_entity: None,
            updates: UpdatesConfig::default(),
        };

        save_config(&path, &config).expect("save");
        let loaded = load_config(&path).expect("load");

        assert_eq!(loaded.ai.as_ref().unwrap().persona, "Senior CPA");
        assert_eq!(loaded.context_dir.as_deref(), Some("~/context"));

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn workspace_config_without_ai_section_is_backwards_compatible() {
        let dir = std::env::temp_dir().join("bursar_test_no_ai_config");
        let path = dir.join("workspace.toml");

        // Write a V1-style workspace.toml with no [ai] section
        let toml = r#"
report_output_dir = "/tmp/reports"

[[entities]]
name = "Acme LLC"
db_path = "/tmp/acme.sqlite"
"#;
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, toml).unwrap();

        let loaded = load_config(&path).expect("load should succeed without [ai] section");
        assert!(loaded.ai.is_none());
        assert!(loaded.context_dir.is_none());
        assert_eq!(loaded.entities[0].name, "Acme LLC");

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn workspace_ai_config_defaults() {
        let ai = WorkspaceAiConfig::default();
        assert_eq!(ai.persona, "Professional Tax Accountant");
        assert_eq!(ai.model, "claude-sonnet-4-20250514");
    }

    // ── Entity TOML tests ─────────────────────────────────────────────────────

    #[test]
    fn entity_toml_single_amount_bank_account_round_trips() {
        let dir = std::env::temp_dir().join("bursar_test_entity_toml_single");
        let path = dir.join("entity.toml");
        fs::create_dir_all(&dir).unwrap();

        let config = EntityTomlConfig {
            ai_persona: Some("Tax Expert".to_string()),
            last_import_dir: Some("/tmp/imports".to_string()),
            bank_accounts: vec![BankAccountConfig {
                name: "SoFi Checking".to_string(),
                linked_account: "1010".to_string(),
                date_column: "Date".to_string(),
                description_column: "Description".to_string(),
                amount_column: Some("Amount".to_string()),
                debit_column: None,
                credit_column: None,
                debit_is_negative: true,
                date_format: "%m/%d/%Y".to_string(),
            }],
        };

        save_entity_toml("entity.toml", &dir, &config).expect("save");
        let loaded = load_entity_toml("entity.toml", &dir).expect("load");

        assert_eq!(loaded.ai_persona, Some("Tax Expert".to_string()));
        assert_eq!(loaded.bank_accounts.len(), 1);
        assert_eq!(loaded.bank_accounts[0].name, "SoFi Checking");
        assert_eq!(
            loaded.bank_accounts[0].amount_column,
            Some("Amount".to_string())
        );
        assert!(loaded.bank_accounts[0].is_valid());

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn entity_toml_split_column_bank_account_round_trips() {
        let dir = std::env::temp_dir().join("bursar_test_entity_toml_split");
        let path = dir.join("entity.toml");
        fs::create_dir_all(&dir).unwrap();

        let config = EntityTomlConfig {
            ai_persona: None,
            last_import_dir: None,
            bank_accounts: vec![BankAccountConfig {
                name: "Chase CC".to_string(),
                linked_account: "2010".to_string(),
                date_column: "Trans Date".to_string(),
                description_column: "Description".to_string(),
                amount_column: None,
                debit_column: Some("Debit".to_string()),
                credit_column: Some("Credit".to_string()),
                debit_is_negative: true,
                date_format: "%Y-%m-%d".to_string(),
            }],
        };

        save_entity_toml("entity.toml", &dir, &config).expect("save");
        let loaded = load_entity_toml("entity.toml", &dir).expect("load");

        assert_eq!(
            loaded.bank_accounts[0].debit_column,
            Some("Debit".to_string())
        );
        assert_eq!(
            loaded.bank_accounts[0].credit_column,
            Some("Credit".to_string())
        );
        assert!(loaded.bank_accounts[0].is_valid());

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn entity_toml_no_bank_accounts() {
        let dir = std::env::temp_dir().join("bursar_test_entity_toml_empty");
        let path = dir.join("entity.toml");
        fs::create_dir_all(&dir).unwrap();

        let config = EntityTomlConfig::default();
        save_entity_toml("entity.toml", &dir, &config).expect("save");
        let loaded = load_entity_toml("entity.toml", &dir).expect("load");

        assert!(loaded.bank_accounts.is_empty());

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn entity_toml_missing_file_returns_default() {
        let dir = std::env::temp_dir().join("bursar_test_entity_toml_missing");
        // Don't create the file
        let loaded = load_entity_toml("nonexistent.toml", &dir).expect("missing file ok");
        assert!(loaded.bank_accounts.is_empty());
        assert!(loaded.ai_persona.is_none());
    }

    #[test]
    fn bank_account_config_validation_amount_column_valid() {
        let config = BankAccountConfig {
            name: "Test".to_string(),
            linked_account: "1010".to_string(),
            date_column: "Date".to_string(),
            description_column: "Desc".to_string(),
            amount_column: Some("Amount".to_string()),
            debit_column: None,
            credit_column: None,
            debit_is_negative: true,
            date_format: "%m/%d/%Y".to_string(),
        };
        assert!(config.is_valid());
    }

    #[test]
    fn bank_account_config_validation_split_columns_valid() {
        let config = BankAccountConfig {
            name: "Test".to_string(),
            linked_account: "1010".to_string(),
            date_column: "Date".to_string(),
            description_column: "Desc".to_string(),
            amount_column: None,
            debit_column: Some("Debit".to_string()),
            credit_column: Some("Credit".to_string()),
            debit_is_negative: true,
            date_format: "%m/%d/%Y".to_string(),
        };
        assert!(config.is_valid());
    }

    #[test]
    fn bank_account_config_validation_no_columns_invalid() {
        let config = BankAccountConfig {
            name: "Test".to_string(),
            linked_account: "1010".to_string(),
            date_column: "Date".to_string(),
            description_column: "Desc".to_string(),
            amount_column: None,
            debit_column: None,
            credit_column: None,
            debit_is_negative: true,
            date_format: "%m/%d/%Y".to_string(),
        };
        assert!(!config.is_valid());
    }

    #[test]
    fn bank_account_config_validation_only_debit_column_invalid() {
        let config = BankAccountConfig {
            name: "Test".to_string(),
            linked_account: "1010".to_string(),
            date_column: "Date".to_string(),
            description_column: "Desc".to_string(),
            amount_column: None,
            debit_column: Some("Debit".to_string()),
            credit_column: None,
            debit_is_negative: true,
            date_format: "%m/%d/%Y".to_string(),
        };
        assert!(!config.is_valid());
    }

    #[test]
    fn test_parse_minimal_config() {
        let toml_str = r#"report_output_dir = "~/bursar/reports""#;
        let config: WorkspaceConfig =
            toml::from_str(toml_str).expect("should parse minimal config");
        assert!(config.entities.is_empty());
    }

    #[test]
    fn test_parse_empty_config() {
        let toml_str = "";
        let config: WorkspaceConfig = toml::from_str(toml_str).expect("should parse empty config");
        assert!(config.entities.is_empty());
    }

    #[test]
    fn test_updates_config_default_is_none() {
        let config: WorkspaceConfig = toml::from_str("").expect("empty config");
        assert!(config.updates.github_repo.is_none());
        assert!(config.updates_github_repo().is_none());
    }

    #[test]
    fn test_updates_config_parses() {
        let toml_str = r#"
[updates]
github_repo = "owner/bursar"
"#;
        let config: WorkspaceConfig = toml::from_str(toml_str).expect("parse");
        assert_eq!(config.updates_github_repo(), Some("owner/bursar"));
    }

    #[test]
    fn load_secrets_returns_error_when_file_missing() {
        // Override HOME to a temp dir to avoid reading the real secrets file
        let dir = std::env::temp_dir().join("bursar_test_secrets_missing");
        let _ = fs::create_dir_all(&dir);

        // We can't easily override HOME in a thread-safe way; just verify the
        // secrets_file_path() has the right shape and that load_secrets errors
        // on the real path if not present.
        let path = secrets_file_path();
        if !path.exists() {
            let result = load_secrets();
            assert!(
                result.is_err(),
                "load_secrets should fail when file missing"
            );
        }
    }

    #[test]
    fn secrets_file_path_contains_expected_segments() {
        let path = secrets_file_path();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains(".config"));
        assert!(path_str.contains("bookkeeper"));
        assert!(path_str.contains("secrets.toml"));
    }
}
