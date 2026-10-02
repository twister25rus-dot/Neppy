//! Headless tests for the Pet companion shell: pure helpers plus the lease
//! loop and pause ordering against fake RPC / tray / shortcut host.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::Notify;

use super::hotkeys::*;
use super::indicator::*;
use super::rpc::*;
use super::*;

type Log = Arc<Mutex<Vec<String>>>;

// ---------- fakes ----------

struct FakeTray {
    exists: bool,
    fail_apply: Mutex<bool>,
    specs: Mutex<Vec<IndicatorSpec>>,
    log: Log,
}

impl FakeTray {
    fn new(exists: bool, log: &Log) -> Arc<Self> {
        Arc::new(Self {
            exists,
            fail_apply: Mutex::new(false),
            specs: Mutex::new(Vec::new()),
            log: log.clone(),
        })
    }
    fn last_title(&self) -> Option<&'static str> {
        self.specs.lock().unwrap().last().and_then(|s| s.title)
    }
}

impl TrayIndicator for FakeTray {
    fn exists(&self) -> bool {
        self.exists
    }
    fn apply(&self, spec: &IndicatorSpec) -> Result<(), String> {
        if !self.exists || *self.fail_apply.lock().unwrap() {
            return Err("no tray".into());
        }
        self.log
            .lock()
            .unwrap()
            .push(format!("tray:{}", spec.title.unwrap_or("-")));
        self.specs.lock().unwrap().push(spec.clone());
        Ok(())
    }
}

#[derive(Default)]
struct FakeRpc {
    leases: Mutex<VecDeque<Result<LeaseResponse, RpcError>>>,
    last_lease: Mutex<Option<Result<LeaseResponse, RpcError>>>,
    requests: Mutex<Vec<LeaseRequest>>,
    pause_result: Mutex<Option<Result<StatusLite, RpcError>>>,
    resume_result: Mutex<Option<Result<StatusLite, RpcError>>>,
    gate: Option<Arc<Notify>>,
    started: Arc<Notify>,
    log: Log,
}

impl FakeRpc {
    fn new(log: &Log) -> Self {
        Self {
            log: log.clone(),
            ..Default::default()
        }
    }
    fn push_lease(&self, r: Result<LeaseResponse, RpcError>) {
        self.leases.lock().unwrap().push_back(r);
    }
    async fn wait_gate(&self) {
        self.started.notify_one();
        if let Some(g) = &self.gate {
            g.notified().await;
        }
    }
}

#[async_trait]
impl CompanionRpc for FakeRpc {
    async fn lease(&self, req: &LeaseRequest) -> Result<LeaseResponse, RpcError> {
        self.requests.lock().unwrap().push(req.clone());
        let next = self.leases.lock().unwrap().pop_front();
        match next {
            Some(r) => {
                *self.last_lease.lock().unwrap() = Some(r.clone());
                r
            }
            None => self
                .last_lease
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| Err(RpcError::Transport("none queued".into()))),
        }
    }
    async fn pause(&self, _s: Source) -> Result<StatusLite, RpcError> {
        self.log.lock().unwrap().push("rpc:pause:start".into());
        self.wait_gate().await;
        self.log.lock().unwrap().push("rpc:pause:done".into());
        self.pause_result
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| {
                Ok(StatusLite {
                    state: "paused".into(),
                    paused: true,
                    screen_capture_active: false,
                })
            })
    }
    async fn resume(&self, _s: Source) -> Result<StatusLite, RpcError> {
        self.log.lock().unwrap().push("rpc:resume:start".into());
        self.wait_gate().await;
        self.resume_result
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| {
                Ok(StatusLite {
                    state: "observing".into(),
                    ..Default::default()
                })
            })
    }
    async fn ask(&self, _s: Source) -> Result<ActionOutcome, RpcError> {
        self.log.lock().unwrap().push("rpc:ask".into());
        Ok(ActionOutcome {
            status: "ok".into(),
        })
    }
    async fn capture(&self, _s: Source) -> Result<ActionOutcome, RpcError> {
        self.log.lock().unwrap().push("rpc:capture".into());
        Ok(ActionOutcome {
            status: "ok".into(),
        })
    }
}

