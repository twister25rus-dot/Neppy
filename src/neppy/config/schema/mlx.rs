//! MLX managed-runtime configuration.
//!
//! Neppy supervises MLX servers as child processes rather than expecting the
//! user to start one by hand. Each `[[mlx.server]]` block describes one
//! process; the supervisor in `inference::local::service::mlx_admin` reconciles
//! running processes against these blocks.
//!
//! Two binaries are supported, selected per block by `kind`:
//!
//! - `vlm` (default) — `mlx_vlm.server`. The unified runtime: one process
//!   serves chat, reasoning, vision, embeddings, rerank, STT and TTS from
//!   separate model slots, and exposes live admin routes (`/health`,
//!   `/metrics`, `/settings`, `/unload`).
//! - `lm` — `mlx_lm.server`. Text-only and lighter, for when the vision stack
//!   is not wanted.
//!
//! This section is Neppy-owned. It deliberately does **not** extend
//! `LocalAiConfig`, which lives in the vendored `tinymemory` submodule and
//! whose provider set is fixed upstream at `ollama | lm_studio | omlx`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Server binary family. Determines which flags are meaningful for a block.
pub const KIND_VLM: &str = "vlm";
pub const KIND_LM: &str = "lm";

/// Embedding backends selectable while MLX is the active runtime.
pub const EMBEDDINGS_BACKEND_OLLAMA: &str = "ollama";
pub const EMBEDDINGS_BACKEND_MLX: &str = "mlx";

