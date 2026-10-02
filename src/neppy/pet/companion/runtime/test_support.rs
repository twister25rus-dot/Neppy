//! Fakes for the companion runtime tests: a scriptable sensor with a call log,
//! a recording generator, a controllable hand-off runner and a clock.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tempfile::TempDir;

use super::generate::{GenProvider, Generator, ProviderCaps};
use super::handoff::{HandoffRunner, StartedHandoff};
use super::sensor::*;
use super::state::{Clock, Runtime, Timing};
use crate::neppy::config::Config;
use crate::neppy::desktop::accessibility::{PermissionState, SensorError};

type Hook = Box<dyn Fn() + Send + Sync>;

pub struct FakeSensor {
    pub calls: Mutex<Vec<String>>,
    pub app: Mutex<AppIdentity>,
    pub title: Mutex<Option<String>>,
    pub selection: Mutex<Option<String>>,
    pub secure_field: AtomicBool,
    pub clip_count: Mutex<i64>,
    pub clip_text: Mutex<Option<String>>,
    pub clip_sensitive: AtomicBool,
    pub access_behavior: Mutex<Option<String>>,
    pub screen_perm: Mutex<PermissionState>,
    pub screen_text: Mutex<Option<String>>,
    pub region: Mutex<Option<String>>,
    pub supported: AtomicBool,
    pub app_error: Mutex<Option<SensorError>>,
    pub screen_requests: AtomicU32,
    pub resets: AtomicU32,
    /// Runs inside `frontmost_app` (e.g. to pause mid-call).
    pub on_app: Mutex<Option<Hook>>,
}

impl Default for FakeSensor {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            app: Mutex::new(app("Terminal", "com.apple.Terminal", 100)),
            title: Mutex::new(Some("zsh: ~/project".into())),
            selection: Mutex::new(None),
            secure_field: AtomicBool::new(false),
            clip_count: Mutex::new(1),
            clip_text: Mutex::new(None),
            clip_sensitive: AtomicBool::new(false),
            access_behavior: Mutex::new(Some("always_allow".into())),
            screen_perm: Mutex::new(PermissionState::Granted),
            screen_text: Mutex::new(None),
            region: Mutex::new(None),
            supported: AtomicBool::new(true),
            app_error: Mutex::new(None),
            screen_requests: AtomicU32::new(0),
            resets: AtomicU32::new(0),
            on_app: Mutex::new(None),
        }
    }
}

pub fn app(name: &str, bundle: &str, pid: i32) -> AppIdentity {
    AppIdentity {
        app_name: name.into(),
        bundle_id: Some(bundle.into()),
        pid,
        idle_secs: 1.0,
        is_secure_field: false,
    }
}

impl FakeSensor {
    fn log(&self, s: impl Into<String>) {
        self.calls.lock().unwrap().push(s.into());
    }
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    pub fn count(&self, prefix: &str) -> usize {
        self.calls()
            .iter()
            .filter(|c| c.starts_with(prefix))
            .count()
    }
    pub fn set_app(&self, name: &str, bundle: &str, pid: i32) {
        *self.app.lock().unwrap() = app(name, bundle, pid);
    }
    pub fn copy(&self, text: &str) {
        *self.clip_count.lock().unwrap() += 1;
        *self.clip_text.lock().unwrap() = Some(text.into());
    }
}

