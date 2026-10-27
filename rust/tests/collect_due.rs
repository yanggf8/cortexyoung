use cort::collect::{
    batch_cursors, batch_due, batch_pending, health_cursor, health_due, min_interval_ms,
    CollectConfig, CollectState, Failure, HealthState,
};
use cort::measure_progress::Progress;
use cort::usage::KickCounters;

fn cursors(s: &mut CollectState, command: i64, health: i64) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("measure-progress.json");
    let mut progress = Progress::default();
    let machine = cort::usage::machine_id();
    progress.set_watermark(machine, "cort_command", command, "fingerprint".into());
    progress.set_watermark(machine, "cort_index_health", health, "fingerprint".into());
    progress.save(&path).unwrap();
    s.progress_path = Some(path);
    dir
}

const NOW: i64 = 1_800_000_000_000;

/// `max_any_command_id` defaults to the larger of the two explicit maxima, so every fixture that
/// was written before that field existed keeps its meaning: its cursors still compare against the
/// same ceiling, and only the tests below move the two maxima apart.
fn counters(command: i64, health: i64) -> KickCounters {
    KickCounters {
        instance_id: "db-a".into(),
        max_command_id: command,
        max_any_command_id: command.max(health),
        max_health_id: health,
    }
}

fn state(elapsed_ms: i64) -> CollectState {
    CollectState {
        instance_id: Some("db-a".into()),
        last_run_ms: Some(NOW - elapsed_ms),
        ..Default::default()
    }
}

#[test]
fn defaults_match_spec() {
    let c = CollectConfig::from_lookup(|_| None);
    assert_eq!(c.base_interval_ms, 300_000);
    assert_eq!(c.max_interval_ms, 3_600_000);
    assert_eq!(c.batch_rows, 2_000);
    assert_eq!(c.idle_due_ms, 900_000);
    assert_eq!(c.health_interval_ms, 600_000);
    assert_eq!(c.net_timeout_ms, 120_000);
    assert_eq!(c.run_deadline_ms, 600_000);
    assert_eq!(c.manual_wait_ms, 60_000);
    assert_eq!(c.root_timeout_ms, 30_000);
}

#[test]
fn remote_errors_are_classified_by_prefix() {
    assert_eq!(
        cort::collect::classify_error("remote unavailable: blocked"),
        cort::collect::ErrorKind::Network
    );
    assert_eq!(
        cort::collect::classify_error("disk I/O error"),
        cort::collect::ErrorKind::Storage
    );
}

#[test]
fn old_pending_state_loads_with_a_start_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("measure-collect-state.json");
    std::fs::write(&path, r#"{"push_pending":true,"uploaded_through_id":42}"#).unwrap();
    let state = CollectState::load(&path);
    assert!(state.upload_pending);
    assert!(state.pending_since_ms.is_some());
    assert_eq!(state.instance_id, None);
}

#[test]
fn missing_and_corrupt_progress_reset_cursors() {
    let mut state = state(900_000);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("measure-progress.json");
    state.progress_path = Some(path.clone());
    assert_eq!(batch_cursors(&state, &counters(9, 9)), (0, 0));
    std::fs::write(path, "{broken").unwrap();
    assert_eq!(batch_cursors(&state, &counters(9, 9)), (0, 0));
    assert_eq!(batch_pending(&state, &counters(9, 9)), 18);
}

#[test]
fn garbage_env_keeps_default() {
    let c = CollectConfig::from_lookup(|key| match key {
        "CORT_COLLECT_BASE_SECS" => Some("abc".into()),
        "CORT_HEALTH_INTERVAL_SECS" => Some(" 7 ".into()),
        "CORT_COLLECT_NET_TIMEOUT_SECS" => Some(" 7 ".into()),
        _ => None,
    });
    assert_eq!(c.base_interval_ms, 300_000);
    assert_eq!(c.health_interval_ms, 7_000);
    assert_eq!(c.net_timeout_ms, 7_000);
}

#[test]
fn never_run_with_pending_is_due() {
    assert!(batch_due(
        &CollectState::default(),
        &counters(1, 0),
        "codex",
        NOW,
        &CollectConfig::from_lookup(|_| None),
    ));
}

