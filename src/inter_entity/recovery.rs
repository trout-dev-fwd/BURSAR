//! Startup recovery for orphaned inter-entity drafts.
//!
//! Finds Draft journal entries with `inter_entity_uuid` set, checks the matching
//! entry in the other entity's database, and resolves the inconsistency.
//!
//! **Scenarios and resolutions**:
//! - `BothDraft` — either **post both** or **delete both drafts**.
//! - `ActiveDraftOtherPosted` — either **complete** (post active draft) or
//!   **roll back** (reverse other's posted entry + delete active draft).
//! - `PeerNotFound` — the other entity has no entry for this UUID; offer to delete.

use std::str::FromStr;

use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::params;

use crate::db::EntityDb;
use crate::db::journal_repo::JournalEntry;
use crate::services::journal::{post_journal_entry, reverse_journal_entry};
use crate::types::{JournalEntryId, JournalEntryStatus};

// ── Data types ────────────────────────────────────────────────────────────────

/// Status of the paired entry found in the peer entity's database.
#[derive(Debug, PartialEq)]
pub enum PeerStatus {
    /// Peer has a Draft JE with the same UUID.
    Draft(JournalEntryId),
    /// Peer has a Posted JE with the same UUID.
    Posted(JournalEntryId),
    /// No JE with this UUID exists in the peer DB.
    NotFound,
}

// ── Query helpers ─────────────────────────────────────────────────────────────

/// Returns all Draft JEs with a non-null `inter_entity_uuid` in `db`.
/// These are candidates for recovery.
pub fn find_orphaned_drafts(db: &EntityDb) -> Result<Vec<JournalEntry>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, je_number, entry_date, memo, status, is_reversed,
                reversed_by_je_id, reversal_of_je_id, inter_entity_uuid,
                source_entity_name, fiscal_period_id, created_at, updated_at
         FROM journal_entries
         WHERE status = 'Draft' AND inter_entity_uuid IS NOT NULL
         ORDER BY entry_date, id",
    )?;

    let entries = stmt
        .query_map([], crate::db::journal_repo::row_to_entry)?
        .map(|r| r.map_err(anyhow::Error::from))
        .collect::<Result<Vec<_>>>()?;

    Ok(entries)
}

