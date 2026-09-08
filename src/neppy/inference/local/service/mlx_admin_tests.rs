//! Tests for the managed MLX runtime.
//!
//! Argument construction is the surface most likely to break silently — both
//! MLX binaries abort on an unrecognised flag, so a flag leaking onto the
//! wrong `kind` takes the server down at spawn time with an opaque exit code.
//! These tests pin the flag partition, the "unset" sentinels, and the two
//! security narrowings (loopback binding, no wildcard CORS).

use super::argv::{build_argv, redact_argv};
use super::binary::resolve_binary;
use crate::neppy::config::schema::{MlxConfig, MlxServerConfig};
use crate::neppy::config::Config;

/// Read the value following `flag`, or `None` when the flag is absent.
fn value_of<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let idx = args.iter().position(|a| a == flag)?;
    args.get(idx + 1).map(String::as_str)
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn vlm_server() -> MlxServerConfig {
    MlxServerConfig::default()
}

fn lm_server() -> MlxServerConfig {
    MlxServerConfig {
        kind: "lm".to_string(),
        ..MlxServerConfig::default()
    }
}

#[test]
fn port_comes_from_the_resolved_value_not_the_config() {
    // Config port 0 means "assign one at start"; the supervisor resolves it
    // and passes it in. Emitting the literal 0 would bind a random port that
    // nothing could then reach.
    let server = MlxServerConfig {
        port: 0,
        ..vlm_server()
    };
    let args = build_argv(&server, 8123);
    assert_eq!(value_of(&args, "--port"), Some("8123"));
}

#[test]
fn binds_loopback_even_when_config_names_a_public_host() {
    // mlx_vlm.server defaults to 0.0.0.0. Without allow_lan, a stray host
    // value must not publish a local model to the network.
    let server = MlxServerConfig {
        host: "0.0.0.0".to_string(),
        allow_lan: false,
        ..vlm_server()
    };
    let args = build_argv(&server, 8080);
    assert_eq!(value_of(&args, "--host"), Some("127.0.0.1"));
}

#[test]
fn honours_an_explicit_lan_opt_in() {
    let server = MlxServerConfig {
        host: "0.0.0.0".to_string(),
        allow_lan: true,
        ..vlm_server()
    };
    let args = build_argv(&server, 8080);
    assert_eq!(value_of(&args, "--host"), Some("0.0.0.0"));
}

#[test]
fn vlm_slots_become_flags() {
    let server = MlxServerConfig {
        model: "mlx-community/Qwen3.8-27B-nvfp4".to_string(),
        embedding_model: "mlx-community/bge-m3".to_string(),
        reranker_model: "mlx-community/bge-reranker".to_string(),
        stt_model: "mlx-community/whisper".to_string(),
        tts_model: "mlx-community/kokoro".to_string(),
        image_model: "mlx-community/sdxl".to_string(),
        ..vlm_server()
    };
    let args = build_argv(&server, 8080);

    assert_eq!(
        value_of(&args, "--model"),
        Some("mlx-community/Qwen3.8-27B-nvfp4")
    );
    assert_eq!(
        value_of(&args, "--embedding-model"),
        Some("mlx-community/bge-m3")
    );
    assert_eq!(
        value_of(&args, "--reranker-model"),
        Some("mlx-community/bge-reranker")
    );
    assert_eq!(
        value_of(&args, "--stt-model"),
        Some("mlx-community/whisper")
    );
    assert_eq!(value_of(&args, "--tts-model"), Some("mlx-community/kokoro"));
    assert_eq!(value_of(&args, "--image-model"), Some("mlx-community/sdxl"));
}

#[test]
fn vlm_only_slots_are_dropped_for_an_lm_server() {
    // mlx_lm.server has no such flags and aborts on unrecognised arguments.
    let server = MlxServerConfig {
        embedding_model: "mlx-community/bge-m3".to_string(),
        stt_model: "mlx-community/whisper".to_string(),
        enable_thinking: true,
        kv_bits: 4.0,
        max_num_seqs: 8,
        ..lm_server()
    };
    let args = build_argv(&server, 8080);

    assert!(!has_flag(&args, "--embedding-model"));
    assert!(!has_flag(&args, "--stt-model"));
    assert!(!has_flag(&args, "--enable-thinking"));
    assert!(!has_flag(&args, "--kv-bits"));
    assert!(!has_flag(&args, "--max-num-seqs"));
}