#[test]
fn within_min_interval_not_due() {
    let cfg = CollectConfig::from_lookup(|_| None);
    assert!(!batch_due(
        &state(299_000),
        &counters(2_000, 0),
        "codex",
        NOW,
        &cfg
    ));
    assert!(!batch_due(
        &state(300_000),
        &counters(1, 0),
        "codex",
        NOW,
        &cfg
    ));
}

#[test]
fn big_backlog_due_after_min_interval() {
    let cfg = CollectConfig::from_lookup(|_| None);
    assert!(batch_due(
        &state(301_000),
        &counters(2_000, 0),
        "codex",
        NOW,
        &cfg
    ));
    assert!(!batch_due(
        &state(301_000),
        &counters(1_999, 0),
        "codex",
        NOW,
        &cfg
    ));
}

#[test]
fn idle_rule() {
    assert!(batch_due(
        &state(900_000),
        &counters(1, 0),
        "codex",
        NOW,
        &CollectConfig::from_lookup(|_| None),
    ));
}

#[test]
fn upload_pending_due_with_zero_pending() {
    let mut s = state(300_000);
    s.upload_pending = true;
    assert!(batch_due(
        &s,
        &counters(0, 0),
        "codex",
        NOW,
        &CollectConfig::from_lookup(|_| None)
    ));
}

#[test]
fn refused_excludes_command_pending() {
    let mut s = state(900_000);
    s.refused = true;
    let cfg = CollectConfig::from_lookup(|_| None);
    assert!(!batch_due(&s, &counters(5_000, 0), "codex", NOW, &cfg));
    assert!(batch_due(&s, &counters(5_000, 1), "codex", NOW, &cfg));
}

/// A cursor past every row's id is a rewind: it resets to 0, so the whole command term counts as
/// pending. (The per-term clamp in `batch_due` is not defensive: a cursor may legitimately exceed
/// `max_command_id` by the `measure` rows it already copied, so the clamp is what keeps that
/// negative from eating the health term.)
#[test]
fn rewound_cursor_counts_all_rows_as_pending() {
    let mut s = state(900_000);
    let _progress = cursors(&mut s, 101, 0);
    assert!(batch_due(
        &s,
        &counters(100, 1),
        "codex",
        NOW,
        &CollectConfig::from_lookup(|_| None)
    ));
}

#[test]
fn backoff_is_per_harness() {
    let mut s = state(2_400_000);
    s.failures.insert(
        "codex".into(),
        Failure {
            count: 3,
            last_error_kind: "network".into(),
            last_failed_ms: NOW,
        },
    );
    let cfg = CollectConfig::from_lookup(|_| None);
    assert_eq!(min_interval_ms(&s, "codex", &cfg), 2_400_000);
    assert_eq!(min_interval_ms(&s, "claude-code", &cfg), 300_000);
    assert!(batch_due(&s, &counters(2_000, 0), "codex", NOW, &cfg));
    assert!(batch_due(&s, &counters(2_000, 0), "claude-code", NOW, &cfg));
    s.failures.get_mut("codex").unwrap().count = 10;
    assert_eq!(min_interval_ms(&s, "codex", &cfg), 3_600_000);
    assert!(!batch_due(&s, &counters(2_000, 0), "codex", NOW, &cfg));
}

#[test]
fn instance_change_resets_cursors() {
    let mut s = state(900_000);
    let _progress = cursors(&mut s, 40, 20);
    s.instance_id = Some("db-old".into());
    let c = counters(40, 20);
    assert_eq!(batch_cursors(&s, &c), (0, 0));
    assert!(batch_due(
        &s,
        &c,
        "codex",
        NOW,
        &CollectConfig::from_lookup(|_| None)
    ));
    let h = HealthState {
        instance_id: Some("db-old".into()),
        health_cursor: 40,
        last_run_ms: Some(NOW - 600_000),
        ..Default::default()
    };
    assert_eq!(health_cursor(&h, &c), 0);
    assert!(health_due(
        &h,
        &c,
        NOW,
        &CollectConfig::from_lookup(|_| None)
    ));
}

