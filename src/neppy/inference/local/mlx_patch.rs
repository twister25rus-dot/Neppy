//! Partial updates to an `[[mlx.server]]` block.
//!
//! Every field of the block is editable from the UI, so the RPC takes a patch
//! rather than a fixed set of setters: one `mlx.update_server` instead of a
//! `set_*` per parameter, and adding a flag upstream means adding one field
//! here instead of a new RPC method.
//!
//! Absent means "leave alone", which is what makes the panel able to save one
//! field without resending the other forty and racing whatever else changed.

use serde::Deserialize;

use crate::neppy::config::schema::MlxServerConfig;

/// Which fields, if any, changed. Returned so the caller can skip a config
/// save and a restart when a form submits values identical to what is stored.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct PatchOutcome {
    pub(crate) changed: Vec<&'static str>,
    /// Whether any changed field is baked into the command line, and so needs
    /// a restart to take effect. The model slots and every tuning flag are;
    /// `autostart` is not.
    pub(crate) needs_restart: bool,
}

impl PatchOutcome {
    pub(crate) fn is_empty(&self) -> bool {
        self.changed.is_empty()
    }
}

/// A partial `[[mlx.server]]`. Every field is optional; `None` leaves the
/// stored value untouched.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ServerPatch {
    // Model slots. `mlx_vlm.server` serves all six from one process, which is
    // why several models can be selected at once rather than one.
    pub model: Option<String>,
    pub embedding_model: Option<String>,
    pub reranker_model: Option<String>,
    pub stt_model: Option<String>,
    pub tts_model: Option<String>,
    pub image_model: Option<String>,
    pub adapter_path: Option<String>,
    pub model_discovery: Option<String>,

    // Networking and auth.
    pub host: Option<String>,
    pub allow_lan: Option<bool>,
    pub port: Option<u16>,
    pub auth: Option<String>,
    pub api_key: Option<String>,

    // Generation.
    pub max_tokens: Option<u32>,
    pub temp: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<i32>,
    pub min_p: Option<f32>,

    // Reasoning.
    pub enable_thinking: Option<bool>,
    pub thinking_budget: Option<u32>,
    pub thinking_start_token: Option<String>,
    pub thinking_end_token: Option<String>,

    // Memory and throughput.
    pub kv_bits: Option<f32>,
    pub kv_quant_scheme: Option<String>,
    pub kv_group_size: Option<u32>,
    pub max_kv_size: Option<u32>,
    pub quantized_kv_start: Option<u32>,
    pub max_num_seqs: Option<u32>,
    pub prefill_step_size: Option<u32>,
    pub expert_cache_gb: Option<f32>,
    pub vision_cache_size: Option<u32>,
    pub decode_concurrency: Option<u32>,
    pub prompt_concurrency: Option<u32>,
    pub prompt_cache_size: Option<u32>,
    pub prompt_cache_bytes: Option<u64>,

    // Speculative decoding.
    pub draft_model: Option<String>,
    pub draft_kind: Option<String>,
    pub draft_block_size: Option<u32>,
    pub num_draft_tokens: Option<u32>,

    // Templates.
    pub chat_template: Option<String>,
    pub chat_template_args: Option<String>,
    pub use_default_chat_template: Option<bool>,

    // Misc.
    pub allowed_origins: Option<String>,
    pub pipeline: Option<bool>,
    pub trust_remote_code: Option<bool>,
    pub log_level: Option<String>,
    /// Not part of the command line, so changing it needs no restart.
    pub autostart: Option<bool>,
}

/// Apply `patch` to `server`, reporting what actually changed.
pub(crate) fn apply(server: &mut MlxServerConfig, patch: ServerPatch) -> PatchOutcome {
    let mut changed: Vec<&'static str> = Vec::new();

    // Each arm is the same shape: assign when the incoming value differs, and
    // record the field name. A macro keeps forty near-identical blocks from
    // becoming forty opportunities to paste the wrong field name.
    macro_rules! set {
        ($field:ident) => {
            if let Some(value) = patch.$field {
                if server.$field != value {
                    server.$field = value;
                    changed.push(stringify!($field));
                }
            }
        };
    }

    set!(model);
    set!(embedding_model);
    set!(reranker_model);
    set!(stt_model);
    set!(tts_model);
    set!(image_model);
    set!(adapter_path);
    set!(model_discovery);

    set!(host);
    set!(allow_lan);
    set!(port);
    set!(auth);
    set!(api_key);

    set!(max_tokens);
    set!(temp);
    set!(top_p);
    set!(top_k);
    set!(min_p);

    set!(enable_thinking);
    set!(thinking_budget);
    set!(thinking_start_token);
    set!(thinking_end_token);

    set!(kv_bits);
    set!(kv_quant_scheme);
    set!(kv_group_size);
    set!(max_kv_size);
    set!(quantized_kv_start);
    set!(max_num_seqs);
    set!(prefill_step_size);
    set!(expert_cache_gb);
    set!(vision_cache_size);
    set!(decode_concurrency);
    set!(prompt_concurrency);
    set!(prompt_cache_size);
    set!(prompt_cache_bytes);

    set!(draft_model);
    set!(draft_kind);
    set!(draft_block_size);
    set!(num_draft_tokens);

    set!(chat_template);
    set!(chat_template_args);
    set!(use_default_chat_template);

    set!(allowed_origins);
    set!(pipeline);
    set!(trust_remote_code);
    set!(log_level);
    set!(autostart);

    // Everything except `autostart` is baked into the argv at spawn time.
    let needs_restart = changed.iter().any(|field| *field != "autostart");

    PatchOutcome {
        changed,
        needs_restart,
    }
}

#[cfg(test)]
#[path = "mlx_patch_tests.rs"]
mod tests;
