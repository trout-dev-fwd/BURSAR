use std::{io, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use crossterm::{
    event, execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use bursar::{
    app::{App, EntityContext},
    config::load_config,
    db::EntityDb,
    startup::run_startup_checks,
    startup_screen::{StartupAction, StartupScreen, render_splash},
};

fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("bursar")
        .join("workspace.toml")
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
}

/// Drop guard: restores terminal state even on panic.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

enum AppState {
    Splash,
    Startup(StartupScreen),
    Running(Box<App>),
}

fn main() -> Result<()> {
    // Write tracing output to a log file so it never bleeds into the TUI display.
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/bursar.log")
        .unwrap_or_else(|_| {
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

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);

    let config = load_config(&config_path)
        .with_context(|| format!("Failed to load config from {}", config_path.display()))?;

    // Set up terminal once for the full session.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let _guard = TerminalGuard;

    let mut state = AppState::Splash;

    // Local transition type — avoids holding borrows across state changes.
    enum Transition {
        Continue,
        ToStartup,
        ToRunning(Box<App>),
        Quit,
    }

    loop {
        let transition = match &mut state {
            AppState::Splash => {
                terminal.draw(render_splash)?;
                // Task 4 will perform an update check here; for now just pause briefly.
                std::thread::sleep(Duration::from_secs(1));
                Transition::ToStartup
            }

            AppState::Startup(screen) => {
                terminal.draw(|f| screen.render(f))?;
                if event::poll(Duration::from_millis(500))? {
                    let evt = event::read()?;
                    match screen.handle_event(&evt) {
                        StartupAction::OpenEntity(idx) => {
                            let entity_cfg = &config.entities[idx];
                            let entity_name = entity_cfg.name.clone();
                            let db_path = entity_cfg.db_path.clone();
                            let report_dir = config.report_output_dir.clone();

                            // Persist the selection so it is pre-selected on next launch.
                            write_last_opened(&config_path, &entity_name)?;

                            // Startup checks manage their own terminal session,
                            // so briefly leave the alternate screen before calling them.
                            disable_raw_mode()?;
                            execute!(io::stdout(), LeaveAlternateScreen)?;

                            let db = EntityDb::open(&db_path)?;
                            run_startup_checks(&db, &entity_name, &config)?;

                            // Re-enter the alternate screen for the Running state.
                            enable_raw_mode()?;
                            execute!(io::stdout(), EnterAlternateScreen)?;
                            terminal.clear()?;

                            let entity_ctx = EntityContext::new(db, entity_name, report_dir);
                            let app = Box::new(App::new(entity_ctx, config.clone()));
                            Transition::ToRunning(app)
                        }
                        StartupAction::Quit => Transition::Quit,
                        StartupAction::None => Transition::Continue,
                    }
                } else {
                    Transition::Continue
                }
            }

            AppState::Running(app) => {
                app.render(&mut terminal)?;
                if event::poll(Duration::from_millis(500))? {
                    let evt = event::read()?;
                    app.handle_event(&evt);
                }
                app.process_pending(&mut terminal);
                app.tick();
                if app.should_quit() {
                    Transition::Quit
                } else {
                    Transition::Continue
                }
            }
        };

        match transition {
            Transition::Continue => {}
            Transition::ToStartup => {
                state = AppState::Startup(StartupScreen::new(&config, config_path.clone()));
            }
            Transition::ToRunning(app) => {
                state = AppState::Running(app);
            }
            Transition::Quit => break,
        }
    }

    restore_terminal();
    Ok(())
}

/// Writes `last_opened_entity = "<name>"` into `workspace.toml` using `toml_edit` so
/// that existing formatting and comments in the file are preserved.
fn write_last_opened(config_path: &std::path::Path, entity_name: &str) -> Result<()> {
    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("Failed to parse {}", config_path.display()))?;
    doc["last_opened_entity"] = toml_edit::value(entity_name);
    std::fs::write(config_path, doc.to_string())
        .with_context(|| format!("Failed to write {}", config_path.display()))?;
    Ok(())
}