#[test]
fn lm_only_flags_are_dropped_for_a_vlm_server() {
    let server = MlxServerConfig {
        temp: 0.7,
        top_k: 40,
        chat_template_args: r#"{"enable_thinking":false}"#.to_string(),
        prompt_cache_bytes: 2_147_483_648,
        pipeline: true,
        ..vlm_server()
    };
    let args = build_argv(&server, 8080);

    assert!(!has_flag(&args, "--temp"));
    assert!(!has_flag(&args, "--top-k"));
    assert!(!has_flag(&args, "--chat-template-args"));
    assert!(!has_flag(&args, "--prompt-cache-bytes"));
    assert!(!has_flag(&args, "--pipeline"));
}

#[test]
fn reasoning_is_flags_on_the_same_process() {
    // The former "reasoning provider" is this, and nothing more.
    let server = MlxServerConfig {
        enable_thinking: true,
        thinking_budget: 2048,
        thinking_start_token: "<think>".to_string(),
        thinking_end_token: "</think>".to_string(),
        ..vlm_server()
    };
    let args = build_argv(&server, 8080);

    assert!(has_flag(&args, "--enable-thinking"));
    assert_eq!(value_of(&args, "--thinking-budget"), Some("2048"));
    assert_eq!(value_of(&args, "--thinking-start-token"), Some("<think>"));
    assert_eq!(value_of(&args, "--thinking-end-token"), Some("</think>"));
}

#[test]
fn bearer_auth_is_the_former_omlx_provider() {
    let server = MlxServerConfig {
        auth: "bearer".to_string(),
        api_key: "secret-token".to_string(),
        ..vlm_server()
    };
    let args = build_argv(&server, 8080);
    assert_eq!(value_of(&args, "--api-key"), Some("secret-token"));
}

#[test]
fn bearer_auth_without_a_key_emits_no_flag() {
    // `--api-key ""` would start an unauthenticated server that the client
    // still sends a bearer header to. Better to omit it and let validate()
    // surface the misconfiguration.
    let server = MlxServerConfig {
        auth: "bearer".to_string(),
        api_key: "   ".to_string(),
        ..vlm_server()
    };
    let args = build_argv(&server, 8080);
    assert!(!has_flag(&args, "--api-key"));
}

#[test]
fn zero_means_leave_the_server_default_alone() {
    let server = vlm_server();
    let args = build_argv(&server, 8080);

    assert!(!has_flag(&args, "--max-tokens"));
    assert!(!has_flag(&args, "--kv-bits"));
    assert!(!has_flag(&args, "--max-kv-size"));
    assert!(!has_flag(&args, "--thinking-budget"));
}

#[test]
fn zero_temperature_is_sent_because_it_is_meaningful() {
    // Greedy decoding is a real setting, so sampling flags use a negative
    // sentinel for "unset" rather than treating 0.0 as absent.
    let explicit = MlxServerConfig {
        temp: 0.0,
        ..lm_server()
    };
    assert_eq!(value_of(&build_argv(&explicit, 8080), "--temp"), Some("0"));

    let unset = lm_server();
    assert!(!has_flag(&build_argv(&unset, 8080), "--temp"));
}

#[test]
fn lm_never_inherits_the_wildcard_cors_default() {
    // Upstream defaults --allowed-origins to `*`.
    let args = build_argv(&lm_server(), 8080);
    assert_eq!(
        value_of(&args, "--allowed-origins"),
        Some("http://localhost")
    );

    let configured = MlxServerConfig {
        allowed_origins: "tauri://localhost".to_string(),
        ..lm_server()
    };
    assert_eq!(
        value_of(&build_argv(&configured, 8080), "--allowed-origins"),
        Some("tauri://localhost")
    );
}

