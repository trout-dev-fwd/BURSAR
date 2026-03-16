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
/// Expands leading `~` in `report_output_dir` and entity `db_path` values.
pub fn load_config(path: &Path) -> Result<WorkspaceConfig> {
    if !path.exists() {
        let default = WorkspaceConfig::default();
        save_config(path, &default)
            .with_context(|| format!("Failed to create default config at {}", path.display()))?;
        return Ok(expand_config_paths(default));
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;
    let config: WorkspaceConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
    Ok(expand_config_paths(config))
}

/// Applies tilde expansion to all path fields in the config.
fn expand_config_paths(mut config: WorkspaceConfig) -> WorkspaceConfig {
    config.report_output_dir = expand_tilde(&config.report_output_dir);
    for entity in &mut config.entities {
        entity.db_path = expand_tilde(&entity.db_path);
    }
    config
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

    #[test]
    fn expand_tilde_replaces_home() {
        let home = std::env::var("HOME").expect("HOME must be set");
        let expanded = expand_tilde(Path::new("~/accounting/reports"));
        assert_eq!(
            expanded,
            PathBuf::from(format!("{}/accounting/reports", home))
        );
    }

    #[test]
    fn expand_tilde_leaves_absolute_paths_unchanged() {
        let p = Path::new("/tmp/reports");
        assert_eq!(expand_tilde(p), PathBuf::from("/tmp/reports"));
    }

    #[test]
    fn load_config_expands_tilde_in_entity_db_path() {
        let dir = std::env::temp_dir().join("accounting_test_tilde_entity");
        let path = dir.join("workspace.toml");

        let config = WorkspaceConfig {
            report_output_dir: PathBuf::from("~/reports"),
            entities: vec![EntityConfig {
                name: "Test".to_owned(),
                db_path: PathBuf::from("~/data/test.sqlite"),
            }],
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
