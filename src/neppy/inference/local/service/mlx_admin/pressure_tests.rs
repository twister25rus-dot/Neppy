//! Hysteresis table and parser tests for `pressure`.

use std::time::{Duration, Instant};

use super::*;

const GIB: u64 = 1024 * 1024 * 1024;

fn reading(level: u8, avail_pct: f64) -> SystemMemory {
    let total = 36 * GIB;
    SystemMemory {
        total_bytes: total,
        avail_pct,
        avail_bytes: (total as f64 * avail_pct / 100.0) as u64,
        pressure_level: level,
        swap_used_bytes: 0,
        compressed_bytes: 0,
    }
}

fn cfg() -> MlxWorkerConfig {
    MlxWorkerConfig::default()
}

/// `(offset_secs, level, avail_pct, footprint)`.
type Row = (u64, u8, f64, Option<u64>);
/// `(name, rows, expected states)`.
type Case = (&'static str, Vec<Row>, Vec<PressureState>);

/// Feed `(offset_secs, level, avail_pct, footprint)` rows and return the
/// state after each.
fn run(rows: &[Row], budget: u64) -> Vec<PressureState> {
    let base = Instant::now();
    let mut tracker = PressureTracker::new();
    rows.iter()
        .map(|&(at, level, avail, footprint)| {
            tracker
                .observe(
                    &reading(level, avail),
                    footprint,
                    budget,
                    base + Duration::from_secs(at),
                    &cfg(),
                )
                .0
        })
        .collect()
}

use PressureState::{Critical as C, Elevated as E, Normal as N};

#[test]
fn hysteresis_table() {
    let cases: Vec<Case> = vec![
        ("steady normal", vec![(0, 1, 70.0, None)], vec![N]),
        (
            "level 2 elevates",
            vec![(0, 1, 70.0, None), (2, 2, 70.0, None)],
            vec![N, E],
        ),
        ("avail below 20 elevates", vec![(0, 1, 19.9, None)], vec![E]),
        (
            "avail exactly 20 is normal",
            vec![(0, 1, 20.0, None)],
            vec![N],
        ),
        ("level 4 is critical", vec![(0, 4, 70.0, None)], vec![C]),
        (
            "avail below 10 is critical",
            vec![(0, 1, 9.0, None)],
            vec![C],
        ),
        (
            "25% after elevated stays elevated (below recover threshold)",
            vec![
                (0, 1, 15.0, None),
                (40, 1, 25.0, None),
                (100, 1, 25.0, None),
            ],
            vec![E, E, E],
        ),
        (
            "recovery needs 30s at >=30%",
            vec![
                (0, 1, 15.0, None),
                (10, 1, 35.0, None),
                (20, 1, 35.0, None),
                (39, 1, 35.0, None),
                (40, 1, 35.0, None),
            ],
            vec![E, E, E, E, N],
        ),
        (
            "a dip resets the recovery clock",
            vec![
                (0, 1, 15.0, None),
                (10, 1, 35.0, None),
                (30, 1, 25.0, None),
                (45, 1, 35.0, None),
                (70, 1, 35.0, None),
                (75, 1, 35.0, None),
            ],
            vec![E, E, E, E, E, N],
        ),
        (
            "critical steps down to elevated, then recovers with hold",
            vec![
                (0, 4, 50.0, None),
                (2, 1, 50.0, None),
                (20, 1, 50.0, None),
                (32, 1, 50.0, None),
            ],
            vec![C, E, E, N],
        ),
        (
            "critical to 15% is elevated",
            vec![(0, 1, 5.0, None), (2, 1, 15.0, None)],
            vec![C, E],
        ),
        (
            "worker over budget elevates",
            vec![(0, 1, 70.0, Some(15 * GIB))],
            vec![E],
        ),
        (
            "worker within budget is normal",
            vec![(0, 1, 70.0, Some(10 * GIB))],
            vec![N],
        ),
        (
            "level 2 blocks recovery even at high avail",
            vec![(0, 2, 70.0, None), (60, 2, 70.0, None)],
            vec![E, E],
        ),
    ];

    for (name, rows, expected) in cases {
        assert_eq!(run(&rows, 14 * GIB), expected, "case: {name}");
    }
}

#[test]
fn a_zero_budget_disables_the_footprint_signal() {
    assert_eq!(run(&[(0, 1, 70.0, Some(100 * GIB))], 0), vec![N]);
}

#[test]
fn swap_growth_since_load_elevates() {
    let mut tracker = PressureTracker::new();
    let now = Instant::now();
    tracker.set_swap_baseline(Some(GIB));
    let mut sample = reading(1, 70.0);
    sample.swap_used_bytes = GIB + SWAP_GROWTH_LIMIT_BYTES;
    assert_eq!(tracker.observe(&sample, None, 0, now, &cfg()).0, N);
    sample.swap_used_bytes = GIB + SWAP_GROWTH_LIMIT_BYTES + 1;
    let (state, transition) = tracker.observe(&sample, None, 0, now, &cfg());
    assert_eq!(state, E);
    assert!(transition.expect("a transition").reason.contains("swap"));

    tracker.set_swap_baseline(None);
    assert!(tracker.swap_baseline.is_none());
}

#[test]
fn transitions_are_reported_once() {
    let mut tracker = PressureTracker::new();
    let now = Instant::now();
    let (_, first) = tracker.observe(&reading(2, 70.0), None, 0, now, &cfg());
    let first = first.expect("normal -> elevated");
    assert_eq!((first.from, first.to), (N, E));
    let (_, second) = tracker.observe(&reading(2, 70.0), None, 0, now, &cfg());
    assert!(second.is_none(), "no transition while the state holds");
    assert_eq!(tracker.state(), E);
}

#[test]
fn reserve_defaults_to_the_larger_of_8_gib_and_a_quarter() {
    let config = cfg();
    assert_eq!(reserve_bytes(&config, 36 * GIB), 9 * GIB);
    assert_eq!(reserve_bytes(&config, 16 * GIB), 8 * GIB);
    let explicit = MlxWorkerConfig {
        reserve_gib: 2.5,
        ..cfg()
    };
    assert_eq!(reserve_bytes(&explicit, 36 * GIB), 5 * GIB / 2);
}

#[test]
fn gpu_counter_is_parsed_and_the_driver_variant_skipped() {
    let text = r#"{"In use system memory (driver)"=0,"Alloc system memory"=12092997632,"In use system memory"=4543037440,"x"=1}"#;
    assert_eq!(parse_gpu_in_use(text), Some(4_543_037_440));
    assert_eq!(parse_gpu_in_use("nothing here"), None);
}

#[test]
fn meminfo_is_parsed() {
    let text = "MemTotal:       16000000 kB\nMemFree: 1 kB\nMemAvailable:    4000000 kB\n\
                SwapTotal:       2000000 kB\nSwapFree:        1500000 kB\n";
    assert_eq!(
        parse_meminfo(text),
        Some((16_000_000 * 1024, 4_000_000 * 1024, 500_000 * 1024))
    );
    assert_eq!(parse_meminfo("garbage"), None);
}

#[test]
fn psi_maps_to_levels() {
    let calm = "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0";
    let warn = "some avg10=12.50 avg60=1.00 avg300=0.00 total=5\nfull avg10=1.00 avg60=0.00 avg300=0.00 total=1";
    let crit = "some avg10=40.00 avg60=1.00 avg300=0.00 total=5\nfull avg10=20.00 avg60=0.00 avg300=0.00 total=1";
    assert_eq!(parse_psi_level(calm), 1);
    assert_eq!(parse_psi_level(warn), 2);
    assert_eq!(parse_psi_level(crit), 4);
}

#[test]
fn a_live_sample_is_plausible() {
    let sample = sample_system();
    assert!(sample.total_bytes > 0);
    assert!((0.0..=100.0).contains(&sample.avail_pct));
    assert!(matches!(sample.pressure_level, 1 | 2 | 4));
    let own = sample_process(std::process::id()).expect("own process is readable");
    assert!(own.rss_bytes > 0);
}

#[test]
fn a_missing_process_samples_as_none() {
    // PIDs this high are not allocated on any supported platform.
    assert!(sample_process(u32::MAX - 7).is_none());
}

#[test]
fn percent_of_zero_total_is_full() {
    assert_eq!(percent(5, 0), 100.0);
    assert_eq!(percent(25, 100), 25.0);
}
