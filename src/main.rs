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
    // Write tracing output to a log file so it never bleeds into the TUI display.
    // The TUI uses the alternate screen buffer but stderr can still corrupt rendering.
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/bursar.log")
        .unwrap_or_else(|_| {
            // If we can't open the log file, open /dev/null to suppress output entirely.
            std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/null")
                .expect("failed to open /dev/null")
        });
    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(log_file))
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_ansi(false)
        .init();

    // Parse optional config path from command-line arguments.
    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);

    // Load (or create) workspace config.
    let mut config = accounting::config::load_config(&config_path)
        .with_context(|| format!("Failed to load config from {}", config_path.display()))?;

    // If no entities are configured, run the entity creation wizard.
    let entity = if config.entities.is_empty() {
        accounting::app::run_entity_creation_wizard(&config_path, &mut config)?
    } else {
        // If one entity: open directly. If multiple: show picker.
        accounting::app::run_entity_picker(&config).with_context(|| "Failed to open entity")?
    };

    // Run startup checks (recurring entries due, pending depreciation, orphaned drafts).
    let entity_name = entity.name.clone();
    accounting::startup::run_startup_checks(&entity.db, &entity_name, &config)?;

    let mut app = accounting::app::App::new(entity, config);
    app.run()
}