#[derive(Default)]
struct FakeHost {
    registered: Mutex<Vec<String>>,
    fail: Mutex<HashSet<String>>,
    dictation: Mutex<Vec<String>>,
    ptt: Mutex<Vec<String>>,
}

impl ShortcutHost for FakeHost {
    fn register(&self, accelerator: &str, _on_press: PressHandler) -> Result<(), String> {
        if self.fail.lock().unwrap().contains(accelerator) {
            return Err("in use".into());
        }
        self.registered
            .lock()
            .unwrap()
            .push(accelerator.to_string());
        Ok(())
    }
    fn unregister(&self, accelerator: &str) -> Result<(), String> {
        self.registered.lock().unwrap().retain(|a| a != accelerator);
        Ok(())
    }
    fn dictation_current(&self) -> Vec<String> {
        self.dictation.lock().unwrap().clone()
    }
    fn ptt_current(&self) -> Vec<String> {
        self.ptt.lock().unwrap().clone()
    }
}

fn lease(enabled: bool, state: &str, paused: bool, screen: bool) -> LeaseResponse {
    LeaseResponse {
        enabled,
        state: state.into(),
        paused,
        platform_supported: true,
        screen_capture_active: screen,
        hotkeys: HotkeyConfig {
            pause: Some("Alt+Shift+Cmd+P".into()),
            ask: Some("Alt+Shift+Cmd+Space".into()),
            capture: Some("Alt+Shift+Cmd+S".into()),
        },
    }
}

struct Rig {
    shell: Arc<Shell>,
    rpc: Arc<FakeRpc>,
    tray: Arc<FakeTray>,
    host: Arc<FakeHost>,
    log: Log,
}

fn rig_with(tray_exists: bool, gate: Option<Arc<Notify>>, sensors: bool) -> Rig {
    let log: Log = Arc::default();
    let mut rpc = FakeRpc::new(&log);
    rpc.gate = gate;
    let rpc = Arc::new(rpc);
    let tray = FakeTray::new(tray_exists, &log);
    let host = Arc::new(FakeHost::default());
    let shell = Shell::new(rpc.clone(), tray.clone(), host.clone(), sensors);
    Rig {
        shell,
        rpc,
        tray,
        host,
        log,
    }
}

fn rig(tray_exists: bool) -> Rig {
    rig_with(tray_exists, None, true)
}

// ---------- pure: indicator ----------

#[test]
fn indicator_states_are_distinguishable() {
    let obs = indicator_for(IndicatorState::Observing);
    let scr = indicator_for(IndicatorState::ObservingScreen);
    let pau = indicator_for(IndicatorState::Paused);
    let off = indicator_for(IndicatorState::Off);
    assert_eq!(obs.title, Some("●"));
    assert_eq!(pau.title, Some("‖"));
    assert_eq!(off.title, None);
    assert_ne!(obs.title, scr.title);
    assert!(scr.title.unwrap().contains("screen"));
    assert!(scr.tooltip.unwrap().to_lowercase().contains("screen"));
    assert!(!obs.tooltip.unwrap().to_lowercase().contains("screen"));
    assert!(obs.tooltip.unwrap().contains("⌥⇧⌘P"));
}

#[test]
fn indicator_menu_labels() {
    assert_eq!(
        indicator_for(IndicatorState::Observing).toggle_label,
        LABEL_PAUSE
    );
    assert_eq!(
        indicator_for(IndicatorState::ObservingScreen).toggle_label,
        LABEL_PAUSE
    );
    assert_eq!(
        indicator_for(IndicatorState::Paused).toggle_label,
        LABEL_RESUME
    );
    assert!(!indicator_for(IndicatorState::Off).toggle_enabled);
    assert!(indicator_for(IndicatorState::Paused).toggle_enabled);
}

