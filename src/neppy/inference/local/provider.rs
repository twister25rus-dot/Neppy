//! Local AI provider selection helpers.

use crate::neppy::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalAiProvider {
    Ollama,
    LmStudio,
    /// Managed MLX runtime — `mlx_vlm.server` or `mlx_lm.server` supervised by
    /// `service::mlx_admin`. The former `omlx` slug maps here too: it was the
    /// same runtime behind a bearer token, which is now `auth = "bearer"` on an
    /// `[[mlx.server]]` block rather than a provider of its own.
    Mlx,
}

impl LocalAiProvider {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::LmStudio => "lm_studio",
            Self::Mlx => "mlx",
        }
    }

    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LmStudio => "LM Studio",
            Self::Mlx => "MLX",
        }
    }
}

pub(crate) fn normalize_provider(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "lmstudio" | "lm-studio" | "lm_studio" => LocalAiProvider::LmStudio.as_str().to_string(),
        // MLX and OMLX are keyed OpenAI-v1 local runtimes handled by the provider
        // factory (`mlx:<model>` / `omlx:<model>`), not by `LocalAiProvider`.
        // Preserve the slug so the saved config keeps its value instead of
        // collapsing to ollama.
        //
        // `mlx` was missing here: because the catch-all arm returns `ollama`,
        // `provider = "mlx"` silently became Ollama — the same class of quiet
        // substitution as the model allowlist. Apple-silicon MLX is the fastest
        // local runtime on this hardware, so it must be selectable by name.
        "mlx" | "mlx-server" => {
            log::trace!(
                "[local-provider] normalized provider '{}' -> mlx (factory-resolved local runtime)",
                value.trim()
            );
            "mlx".to_string()
        }
        "omlx" | "omlx-server" => {
            log::trace!(
                "[local-provider] normalized provider '{}' -> omlx (factory-resolved local runtime)",
                value.trim()
            );
            "omlx".to_string()
        }
        _ => LocalAiProvider::Ollama.as_str().to_string(),
    }
}

/// Map a `local_ai.provider` slug to the provider-string prefix the inference
/// factory dispatches on (`factory::{MLX,OMLX,LM_STUDIO,…}_PROVIDER_PREFIX`).
///
/// Callers compose `"{prefix}{model_id}"`. Kept here, beside
/// [`normalize_provider`], so the slug vocabulary has ONE definition: the
/// triage fallback previously hard-coded a two-way `local-openai:` / `ollama:`
/// choice, which silently routed `lm_studio`, `mlx` and `omlx` users to Ollama.
pub(crate) fn local_provider_prefix(slug: &str) -> &'static str {
    match slug.trim().to_ascii_lowercase().as_str() {
        "lmstudio" | "lm-studio" | "lm_studio" => "lmstudio:",
        "mlx" | "mlx-server" => "mlx:",
        "omlx" | "omlx-server" => "omlx:",
        "llamacpp" | "llama-server" | "custom_openai" | "local-openai" | "local_openai" => {
            "local-openai:"
        }
        _ => "ollama:",
    }
}

/// Resolve the runtime a config selects.
///
/// The arms are explicit and the fallback logs. This used to be
/// `_ => Ollama`, which meant `provider = "mlx"` booted Ollama in silence —
/// `normalize_provider` preserved the slug, and then this threw it away. A
/// typo should degrade loudly, not swap the user's runtime.
pub(crate) fn provider_from_config(config: &Config) -> LocalAiProvider {
    let normalized = normalize_provider(&config.local_ai.provider);
    match normalized.as_str() {
        "lm_studio" => LocalAiProvider::LmStudio,
        // `omlx` is MLX with a bearer token, not a separate runtime.
        "mlx" | "omlx" => LocalAiProvider::Mlx,
        "ollama" => LocalAiProvider::Ollama,
        other => {
            log::warn!(
                "[local-provider] unknown local_ai.provider `{other}`; falling back to ollama"
            );
            LocalAiProvider::Ollama
        }
    }
}

