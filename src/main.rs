use std::path::PathBuf;

use anyhow::{Context, Result};

fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("accounting")
        .join("workspace.toml")
}

fn main() -> Result<()> {
    // Initialize tracing subscriber for logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .init();

    // Parse optional config path from command-line arguments.
    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);

    // Load (or create) workspace config.
    let config = accounting::config::load_config(&config_path)
        .with_context(|| format!("Failed to load config from {}", config_path.display()))?;

    // If no entities are configured, print a message and exit.
    if config.entities.is_empty() {
        println!(
            "No entities configured in {}.\n\
             Please create an entity first by running with the TUI (entity creation is in Task 18).",
            config_path.display()
        );
        return Ok(());
    }

    // Open the first entity.
    // TODO(Task 19): show entity picker if multiple entities exist.
    let entity_cfg = &config.entities[0];
    let db = accounting::db::EntityDb::open(&entity_cfg.db_path).with_context(|| {
        format!(
            "Failed to open entity database: {}",
            entity_cfg.db_path.display()
        )
    })?;

    let entity = accounting::app::EntityContext::new(db, entity_cfg.name.clone());
    let mut app = accounting::app::App::new(entity, config);
    app.run()
}
