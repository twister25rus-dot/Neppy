use chrono::{Duration, Utc};
use tempfile::TempDir;

use super::store::{self, PetUpdate};
use super::store_feed;
use super::store_notes::{self, NewNote};
use super::surfacer::SurfacedNote;
use super::types::*;
use crate::neppy::config::Config;

pub(super) fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

pub(super) fn new_note(pet_id: &str, title: &str) -> NewNote {
    NewNote {
        pet_id: pet_id.into(),
        job_id: Some("job-1".into()),
        source: PetNoteSource::Email,
        kind: PetNoteKind::Request,
        title: title.into(),
        body: String::new(),
        urgency: 2,
        due_at: None,
        goal_ids: vec![],
        proposed_action: None,
        fingerprint: format!("fp:{title}"),
        injection_flagged: false,
    }
}

#[test]
fn schema_is_idempotent_and_versioned() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    for _ in 0..2 {
        let v: i64 = store::with_connection(&config, |c| {
            Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?)
        })
        .unwrap();
        assert_eq!(v, store::SCHEMA_VERSION);
    }
    assert!(config.workspace_dir.join("pet").join("pet.db").exists());
}

#[test]
fn ensure_primary_creates_exactly_one_disabled_pet() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(store::primary_pet(&config).unwrap().is_none());
    let a = store::ensure_primary(&config, Utc::now()).unwrap();
    let b = store::ensure_primary(&config, Utc::now()).unwrap();
    assert_eq!(a.id, b.id);
    assert!(!a.enabled);
    assert_eq!(a.name, DEFAULT_PET_NAME);
    assert_eq!(a.research_preset, ResearchPreset::Standard);
    assert_eq!(a.digest_time, "07:00");
    assert_eq!(a.notify_budget_per_day, 3);
    assert_eq!(a.sources.len(), 4);
    let count: i64 = store::with_connection(&config, |c| {
        Ok(c.query_row("SELECT COUNT(*) FROM pet", [], |r| r.get(0))?)
    })
    .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn update_pet_applies_only_given_fields() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let pet = store::ensure_primary(&config, Utc::now()).unwrap();
    let updated = store::update_pet(
        &config,
        &pet.id,
        &PetUpdate {
            name: Some("Pip".into()),
            sources: Some(vec![PetSource::Memory]),
            ..PetUpdate::default()
        },
        Utc::now(),
    )
    .unwrap();
    assert_eq!(updated.name, "Pip");
    assert_eq!(updated.sources, vec![PetSource::Memory]);
    assert_eq!(updated.digest_time, "07:00");
    store::set_research_job_id(&config, &pet.id, Some("job-9")).unwrap();
    assert_eq!(
        store::find_pet_by_job(&config, "job-9").unwrap().as_deref(),
        Some(pet.id.as_str())
    );
    assert!(store::find_pet_by_job(&config, "other").unwrap().is_none());
}

#[test]
fn goals_add_list_and_archive() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let pet = store::ensure_primary(&config, Utc::now()).unwrap();
    let g = store::add_goal(&config, &pet.id, "Finish the portfolio", Utc::now()).unwrap();
    assert_eq!(
        store::list_goals(&config, &pet.id).unwrap(),
        vec![g.clone()]
    );
    assert!(store::archive_goal(&config, &pet.id, &g.id, Utc::now()).unwrap());
    assert!(!store::archive_goal(&config, &pet.id, &g.id, Utc::now()).unwrap());
    assert!(store::list_goals(&config, &pet.id).unwrap().is_empty());
}

#[test]
fn fingerprint_window_excludes_new_and_old_notes() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let pet = store::ensure_primary(&config, Utc::now()).unwrap();
    let now = Utc::now();
    let (fresh, _) =
        store_notes::insert_note(&config, &new_note(&pet.id, "a"), false, now).unwrap();
    let (old, _) = store_notes::insert_note(
        &config,
        &new_note(&pet.id, "b"),
        false,
        now - Duration::days(8),
    )
    .unwrap();
    // `new` notes never count as seen.
    let since = now - Duration::days(7);
    assert!(
        store_notes::recent_fingerprints(&config, &pet.id, since, 100)
            .unwrap()
            .is_empty()
    );
    let surfaced: Vec<SurfacedNote> = [&fresh, &old]
        .iter()
        .map(|n| SurfacedNote {
            note_id: n.id.clone(),
            score: 50,
            bucket: Bucket::Digest,
            state: PetNoteState::Queued,
        })
        .collect();
    store_notes::apply_surfacing(&config, &surfaced, now).unwrap();
    let fps = store_notes::recent_fingerprints(&config, &pet.id, since, 100).unwrap();
    assert_eq!(fps, vec![("fp:a".to_string(), "a".to_string())]);
    assert_eq!(
        store_notes::count_pass_notes(&config, &pet.id, "job-1", now - Duration::hours(2)).unwrap(),
        0,
        "surfaced notes no longer count towards the pass cap"
    );
}

