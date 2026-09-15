//! User-facing message contract for the direct-mode Composio set-key path,
//! and the lowercase anchor the observability classifier keys on. Kept in one
//! place so the message ([`COMPOSIO_INVALID_API_KEY_USER_MESSAGE`]) and the
//! substring the TAURI-RUST-K27 demotion arm matches on
//! ([`COMPOSIO_INVALID_API_KEY_ANCHOR`]) are a single, drift-guarded contract.

/// User-facing rejection returned by `composio_set_api_key`'s validate-before-store
/// probe (`ops/direct_mode.rs`) when the entered BYO key fails Composio's v3 auth
/// wall. This is the **single source of truth** for that string: the
/// `expected_error_kind` classifier (TAURI-RUST-K27 arm in `core::observability`)
/// keys on [`COMPOSIO_INVALID_API_KEY_ANCHOR`] within it to demote the RPC-boundary
/// `report_error` to `ProviderUserState`. Unlike the runtime
/// `[composio-direct] … HTTP 401: Invalid API key` wire body (TAURI-RUST-X9), this
/// prose carries no `[composio-direct]` prefix and — because the word `Composio`
/// splits `Invalid … api key` — does not match the X9 anchor, so it needs its own
/// arm. Reword the tail freely, but keep the anchor phrase or the drift-coupling
/// test `demotes_composio_set_key_invalid_key_rejection` fails CI.
///
/// The tail names the key TYPE because Composio issues two and only one works
/// here: `x-api-key` takes a project key, while an organization key belongs on
/// `x-org-api-key` and is accepted only by org-scoped routes. The probe reads
/// `/api/v3/connected_accounts`, which is project-scoped, so an org key comes
/// back as a plain 401 indistinguishable from a typo. "Re-enter a valid key"
/// gave someone holding the wrong KIND of key nothing to act on; the call site
/// additionally appends what Composio itself said, which carries a redacted
/// fingerprint of the key that was rejected.
pub(crate) const COMPOSIO_INVALID_API_KEY_USER_MESSAGE: &str =
    "Invalid Composio API key. Paste a PROJECT key (app.composio.dev > Settings > Project \
     Settings > API Keys) in Connections > Composio. An organization key will not work here: \
     this reads project-scoped data.";

/// Lowercase substring the observability classifier's TAURI-RUST-K27 arm matches
/// on to demote the set-key rejection. Shared with the runtime matcher so the
/// classifier keys off this named contract rather than a copied literal; the
/// `composio_set_key_anchor_is_substring_of_message` test asserts it stays a
/// lowercase substring of [`COMPOSIO_INVALID_API_KEY_USER_MESSAGE`].
pub(crate) const COMPOSIO_INVALID_API_KEY_ANCHOR: &str = "invalid composio api key";
