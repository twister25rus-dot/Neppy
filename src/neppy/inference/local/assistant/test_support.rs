//! Shared fixtures for the assistant's tests: a scripted model, a scratch
//! project and workspace, and a crash injector.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::neppy::config::schema::LocalAssistantConfig;
use crate::neppy::security::policy::AutonomyLevel;
use crate::neppy::security::SecurityPolicy;

use super::faults::{FaultPoint, Faults, NoFaults};
use super::model::{ModelFailure, ModelReply, StepModel};
use super::runner::RunEnv;
use super::store::StateStore;
use super::types::TaskSpec;

pub(crate) fn store() -> (tempfile::TempDir, StateStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    (dir, store)
}

pub(crate) fn spec(goal: &str) -> TaskSpec {
    TaskSpec {
        project_root: "/p".into(),
        goal: goal.into(),
        allow_edits: true,
        test_command: Some("true".into()),
        max_steps: None,
    }
}

/// Replies handed out in order; `calls` counts every model call made.
pub(crate) struct ScriptedModel {
    replies: Mutex<VecDeque<Result<ModelReply, ModelFailure>>>,
    pub(crate) calls: AtomicUsize,
    pub(crate) prompts: Mutex<Vec<String>>,
}

impl ScriptedModel {
    pub(crate) fn new(replies: Vec<Result<ModelReply, ModelFailure>>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into()),
            calls: AtomicUsize::new(0),
            prompts: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl StepModel for ScriptedModel {
    async fn complete(
        &self,
        _system: &str,
        user: &str,
        _max_tokens: u32,
    ) -> Result<ModelReply, ModelFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.prompts.lock().push(user.to_string());
        self.replies
            .lock()
            .pop_front()
            .unwrap_or_else(|| Err(ModelFailure::Other("script exhausted".into())))
    }
}

/// A model that never answers, for cancellation.
pub(crate) struct HangingModel {
    pub(crate) started: tokio::sync::Notify,
    pub(crate) dropped: Arc<AtomicBool>,
}

struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl StepModel for HangingModel {
    async fn complete(&self, _: &str, _: &str, _: u32) -> Result<ModelReply, ModelFailure> {
        let _flag = DropFlag(Arc::clone(&self.dropped));
        self.started.notify_one();
        std::future::pending::<()>().await;
        unreachable!()
    }
}

pub(crate) fn ok_reply(json: &str) -> Result<ModelReply, ModelFailure> {
    Ok(ModelReply {
        text: json.to_string(),
        prompt_tokens: Some(1000),
        completion_tokens: Some(100),
    })
}

pub(crate) fn plan_json(
    summary: &str,
    edits: &[(&str, &str, &str)],
    run_tests: bool,
    next_step: &str,
    done: bool,
) -> String {
    serde_json::json!({
        "summary": summary,
        "decisions": [format!("decided in {summary}")],
        "edits": edits.iter().map(|(p, s, r)| serde_json::json!({"path": p, "search": s, "replace": r})).collect::<Vec<_>>(),
        "run_tests": run_tests,
        "next_step": next_step,
        "done": done,
        "search_queries": ["alpha"],
    })
    .to_string()
}

pub(crate) struct CrashOnce {
    point: FaultPoint,
    fired: AtomicBool,
}

impl CrashOnce {
    pub(crate) fn new(point: FaultPoint) -> Arc<Self> {
        Arc::new(Self {
            point,
            fired: AtomicBool::new(false),
        })
    }

    pub(crate) fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }
}

impl Faults for CrashOnce {
    fn crash_at(&self, point: FaultPoint) -> bool {
        point == self.point && !self.fired.swap(true, Ordering::SeqCst)
    }
}

/// A scratch project, workspace and state store, plus a counter file the test
/// command appends to so its invocations can be counted.
pub(crate) struct Fixture {
    pub(crate) project: tempfile::TempDir,
    pub(crate) workspace: tempfile::TempDir,
    pub(crate) counter_dir: tempfile::TempDir,
    pub(crate) store: Arc<StateStore>,
    pub(crate) cfg: LocalAssistantConfig,
    pub(crate) autonomy: AutonomyLevel,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join("src")).unwrap();
        std::fs::write(project.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let store = Arc::new(StateStore::open(workspace.path()).unwrap());
        Self {
            project,
            workspace,
            counter_dir: tempfile::tempdir().unwrap(),
            store,
            cfg: LocalAssistantConfig::default(),
            autonomy: AutonomyLevel::Full,
        }
    }

    pub(crate) fn counter_path(&self) -> std::path::PathBuf {
        self.counter_dir.path().join("count.txt")
    }

    /// How many times the test command has run.
    pub(crate) fn test_runs(&self) -> usize {
        std::fs::read_to_string(self.counter_path())
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    pub(crate) fn test_command(&self) -> String {
        format!("echo run >> {}", self.counter_path().display())
    }

    pub(crate) fn lib(&self) -> String {
        std::fs::read_to_string(self.project.path().join("src/lib.rs")).unwrap()
    }

    pub(crate) fn new_task(&self, allow_edits: bool, with_tests: bool) -> String {
        let root = self.project.path().canonicalize().unwrap();
        let spec = TaskSpec {
            project_root: root.clone(),
            goal: "Improve alpha".into(),
            allow_edits,
            test_command: with_tests.then(|| self.test_command()),
            max_steps: None,
        };
        self.store
            .create_task(&spec, &root.to_string_lossy(), self.cfg.max_steps)
            .unwrap()
            .id
    }

    pub(crate) fn env(&self, model: Arc<dyn StepModel>, faults: Arc<dyn Faults>) -> RunEnv {
        RunEnv {
            store: Arc::clone(&self.store),
            workspace: self.workspace.path().to_path_buf(),
            cfg: self.cfg.clone(),
            policy: Arc::new(SecurityPolicy {
                autonomy: self.autonomy,
                workspace_dir: self.workspace.path().to_path_buf(),
                ..SecurityPolicy::default()
            }),
            model,
            fallback: None,
            faults,
            metrics: None,
            retry_delay: Duration::ZERO,
        }
    }

    pub(crate) fn plain_env(&self, model: Arc<dyn StepModel>) -> RunEnv {
        self.env(model, Arc::new(NoFaults))
    }
}
