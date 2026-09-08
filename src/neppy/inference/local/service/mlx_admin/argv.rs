//! Command-line construction for a managed MLX server.
//!
//! Kept as a pure function over `MlxServerConfig` so every flag is unit
//! testable without spawning a process or loading model weights.
//!
//! The two binaries take overlapping but different flags. `mlx_vlm.server`
//! owns the multi-slot model surface, thinking controls, KV quantization and
//! `--api-key`; `mlx_lm.server` owns the sampling defaults, chat-template
//! overrides and prompt-cache sizing. A flag set on the wrong `kind` is
//! dropped here rather than passed through, because both binaries abort on an
//! unrecognised argument.

use crate::neppy::config::schema::MlxServerConfig;

/// Build the argument vector for `server`, bound to `resolved_port`.
///
/// `resolved_port` is passed in rather than read from the config because port
/// `0` means "assign a free one at start", and the caller does that assignment.
pub(crate) fn build_argv(server: &MlxServerConfig, resolved_port: u16) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();

    // Bind address. `effective_host` downgrades a non-loopback host unless
    // `allow_lan` is set — upstream's default is 0.0.0.0, which would publish
    // a local model to the network.
    push_str(&mut args, "--host", server.effective_host());
    args.push("--port".to_string());
    args.push(resolved_port.to_string());

    push_opt_str(&mut args, "--model", &server.model);
    push_opt_str(&mut args, "--adapter-path", &server.adapter_path);
    push_opt_str(&mut args, "--log-level", &server.log_level);
    push_u32(&mut args, "--max-tokens", server.max_tokens);
    push_u32(&mut args, "--prefill-step-size", server.prefill_step_size);
    push_opt_str(&mut args, "--draft-model", &server.draft_model);

    if server.trust_remote_code {
        args.push("--trust-remote-code".to_string());
    }

    if server.is_vlm() {
        push_vlm_args(&mut args, server);
    } else {
        push_lm_args(&mut args, server);
    }

    args
}

/// Flags understood only by `mlx_vlm.server`.
fn push_vlm_args(args: &mut Vec<String>, server: &MlxServerConfig) {
    // Model slots. This is what makes one process serve every role.
    push_opt_str(args, "--embedding-model", &server.embedding_model);
    push_opt_str(args, "--reranker-model", &server.reranker_model);
    push_opt_str(args, "--stt-model", &server.stt_model);
    push_opt_str(args, "--tts-model", &server.tts_model);
    push_opt_str(args, "--image-model", &server.image_model);
    push_opt_str(args, "--model-discovery", &server.model_discovery);

    // Reasoning — the former "reasoning provider", as flags.
    if server.enable_thinking {
        args.push("--enable-thinking".to_string());
    }
    push_u32(args, "--thinking-budget", server.thinking_budget);
    push_opt_str(args, "--thinking-start-token", &server.thinking_start_token);
    push_opt_str(args, "--thinking-end-token", &server.thinking_end_token);

    // Memory and throughput.
    push_f32(args, "--kv-bits", server.kv_bits);
    push_opt_str(args, "--kv-quant-scheme", &server.kv_quant_scheme);
    push_u32(args, "--kv-group-size", server.kv_group_size);
    push_u32(args, "--max-kv-size", server.max_kv_size);
    push_u32(args, "--quantized-kv-start", server.quantized_kv_start);
    push_u32(args, "--max-num-seqs", server.max_num_seqs);
    push_f32(args, "--expert-cache-gb", server.expert_cache_gb);
    push_u32(args, "--vision-cache-size", server.vision_cache_size);

    // Speculative decoding.
    push_opt_str(args, "--draft-kind", &server.draft_kind);
    push_u32(args, "--draft-block-size", server.draft_block_size);

    // Auth — the former "omlx provider", as a flag. `mlx_lm.server` has no
    // equivalent, which is why bearer auth requires a vlm block.
    if server.uses_bearer() && !server.api_key.trim().is_empty() {
        push_str(args, "--api-key", server.api_key.trim());
    }
}

/// Flags understood only by `mlx_lm.server`.
fn push_lm_args(args: &mut Vec<String>, server: &MlxServerConfig) {
    push_f32_signed(args, "--temp", server.temp);
    push_f32_signed(args, "--top-p", server.top_p);
    push_i32_signed(args, "--top-k", server.top_k);
    push_f32_signed(args, "--min-p", server.min_p);

    push_u32(args, "--num-draft-tokens", server.num_draft_tokens);
    push_u32(args, "--decode-concurrency", server.decode_concurrency);
    push_u32(args, "--prompt-concurrency", server.prompt_concurrency);
    push_u32(args, "--prompt-cache-size", server.prompt_cache_size);
    push_u64(args, "--prompt-cache-bytes", server.prompt_cache_bytes);

    push_opt_str(args, "--chat-template", &server.chat_template);
    push_opt_str(args, "--chat-template-args", &server.chat_template_args);
    if server.use_default_chat_template {
        args.push("--use-default-chat-template".to_string());
    }

    // Upstream defaults `--allowed-origins` to `*`. We always narrow it: an
    // explicit value when configured, otherwise no cross-origin callers at all.
    let origins = server.allowed_origins.trim();
    push_str(
        args,
        "--allowed-origins",
        if origins.is_empty() {
            "http://localhost"
        } else {
            origins
        },
    );

    if server.pipeline {
        args.push("--pipeline".to_string());
    }
}

fn push_str(args: &mut Vec<String>, flag: &str, value: &str) {
    args.push(flag.to_string());
    args.push(value.to_string());
}

/// Push `flag value` only when `value` is non-blank.
fn push_opt_str(args: &mut Vec<String>, flag: &str, value: &str) {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        push_str(args, flag, trimmed);
    }
}

/// `0` means "leave the server's own default in place".
fn push_u32(args: &mut Vec<String>, flag: &str, value: u32) {
    if value > 0 {
        push_str(args, flag, &value.to_string());
    }
}

fn push_u64(args: &mut Vec<String>, flag: &str, value: u64) {
    if value > 0 {
        push_str(args, flag, &value.to_string());
    }
}

fn push_f32(args: &mut Vec<String>, flag: &str, value: f32) {
    if value > 0.0 {
        push_str(args, flag, &format_f32(value));
    }
}

/// Sampling flags where `0.0` is a meaningful value (a temperature of zero is
/// greedy decoding), so "unset" is signalled by a negative sentinel instead.
fn push_f32_signed(args: &mut Vec<String>, flag: &str, value: f32) {
    if value >= 0.0 {
        push_str(args, flag, &format_f32(value));
    }
}

fn push_i32_signed(args: &mut Vec<String>, flag: &str, value: i32) {
    if value >= 0 {
        push_str(args, flag, &value.to_string());
    }
}

/// Render without a trailing `.0`, so `--kv-bits 4` is not sent as `4.0`
/// (upstream parses both, but the logs and the UI read better).
fn format_f32(value: f32) -> String {
    if (value.fract()).abs() < f32::EPSILON {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Redact the bearer token so an argv can be logged or shown in the UI.
pub(crate) fn redact_argv(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut redact_next = false;
    for arg in args {
        if redact_next {
            out.push("***".to_string());
            redact_next = false;
            continue;
        }
        redact_next = arg == "--api-key";
        out.push(arg.clone());
    }
    out
}
