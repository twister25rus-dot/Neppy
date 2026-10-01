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
        GateError::Yielded,
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

// ---- priority: background callers yield to interactive ones --------------

/// A gate that asks a background holder to yield after 60 ms.
fn quick_gate() -> Arc<InferenceGate> {
    Arc::new(InferenceGate::new().with_yield_after(Duration::from_millis(60)))
}

/// Take the slot as a background caller and report how it ended: `Ok(())`
/// when released by the caller, `Err(reason)` when cancelled.
fn background_holder(
    gate: &Arc<InferenceGate>,
    hold_for: Duration,
) -> tokio::task::JoinHandle<Result<(), GateError>> {
    let gate = Arc::clone(gate);
    tokio::spawn(background_scope(async move {
        let permit = gate.acquire(&cfg()).await.expect("background acquires");
        tokio::select! {
            _ = permit.cancelled() => Err(permit.cancel_error()),
            _ = tokio::time::sleep(hold_for) => Ok(()),
        }
    }))
}

#[tokio::test]
async fn an_interactive_caller_that_waits_makes_a_background_holder_yield() {
    let gate = quick_gate();
    let holder = background_holder(&gate, Duration::from_secs(30));
    for _ in 0..100 {
        if gate.snapshot().active == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let started = std::time::Instant::now();
    let chat = gate.acquire(&cfg()).await.expect("chat acquires");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "chat got the slot long before the background step would have ended"
    );
    assert_eq!(
        holder.await.expect("task"),
        Err(GateError::Yielded),
        "the background request is told it yielded, not that memory was critical"
    );
    drop(chat);
}

#[tokio::test]
async fn a_background_holder_is_not_asked_to_yield_before_the_wait_is_long() {
    let gate = Arc::new(InferenceGate::new()); // the real 5 s threshold
    let holder = background_holder(&gate, Duration::from_millis(150));
    for _ in 0..100 {
        if gate.snapshot().active == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let chat = gate
        .acquire(&cfg())
        .await
        .expect("chat acquires after the step");
    assert_eq!(holder.await.expect("task"), Ok(()), "a short step finishes");
    drop(chat);
}

#[tokio::test]
async fn an_interactive_holder_is_never_asked_to_yield() {
    let gate = quick_gate();
    let chat = gate.acquire(&cfg()).await.expect("first chat");
    let second = {
        let gate = Arc::clone(&gate);
        tokio::spawn(async move { gate.acquire(&cfg()).await.map(|_| ()) })
    };
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(!chat.is_cancelled(), "yielding is for background work only");
    assert!(!second.is_finished());
    drop(chat);
    second.await.expect("task").expect("second chat acquires");
}

#[tokio::test]
async fn a_background_caller_queued_first_still_lets_chat_go_ahead() {
    let gate = quick_gate();
    let first = gate.acquire(&cfg()).await.expect("holder");
    let order = Arc::new(parking_lot::Mutex::new(Vec::<&'static str>::new()));
    let background = {
        let (gate, order) = (Arc::clone(&gate), Arc::clone(&order));
        tokio::spawn(background_scope(async move {
            let permit = gate.acquire(&cfg()).await.expect("background");
            order.lock().push("background");
            drop(permit);
        }))
    };
    tokio::time::sleep(Duration::from_millis(30)).await; // background queues first
    let chat = {
        let (gate, order) = (Arc::clone(&gate), Arc::clone(&order));
        tokio::spawn(async move {
            let permit = gate.acquire(&cfg()).await.expect("chat");
            order.lock().push("chat");
            // Hold long enough that a background caller that did not step
            // aside would already have run.
            tokio::time::sleep(Duration::from_millis(80)).await;
            drop(permit);
        })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    drop(first);
    chat.await.expect("chat task");
    background.await.expect("background task");
    assert_eq!(*order.lock(), vec!["chat", "background"]);
}

#[tokio::test]
async fn a_critical_preempt_is_not_reported_as_a_yield() {
    let gate = quick_gate();
    let holder = background_holder(&gate, Duration::from_secs(30));
    for _ in 0..100 {
        if gate.snapshot().active == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    gate.preempt();
    assert_eq!(holder.await.expect("task"), Err(GateError::Preempted));
}

#[tokio::test]
async fn the_background_flag_does_not_leak_out_of_its_scope() {
    assert!(!is_background());
    background_scope(async { assert!(is_background()) }).await;
    assert!(!is_background());
}

#[tokio::test]
async fn maintenance_takes_the_slot_only_when_it_is_free() {
    let gate = InferenceGate::new();
    let held = gate.acquire(&cfg()).await.expect("request");
    assert!(gate.try_acquire().is_none(), "a request holds the slot");
    drop(held);
    tokio::time::sleep(Duration::from_millis(40)).await;
    let before = gate.snapshot().idle_for;
    {
        let maintenance = gate.try_acquire().expect("free");
        assert_eq!(gate.snapshot().active, 1);
        assert!(gate.try_acquire().is_none(), "one at a time");
        // A request arriving now waits for it.
        let waiting_cfg = MlxWorkerConfig {
            acquire_timeout_secs: 1,
            ..cfg()
        };
        let waiter = gate.acquire(&waiting_cfg);
        tokio::pin!(waiter);
        assert!(
            tokio::time::timeout(Duration::from_millis(60), &mut waiter)
                .await
                .is_err(),
            "the request queues behind the maintenance"
        );
        drop(maintenance);
        waiter.await.expect("acquires once maintenance ends");
    }
    assert!(before >= Duration::from_millis(40));
}

#[tokio::test]
async fn maintenance_does_not_restart_the_idle_clock() {
    let gate = InferenceGate::new();
    tokio::time::sleep(Duration::from_millis(60)).await;
    drop(gate.try_acquire().expect("free"));
    assert!(
        gate.snapshot().idle_for >= Duration::from_millis(60),
        "an unload must not postpone the stop that follows it"
    );
}

#[tokio::test]
async fn maintenance_works_while_the_gate_is_paused() {
    let gate = InferenceGate::new();
    gate.pause(PressureState::Critical);
    assert!(gate.try_acquire().is_some());
}

#[tokio::test]
async fn a_pinned_background_holder_is_never_asked_to_yield() {
    let gate = quick_gate();
    let holder = {
        let gate = Arc::clone(&gate);
        tokio::spawn(background_scope(pinned_scope(true, async move {
            let permit = gate.acquire(&cfg()).await.expect("acquires");
            tokio::select! {
                _ = permit.cancelled() => Err(permit.cancel_error()),
                _ = tokio::time::sleep(Duration::from_millis(400)) => Ok(()),
            }
        })))
    };
    for _ in 0..100 {
        if gate.snapshot().active == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let chat = gate
        .acquire(&cfg())
        .await
        .expect("chat gets in once the step is done");
    assert_eq!(
        holder.await.expect("task"),
        Ok(()),
        "chat waited several yield periods and the pinned step still was not cancelled"
    );
    drop(chat);
}

#[tokio::test]
async fn pinned_scope_is_off_by_default_and_scoped() {
    assert!(!is_pinned());
    pinned_scope(true, async { assert!(is_pinned()) }).await;
    pinned_scope(false, async { assert!(!is_pinned()) }).await;
    assert!(!is_pinned());
}