#[test]
fn rewind_resets_cursors() {
    let mut s = state(900_000);
    let _progress = cursors(&mut s, 101, 10);
    let c = counters(100, 10);
    assert_eq!(batch_cursors(&s, &c), (0, 10));
    assert!(batch_due(
        &s,
        &c,
        "codex",
        NOW,
        &CollectConfig::from_lookup(|_| None)
    ));
    let h = HealthState {
        instance_id: Some("db-a".into()),
        health_cursor: 101,
        last_run_ms: Some(NOW - 600_000),
        ..Default::default()
    };
    assert_eq!(health_cursor(&h, &c), 0);
    assert!(health_due(
        &h,
        &c,
        NOW,
        &CollectConfig::from_lookup(|_| None)
    ));
}

/// The health watermark resets on its own: the due rule clamps a negative pending to zero, so only
/// the returned cursor can tell a reset from a cursor left past the end.
#[test]
fn health_watermark_rewind_resets_to_zero() {
    let mut s = state(900_000);
    let _progress = cursors(&mut s, 5, 10);
    assert_eq!(batch_cursors(&s, &counters(100, 5)), (5, 0));
}

/// A state file is only JSON: a restored or hand-edited one can hold i64::MIN. Elapsed time must
/// saturate, not wrap negative (release) or panic (debug, on the hook path).
#[test]
fn extreme_past_last_run_is_due_not_stalled() {
    let cfg = CollectConfig::from_lookup(|_| None);
    let mut s = state(0);
    s.last_run_ms = Some(i64::MIN);
    assert!(batch_due(&s, &counters(2_000, 0), "codex", NOW, &cfg));
    let health = HealthState {
        instance_id: Some("db-a".into()),
        health_cursor: 1,
        last_run_ms: Some(i64::MIN),
        ..Default::default()
    };
    assert!(health_due(&health, &counters(2, 0), NOW, &cfg));
}

/// The normal state after a batch: the last rows are `measure`'s own, so the command cursor sits
/// past the non-`measure` max. Only the un-filtered max may call that a rewind -- otherwise the
/// cursor resets, the whole history counts as pending, and collection keeps spawning itself with
/// nothing to do.
#[test]
fn watermark_past_trailing_measure_rows_is_not_a_rewind() {
    let mut s = state(3_600_000);
    let _progress = cursors(&mut s, 13, 0);
    let c = KickCounters {
        instance_id: "db-a".into(),
        max_command_id: 10,
        max_any_command_id: 13,
        max_health_id: 0,
    };
    assert_eq!(batch_cursors(&s, &c), (13, 0));
    assert!(!s.upload_pending);
    assert!(!batch_due(
        &s,
        &c,
        "codex",
        NOW,
        &CollectConfig::from_lookup(|_| None)
    ));
}

#[test]
fn cursor_past_every_row_is_still_a_rewind() {
    let mut s = state(900_000);
    let _progress = cursors(&mut s, 20, 0);
    let c = KickCounters {
        instance_id: "db-a".into(),
        max_command_id: 10,
        max_any_command_id: 13,
        max_health_id: 0,
    };
    assert_eq!(batch_cursors(&s, &c), (0, 0));
    assert!(batch_due(
        &s,
        &c,
        "codex",
        NOW,
        &CollectConfig::from_lookup(|_| None)
    ));
}

#[test]
fn health_cursor_past_trailing_measure_rows_is_not_a_rewind() {
    let h = HealthState {
        instance_id: Some("db-a".into()),
        health_cursor: 13,
        last_run_ms: Some(NOW - 3_600_000),
        ..Default::default()
    };
    let c = KickCounters {
        instance_id: "db-a".into(),
        max_command_id: 10,
        max_any_command_id: 13,
        max_health_id: 0,
    };
    assert_eq!(health_cursor(&h, &c), 13);
    assert!(!health_due(
        &h,
        &c,
        NOW,
        &CollectConfig::from_lookup(|_| None)
    ));
}