/// How a local runtime exposes its installed-model catalog.
///
/// Ollama serves a native `GET /api/tags` listing; every OpenAI-compatible
/// runtime (LM Studio, OMLX, `local-openai`, and any custom BYOK endpoint that
/// speaks the OpenAI `/v1` surface) serves `GET /v1/models`. Sending an Ollama
/// probe to an OpenAI-compatible server produces `GET /v1/api/tags`, which
/// LM Studio logs as `Unexpected endpoint or method` and answers with an empty
/// catalog — so model discovery silently fails and the model never appears as
/// selectable (GH #5053).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelDiscoveryApi {
    /// Ollama-native `/api/tags`.
    OllamaTags,
    /// OpenAI-compatible `/v1/models`.
    OpenAiModels,
}

/// True when `base_url`'s path is the OpenAI-compatible `/v1` root — the
/// canonical marker of an OpenAI-style server (LM Studio, OMLX, LiteLLM, …).
///
/// A genuine Ollama base URL is host-rooted (`http://localhost:11434`) with no
/// `/v1` path segment, so this cleanly separates the two API shapes by
/// **endpoint type**, not by "is it localhost" (which would misidentify a
/// localhost LM Studio server as Ollama — the #5053 conflation).
pub(crate) fn endpoint_is_openai_v1(base_url: &str) -> bool {
    if let Ok(url) = reqwest::Url::parse(base_url.trim()) {
        let path = url.path().trim_end_matches('/').to_ascii_lowercase();
        return path.ends_with("/v1");
    }
    // Fall back to a lexical check when the URL doesn't parse cleanly.
    base_url
        .trim()
        .trim_end_matches('/')
        .to_ascii_lowercase()
        .ends_with("/v1")
}