impl Sensor for FakeSensor {
    fn platform_supported(&self) -> bool {
        self.supported.load(Ordering::SeqCst)
    }
    fn accessibility(&self) -> PermissionState {
        PermissionState::Granted
    }
    fn screen_recording(&self) -> PermissionState {
        self.screen_perm.lock().unwrap().clone()
    }
    fn request_screen_recording(&self) -> PermissionState {
        self.screen_requests.fetch_add(1, Ordering::SeqCst);
        self.screen_recording()
    }
    fn request_accessibility(&self) -> PermissionState {
        PermissionState::Granted
    }
    fn frontmost_app(&self) -> Result<AppIdentity, SensorError> {
        self.log("app");
        if let Some(h) = self.on_app.lock().unwrap().as_ref() {
            h();
        }
        if let Some(e) = self.app_error.lock().unwrap().clone() {
            return Err(e);
        }
        let mut a = self.app.lock().unwrap().clone();
        a.is_secure_field = self.secure_field.load(Ordering::SeqCst);
        Ok(a)
    }
    fn frontmost_context(
        &self,
        want_selection: bool,
        _max: usize,
    ) -> Result<FrontmostContext, SensorError> {
        self.log(if want_selection {
            "context+selection"
        } else {
            "context"
        });
        Ok(FrontmostContext {
            pid: self.app.lock().unwrap().pid,
            window_title: self.title.lock().unwrap().clone(),
            is_secure_field: self.secure_field.load(Ordering::SeqCst),
            selected_text: if want_selection {
                self.selection.lock().unwrap().clone()
            } else {
                None
            },
        })
    }
    fn clipboard_peek(&self) -> Result<ClipboardPeek, SensorError> {
        self.log("peek");
        Ok(ClipboardPeek {
            change_count: *self.clip_count.lock().unwrap(),
            access_behavior: self.access_behavior.lock().unwrap().clone(),
        })
    }
    fn clipboard_read(&self, _max: usize) -> Result<ClipboardRead, SensorError> {
        self.log("read");
        let sensitive = self.clip_sensitive.load(Ordering::SeqCst);
        Ok(ClipboardRead {
            change_count: *self.clip_count.lock().unwrap(),
            sensitive,
            text: if sensitive {
                None
            } else {
                self.clip_text.lock().unwrap().clone()
            },
        })
    }
    fn screen_sample(
        &self,
        _pid: Option<i32>,
        force: bool,
        _l: &[String],
    ) -> Result<ScreenSample, SensorError> {
        self.log(if force { "screen+force" } else { "screen" });
        Ok(match self.screen_text.lock().unwrap().take() {
            Some(text) => ScreenSample::Text { text, ocr_ms: 5 },
            None => ScreenSample::Unchanged,
        })
    }
    fn screen_reset(&self) {
        self.resets.fetch_add(1, Ordering::SeqCst);
    }
    fn capture_region(&self, _t: Duration, _l: &[String]) -> Result<RegionSample, SensorError> {
        self.log("region");
        Ok(match self.region.lock().unwrap().clone() {
            Some(text) => RegionSample::Text { text, ocr_ms: 7 },
            None => RegionSample::Cancelled,
        })
    }
}

pub struct FakeGenerator {
    pub caps: Mutex<ProviderCaps>,
    pub prompts: Mutex<Vec<String>>,
    pub providers: Mutex<Vec<GenProvider>>,
    pub reply: Mutex<String>,
    pub delay: Mutex<Duration>,
}

impl Default for FakeGenerator {
    fn default() -> Self {
        Self {
            caps: Mutex::new(ProviderCaps {
                chat_is_local: false,
                local_available: false,
            }),
            prompts: Mutex::new(Vec::new()),
            providers: Mutex::new(Vec::new()),
            reply: Mutex::new("The types differ: convert the value with .into().".into()),
            delay: Mutex::new(Duration::ZERO),
        }
    }
}

#[async_trait]
impl Generator for FakeGenerator {
    fn caps(&self, _config: &Config) -> ProviderCaps {
        *self.caps.lock().unwrap()
    }
    async fn run(
        &self,
        _c: &Config,
        provider: GenProvider,
        _job: &str,
        prompt: &str,
    ) -> Result<String, String> {
        self.prompts.lock().unwrap().push(prompt.to_string());
        self.providers.lock().unwrap().push(provider);
        let delay = *self.delay.lock().unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        Ok(self.reply.lock().unwrap().clone())
    }
}