#[test]
fn from_wire_maps_states_conservatively() {
    use IndicatorState::*;
    assert_eq!(
        IndicatorState::from_wire(Some(false), "observing", false, true),
        Off
    );
    assert_eq!(
        IndicatorState::from_wire(Some(true), "off", false, false),
        Off
    );
    assert_eq!(
        IndicatorState::from_wire(Some(true), "observing", false, false),
        Observing
    );
    assert_eq!(
        IndicatorState::from_wire(Some(true), "observing", false, true),
        ObservingScreen
    );
    assert_eq!(
        IndicatorState::from_wire(Some(true), "paused", true, true),
        Paused
    );
    assert_eq!(
        IndicatorState::from_wire(Some(true), "observing", true, true),
        Paused
    );
    assert_eq!(
        IndicatorState::from_wire(Some(true), "suspended", false, true),
        Suspended
    );
    // screen flag alone never claims observation
    assert_eq!(
        IndicatorState::from_wire(Some(true), "suspended", false, true).is_observing(),
        false
    );
    assert_eq!(
        IndicatorState::from_wire(Some(true), "weird", false, false),
        Off
    );
    assert_eq!(
        IndicatorState::from_wire(None, "paused", true, false),
        Paused
    );
}

// ---------- pure: lease interval ----------

#[test]
fn lease_interval_values() {
    assert_eq!(lease_interval(true, 0), Duration::from_secs(2));
    assert_eq!(lease_interval(false, 0), Duration::from_secs(10));
    assert_eq!(lease_interval(true, 1), Duration::from_secs(4));
    assert_eq!(lease_interval(true, 2), Duration::from_secs(8));
    assert_eq!(lease_interval(true, 3), Duration::from_secs(10));
    assert_eq!(lease_interval(true, 200), Duration::from_secs(10));
    assert_eq!(lease_interval(false, 5), Duration::from_secs(10));
}

// ---------- pure: hotkeys ----------

fn reg(items: &[(HotkeyAction, &[&str])]) -> Registered {
    items
        .iter()
        .map(|(a, v)| (*a, v.iter().map(|s| s.to_string()).collect()))
        .collect()
}

#[test]
fn normalize_accepts_glyphs_and_aliases() {
    assert_eq!(normalize_accelerator("⌥⇧⌘P"), "Alt+Shift+Cmd+P");
    assert_eq!(normalize_accelerator("⌥⇧⌘Space"), "Alt+Shift+Cmd+Space");
    assert_eq!(normalize_accelerator("Option + Command + S"), "Alt+Cmd+S");
}

#[test]
fn expand_requires_a_modifier_and_a_key() {
    assert!(expand_accelerator("P").is_err());
    assert!(expand_accelerator("Alt+Shift").is_err());
    assert!(expand_accelerator("").is_err());
    assert!(expand_accelerator("Alt++P").is_err());
    assert_eq!(expand_accelerator("⌥⇧⌘P").unwrap(), vec!["Alt+Shift+Cmd+P"]);
}

#[test]
fn desired_hotkeys_respects_enabled_and_platform() {
    let cfg = HotkeyConfig::default();
    let (none, _) = desired_hotkeys(&cfg, false, true);
    assert!(none.is_empty());
    let (all, errs) = desired_hotkeys(&cfg, true, true);
    assert!(errs.is_empty());
    assert_eq!(all.len(), 3);
    let (pause_only, _) = desired_hotkeys(&cfg, true, false);
    assert_eq!(
        pause_only.keys().copied().collect::<Vec<_>>(),
        vec![HotkeyAction::Pause]
    );
}

#[test]
fn desired_hotkeys_reports_invalid_and_skips_disabled() {
    let cfg = HotkeyConfig {
        pause: Some("P".into()),
        ask: None,
        capture: Some("".into()),
    };
    let (out, errs) = desired_hotkeys(&cfg, true, true);
    assert!(out.is_empty());
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].name, "pause");
}

#[test]
fn d6_defaults_are_the_documented_chords() {
    assert_eq!(default_accelerator("pause", true), "Alt+Shift+Cmd+P");
    assert_eq!(default_accelerator("ask", true), "Alt+Shift+Cmd+Space");
    assert_eq!(default_accelerator("capture", true), "Alt+Shift+Cmd+S");
}