/// Checks the peer entity's database for a JE with the given `uuid`.
/// Returns the first match found (ordered by id).
pub fn classify_peer(other_db: &EntityDb, uuid: &str) -> Result<PeerStatus> {
    let result = other_db.conn().query_row(
        "SELECT id, status FROM journal_entries WHERE inter_entity_uuid = ?1 LIMIT 1",
        params![uuid],
        |row| {
            let id: i64 = row.get(0)?;
            let status: String = row.get(1)?;
            Ok((id, status))
        },
    );

    match result {
        Ok((id, status_str)) => {
            let je_id = JournalEntryId::from(id);
            let status =
                JournalEntryStatus::from_str(&status_str).map_err(|e| anyhow::anyhow!("{e}"))?;
            match status {
                JournalEntryStatus::Draft => Ok(PeerStatus::Draft(je_id)),
                JournalEntryStatus::Posted => Ok(PeerStatus::Posted(je_id)),
            }
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(PeerStatus::NotFound),
        Err(e) => Err(e.into()),
    }
}

// ── Resolution functions ───────────────────────────────────────────────────────

/// **BothDraft → Post both**.
///
/// Posts the active draft, then the peer draft.
/// If posting active fails, nothing is changed.
/// If posting peer fails, active remains Posted (an inconsistency that will be
/// caught on the next startup recovery pass).
pub fn resolve_post_both(
    active_db: &EntityDb,
    active_name: &str,
    other_db: &EntityDb,
    other_name: &str,
    active_je_id: JournalEntryId,
    other_je_id: JournalEntryId,
) -> Result<()> {
    post_journal_entry(active_db, active_je_id, active_name)?;
    post_journal_entry(other_db, other_je_id, other_name)?;
    Ok(())
}

/// **BothDraft → Delete both**.
///
/// Deletes both draft entries. Both must be in Draft status.
pub fn resolve_delete_both(
    active_db: &EntityDb,
    other_db: &EntityDb,
    active_je_id: JournalEntryId,
    other_je_id: JournalEntryId,
) -> Result<()> {
    active_db.journals().delete_draft(active_je_id)?;
    other_db.journals().delete_draft(other_je_id)?;
    Ok(())
}

/// **ActiveDraftOtherPosted → Complete**.
///
/// Posts the active entity's draft (the other side is already Posted).
pub fn resolve_complete(
    active_db: &EntityDb,
    active_name: &str,
    active_je_id: JournalEntryId,
) -> Result<()> {
    post_journal_entry(active_db, active_je_id, active_name)?;
    Ok(())
}

/// **ActiveDraftOtherPosted → Roll back**.
///
/// Reverses the other entity's posted entry, then deletes the active draft.
/// Uses the other JE's `entry_date` as the reversal date.
pub fn resolve_rollback(
    active_db: &EntityDb,
    other_db: &EntityDb,
    other_name: &str,
    active_je_id: JournalEntryId,
    other_je_id: JournalEntryId,
) -> Result<()> {
    // Get the other JE's entry_date for use as the reversal date.
    let (other_je, _) = other_db.journals().get_with_lines(other_je_id)?;
    let reversal_date: NaiveDate = other_je.entry_date;
    reverse_journal_entry(other_db, other_je_id, reversal_date, other_name)?;
    active_db.journals().delete_draft(active_je_id)?;
    Ok(())
}

/// **PeerNotFound → Delete orphan**.
///
/// Deletes the active draft when the peer has no matching entry.
pub fn resolve_delete_orphan(active_db: &EntityDb, active_je_id: JournalEntryId) -> Result<()> {
    active_db.journals().delete_draft(active_je_id)?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::EntityDb;
    use crate::db::account_repo::NewAccount;
    use crate::db::journal_repo::{NewJournalEntry, NewJournalEntryLine};
    use crate::db::schema::{initialize_schema, seed_default_accounts};
    use crate::services::journal::post_journal_entry;
    use crate::types::{AccountId, AccountType, FiscalPeriodId, JournalEntryStatus, Money};
    use chrono::NaiveDate;
    use rusqlite::Connection;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_entity_db_with_fy() -> (EntityDb, FiscalPeriodId) {
        let conn = Connection::open_in_memory().expect("in-memory db");
        initialize_schema(&conn).expect("schema");
        seed_default_accounts(&conn).expect("seed accounts");
        let db = crate::db::entity_db_from_conn(conn);
        let fy_id = db.fiscal().create_fiscal_year(1, 2026).expect("create FY");
        let periods = db.fiscal().list_periods(fy_id).expect("list periods");
        (db, periods[0].id)
    }

    fn make_account(db: &EntityDb, number: &str, name: &str) -> AccountId {
        db.accounts()
            .create(&NewAccount {
                number: number.to_string(),
                name: name.to_string(),
                account_type: AccountType::Asset,
                parent_id: None,
                is_contra: false,
                is_placeholder: false,
            })
            .expect("create account")
    }

    fn create_draft_with_uuid(
        db: &EntityDb,
        period_id: FiscalPeriodId,
        uuid: &str,
        source_name: &str,
    ) -> JournalEntryId {
        let a1 = make_account(db, "1010", "Cash");
        let a2 = make_account(db, "3010", "Equity");
        let je_id = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                memo: Some("inter-entity test".to_string()),
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: a1,
                        debit_amount: Money(10_000_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: a2,
                        debit_amount: Money(0),
                        credit_amount: Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("create draft");
        db.journals()
            .set_inter_entity_metadata(je_id, uuid, source_name)
            .expect("set metadata");
        je_id
    }

    // ── find_orphaned_drafts ──────────────────────────────────────────────────

    #[test]
    fn find_orphaned_drafts_returns_only_draft_with_uuid() {
        let (db, period_id) = make_entity_db_with_fy();

        // Create one draft with UUID (orphaned candidate).
        let je_id = create_draft_with_uuid(&db, period_id, "uuid-1", "Entity B");

        // Create a plain draft (no UUID) — should NOT appear.
        let a1 = make_account(&db, "1020", "Cash2");
        let a2 = make_account(&db, "3020", "Equity2");
        let _ = db
            .journals()
            .create_draft(&NewJournalEntry {
                entry_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                memo: None,
                fiscal_period_id: period_id,
                reversal_of_je_id: None,
                lines: vec![
                    NewJournalEntryLine {
                        account_id: a1,
                        debit_amount: Money(10_000_000_000),
                        credit_amount: Money(0),
                        line_memo: None,
                        sort_order: 0,
                    },
                    NewJournalEntryLine {
                        account_id: a2,
                        debit_amount: Money(0),
                        credit_amount: Money(10_000_000_000),
                        line_memo: None,
                        sort_order: 1,
                    },
                ],
            })
            .expect("plain draft");

        let orphans = find_orphaned_drafts(&db).expect("find orphaned");
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].id, je_id);
        assert_eq!(orphans[0].inter_entity_uuid.as_deref(), Some("uuid-1"));
    }

    #[test]
    fn find_orphaned_drafts_ignores_posted_with_uuid() {
        let (db, period_id) = make_entity_db_with_fy();

        // Create draft with UUID, then post it.
        let je_id = create_draft_with_uuid(&db, period_id, "uuid-posted", "Entity B");
        post_journal_entry(&db, je_id, "Entity A").expect("post");

        // Should NOT be returned (it's Posted, not Draft).
        let orphans = find_orphaned_drafts(&db).expect("find orphaned");
        assert!(
            orphans.is_empty(),
            "posted entry should not appear in orphan list"
        );
    }

    // ── classify_peer ─────────────────────────────────────────────────────────

    #[test]
    fn classify_peer_returns_draft_when_peer_has_draft() {
        let (peer_db, period_id) = make_entity_db_with_fy();
        let peer_je_id = create_draft_with_uuid(&peer_db, period_id, "test-uuid", "Active Entity");

        let status = classify_peer(&peer_db, "test-uuid").expect("classify");
        assert_eq!(status, PeerStatus::Draft(peer_je_id));
    }

    #[test]
    fn classify_peer_returns_posted_when_peer_has_posted() {
        let (peer_db, period_id) = make_entity_db_with_fy();
        let peer_je_id =
            create_draft_with_uuid(&peer_db, period_id, "posted-uuid", "Active Entity");
        post_journal_entry(&peer_db, peer_je_id, "Peer Entity").expect("post");

        let status = classify_peer(&peer_db, "posted-uuid").expect("classify");
        assert_eq!(status, PeerStatus::Posted(peer_je_id));
    }

    #[test]
    fn classify_peer_returns_not_found_when_uuid_absent() {
        let (peer_db, _) = make_entity_db_with_fy();
        let status = classify_peer(&peer_db, "nonexistent-uuid").expect("classify");
        assert_eq!(status, PeerStatus::NotFound);
    }

    // ── resolve_post_both (BothDraft → Post both) ─────────────────────────────

    #[test]
    fn resolve_post_both_posts_both_entries() {
        let (active_db, active_period) = make_entity_db_with_fy();
        let (other_db, other_period) = make_entity_db_with_fy();

        let uuid = "both-draft-post-uuid";
        let active_je = create_draft_with_uuid(&active_db, active_period, uuid, "Entity B");
        let other_je = create_draft_with_uuid(&other_db, other_period, uuid, "Entity A");

        resolve_post_both(
            &active_db, "Entity A", &other_db, "Entity B", active_je, other_je,
        )
        .expect("resolve_post_both");

        let (active_entry, _) = active_db.journals().get_with_lines(active_je).expect("get");
        let (other_entry, _) = other_db.journals().get_with_lines(other_je).expect("get");

        assert_eq!(active_entry.status, JournalEntryStatus::Posted);
        assert_eq!(other_entry.status, JournalEntryStatus::Posted);
    }

    // ── resolve_delete_both (BothDraft → Delete both) ─────────────────────────

    #[test]
    fn resolve_delete_both_removes_both_entries() {
        let (active_db, active_period) = make_entity_db_with_fy();
        let (other_db, other_period) = make_entity_db_with_fy();

        let uuid = "both-draft-delete-uuid";
        let active_je = create_draft_with_uuid(&active_db, active_period, uuid, "Entity B");
        let other_je = create_draft_with_uuid(&other_db, other_period, uuid, "Entity A");

        resolve_delete_both(&active_db, &other_db, active_je, other_je)
            .expect("resolve_delete_both");

        let active_list = active_db
            .journals()
            .list(&crate::db::journal_repo::JournalFilter::default())
            .expect("list active");
        let other_list = other_db
            .journals()
            .list(&crate::db::journal_repo::JournalFilter::default())
            .expect("list other");

        assert!(active_list.is_empty(), "active draft should be deleted");
        assert!(other_list.is_empty(), "other draft should be deleted");
    }

    // ── resolve_complete (ActiveDraftOtherPosted → Complete) ──────────────────

    #[test]
    fn resolve_complete_posts_active_draft() {
        let (active_db, active_period) = make_entity_db_with_fy();

        let uuid = "complete-uuid";
        let active_je = create_draft_with_uuid(&active_db, active_period, uuid, "Entity B");

        resolve_complete(&active_db, "Entity A", active_je).expect("resolve_complete");

        let (entry, _) = active_db.journals().get_with_lines(active_je).expect("get");
        assert_eq!(entry.status, JournalEntryStatus::Posted);
    }

    // ── resolve_rollback (ActiveDraftOtherPosted → Roll back) ─────────────────

    #[test]
    fn resolve_rollback_reverses_other_and_deletes_active() {
        let (active_db, active_period) = make_entity_db_with_fy();
        let (other_db, other_period) = make_entity_db_with_fy();

        let uuid = "rollback-uuid";
        let active_je = create_draft_with_uuid(&active_db, active_period, uuid, "Entity B");

        // Post the other JE (simulating "other is already posted").
        let other_je = create_draft_with_uuid(&other_db, other_period, uuid, "Entity A");
        post_journal_entry(&other_db, other_je, "Entity B").expect("post other");

        resolve_rollback(&active_db, &other_db, "Entity B", active_je, other_je)
            .expect("resolve_rollback");

        // Active DB should be empty (draft deleted).
        let active_list = active_db
            .journals()
            .list(&crate::db::journal_repo::JournalFilter::default())
            .expect("list active");
        assert!(active_list.is_empty(), "active draft should be deleted");

        // Other DB should have the original + its reversal.
        let other_list = other_db
            .journals()
            .list(&crate::db::journal_repo::JournalFilter::default())
            .expect("list other");
        assert_eq!(other_list.len(), 2, "other should have original + reversal");

        let original = other_list
            .iter()
            .find(|e| e.reversal_of_je_id.is_none())
            .expect("original");
        assert!(original.is_reversed);

        let reversal = other_list
            .iter()
            .find(|e| e.reversal_of_je_id.is_some())
            .expect("reversal");
        assert_eq!(reversal.reversal_of_je_id, Some(other_je));
    }

    // ── resolve_delete_orphan ─────────────────────────────────────────────────

    #[test]
    fn resolve_delete_orphan_removes_active_draft() {
        let (active_db, active_period) = make_entity_db_with_fy();
        let uuid = "orphan-uuid";
        let active_je = create_draft_with_uuid(&active_db, active_period, uuid, "Entity B");

        resolve_delete_orphan(&active_db, active_je).expect("resolve_delete_orphan");

        let list = active_db
            .journals()
            .list(&crate::db::journal_repo::JournalFilter::default())
            .expect("list");
        assert!(list.is_empty(), "orphan draft should be deleted");
    }
}
