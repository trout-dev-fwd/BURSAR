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
    startup_screen::{SplashState, StartupAction, StartupScreen, UpdateProgress, render_splash},
    update::{
        UpdateCheck, check_for_update, cleanup_old_binary, download_with_progress,
        fetch_expected_checksum, preflight_check, replace_and_restart, verify_checksum,
    },
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
    Startup(Box<StartupScreen>),
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

    // Clean up any .old binary left by a previous update before doing anything else.
    cleanup_old_binary();

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
        ToStartup(Option<String>),
        ToRunning(Box<App>),
        Quit,
    }

    loop {
        let transition = match &mut state {
            AppState::Splash => {
                let start = std::time::Instant::now();

                // Render initial splash with logo + version.
                terminal.draw(|f| render_splash(f, &SplashState::default()))?;

                // Attempt full update pipeline, using the configured repo or the default.
                let github_repo = config
                    .updates_github_repo()
                    .unwrap_or("trout-dev-fwd/bursar")
                    .to_string();
                let mut splash = SplashState::default();
                let update_notice = attempt_update(&github_repo, &mut splash, &mut terminal).err();

                // Enforce 1-second minimum splash display time. If an update was
                // downloading, elapsed time will already exceed 1 second naturally.
                let elapsed = start.elapsed();
                if elapsed < Duration::from_secs(1) {
                    std::thread::sleep(Duration::from_secs(1) - elapsed);
                }

                Transition::ToStartup(update_notice)
            }

            AppState::Startup(screen) => {
                terminal.draw(|f| screen.render(f))?;
                if event::poll(Duration::from_millis(500))? {
                    let evt = event::read()?;
                    match screen.handle_event(&evt) {
                        StartupAction::OpenEntity { name, db_path } => {
                            // Re-read config to pick up any entities added/edited/deleted
                            // during the startup screen session.
                            let config = load_config(&config_path).with_context(|| {
                                format!("Failed to reload config from {}", config_path.display())
                            })?;

                            let report_dir = config.report_output_dir.clone();

                            // Persist the selection so it is pre-selected on next launch.
                            write_last_opened(&config_path, &name)?;

                            let db = EntityDb::open(&db_path)?;
                            run_startup_checks(&mut terminal, &db, &name, &config)?;

                            let entity_ctx = EntityContext::new(db, name, report_dir);
                            let app = Box::new(App::new(entity_ctx, config));
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
            Transition::ToStartup(update_notice) => {
                state = AppState::Startup(Box::new(StartupScreen::new(
                    &config,
                    config_path.clone(),
                    update_notice,
                )));
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

/// Attempts the full update flow: check → pre-flight → download → verify → replace → restart.
///
/// Returns `Ok(())` if no update is available or the update is not needed.
/// Returns `Err(warning)` if the update was available but failed at any step.
///
/// On success with an available update, `replace_and_restart` never returns (the process
/// is replaced by exec/spawn+exit). So `Ok(())` here means "no update needed".
fn attempt_update(
    github_repo: &str,
    splash: &mut SplashState,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), String> {
    // Show "Checking for updates..." and render before the blocking HTTP call.
    splash.update_status = Some("Checking for updates...".to_string());
    let _ = terminal.draw(|f| render_splash(f, splash));

    let (version, asset_url, checksum_url, asset_name) = match check_for_update(github_repo) {
        UpdateCheck::UpToDate => return Ok(()),
        // Network/API errors are silent: fall through and launch normally.
        UpdateCheck::Failed(_) => return Ok(()),
        UpdateCheck::Available {
            version,
            asset_url,
            checksum_url,
            asset_name,
        } => (version, asset_url, checksum_url, asset_name),
    };

    // Pre-flight: verify binary is replaceable before wasting bandwidth.
    let exe_path = preflight_check().map_err(|reason| {
        format!(
            "Update to v{version} failed: {reason}. Running v{}.",
            env!("CARGO_PKG_VERSION")
        )
    })?;

    // Construct the temp download path alongside the current binary.
    let parent_dir = exe_path.parent().unwrap_or(std::path::Path::new("."));
    let new_binary = parent_dir.join(format!("{asset_name}.new"));

    // Show download status.
    splash.update_status = Some(format!("Updating to v{version}. . ."));
    splash.progress = Some(UpdateProgress::Indeterminate);
    let _ = terminal.draw(|f| render_splash(f, splash));

    // Download with progress bar updates between chunk reads.
    let download_result = download_with_progress(&asset_url, &new_binary, |bytes, total| {
        splash.progress = Some(match total {
            Some(t) if t > 0 => UpdateProgress::Determinate {
                percent: ((bytes * 100) / t).min(100) as u8,
            },
            _ => UpdateProgress::Indeterminate,
        });
        let _ = terminal.draw(|f| render_splash(f, splash));
    });

    if let Err(e) = download_result {
        let _ = std::fs::remove_file(&new_binary);
        return Err(format!(
            "Update to v{version} failed: {e}. Running v{}.",
            env!("CARGO_PKG_VERSION")
        ));
    }

    // Fetch and verify checksum.
    let expected_hash = fetch_expected_checksum(&checksum_url, &asset_name).map_err(|e| {
        let _ = std::fs::remove_file(&new_binary);
        format!(
            "Update to v{version} failed: {e}. Running v{}.",
            env!("CARGO_PKG_VERSION")
        )
    })?;

    if let Err(e) = verify_checksum(&new_binary, &expected_hash) {
        let _ = std::fs::remove_file(&new_binary);
        return Err(format!(
            "Update to v{version} failed: {e}. Running v{}.",
            env!("CARGO_PKG_VERSION")
        ));
    }

    // Show completion bar briefly before restart.
    splash.progress = Some(UpdateProgress::Complete);
    let _ = terminal.draw(|f| render_splash(f, splash));

    // Replace binary and restart. On Linux this never returns on success.
    // On Windows this spawns a new process and exits.
    replace_and_restart(&exe_path, &new_binary, terminal).map_err(|e| {
        let _ = std::fs::remove_file(&new_binary);
        format!(
            "Update to v{version} failed: {e}. Running v{}.",
            env!("CARGO_PKG_VERSION")
        )
    })
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