#[derive(Default)]
pub struct FakeHandoff {
    pub prompts: Mutex<Vec<String>>,
    pub senders: Mutex<Vec<tokio::sync::oneshot::Sender<Result<String, String>>>>,
}

#[async_trait]
impl HandoffRunner for FakeHandoff {
    async fn start(
        &self,
        _c: Config,
        _job: &str,
        _title: &str,
        prompt: &str,
    ) -> Result<StartedHandoff, String> {
        self.prompts.lock().unwrap().push(prompt.to_string());
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.senders.lock().unwrap().push(tx);
        let join = tokio::spawn(std::future::pending::<()>());
        Ok(StartedHandoff {
            run_id: "pet-handoff-test".into(),
            thread_id: Some("thread-1".into()),
            result: rx,
            abort: join.abort_handle(),
        })
    }
}

pub struct FakeClock {
    base: Instant,
    utc: DateTime<Utc>,
    offset: Mutex<Duration>,
}

impl FakeClock {
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            utc: Utc::now(),
            offset: Mutex::new(Duration::ZERO),
        }
    }
    pub fn advance(&self, d: Duration) {
        *self.offset.lock().unwrap() += d;
    }
}

impl Clock for FakeClock {
    fn instant(&self) -> Instant {
        self.base + *self.offset.lock().unwrap()
    }
    fn utc(&self) -> DateTime<Utc> {
        self.utc + chrono::Duration::from_std(*self.offset.lock().unwrap()).unwrap()
    }
}

pub struct Harness {
    pub rt: Arc<Runtime>,
    pub sensor: Arc<FakeSensor>,
    pub generator: Arc<FakeGenerator>,
    pub handoff: Arc<FakeHandoff>,
    pub clock: Arc<FakeClock>,
    pub config: Config,
    pub _tmp: TempDir,
}

pub fn fast_timing() -> Timing {
    Timing {
        active: Duration::from_millis(10),
        idle: Duration::from_millis(10),
        parked: Duration::from_millis(10),
        capture_floor: Duration::from_secs(3),
    }
}

/// A runtime over fakes, bound to a fresh workspace with quiet hours off.
/// The companion is NOT enabled; call [`Harness::enable`].
pub fn harness() -> Harness {
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let now = Utc::now();
    let pet = crate::neppy::pet::store::ensure_primary(&config, now).unwrap();
    crate::neppy::pet::store::update_pet(
        &config,
        &pet.id,
        &crate::neppy::pet::store::PetUpdate {
            quiet_start: Some("00:00".into()),
            quiet_end: Some("00:00".into()),
            ..Default::default()
        },
        now,
    )
    .unwrap();
    let sensor = Arc::new(FakeSensor::default());
    let generator = Arc::new(FakeGenerator::default());
    let handoff = Arc::new(FakeHandoff::default());
    let clock = Arc::new(FakeClock::new());
    let rt = Runtime::new(
        sensor.clone(),
        generator.clone(),
        handoff.clone(),
        clock.clone(),
        fast_timing(),
    );
    rt.bind(&config).unwrap();
    Harness {
        rt,
        sensor,
        generator,
        handoff,
        clock,
        config,
        _tmp: tmp,
    }
}

impl Harness {
    pub fn patch(&self, v: serde_json::Value) -> serde_json::Value {
        let map = v.as_object().unwrap().clone();
        super::ops::update(&self.rt, &self.config, map).unwrap()
    }
    /// Enable the companion, but keep the sampler thread out of the way:
    /// tests drive `sample_once` themselves unless they opt into the thread.
    pub fn enable(&self) {
        self.patch(serde_json::json!({ "enabled": true }));
        self.rt.stop_sampler();
    }
    pub fn lease(&self) {
        self.rt.grant_lease(true);
    }
    pub fn sample(&self) {
        super::observer::sample_once(&self.rt);
    }
}