/// Loopback address. `mlx_vlm.server` defaults to `0.0.0.0`, which would
/// publish a local model to the LAN; every block binds here unless the user
/// explicitly opts into `allow_lan`.
pub const LOOPBACK_HOST: &str = "127.0.0.1";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MlxConfig {
    /// Master switch for the MLX supervisor. When `false`, Neppy never spawns
    /// an MLX process; a manually started server can still be reached through
    /// the `mlx:` provider prefix.
    #[serde(default)]
    pub enabled: bool,

    /// Directory holding `mlx_vlm.server` / `mlx_lm.server`. Empty string means
    /// "search `PATH`, then the usual uv tool location".
    #[serde(default)]
    pub bin_dir: String,

    /// Ceiling, in GiB, for the total resident size of all MLX processes.
    /// `0.0` means "derive it from physical memory" (see `memory.rs`).
    #[serde(default)]
    pub memory_budget_gib: f64,

    /// Which backend serves embeddings while MLX is the active runtime.
    ///
    /// `ollama` (default) keeps embeddings on the existing bge-m3 path, so
    /// vectors already in the memory tree stay valid — that tree is fixed at
    /// 1024 dimensions. `mlx` serves them in-process from the block's
    /// `embedding_model`, which requires a 1024-dim model to avoid a full
    /// re-index.
    #[serde(default = "default_embeddings_backend")]
    pub embeddings_backend: String,

    /// Configured servers. Defaults to a single `primary` block.
    #[serde(default = "default_servers", rename = "server")]
    pub servers: Vec<MlxServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MlxServerConfig {
    /// Stable identifier, used by the RPC surface to address this process and
    /// to key its spawn marker. Must be unique within the section.
    #[serde(default = "default_id")]
    pub id: String,

    /// `vlm` (default) or `lm`. See the module docs.
    #[serde(default = "default_kind")]
    pub kind: String,

    /// Bind address. Forced to loopback unless `allow_lan` is set.
    #[serde(default = "default_host")]
    pub host: String,

    /// Opt-in to binding a non-loopback address. Off by default: the upstream
    /// `0.0.0.0` default would expose the model to the local network.
    #[serde(default)]
    pub allow_lan: bool,

    /// Listen port. `0` means "assign a free port at start".
    #[serde(default)]
    pub port: u16,

    /// `none` (default) or `bearer`. `bearer` is what the former `omlx`
    /// provider modelled: the same server behind an API key.
    #[serde(default = "default_auth")]
    pub auth: String,

    /// Bearer token, used only when `auth = "bearer"`. Never logged.
    #[serde(default)]
    pub api_key: String,

    // ── Model slots ──────────────────────────────────────────────────────
    //
    // `vlm` serves every slot from one process. `lm` supports `model` only.
    /// Primary language or vision-language model. Empty means "load nothing at
    /// startup"; with `model_discovery = "hf-cache"` the server still lists and
    /// can serve cached models on demand.
    #[serde(default)]
    pub model: String,
    /// Embedding model slot (`vlm` only). Serves `/v1/embeddings`.
    #[serde(default)]
    pub embedding_model: String,
    /// Reranker slot (`vlm` only). Serves `/v1/rerank`.
    #[serde(default)]
    pub reranker_model: String,
    /// Speech-to-text slot (`vlm` only).
    #[serde(default)]
    pub stt_model: String,
    /// Text-to-speech slot (`vlm` only).
    #[serde(default)]
    pub tts_model: String,
    /// Image-generation slot (`vlm` only).
    #[serde(default)]
    pub image_model: String,

    /// LoRA / adapter weights to load alongside `model`.
    #[serde(default)]
    pub adapter_path: String,

    /// `served` lists only loaded models on `/v1/models`; `hf-cache` also scans
    /// the shared Hugging Face cache (`vlm` only).
    #[serde(default = "default_model_discovery")]
    pub model_discovery: String,

    // ── Generation defaults ──────────────────────────────────────────────
    /// Default generation cap. `0` leaves the server default.
    #[serde(default)]
    pub max_tokens: u32,
    /// Default sampling temperature (`lm` only). Negative means "unset".
    #[serde(default = "default_unset_f32")]
    pub temp: f32,
    /// Nucleus sampling (`lm` only). Negative means "unset".
    #[serde(default = "default_unset_f32")]
    pub top_p: f32,
    /// Top-k sampling (`lm` only). Negative means "unset".
    #[serde(default = "default_unset_i32")]
    pub top_k: i32,
    /// Min-p sampling (`lm` only). Negative means "unset".
    #[serde(default = "default_unset_f32")]
    pub min_p: f32,

    // ── Reasoning ────────────────────────────────────────────────────────
    //
    // What the "reasoning" pseudo-provider actually is: flags on this process.
    /// Enable thinking mode by default for requests that do not set it
    /// explicitly (`vlm` only).
    #[serde(default)]
    pub enable_thinking: bool,
    /// Token ceiling inside a thinking block. `0` leaves it unbounded.
    #[serde(default)]
    pub thinking_budget: u32,
    /// Token opening a thinking block. Empty uses the model's own.
    #[serde(default)]
    pub thinking_start_token: String,
    /// Token closing a thinking block. Empty uses the model's own.
    #[serde(default)]
    pub thinking_end_token: String,

    // ── Memory and throughput ────────────────────────────────────────────
    /// KV-cache quantization bit width (`vlm` only); `3.5` selects TurboQuant.
    /// `0.0` leaves the cache unquantized.
    #[serde(default)]
    pub kv_bits: f32,
    /// KV quantization backend: `uniform`, `turboquant`, or empty for default.
    #[serde(default)]
    pub kv_quant_scheme: String,
    /// Group size for uniform KV quantization. `0` uses the server default.
    #[serde(default)]
    pub kv_group_size: u32,
    /// Maximum KV cache size in tokens. `0` is unbounded.
    #[serde(default)]
    pub max_kv_size: u32,
    /// Token index at which KV quantization begins. `0` uses the default.
    #[serde(default)]
    pub quantized_kv_start: u32,
    /// Concurrently decoded sequences; requests beyond this queue, bounding
    /// peak memory (`vlm` only). `0` is unbounded.
    #[serde(default)]
    pub max_num_seqs: u32,
    /// Tokens per prefill step. `0` uses the server default.
    #[serde(default)]
    pub prefill_step_size: u32,
    /// Resident routed-expert budget in GB for MoE-offload checkpoints
    /// (`vlm` only). `0.0` uses the server default.
    #[serde(default)]
    pub expert_cache_gb: f32,
    /// Cached vision features (`vlm` only). `0` uses the server default.
    #[serde(default)]
    pub vision_cache_size: u32,
    /// Batched decode width (`lm` only). `0` uses the server default.
    #[serde(default)]
    pub decode_concurrency: u32,
    /// Batched prefill width (`lm` only). `0` uses the server default.
    #[serde(default)]
    pub prompt_concurrency: u32,
    /// Prompt cache entry cap (`lm` only). `0` uses the server default.
    #[serde(default)]
    pub prompt_cache_size: u32,
    /// Prompt cache byte cap (`lm` only). `0` uses the server default.
    #[serde(default)]
    pub prompt_cache_bytes: u64,

    // ── Speculative decoding ─────────────────────────────────────────────
    /// Drafter model path or Hugging Face id.
    #[serde(default)]
    pub draft_model: String,
    /// Drafter family: `dflash`, `eagle3` or `mtp` (`vlm` only). Empty
    /// auto-detects from the drafter's `model_type`.
    #[serde(default)]
    pub draft_kind: String,
    /// Override the drafter's block size (`vlm` only). `0` uses its own.
    #[serde(default)]
    pub draft_block_size: u32,
    /// Tokens to draft per step (`lm` only). `0` uses the server default.
    #[serde(default)]
    pub num_draft_tokens: u32,

    // ── Templates ────────────────────────────────────────────────────────
    /// Explicit chat template (`lm` only). Empty uses the tokenizer's.
    #[serde(default)]
    pub chat_template: String,
    /// JSON arguments passed to `apply_chat_template` (`lm` only), e.g.
    /// `{"enable_thinking": false}`.
    #[serde(default)]
    pub chat_template_args: String,
    /// Force the tokenizer's default chat template (`lm` only).
    #[serde(default)]
    pub use_default_chat_template: bool,

    // ── Misc ─────────────────────────────────────────────────────────────
    /// CORS origins (`lm` only). Empty means "Neppy's origin only"; the
    /// upstream default is `*`, which we never pass through.
    #[serde(default)]
    pub allowed_origins: String,
    /// Enable pipeline parallelism (`lm` only).
    #[serde(default)]
    pub pipeline: bool,
    /// Execute model-supplied code when loading from the Hub. Dangerous:
    /// off by default and gated behind an explicit confirmation in the UI.
    #[serde(default)]
    pub trust_remote_code: bool,
    /// Server log level: `DEBUG`, `INFO`, `WARNING`, `ERROR`, `CRITICAL`.
    #[serde(default = "default_log_level")]
    pub log_level: String,
    /// Start this block automatically when the runtime boots.
    #[serde(default = "default_autostart")]
    pub autostart: bool,
}

fn default_embeddings_backend() -> String {
    EMBEDDINGS_BACKEND_OLLAMA.to_string()
}

fn default_servers() -> Vec<MlxServerConfig> {
    vec![MlxServerConfig::default()]
}

fn default_id() -> String {
    "primary".to_string()
}

fn default_kind() -> String {
    KIND_VLM.to_string()
}

fn default_host() -> String {
    LOOPBACK_HOST.to_string()
}

fn default_auth() -> String {
    "none".to_string()
}

fn default_model_discovery() -> String {
    "hf-cache".to_string()
}

fn default_log_level() -> String {
    "INFO".to_string()
}

fn default_autostart() -> bool {
    true
}

/// Sentinel for "leave the server's own default in place" on float flags.
fn default_unset_f32() -> f32 {
    -1.0
}

/// Sentinel for "leave the server's own default in place" on signed int flags.
fn default_unset_i32() -> i32 {
    -1
}

impl Default for MlxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bin_dir: String::new(),
            memory_budget_gib: 0.0,
            embeddings_backend: default_embeddings_backend(),
            servers: default_servers(),
        }
    }
}

