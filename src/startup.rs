//! Startup checks run once per session, before the main UI loads.
//!
//! Three check categories in order:
//! 1. **Orphaned inter-entity drafts**: Draft JEs with `inter_entity_uuid` set whose
//!    counterpart in the other entity is absent. Detection only; full resolution is Phase 6.
//! 2. **Recurring entries due**: Active templates whose `next_due_date` ≤ today.
//!    User is offered a Y/N prompt to generate them as drafts.
//! 3. **Pending depreciation**: Depreciable assets with un-generated months through today.
//!    User is offered a Y/N prompt to generate draft JEs.
//!
//! Each check is presented in a simple TUI loop before `App::run()`.
//! If the user presses Esc on a Y/N prompt the generation is skipped.

use std::io;

use anyhow::Result;
use chrono::Local;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use crate::db::EntityDb;
use crate::db::recurring_repo::RecurringTemplate;

// ── Findings collection ───────────────────────────────────────────────────────

/// Summary of all startup findings for one entity.
pub struct StartupFindings {
    /// Number of Draft JEs with a non-null `inter_entity_uuid`.
    pub orphaned_draft_count: usize,
    /// Active recurring templates whose `next_due_date` ≤ today.
    pub due_recurring: Vec<RecurringTemplate>,
    /// Number of pending depreciation draft JEs that *would* be created through today.
    pub pending_depreciation_count: usize,
}

/// Collects startup findings without mutating any data.
///
/// This is a pure query — safe to call at any point.
pub fn collect_findings(db: &EntityDb) -> Result<StartupFindings> {
    let today = Local::now().date_naive();

    // 1. Orphaned inter-entity drafts.
    let orphaned_draft_count: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM journal_entries
         WHERE status = 'Draft' AND inter_entity_uuid IS NOT NULL",
        [],
        |row| row.get(0),
    )?;

    // 2. Due recurring templates.
    let all_upcoming = db.recurring().list_upcoming()?;
    let due_recurring: Vec<RecurringTemplate> = all_upcoming
        .into_iter()
        .filter(|t| t.next_due_date <= today)
        .collect();

    // 3. Pending depreciation: find the open fiscal period for today, then count.
    let pending_depreciation_count = match db.fiscal().get_period_for_date(today) {
        Ok(period) => {
            let entries = db.assets().generate_pending_depreciation(period.id)?;
            entries.len()
        }
        Err(_) => 0, // No open period → nothing to generate.
    };

    Ok(StartupFindings {
        orphaned_draft_count: orphaned_draft_count as usize,
        due_recurring,
        pending_depreciation_count,
    })
}

// ── TUI startup check runner ──────────────────────────────────────────────────

/// Runs all startup checks in the terminal, presenting prompts to the user.
/// Returns `Ok(())` when all checks have been acknowledged or resolved.
/// Call this after the entity is opened but before `App::run()`.
pub fn run_startup_checks(db: &EntityDb, entity_name: &str) -> Result<()> {
    let findings = collect_findings(db)?;

    // Fast path: nothing to report.
    if findings.orphaned_draft_count == 0
        && findings.due_recurring.is_empty()
        && findings.pending_depreciation_count == 0
    {
        return Ok(());
    }

    // Set up terminal for the startup check screens.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_check_loop(&mut terminal, db, entity_name, &findings);

    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    result
}

fn run_check_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    db: &EntityDb,
    entity_name: &str,
    findings: &StartupFindings,
) -> Result<()> {
    // ── Check 1: orphaned inter-entity drafts (acknowledge only) ──────────────
    if findings.orphaned_draft_count > 0 {
        let msg = format!(
            "Found {} Draft journal entr{} with an inter-entity UUID.\n\n\
             These may be orphaned from a previous inter-entity transaction.\n\
             Full recovery is available in Phase 6.\n\n\
             Press Enter to acknowledge.",
            findings.orphaned_draft_count,
            if findings.orphaned_draft_count == 1 {
                "y"
            } else {
                "ies"
            },
        );
        show_acknowledge(terminal, entity_name, "Orphaned Inter-Entity Drafts", &msg)?;
    }

    // ── Check 2: recurring entries due ────────────────────────────────────────
    if !findings.due_recurring.is_empty() {
        let n = findings.due_recurring.len();
        let first_date = findings.due_recurring[0].next_due_date;
        let msg = format!(
            "{} recurring entr{} {} due (earliest: {}).\n\n\
             Generate draft JEs now for review?\n\n\
             Y — generate  N / Esc — skip",
            n,
            if n == 1 { "y" } else { "ies" },
            if n == 1 { "is" } else { "are" },
            first_date,
        );
        if show_yes_no(terminal, entity_name, "Recurring Entries Due", &msg)? {
            let today = Local::now().date_naive();
            let _ = db.recurring().generate_entries(today)?;
        }
    }

    // ── Check 3: pending depreciation ────────────────────────────────────────
    if findings.pending_depreciation_count > 0 {
        let n = findings.pending_depreciation_count;
        let msg = format!(
            "{} pending depreciation entr{} not yet generated through today.\n\n\
             Generate draft JEs now for review?\n\n\
             Y — generate  N / Esc — skip",
            n,
            if n == 1 { "y is" } else { "ies are" },
        );
        if show_yes_no(terminal, entity_name, "Pending Depreciation", &msg)? {
            let today = Local::now().date_naive();
            if let Ok(period) = db.fiscal().get_period_for_date(today) {
                let entries = db.assets().generate_pending_depreciation(period.id)?;
                let je_repo = db.journals();
                for entry in entries {
                    let _ = je_repo.create_draft(&entry);
                }
            }
        }
    }

    Ok(())
}