/// Select the model-discovery API for a local runtime from its provider slug
/// and base URL — by provider **type**, never by "is it localhost".
///
/// Genuine Ollama (`provider = "ollama"` on a host-rooted base) uses
/// `/api/tags`. Every OpenAI-compatible runtime uses `/v1/models`: an explicit
/// `lm_studio` / `omlx` slug, OR any endpoint whose path is the OpenAI `/v1`
/// root. The `/v1` endpoint clause is what rescues a custom BYOK localhost
/// endpoint (e.g. LM Studio on `http://localhost:1234/v1`) whose provider tag
/// still defaults to `ollama`: the endpoint type wins over the fallback slug
/// (GH #5053).
pub(crate) fn model_discovery_api(provider: &str, base_url: &str) -> ModelDiscoveryApi {
    if endpoint_is_openai_v1(base_url) {
        return ModelDiscoveryApi::OpenAiModels;
    }
    match normalize_provider(provider).as_str() {
        "ollama" => ModelDiscoveryApi::OllamaTags,
        // lm_studio, omlx, and any other OpenAI-compatible local runtime.
        _ => ModelDiscoveryApi::OpenAiModels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_provider_accepts_lm_studio_aliases() {
        assert_eq!(normalize_provider("lmstudio"), "lm_studio");
        assert_eq!(normalize_provider("lm-studio"), "lm_studio");
        assert_eq!(normalize_provider("LM_Studio"), "lm_studio");
    }

    #[test]
    fn normalize_provider_falls_back_to_ollama() {
        assert_eq!(normalize_provider(""), "ollama");
        assert_eq!(normalize_provider("unknown"), "ollama");
    }

    #[test]
    fn normalize_provider_keeps_omlx() {
        assert_eq!(normalize_provider("omlx"), "omlx");
        // MLX must survive normalization. Before this it hit the catch-all and
        // became "ollama" — a silent runtime swap on the fastest local backend
        // available on Apple silicon.
        assert_eq!(normalize_provider("mlx"), "mlx");
        assert_eq!(normalize_provider("MLX"), "mlx");
        assert_eq!(normalize_provider("mlx-server"), "mlx");
    }

    #[test]
    fn local_provider_prefix_maps_every_runtime_not_just_ollama() {
        // The triage fallback used to pick between `local-openai:` and
        // `ollama:` only, so lm_studio / mlx / omlx were routed to Ollama —
        // wrong endpoint and wrong wire dialect.
        assert_eq!(local_provider_prefix("mlx"), "mlx:");
        assert_eq!(local_provider_prefix("mlx-server"), "mlx:");
        assert_eq!(local_provider_prefix("omlx"), "omlx:");
        assert_eq!(local_provider_prefix("lm_studio"), "lmstudio:");
        assert_eq!(local_provider_prefix("lmstudio"), "lmstudio:");
        assert_eq!(local_provider_prefix("llamacpp"), "local-openai:");
        assert_eq!(local_provider_prefix("custom_openai"), "local-openai:");
        assert_eq!(local_provider_prefix("ollama"), "ollama:");
        assert_eq!(local_provider_prefix("anything-else"), "ollama:");
        assert_eq!(normalize_provider("omlx-server"), "omlx");
        assert_eq!(normalize_provider("OMLX"), "omlx");
    }

    #[test]
    fn provider_from_config_resolves_mlx_instead_of_silently_booting_ollama() {
        let mut config = Config::default();

        config.local_ai.provider = "mlx".to_string();
        assert_eq!(provider_from_config(&config), LocalAiProvider::Mlx);

        // omlx collapses into the one MLX runtime — it was never a separate
        // server, only the same one behind a bearer token.
        config.local_ai.provider = "omlx".to_string();
        assert_eq!(provider_from_config(&config), LocalAiProvider::Mlx);

        config.local_ai.provider = "lm_studio".to_string();
        assert_eq!(provider_from_config(&config), LocalAiProvider::LmStudio);

        config.local_ai.provider = "ollama".to_string();
        assert_eq!(provider_from_config(&config), LocalAiProvider::Ollama);

        // An unknown value still falls back, but loudly (see the warn above).
        config.local_ai.provider = "nonsense".to_string();
        assert_eq!(provider_from_config(&config), LocalAiProvider::Ollama);
    }

    #[test]
    fn endpoint_is_openai_v1_detects_v1_root() {
        assert!(endpoint_is_openai_v1("http://localhost:1234/v1"));
        assert!(endpoint_is_openai_v1("http://localhost:1234/v1/"));
        assert!(endpoint_is_openai_v1("https://box.local:1234/openai/v1"));
        // Genuine Ollama base is host-rooted with no /v1 path.
        assert!(!endpoint_is_openai_v1("http://localhost:11434"));
        assert!(!endpoint_is_openai_v1("http://localhost:11434/"));
        // A `/v1` embedded mid-path is not the OpenAI root.
        assert!(!endpoint_is_openai_v1("http://localhost:11434/v1/models"));
    }

    #[test]
    fn model_discovery_api_uses_tags_for_genuine_ollama() {
        // Ollama slug on its host-rooted native base -> /api/tags.
        assert_eq!(
            model_discovery_api("ollama", "http://localhost:11434"),
            ModelDiscoveryApi::OllamaTags
        );
        assert_eq!(
            model_discovery_api("", "http://localhost:11434"),
            ModelDiscoveryApi::OllamaTags
        );
    }

    #[test]
    fn model_discovery_api_uses_v1_models_for_openai_compatible() {
        // Explicit LM Studio / OMLX slugs are OpenAI-compatible.
        assert_eq!(
            model_discovery_api("lm_studio", "http://localhost:1234/v1"),
            ModelDiscoveryApi::OpenAiModels
        );
        assert_eq!(
            model_discovery_api("omlx", "http://localhost:8080/v1"),
            ModelDiscoveryApi::OpenAiModels
        );
        // The #5053 case: a custom BYOK OpenAI-compatible endpoint on localhost
        // whose provider tag still defaults to `ollama` must NOT be probed with
        // /api/tags — the `/v1` endpoint type wins.
        assert_eq!(
            model_discovery_api("ollama", "http://localhost:1234/v1"),
            ModelDiscoveryApi::OpenAiModels
        );
        assert_eq!(
            model_discovery_api("custom-byok", "http://localhost:1234/v1"),
            ModelDiscoveryApi::OpenAiModels
        );
    }
}