impl Default for MlxServerConfig {
    fn default() -> Self {
        Self {
            id: default_id(),
            kind: default_kind(),
            host: default_host(),
            allow_lan: false,
            port: 0,
            auth: default_auth(),
            api_key: String::new(),
            model: String::new(),
            embedding_model: String::new(),
            reranker_model: String::new(),
            stt_model: String::new(),
            tts_model: String::new(),
            image_model: String::new(),
            adapter_path: String::new(),
            model_discovery: default_model_discovery(),
            max_tokens: 0,
            temp: default_unset_f32(),
            top_p: default_unset_f32(),
            top_k: default_unset_i32(),
            min_p: default_unset_f32(),
            enable_thinking: false,
            thinking_budget: 0,
            thinking_start_token: String::new(),
            thinking_end_token: String::new(),
            kv_bits: 0.0,
            kv_quant_scheme: String::new(),
            kv_group_size: 0,
            max_kv_size: 0,
            quantized_kv_start: 0,
            max_num_seqs: 0,
            prefill_step_size: 0,
            expert_cache_gb: 0.0,
            vision_cache_size: 0,
            decode_concurrency: 0,
            prompt_concurrency: 0,
            prompt_cache_size: 0,
            prompt_cache_bytes: 0,
            draft_model: String::new(),
            draft_kind: String::new(),
            draft_block_size: 0,
            num_draft_tokens: 0,
            chat_template: String::new(),
            chat_template_args: String::new(),
            use_default_chat_template: false,
            allowed_origins: String::new(),
            pipeline: false,
            trust_remote_code: false,
            log_level: default_log_level(),
            autostart: default_autostart(),
        }
    }
}