// ── Screen helpers ────────────────────────────────────────────────────────────

/// Shows an acknowledgement screen. Waits for Enter (or Esc/q) to continue.
fn show_acknowledge<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    entity_name: &str,
    title: &str,
    body: &str,
) -> Result<()> {
    loop {
        terminal.draw(|frame| render_screen(frame, entity_name, title, body))?;
        if event::poll(std::time::Duration::from_millis(500))?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
                _ => {}
            }
        }
    }
}

/// Shows a Y/N prompt. Returns `true` if user pressed Y.
fn show_yes_no<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    entity_name: &str,
    title: &str,
    body: &str,
) -> Result<bool> {
    loop {
        terminal.draw(|frame| render_screen(frame, entity_name, title, body))?;
        if event::poll(std::time::Duration::from_millis(500))?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => return Ok(true),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => return Ok(false),
                _ => {}
            }
        }
    }
}

fn render_screen(frame: &mut ratatui::Frame, entity_name: &str, title: &str, body: &str) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // Header bar.
    frame.render_widget(
        Paragraph::new(format!(" {entity_name} — Startup Checks "))
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL)),
        chunks[0],
    );

    // Body.
    frame.render_widget(
        Paragraph::new(format!("\n{body}"))
            .block(
                Block::default()
                    .title(format!(" {title} "))
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Cyan)),
            )
            .alignment(Alignment::Left),
        chunks[1],
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::account_repo::NewAccount;
    use crate::db::entity_db_from_conn;
    use crate::db::journal_repo::{NewJournalEntry, NewJournalEntryLine};
    use crate::db::schema::initialize_schema;
    use crate::services::journal::post_journal_entry;
    use crate::types::{AccountId, AccountType, FiscalPeriodId, Money};
    use chrono::NaiveDate;
    use rusqlite::Connection;

    fn make_db() -> (EntityDb, FiscalPeriodId) {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema");
        let db = entity_db_from_conn(conn);
        let fy = db.fiscal().create_fiscal_year(1, 2026).expect("fy");
        let periods = db.fiscal().list_periods(fy).expect("periods");
        (db, periods[0].id)
    }

    fn create_account(
        db: &EntityDb,
        number: &str,
        name: &str,
        account_type: AccountType,
    ) -> AccountId {
        db.accounts()
            .create(&NewAccount {
                number: number.to_owned(),
                name: name.to_owned(),
                account_type,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create account")
    }

    fn post_je(
        db: &EntityDb,
        period_id: FiscalPeriodId,
        date: NaiveDate,
        debit_id: AccountId,
        credit_id: AccountId,
        amount: Money,
    ) -> crate::types::JournalEntryId {
        let je_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: date,
                memo: None,
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: debit_id,
                        debit_amount: amount,
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: credit_id,
                        debit_amount: Money(0),
                        credit_amount: amount,
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("draft");
        post_journal_entry(db, je_id, "Test").expect("post");
        je_id
    }

    #[test]
    fn collect_findings_empty_db_has_no_findings() {
        let (db, _) = make_db();
        let findings = collect_findings(&db).expect("findings");
        assert_eq!(findings.orphaned_draft_count, 0);
        assert!(findings.due_recurring.is_empty());
        assert_eq!(findings.pending_depreciation_count, 0);
    }

    #[test]
    fn collect_findings_detects_orphaned_draft() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Cash", AccountType::Asset);
        let equity = create_account(&db, "3100", "Equity", AccountType::Equity);
        // Create a draft JE with inter_entity_uuid set.
        let je_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
                memo: None,
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: cash,
                        debit_amount: Money::from_dollars(100.0),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: equity,
                        debit_amount: Money(0),
                        credit_amount: Money::from_dollars(100.0),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("draft");
        // Manually set inter_entity_uuid to simulate an inter-entity JE.
        db.conn()
            .execute(
                "UPDATE journal_entries SET inter_entity_uuid = 'test-uuid' WHERE id = ?1",
                rusqlite::params![i64::from(je_id)],
            )
            .expect("update uuid");
        let findings = collect_findings(&db).expect("findings");
        assert_eq!(findings.orphaned_draft_count, 1);
    }

    #[test]
    fn collect_findings_detects_due_recurring() {
        let (db, period_id) = make_db();
        let cash = create_account(&db, "1110", "Cash", AccountType::Asset);
        let revenue = create_account(&db, "4100", "Revenue", AccountType::Revenue);
        let je_id = post_je(
            &db,
            period_id,
            NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            cash,
            revenue,
            Money::from_dollars(100.0),
        );
        // Create template with a past due date.
        db.recurring()
            .create_template(
                je_id,
                crate::types::EntryFrequency::Monthly,
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), // past date
            )
            .expect("create template");
        let findings = collect_findings(&db).expect("findings");
        assert_eq!(findings.due_recurring.len(), 1);
    }
}