#[test]
fn apply_surfacing_only_moves_new_notes_and_stamps_notified() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let pet = store::ensure_primary(&config, Utc::now()).unwrap();
    let now = Utc::now();
    let (n, _) = store_notes::insert_note(&config, &new_note(&pet.id, "x"), false, now).unwrap();
    let verdict = SurfacedNote {
        note_id: n.id.clone(),
        score: 95,
        bucket: Bucket::Notify,
        state: PetNoteState::Notified,
    };
    store_notes::apply_surfacing(&config, std::slice::from_ref(&verdict), now).unwrap();
    let stored = store_notes::get_note(&config, &n.id).unwrap().unwrap();
    assert_eq!(stored.state, PetNoteState::Notified);
    assert_eq!(stored.score, Some(95));
    assert!(stored.notified_at.is_some());
    assert_eq!(
        store_notes::notified_count_since(&config, &pet.id, now - Duration::hours(1)).unwrap(),
        1
    );
    // A second application is a no-op (only `new` rows move).
    let dropped = SurfacedNote {
        state: PetNoteState::Dropped,
        bucket: Bucket::Drop,
        ..verdict
    };
    store_notes::apply_surfacing(&config, &[dropped], now).unwrap();
    assert_eq!(
        store_notes::get_note(&config, &n.id)
            .unwrap()
            .unwrap()
            .state,
        PetNoteState::Notified
    );
}

#[test]
fn proposals_expire_lazily_and_decide_only_when_pending() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let pet = store::ensure_primary(&config, Utc::now()).unwrap();
    let now = Utc::now();
    let mut note = new_note(&pet.id, "reply");
    note.proposed_action = Some("Reply to Sam".into());
    let (stored, pid) = store_notes::insert_note(&config, &note, true, now).unwrap();
    let pid = pid.expect("proposal created");
    let pending = store_feed::list_pending_proposals(&config, &pet.id, now).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].note_title, "reply");
    assert_eq!(pending[0].note_id, stored.id);
    // Eight days later it has expired and cannot be decided.
    let later = now + Duration::days(8);
    assert!(store_feed::list_pending_proposals(&config, &pet.id, later)
        .unwrap()
        .is_empty());
    assert!(!store_feed::decide_proposal(&config, &pid, ProposalState::Accepted, later).unwrap());
    assert_eq!(
        store_feed::get_proposal(&config, &pid)
            .unwrap()
            .unwrap()
            .state,
        ProposalState::Expired
    );
}

#[test]
fn dismiss_note_dismisses_its_pending_proposal() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let pet = store::ensure_primary(&config, Utc::now()).unwrap();
    let mut note = new_note(&pet.id, "d");
    note.proposed_action = Some("Do it".into());
    let (stored, pid) = store_notes::insert_note(&config, &note, true, Utc::now()).unwrap();
    let dismissed = store_notes::dismiss_note(&config, &pet.id, &stored.id, Utc::now())
        .unwrap()
        .unwrap();
    assert_eq!(dismissed.state, PetNoteState::Dismissed);
    assert_eq!(
        store_feed::get_proposal(&config, &pid.unwrap())
            .unwrap()
            .unwrap()
            .state,
        ProposalState::Dismissed
    );
    assert!(
        store_notes::dismiss_note(&config, &pet.id, "missing", Utc::now())
            .unwrap()
            .is_none()
    );
}

#[test]
fn prune_applies_retention() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let pet = store::ensure_primary(&config, Utc::now()).unwrap();
    let now = Utc::now();
    let (old, _) = store_notes::insert_note(
        &config,
        &new_note(&pet.id, "old"),
        false,
        now - Duration::days(40),
    )
    .unwrap();
    let (keep, _) = store_notes::insert_note(
        &config,
        &new_note(&pet.id, "old but queued"),
        false,
        now - Duration::days(40),
    )
    .unwrap();
    store_notes::apply_surfacing(
        &config,
        &[
            SurfacedNote {
                note_id: old.id.clone(),
                score: 1,
                bucket: Bucket::Drop,
                state: PetNoteState::Dropped,
            },
            SurfacedNote {
                note_id: keep.id.clone(),
                score: 40,
                bucket: Bucket::Digest,
                state: PetNoteState::Queued,
            },
        ],
        now,
    )
    .unwrap();
    let run = PetRunSummary::started("scheduled");
    for i in 0..(store::KEEP_RUNS + 5) {
        store_feed::insert_run(&config, &pet.id, None, &run, now - Duration::minutes(i)).unwrap();
    }
    store_feed::prune(&config, &pet.id, now).unwrap();
    assert!(store_notes::get_note(&config, &old.id).unwrap().is_none());
    assert!(store_notes::get_note(&config, &keep.id).unwrap().is_some());
    let runs: i64 = store::with_connection(&config, |c| {
        Ok(c.query_row("SELECT COUNT(*) FROM pet_runs", [], |r| r.get(0))?)
    })
    .unwrap();
    assert_eq!(runs, store::KEEP_RUNS);
    let last = store_feed::last_run(&config, &pet.id).unwrap().unwrap();
    assert_eq!(
        last.status, "failed",
        "a `started` summary is stored as not-completed"
    );
}

#[test]
fn digest_insert_marks_notes_and_lists_newest_first() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let pet = store::ensure_primary(&config, Utc::now()).unwrap();
    let now = Utc::now();
    let (n, _) = store_notes::insert_note(&config, &new_note(&pet.id, "q"), false, now).unwrap();
    for (i, id) in ["d1", "d2"].iter().enumerate() {
        let digest = PetDigest {
            id: (*id).into(),
            pet_id: pet.id.clone(),
            created_at: now + Duration::seconds(i as i64),
            local_date: "2026-09-30".into(),
            body_md: "body".into(),
            item_count: 1,
            withheld_count: 0,
        };
        store_feed::insert_digest_and_mark(&config, &digest, std::slice::from_ref(&n.id)).unwrap();
    }
    let digests = store_feed::list_digests(&config, &pet.id, 7).unwrap();
    assert_eq!(digests[0].id, "d2");
    let stored = store_notes::get_note(&config, &n.id).unwrap().unwrap();
    assert_eq!(stored.state, PetNoteState::Digested);
    assert_eq!(stored.digest_id.as_deref(), Some("d2"));
}