#[test]
fn conflicts_with_dictation_ptt_and_each_other_are_reported() {
    let desired = reg(&[
        (HotkeyAction::Pause, &["Alt+Shift+Cmd+P"]),
        (HotkeyAction::Ask, &["alt+shift+cmd+space"]),
        (HotkeyAction::Capture, &["Alt+Shift+Cmd+P"]),
    ]);
    let dictation = vec!["Alt+Shift+Cmd+Space".to_string()];
    let (ok, errs) = filter_conflicts(desired, &dictation, &[]);
    assert_eq!(
        ok.keys().copied().collect::<Vec<_>>(),
        vec![HotkeyAction::Pause]
    );
    assert_eq!(errs.len(), 2);
    assert!(errs
        .iter()
        .any(|e| e.name == "ask" && e.error.contains("dictation")));
    assert!(errs
        .iter()
        .any(|e| e.name == "capture" && e.error.contains("duplicates")));

    let (ok, errs) = filter_conflicts(
        reg(&[(HotkeyAction::Pause, &["Alt+Shift+Cmd+P"])]),
        &[],
        &["ALT+SHIFT+CMD+P".to_string()],
    );
    assert!(ok.is_empty());
    assert!(errs[0].error.contains("push-to-talk"));
}

#[test]
fn hotkey_plan_diffs_register_and_unregister() {
    let old = reg(&[
        (HotkeyAction::Pause, &["Alt+Shift+Cmd+P"]),
        (HotkeyAction::Ask, &["Alt+Shift+Cmd+Space"]),
    ]);
    let new = reg(&[
        (HotkeyAction::Pause, &["alt+shift+cmd+p"]), // same, case differs
        (HotkeyAction::Ask, &["Alt+Shift+Cmd+A"]),   // changed
        (HotkeyAction::Capture, &["Alt+Shift+Cmd+S"]), // new
    ]);
    let plan = hotkey_plan(&old, &new);
    assert_eq!(plan.len(), 2);
    assert_eq!(plan[0].action, HotkeyAction::Ask);
    assert_eq!(plan[0].unregister, vec!["Alt+Shift+Cmd+Space"]);
    assert_eq!(plan[0].register, vec!["Alt+Shift+Cmd+A"]);
    assert_eq!(plan[1].action, HotkeyAction::Capture);
    assert!(plan[1].unregister.is_empty());
    // everything removed
    let plan = hotkey_plan(&old, &Registered::new());
    assert_eq!(plan.len(), 2);
    assert!(plan
        .iter()
        .all(|p| p.register.is_empty() && !p.unregister.is_empty()));
    assert!(hotkey_plan(&old, &old).is_empty());
}

// ---------- pure: rpc parsing ----------

#[test]
fn parse_lease_response_accepts_all_envelopes() {
    let raw = json!({"enabled":true,"state":"observing","paused":false,"platform_supported":true,
        "screen_capture_active":true,
        "hotkeys":{"pause":"⌥⇧⌘P","ask":"⌥⇧⌘Space","capture":"⌥⇧⌘S"}});
    let a = parse_lease_response(raw.clone()).unwrap();
    assert!(a.enabled && a.screen_capture_active);
    let b = parse_lease_response(json!({"result": raw.clone(), "logs": []})).unwrap();
    assert_eq!(a, b);
    let c =
        parse_lease_response(json!({"jsonrpc":"2.0","id":1,"result":{"result": raw,"logs":["x"]}}))
            .unwrap();
    assert_eq!(a, c);
}

