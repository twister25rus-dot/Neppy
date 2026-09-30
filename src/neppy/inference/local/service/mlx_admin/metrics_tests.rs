//! Tests for bounded metrics.

use super::*;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, d).expect("valid date")
}

fn sample(ts_ms: i64) -> MetricsSample {
    MetricsSample {
        ts_ms,
        worker_state: "ready".into(),
        ..MetricsSample::default()
    }
}

#[test]
fn rings_never_exceed_their_caps() {
    let sink = MetricsSink::new();
    for i in 0..(SAMPLE_RING_CAP + 25) {
        sink.record_sample(sample(i as i64));
    }
    for i in 0..(EVENT_RING_CAP + 10) {
        sink.event(event::WORKER_SPAWN, Some("primary"), format!("n={i}"));
    }
    let window = sink.recent(0, usize::MAX, false);
    assert_eq!(window.samples.len(), SAMPLE_RING_CAP);
    assert_eq!(window.events.len(), EVENT_RING_CAP);
    // The oldest were dropped, not the newest.
    assert_eq!(window.samples.first().map(|s| s.ts_ms), Some(25));
    assert_eq!(
        sink.latest_sample().map(|s| s.ts_ms),
        Some((SAMPLE_RING_CAP + 24) as i64)
    );
}

#[test]
fn recent_filters_by_time_limit_and_kind() {
    let sink = MetricsSink::new();
    for ts in [10, 20, 30, 40] {
        sink.record_sample(sample(ts));
    }
    sink.event(event::MODEL_UNLOAD, None, "idle");
    let window = sink.recent(20, 2, false);
    assert_eq!(
        window.samples.iter().map(|s| s.ts_ms).collect::<Vec<_>>(),
        vec![30, 40]
    );
    assert_eq!(window.events.len(), 1);

    let events_only = sink.recent(0, 10, true);
    assert!(events_only.samples.is_empty());
    assert_eq!(events_only.events[0].event, "model_unload");
}

#[test]
fn without_a_directory_nothing_is_written() {
    let sink = MetricsSink::new();
    sink.record_sample(sample(1));
    assert_eq!(sink.prune(1), 0);
}

#[test]
fn lines_are_json_with_a_kind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = MetricsSink::new();
    sink.configure(dir.path().to_path_buf(), 20, 7);
    sink.write_line("sample", &sample(5), day(1));
    sink.write_line(
        "event",
        &MetricsEvent::new(event::WORKER_CRASH, Some("primary"), "signal"),
        day(1),
    );
    let text = std::fs::read_to_string(dir.path().join(file_name(day(1)))).expect("file");
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).expect("json line"))
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["kind"], "sample");
    assert_eq!(lines[0]["ts_ms"], 5);
    assert_eq!(lines[1]["kind"], "event");
    assert_eq!(lines[1]["event"], "worker_crash");
}

#[test]
fn files_rotate_by_day_and_prune_keeps_n() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = MetricsSink::new();
    sink.configure(dir.path().to_path_buf(), 20, 3);
    for d in 1..=6 {
        sink.write_line("sample", &sample(i64::from(d)), day(d));
    }
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .expect("dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    // Rotation prunes before creating the new day's file, so at most
    // retention + 1 exist between prunes; an explicit prune trims to N.
    assert!(names.len() <= 4, "{names:?}");
    assert!(names.contains(&file_name(day(6))));

    // Unrelated files are never touched.
    std::fs::write(dir.path().join("notes.txt"), "keep").expect("write");
    sink.prune(2);
    let mut left: Vec<String> = std::fs::read_dir(dir.path())
        .expect("dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        vec![
            "notes.txt".to_string(),
            file_name(day(5)),
            file_name(day(6))
        ]
    );
}

#[test]
fn the_cap_writes_a_single_truncated_marker() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = MetricsSink::new();
    sink.configure(dir.path().to_path_buf(), 1, 7);
    // Pad each line so a few hundred cross 1 MiB.
    let big = MetricsEvent::new(event::MODEL_READY, None, "x".repeat(8 * 1024));
    for _ in 0..200 {
        sink.write_line("event", &big, day(2));
    }
    let path = dir.path().join(file_name(day(2)));
    let text = std::fs::read_to_string(&path).expect("file");
    let markers = text.lines().filter(|l| l.contains("\"truncated\"")).count();
    assert_eq!(markers, 1);
    assert!(text.lines().last().expect("lines").contains("truncated"));
    let len = std::fs::metadata(&path).expect("meta").len();
    assert!(len <= 1024 * 1024 + 256, "file stayed near the cap: {len}");

    // A new sink (a restart) continues from the size on disk.
    let restarted = MetricsSink::new();
    restarted.configure(dir.path().to_path_buf(), 1, 7);
    restarted.write_line("event", &big, day(2));
    assert_eq!(std::fs::metadata(&path).expect("meta").len(), len);
}

#[test]
fn usage_is_remembered() {
    let sink = MetricsSink::new();
    assert!(sink.last_usage().prompt_tokens.is_none());
    sink.set_last_usage(120, 30);
    let usage = sink.last_usage();
    assert_eq!(
        (usage.prompt_tokens, usage.completion_tokens),
        (Some(120), Some(30))
    );
}

#[test]
fn the_directory_lives_under_the_workspace() {
    let dir = MetricsSink::dir_for_workspace(Path::new("/ws"));
    assert_eq!(dir, PathBuf::from("/ws/local_assistant/metrics"));
}