#[test]
fn trust_remote_code_is_off_unless_asked_for() {
    assert!(!has_flag(
        &build_argv(&vlm_server(), 8080),
        "--trust-remote-code"
    ));

    let trusting = MlxServerConfig {
        trust_remote_code: true,
        ..vlm_server()
    };
    assert!(has_flag(
        &build_argv(&trusting, 8080),
        "--trust-remote-code"
    ));
}

#[test]
fn kv_bits_renders_without_a_trailing_decimal() {
    let whole = MlxServerConfig {
        kv_bits: 4.0,
        ..vlm_server()
    };
    assert_eq!(value_of(&build_argv(&whole, 8080), "--kv-bits"), Some("4"));

    // 3.5 selects TurboQuant, so fractional values must survive intact.
    let fractional = MlxServerConfig {
        kv_bits: 3.5,
        ..vlm_server()
    };
    assert_eq!(
        value_of(&build_argv(&fractional, 8080), "--kv-bits"),
        Some("3.5")
    );
}

#[test]
fn redaction_hides_the_bearer_token() {
    let server = MlxServerConfig {
        auth: "bearer".to_string(),
        api_key: "secret-token".to_string(),
        ..vlm_server()
    };
    let redacted = redact_argv(&build_argv(&server, 8080));

    assert!(!redacted.iter().any(|a| a == "secret-token"));
    assert_eq!(value_of(&redacted, "--api-key"), Some("***"));
    // Everything else survives, or the redacted form is useless for support.
    assert_eq!(value_of(&redacted, "--port"), Some("8080"));
}

#[test]
fn missing_binary_names_the_install_command() {
    let mut config = Config::default();
    config.mlx.bin_dir = "/nonexistent/mlx/bin".to_string();

    // Point the env override at nothing so PATH and ~/.local/bin decide.
    let server = MlxServerConfig {
        kind: "vlm".to_string(),
        ..MlxServerConfig::default()
    };

    // Only assert the error shape when the binary genuinely is absent; on a
    // developer machine with mlx-vlm installed, resolution correctly succeeds.
    match resolve_binary(&config, &server) {
        Err(message) => {
            assert!(
                message.contains("uv tool install mlx-vlm"),
                "error should name the install command, got: {message}"
            );
            assert!(message.contains("mlx_vlm.server"));
        }
        Ok(resolved) => {
            assert!(resolved.path.ends_with("mlx_vlm.server"));
        }
    }
}

#[test]
fn validate_rejects_duplicate_ids() {
    let config = MlxConfig {
        servers: vec![vlm_server(), vlm_server()],
        ..MlxConfig::default()
    };
    let problems = config.validate();
    assert!(
        problems.iter().any(|p| p.contains("duplicate")),
        "expected a duplicate-id complaint, got: {problems:?}"
    );
}

#[test]
fn validate_flags_bearer_without_a_key() {
    let config = MlxConfig {
        servers: vec![MlxServerConfig {
            auth: "bearer".to_string(),
            ..vlm_server()
        }],
        ..MlxConfig::default()
    };
    assert!(config.validate().iter().any(|p| p.contains("api_key")));
}

#[test]
fn validate_flags_mlx_embeddings_with_no_embedding_model() {
    let config = MlxConfig {
        embeddings_backend: "mlx".to_string(),
        servers: vec![vlm_server()],
        ..MlxConfig::default()
    };
    assert!(config
        .validate()
        .iter()
        .any(|p| p.contains("embedding_model")));
}

#[test]
fn default_config_is_usable() {
    let config = MlxConfig::default();
    assert_eq!(config.validate(), Vec::<String>::new());
    assert_eq!(config.servers.len(), 1);
    assert!(config.servers[0].is_vlm());
    assert_eq!(config.servers[0].binary_name(), "mlx_vlm.server");
    // Ollama stays the embedding backend by default so existing 1024-dim
    // vectors in the memory tree remain valid.
    assert!(!config.embeddings_on_mlx());
}