#[test]
fn parse_lease_response_defaults_and_errors() {
    let r = parse_lease_response(json!({"enabled":true,"state":"observing"})).unwrap();
    assert_eq!(r.hotkeys, HotkeyConfig::default());
    assert!(!r.screen_capture_active);
    let r =
        parse_lease_response(json!({"enabled":true,"hotkeys":{"pause":null,"ask":""}})).unwrap();
    assert_eq!(r.hotkeys.pause, None);
    assert_eq!(r.hotkeys.ask.as_deref(), Some(""));
    assert!(r.hotkeys.capture.is_some());
    let e = parse_lease_response(
        json!({"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"unknown method"}}),
    );
    assert_eq!(e, Err(RpcError::Remote("unknown method".into())));
}

#[test]
fn lease_request_params_shape() {
    let p = LeaseRequest {
        visible: true,
        hotkey_errors: vec![],
    }
    .to_params();
    assert_eq!(p, json!({"indicator":"tray","visible":true}));
    let p = LeaseRequest {
        visible: false,
        hotkey_errors: vec![HotkeyError {
            name: "ask".into(),
            error: "x".into(),
        }],
    }
    .to_params();
    assert_eq!(p["hotkey_errors"][0]["name"], "ask");
    assert_eq!(Source::Hotkey.as_str(), "hotkey");
    assert_eq!(Source::Tray.as_str(), "tray");
}

// ---------- loop with fakes ----------

#[tokio::test]
async fn missing_tray_sends_visible_false() {
    let r = rig(false);
    r.rpc.push_lease(Ok(lease(true, "suspended", false, false)));
    r.shell.lease_tick().await;
    let reqs = r.rpc.requests.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    assert!(!reqs[0].visible);
}

#[tokio::test]
async fn present_tray_sends_visible_true_and_draws_observing() {
    let r = rig(true);
    r.rpc.push_lease(Ok(lease(true, "observing", false, false)));
    let wait = r.shell.lease_tick().await;
    assert!(r.rpc.requests.lock().unwrap()[0].visible);
    assert_eq!(r.tray.last_title(), Some("●"));
    assert_eq!(wait, Duration::from_secs(2));
}

#[tokio::test]
async fn screen_capture_active_shows_observing_screen() {
    let r = rig(true);
    r.rpc.push_lease(Ok(lease(true, "observing", false, true)));
    r.shell.lease_tick().await;
    assert_eq!(r.shell.current_indicator(), IndicatorState::ObservingScreen);
    assert_eq!(r.tray.last_title(), Some("● screen"));
    // capture stops (e.g. excluded app) -> back to plain observing
    r.rpc.push_lease(Ok(lease(true, "observing", false, false)));
    r.shell.lease_tick().await;
    assert_eq!(r.shell.current_indicator(), IndicatorState::Observing);
}

#[tokio::test]
async fn disabled_companion_polls_slowly_and_draws_nothing() {
    let r = rig(true);
    r.rpc.push_lease(Ok(lease(false, "off", false, false)));
    let wait = r.shell.lease_tick().await;
    assert_eq!(wait, Duration::from_secs(10));
    assert_eq!(r.shell.current_indicator(), IndicatorState::Off);
    assert!(r.host.registered.lock().unwrap().is_empty());
}

#[tokio::test]
async fn undrawable_indicator_withdraws_lease_immediately() {
    let r = rig(true);
    *r.tray.fail_apply.lock().unwrap() = true;
    r.rpc.push_lease(Ok(lease(true, "observing", false, false)));
    r.shell.lease_tick().await;
    let reqs = r.rpc.requests.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    assert!(!reqs[1].visible);
    drop(reqs);
    // and the next regular lease also reports not visible
    r.shell.lease_tick().await;
    assert!(!r.rpc.requests.lock().unwrap()[2].visible);
}

#[tokio::test]
async fn unreachable_core_backs_off_and_clears_indicator() {
    let r = rig(true);
    r.rpc.push_lease(Ok(lease(true, "observing", false, false)));
    r.shell.lease_tick().await;
    r.rpc.push_lease(Err(RpcError::Transport("down".into())));
    let w1 = r.shell.lease_tick().await;
    assert_eq!(w1, Duration::from_secs(4));
    assert_eq!(r.shell.current_indicator(), IndicatorState::Observing);
    let w2 = r.shell.lease_tick().await;
    assert_eq!(w2, Duration::from_secs(8));
    assert_eq!(r.shell.current_indicator(), IndicatorState::Off);
    // recovery resets the backoff
    r.rpc.push_lease(Ok(lease(true, "observing", false, false)));
    assert_eq!(r.shell.lease_tick().await, Duration::from_secs(2));
}

#[tokio::test]
async fn hotkeys_register_from_lease_and_change_on_update() {
    let r = rig(true);
    r.rpc.push_lease(Ok(lease(true, "observing", false, false)));
    r.shell.lease_tick().await;
    let mut got = r.host.registered.lock().unwrap().clone();
    got.sort();
    assert_eq!(
        got,
        vec!["Alt+Shift+Cmd+P", "Alt+Shift+Cmd+S", "Alt+Shift+Cmd+Space"]
    );

    let mut l = lease(true, "observing", false, false);
    l.hotkeys.ask = Some("Alt+Shift+Cmd+A".into());
    r.rpc.push_lease(Ok(l));
    r.shell.lease_tick().await;
    let got = r.host.registered.lock().unwrap().clone();
    assert!(got.contains(&"Alt+Shift+Cmd+A".to_string()));
    assert!(!got.contains(&"Alt+Shift+Cmd+Space".to_string()));

    // disabling unregisters everything
    r.rpc.push_lease(Ok(lease(false, "off", false, false)));
    r.shell.lease_tick().await;
    assert!(r.host.registered.lock().unwrap().is_empty());
}

#[tokio::test]
async fn conflicting_hotkey_is_skipped_and_reported_in_next_lease() {
    let r = rig(true);
    *r.host.dictation.lock().unwrap() = vec!["Alt+Shift+Cmd+Space".to_string()];
    r.rpc.push_lease(Ok(lease(true, "observing", false, false)));
    r.shell.lease_tick().await;
    assert!(!r
        .host
        .registered
        .lock()
        .unwrap()
        .contains(&"Alt+Shift+Cmd+Space".to_string()));
    r.shell.lease_tick().await;
    let reqs = r.rpc.requests.lock().unwrap();
    assert_eq!(reqs[1].hotkey_errors.len(), 1);
    assert_eq!(reqs[1].hotkey_errors[0].name, "ask");
}

#[tokio::test]
async fn failed_registration_is_reported_and_not_retried_every_tick() {
    let r = rig(true);
    r.host.fail.lock().unwrap().insert("Alt+Shift+Cmd+S".into());
    r.rpc.push_lease(Ok(lease(true, "observing", false, false)));
    r.shell.lease_tick().await;
    r.shell.lease_tick().await;
    let reqs = r.rpc.requests.lock().unwrap();
    assert_eq!(reqs[1].hotkey_errors[0].name, "capture");
    drop(reqs);
    // error persists across a third tick without re-registering
    r.host.fail.lock().unwrap().clear();
    r.shell.lease_tick().await;
    assert!(!r
        .host
        .registered
        .lock()
        .unwrap()
        .contains(&"Alt+Shift+Cmd+S".to_string()));
    assert_eq!(r.rpc.requests.lock().unwrap()[2].hotkey_errors.len(), 1);
}

#[tokio::test]
async fn non_macos_registers_only_pause() {
    let r = rig_with(true, None, false);
    r.rpc.push_lease(Ok(lease(true, "observing", false, false)));
    r.shell.lease_tick().await;
    assert_eq!(*r.host.registered.lock().unwrap(), vec!["Alt+Shift+Cmd+P"]);
}

// ---------- pause ordering ----------

#[tokio::test]
async fn pause_updates_tray_before_the_rpc_resolves() {
    let gate = Arc::new(Notify::new());
    let r = rig_with(true, Some(gate.clone()), true);
    r.rpc.push_lease(Ok(lease(true, "observing", false, true)));
    r.shell.lease_tick().await;
    assert_eq!(r.shell.current_indicator(), IndicatorState::ObservingScreen);

    let shell = r.shell.clone();
    let started = r.rpc.started.clone();
    let task = tokio::spawn(async move { shell.on_hotkey(HotkeyAction::Pause).await });
    started.notified().await; // RPC has begun and is parked on the gate

    // RPC unresolved, yet the tray already shows paused.
    assert_eq!(r.tray.last_title(), Some("‖"));
    assert_eq!(r.shell.current_indicator(), IndicatorState::Paused);
    {
        let log = r.log.lock().unwrap();
        let tray_idx = log.iter().rposition(|e| e == "tray:‖").unwrap();
        let rpc_idx = log.iter().position(|e| e == "rpc:pause:start").unwrap();
        assert!(
            tray_idx < rpc_idx,
            "tray must flip before the RPC is sent: {log:?}"
        );
        assert!(!log.contains(&"rpc:pause:done".to_string()));
    }
    gate.notify_one();
    task.await.unwrap();
    assert_eq!(r.shell.current_indicator(), IndicatorState::Paused);
}

#[tokio::test]
async fn pause_rpc_failure_reverts_the_tray() {
    let r = rig(true);
    r.rpc.push_lease(Ok(lease(true, "observing", false, true)));
    r.shell.lease_tick().await;
    *r.rpc.pause_result.lock().unwrap() = Some(Err(RpcError::Transport("down".into())));
    let res = r.shell.toggle_pause(Source::Tray).await;
    assert_eq!(res, ToggleResult::Reverted);
    assert_eq!(r.shell.current_indicator(), IndicatorState::ObservingScreen);
}

#[tokio::test]
async fn resume_restores_last_active_state() {
    let r = rig(true);
    r.rpc.push_lease(Ok(lease(true, "observing", false, true)));
    r.shell.lease_tick().await;
    r.shell.toggle_pause(Source::Hotkey).await;
    assert_eq!(r.shell.current_indicator(), IndicatorState::Paused);
    *r.rpc.resume_result.lock().unwrap() = Some(Ok(StatusLite {
        state: "observing".into(),
        paused: false,
        screen_capture_active: true,
    }));
    let res = r.shell.toggle_pause(Source::Tray).await;
    assert_eq!(res, ToggleResult::Applied(IndicatorState::ObservingScreen));
}

#[tokio::test]
async fn toggle_is_ignored_when_companion_off() {
    let r = rig(true);
    r.rpc.push_lease(Ok(lease(false, "off", false, false)));
    r.shell.lease_tick().await;
    assert_eq!(
        r.shell.toggle_pause(Source::Hotkey).await,
        ToggleResult::Ignored
    );
    assert!(!r
        .log
        .lock()
        .unwrap()
        .iter()
        .any(|e| e.starts_with("rpc:pause")));
}

#[tokio::test]
async fn stale_lease_does_not_repaint_over_a_pause() {
    // A lease that began before the pause (reporting "observing") must not
    // overwrite the paused indicator when it lands afterwards.
    struct SlowLease {
        inner: FakeRpc,
        release: Arc<Notify>,
        entered: Arc<Notify>,
    }
    #[async_trait]
    impl CompanionRpc for SlowLease {
        async fn lease(&self, req: &LeaseRequest) -> Result<LeaseResponse, RpcError> {
            self.entered.notify_one();
            self.release.notified().await;
            self.inner.lease(req).await
        }
        async fn pause(&self, s: Source) -> Result<StatusLite, RpcError> {
            self.inner.pause(s).await
        }
        async fn resume(&self, s: Source) -> Result<StatusLite, RpcError> {
            self.inner.resume(s).await
        }
        async fn ask(&self, s: Source) -> Result<ActionOutcome, RpcError> {
            self.inner.ask(s).await
        }
        async fn capture(&self, s: Source) -> Result<ActionOutcome, RpcError> {
            self.inner.capture(s).await
        }
    }
    let log: Log = Arc::default();
    let inner = FakeRpc::new(&log);
    inner.push_lease(Ok(lease(true, "observing", false, false)));
    let release = Arc::new(Notify::new());
    let entered = Arc::new(Notify::new());
    let rpc = Arc::new(SlowLease {
        inner,
        release: release.clone(),
        entered: entered.clone(),
    });
    let tray = FakeTray::new(true, &log);
    let shell = Shell::new(rpc, tray.clone(), Arc::new(FakeHost::default()), true);

    // establish observing (first lease released immediately)
    release.notify_one();
    shell.lease_tick().await;
    entered.notified().await; // drain the permit left by the first lease
    assert_eq!(shell.current_indicator(), IndicatorState::Observing);

    // start a lease that will report "observing", pause while it is in flight
    let s2 = shell.clone();
    let tick = tokio::spawn(async move { s2.lease_tick().await });
    entered.notified().await;
    shell.toggle_pause(Source::Hotkey).await;
    assert_eq!(shell.current_indicator(), IndicatorState::Paused);
    release.notify_one();
    tick.await.unwrap();
    assert_eq!(shell.current_indicator(), IndicatorState::Paused);
}

#[tokio::test]
async fn ask_and_capture_hotkeys_call_core_with_hotkey_source() {
    let r = rig(true);
    r.shell.on_hotkey(HotkeyAction::Ask).await;
    r.shell.on_hotkey(HotkeyAction::Capture).await;
    let log = r.log.lock().unwrap();
    assert!(log.contains(&"rpc:ask".to_string()));
    assert!(log.contains(&"rpc:capture".to_string()));
}
