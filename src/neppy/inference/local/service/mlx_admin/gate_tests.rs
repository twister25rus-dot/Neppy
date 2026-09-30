//! Tests for the single-flight gate.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::*;

fn cfg() -> MlxWorkerConfig {
    MlxWorkerConfig {
        acquire_timeout_secs: 5,
        ..MlxWorkerConfig::default()
    }
}

#[tokio::test]
async fn a_second_acquire_waits_for_the_first_release() {
    let gate = Arc::new(InferenceGate::new());
    let first = gate.acquire(&cfg()).await.expect("first acquires");

    let waiter = {
        let gate = Arc::clone(&gate);
        tokio::spawn(async move { gate.acquire(&cfg()).await.map(|_| ()) })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!waiter.is_finished(), "second caller must wait");
    assert_eq!(gate.snapshot().waiting, 1);

    drop(first);
    waiter
        .await
        .expect("task")
        .expect("second acquires after release");
    assert_eq!(gate.snapshot().active, 0);
    assert_eq!(gate.snapshot().acquired_total, 2);
}

#[tokio::test]
async fn the_fifth_waiter_is_refused_as_busy() {
    let gate = Arc::new(InferenceGate::new());
    let held = gate.acquire(&cfg()).await.expect("holder");

    let mut waiters = Vec::new();
    for _ in 0..4 {
        let gate = Arc::clone(&gate);
        waiters.push(tokio::spawn(async move {
            gate.acquire(&cfg()).await.map(|_| ())
        }));
    }
    // Let all four queue.
    for _ in 0..50 {
        if gate.snapshot().waiting == 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(gate.snapshot().waiting, 4);

    assert_eq!(gate.acquire(&cfg()).await.err(), Some(GateError::Busy));
    assert_eq!(
        gate.snapshot().waiting,
        4,
        "a refused caller leaves no residue"
    );

    drop(held);
    for waiter in waiters {
        waiter.await.expect("task").expect("queued caller acquires");
    }
    assert_eq!(gate.snapshot().waiting, 0);
}

#[tokio::test(start_paused = true)]
async fn acquire_times_out() {
    let gate = InferenceGate::new();
    let _held = gate.acquire(&cfg()).await.expect("holder");
    let config = MlxWorkerConfig {
        acquire_timeout_secs: 1,
        ..cfg()
    };
    assert_eq!(gate.acquire(&config).await.err(), Some(GateError::Timeout));
    assert_eq!(gate.snapshot().waiting, 0);
}

#[tokio::test(start_paused = true)]
async fn a_pause_blocks_new_callers_and_reports_why() {
    let gate = InferenceGate::new();
    gate.pause(PressureState::Elevated);
    let config = MlxWorkerConfig {
        acquire_timeout_secs: 1,
        ..cfg()
    };
    assert_eq!(
        gate.acquire(&config).await.err(),
        Some(GateError::Paused(PressureState::Elevated))
    );
}

#[tokio::test]
async fn resume_releases_paused_waiters() {
    let gate = Arc::new(InferenceGate::new());
    gate.pause(PressureState::Critical);
    assert_eq!(gate.paused(), Some(PressureState::Critical));
    let waiter = {
        let gate = Arc::clone(&gate);
        tokio::spawn(async move { gate.acquire(&cfg()).await.map(|_| ()) })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!waiter.is_finished(), "paused gate admits no one");
    gate.resume();
    waiter
        .await
        .expect("task")
        .expect("resumed waiter acquires");
    assert_eq!(gate.paused(), None);
}

#[tokio::test]
async fn preempt_cancels_the_holder_and_not_the_next_caller() {
    let gate = InferenceGate::new();
    let first = gate.acquire(&cfg()).await.expect("first");
    assert!(!first.is_cancelled());
    gate.preempt();
    assert!(first.is_cancelled());
    tokio::time::timeout(Duration::from_millis(100), first.cancelled())
        .await
        .expect("cancelled() resolves after preempt");
    drop(first);

    let second = gate.acquire(&cfg()).await.expect("second");
    assert!(!second.is_cancelled(), "a fresh token after preempt");
    assert_eq!(gate.snapshot().active, 1);
    drop(second);
    assert_eq!(gate.snapshot().active, 0);
}

#[tokio::test]
async fn a_dropped_waiter_leaves_no_residue() {
    let gate = Arc::new(InferenceGate::new());
    let held = gate.acquire(&cfg()).await.expect("holder");
    let waiter = {
        let gate = Arc::clone(&gate);
        tokio::spawn(async move { gate.acquire(&cfg()).await.map(|_| ()) })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    waiter.abort();
    let _ = waiter.await;
    assert_eq!(gate.snapshot().waiting, 0);
    drop(held);
    let _again = gate.acquire(&cfg()).await.expect("slot is free again");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn active_never_exceeds_one_under_contention() {
    let gate = Arc::new(InferenceGate::new());
    let in_flight = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let config = MlxWorkerConfig {
        max_waiters: 64,
        acquire_timeout_secs: 30,
        ..MlxWorkerConfig::default()
    };

    let mut tasks = Vec::new();
    for _ in 0..50 {
        let gate = Arc::clone(&gate);
        let in_flight = Arc::clone(&in_flight);
        let peak = Arc::clone(&peak);
        let config = config.clone();
        tasks.push(tokio::spawn(async move {
            let permit = gate.acquire(&config).await.expect("acquires");
            let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            assert!(gate.snapshot().active <= 1);
            tokio::task::yield_now().await;
            in_flight.fetch_sub(1, Ordering::SeqCst);
            drop(permit);
        }));
    }
    for task in tasks {
        task.await.expect("task");
    }
    assert_eq!(peak.load(Ordering::SeqCst), 1);
    assert_eq!(gate.snapshot().acquired_total, 50);
    assert_eq!(gate.snapshot().active, 0);
}

#[test]
fn gate_errors_round_trip_through_messages() {
    for err in [
        GateError::Busy,
        GateError::Timeout,
        GateError::Preempted,
        GateError::Paused(PressureState::Elevated),
        GateError::Paused(PressureState::Critical),
    ] {
        let wrapped = format!("model error: {}", err.message());
        assert_eq!(GateError::from_message(&wrapped), Some(err), "{wrapped}");
    }
    assert_eq!(GateError::from_message("unrelated failure"), None);
    assert_eq!(GateError::from_message("[mlx:gate] something new"), None);
}

#[test]
fn idle_time_is_zero_while_held() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let gate = InferenceGate::new();
        let permit = gate.acquire(&cfg()).await.expect("acquire");
        assert_eq!(gate.snapshot().idle_for, Duration::ZERO);
        drop(permit);
        gate.touch();
        assert!(gate.snapshot().idle_for < Duration::from_secs(1));
    });
}

#[tokio::test]
async fn waiters_are_bounded_while_paused_too() {
    let gate = Arc::new(InferenceGate::new());
    gate.pause(PressureState::Elevated);
    let config = MlxWorkerConfig {
        max_waiters: 2,
        ..cfg()
    };
    let mut waiters = Vec::new();
    for _ in 0..2 {
        let gate = Arc::clone(&gate);
        let config = config.clone();
        waiters.push(tokio::spawn(async move {
            gate.acquire(&config).await.map(|_| ())
        }));
    }
    for _ in 0..50 {
        if gate.snapshot().waiting == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(gate.acquire(&config).await.err(), Some(GateError::Busy));
    gate.resume();
    for waiter in waiters {
        waiter.await.expect("task").expect("acquires after resume");
    }
}