#[test]
fn health_cursor_past_every_row_is_still_a_rewind() {
    let h = HealthState {
        instance_id: Some("db-a".into()),
        health_cursor: 20,
        last_run_ms: Some(NOW - 600_000),
        ..Default::default()
    };
    let c = KickCounters {
        instance_id: "db-a".into(),
        max_command_id: 10,
        max_any_command_id: 13,
        max_health_id: 0,
    };
    assert_eq!(health_cursor(&h, &c), 0);
    assert!(health_due(
        &h,
        &c,
        NOW,
        &CollectConfig::from_lookup(|_| None)
    ));
}

#[test]
fn health_due_rules() {
    let cfg = CollectConfig::from_lookup(|_| None);
    let mut h = HealthState {
        instance_id: Some("db-a".into()),
        health_cursor: 4,
        last_run_ms: Some(NOW - 600_000),
        ..Default::default()
    };
    assert!(health_due(&h, &counters(5, 0), NOW, &cfg));
    h.last_run_ms = Some(NOW - 599_999);
    assert!(!health_due(&h, &counters(5, 0), NOW, &cfg));
    h.last_run_ms = None;
    assert!(health_due(&h, &counters(5, 0), NOW, &cfg));
    assert!(!health_due(&h, &counters(4, 0), NOW, &cfg));
}

#[test]
fn state_roundtrip_and_corrupt_file_is_default() {
    let dir = tempfile::tempdir().unwrap();
    let collect_path = dir.path().join("measure-collect-state.json");
    let health_path = dir.path().join("measure-health-state.json");
    let mut s = state(900_000);
    s.failures.insert(
        "codex".into(),
        Failure {
            count: 2,
            last_error_kind: "storage".into(),
            last_failed_ms: NOW,
        },
    );
    s.upload_pending = true;
    let _progress = cursors(&mut s, 8, 9);
    s.refused = true;
    let h = HealthState {
        instance_id: Some("db-a".into()),
        health_cursor: 3,
        last_run_ms: Some(NOW),
        unmapped_count: 6,
        unmapped_recent: vec!["p1".into(), "p2".into()],
    };
    assert!(!CollectState::load(&collect_path).upload_pending);
    assert_eq!(HealthState::load(&health_path), HealthState::default());
    s.save(&collect_path).unwrap();
    h.save(&health_path).unwrap();
    let saved = CollectState::load(&collect_path);
    assert_eq!(saved.instance_id, s.instance_id);
    assert_eq!(saved.failures, s.failures);
    assert_eq!(saved.upload_pending, s.upload_pending);
    assert_eq!(saved.refused, s.refused);
    assert_eq!(HealthState::load(&health_path), h);
    std::fs::write(&collect_path, "{").unwrap();
    std::fs::write(&health_path, "{").unwrap();
    assert!(!CollectState::load(&collect_path).upload_pending);
    assert_eq!(HealthState::load(&health_path), HealthState::default());
    assert!(s.save(dir.path()).is_err());
    assert!(h.save(dir.path()).is_err());
}

#[test]
fn zero_or_negative_env_keeps_default() {
    let defaults = CollectConfig::from_lookup(|_| None);
    for raw in ["0", "-5", " 0 "] {
        let cfg = CollectConfig::from_lookup(|_| Some(raw.to_string()));
        assert_eq!(cfg, defaults, "{raw:?}");
    }
    let cfg = CollectConfig::from_lookup(|key| {
        (key == "CORT_COLLECT_BATCH_ROWS" || key == "CORT_COLLECT_BASE_SECS").then(|| "0".into())
    });
    assert!(!batch_due(
        &state(300_000),
        &counters(0, 0),
        "codex",
        NOW,
        &cfg
    ));
}

#[test]
fn future_last_run_does_not_stall_either_lane() {
    let cfg = CollectConfig::from_lookup(|_| None);
    let ahead = state(-3_600_000);
    assert!(batch_due(&ahead, &counters(2_000, 0), "codex", NOW, &cfg));
    let mut pushing = state(-3_600_000);
    pushing.upload_pending = true;
    assert!(batch_due(&pushing, &counters(0, 0), "codex", NOW, &cfg));
    let health = HealthState {
        instance_id: Some("db-a".into()),
        health_cursor: 1,
        last_run_ms: Some(NOW + 3_600_000),
        ..Default::default()
    };
    assert!(health_due(&health, &counters(2, 0), NOW, &cfg));
}