impl MlxServerConfig {
    /// `true` when this block runs `mlx_vlm.server`.
    pub fn is_vlm(&self) -> bool {
        !self.kind.eq_ignore_ascii_case(KIND_LM)
    }

    /// Executable name for this block's `kind`.
    pub fn binary_name(&self) -> &'static str {
        if self.is_vlm() {
            "mlx_vlm.server"
        } else {
            "mlx_lm.server"
        }
    }

    /// Address to bind. Non-loopback hosts require an explicit `allow_lan`,
    /// so a stray config value cannot publish the model to the network.
    pub fn effective_host(&self) -> &str {
        let host = self.host.trim();
        if host.is_empty() {
            return LOOPBACK_HOST;
        }
        if self.allow_lan {
            host
        } else {
            LOOPBACK_HOST
        }
    }

    /// Base URL for a resolved port, in the shape the provider factory expects.
    pub fn base_url(&self, resolved_port: u16) -> String {
        format!("http://{}:{}/v1", self.effective_host(), resolved_port)
    }

    /// Whether requests to this server carry a bearer token.
    pub fn uses_bearer(&self) -> bool {
        self.auth.eq_ignore_ascii_case("bearer")
    }
}

impl MlxConfig {
    /// Look up a block by id.
    pub fn server(&self, id: &str) -> Option<&MlxServerConfig> {
        self.servers.iter().find(|s| s.id == id)
    }

    /// Whether embeddings should be served by MLX rather than Ollama.
    pub fn embeddings_on_mlx(&self) -> bool {
        self.embeddings_backend
            .eq_ignore_ascii_case(EMBEDDINGS_BACKEND_MLX)
    }

    /// Configuration problems worth surfacing before a spawn is attempted.
    /// Returns human-readable messages; an empty vector means "usable".
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut seen: Vec<&str> = Vec::new();

        for server in &self.servers {
            if server.id.trim().is_empty() {
                problems.push("an [[mlx.server]] block has an empty id".to_string());
            } else if seen.contains(&server.id.as_str()) {
                problems.push(format!("duplicate [[mlx.server]] id `{}`", server.id));
            } else {
                seen.push(server.id.as_str());
            }

            if !server.kind.eq_ignore_ascii_case(KIND_VLM)
                && !server.kind.eq_ignore_ascii_case(KIND_LM)
            {
                problems.push(format!(
                    "server `{}` has unknown kind `{}` (expected `vlm` or `lm`)",
                    server.id, server.kind
                ));
            }

            if server.uses_bearer() && server.api_key.trim().is_empty() {
                problems.push(format!(
                    "server `{}` sets auth = \"bearer\" but has no api_key",
                    server.id
                ));
            }

            if !server.is_vlm() {
                let vlm_only = [
                    ("embedding_model", !server.embedding_model.is_empty()),
                    ("reranker_model", !server.reranker_model.is_empty()),
                    ("stt_model", !server.stt_model.is_empty()),
                    ("tts_model", !server.tts_model.is_empty()),
                    ("image_model", !server.image_model.is_empty()),
                    ("enable_thinking", server.enable_thinking),
                ];
                for (field, set) in vlm_only {
                    if set {
                        problems.push(format!(
                            "server `{}` is kind `lm`, so `{field}` is ignored \
                             (mlx_lm.server has no such slot)",
                            server.id
                        ));
                    }
                }
            }
        }

        if self.embeddings_on_mlx()
            && !self
                .servers
                .iter()
                .any(|s| s.is_vlm() && !s.embedding_model.trim().is_empty())
        {
            problems.push(
                "embeddings_backend = \"mlx\" but no vlm server declares an embedding_model"
                    .to_string(),
            );
        }

        problems
    }
}
