use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Parsed from `workspace.toml`. Shared across all entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub report_output_dir: PathBuf,
    pub entities: Vec<EntityConfig>,
}

/// One entry per entity database in the workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityConfig {
    pub name: String,
    pub db_path: PathBuf,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            report_output_dir: PathBuf::from("~/accounting/reports"),
            entities: Vec::new(),
        }
    }
}

/// Loads `WorkspaceConfig` from `path`. Creates a default config if the file does not exist.
pub fn load_config(path: &Path) -> Result<WorkspaceConfig> {
    if !path.exists() {
        let default = WorkspaceConfig::default();
        save_config(path, &default)
            .with_context(|| format!("Failed to create default config at {}", path.display()))?;
        return Ok(default);
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;
    toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))
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

    #[test]
    fn round_trip_with_two_entities() {
        let dir = std::env::temp_dir().join("accounting_test_config");
        let path = dir.join("workspace.toml");

        let config = WorkspaceConfig {
            report_output_dir: PathBuf::from("/tmp/reports"),
            entities: vec![
                EntityConfig {
                    name: "Entity One".to_owned(),
                    db_path: PathBuf::from("/tmp/entity_one.sqlite"),
                },
                EntityConfig {
                    name: "Entity Two".to_owned(),
                    db_path: PathBuf::from("/tmp/entity_two.sqlite"),
                },
            ],
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
        let dir = std::env::temp_dir().join("accounting_test_config_missing");
        let path = dir.join("workspace.toml");

        // Ensure file does not exist
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);

        let loaded = load_config(&path).expect("load_config should create default");
        assert!(loaded.entities.is_empty());
        assert!(path.exists(), "config file should have been created");

        // Cleanup
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn toml_format_has_report_output_dir() {
        let config = WorkspaceConfig {
            report_output_dir: PathBuf::from("~/accounting/reports"),
            entities: vec![EntityConfig {
                name: "Acme LLC".to_owned(),
                db_path: PathBuf::from("~/accounting/database/acme.sqlite"),
            }],
        };
        let toml_str = toml::to_string_pretty(&config).expect("serialization failed");
        assert!(toml_str.contains("report_output_dir"));
        assert!(toml_str.contains("[[entities]]"));
        assert!(toml_str.contains("Acme LLC"));
    }
}
