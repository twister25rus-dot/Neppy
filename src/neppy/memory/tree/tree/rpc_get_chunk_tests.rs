//! `get_chunk_rpc` wire shape: the additive `body` / `content_path` fields the
//! Brain graph's node detail view reads.
//!
//! The driver behind `get_chunk` is a compiled module a unit test cannot load,
//! so these tests bind a double that answers `chunk_detail` with a fixed value.
//! What is proven here is the host's half: the detail's vault facts reach the
//! response, `chunk` is unchanged, and an absent fact is *absent on the wire*
//! (serde-skipped), which the frontend reads as "fall back to `chunk.content`".

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use serde_json::json;
use tempfile::TempDir;
use tinymemory_api::chunks::{Chunk, Metadata, SourceKind, SourceRef};
use tinymemory_api::null::NullMemoryProvider;

use super::rpc::{get_chunk_rpc, GetChunkRequest};
use crate::neppy::config::Config;
use crate::neppy::memory::api::capabilities::Capabilities;
use crate::neppy::memory::api::error::MemoryError;
use crate::neppy::memory::api::health::MemoryHealth;
use crate::neppy::memory::api::provider::types::{ExportPage, ExportRecord, ImportOutcome};
use crate::neppy::memory::api::provider::{
    ChunkDetail, ChunkEmbedding, ChunkQuery, MemoryChunks, MemoryCore, MemoryPortability,
    MemoryProvider, MemoryRecall, SourceScope,
};
use crate::neppy::memory::api::recall::OwnedRecallOpts;
use crate::neppy::memory::api::types::{
    MemoryCategory, MemoryEntry, MemoryTaint, NamespaceSummary,
};
use crate::neppy::memory::binding;

const CHUNK_ID: &str = "0123456789abcdef0123456789abcdef";
const PREVIEW: &str = "short stored preview";

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.config_path = tmp.path().join("config.toml");
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    (tmp, cfg)
}

fn sample_chunk() -> Chunk {
    let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    let mut metadata = Metadata::point_in_time(SourceKind::Document, "notion:plan", "alice", ts);
    metadata.source_ref = Some(SourceRef::new("notion://page/plan"));
    Chunk {
        id: CHUNK_ID.to_string(),
        content: PREVIEW.to_string(),
        metadata,
        token_count: 4,
        seq_in_source: 0,
        created_at: ts,
        partial_message: false,
    }
}

/// A driver whose chunk family answers `chunk_detail` with a fixed value.
struct DetailDriver {
    inner: NullMemoryProvider,
    detail: Option<ChunkDetail>,
}

impl DetailDriver {
    fn new(detail: Option<ChunkDetail>) -> Self {
        Self {
            inner: NullMemoryProvider::new(),
            detail,
        }
    }
}

#[async_trait]
impl MemoryChunks for DetailDriver {
    async fn list_chunks(
        &self,
        _query: &ChunkQuery,
        _scope: Option<&SourceScope>,
    ) -> Result<Vec<Chunk>, MemoryError> {
        unimplemented!("get_chunk does not list")
    }

    async fn get_chunk(&self, _chunk_id: &str) -> Result<Option<Chunk>, MemoryError> {
        // `get_chunk_rpc` must go through `chunk_detail`; answering here would
        // hide a regression back to the body-less read.
        unimplemented!("get_chunk_rpc must read through chunk_detail")
    }

    async fn chunk_detail(&self, chunk_id: &str) -> Result<Option<ChunkDetail>, MemoryError> {
        Ok(self
            .detail
            .clone()
            .filter(|detail| detail.chunk.id == chunk_id))
    }

    async fn storage_kinds(&self) -> Result<Vec<String>, MemoryError> {
        unimplemented!("get_chunk does not ask for storage kinds")
    }

    async fn chunk_embeddings(
        &self,
        _chunk_ids: &[String],
        _model_signature: &str,
    ) -> Result<Vec<ChunkEmbedding>, MemoryError> {
        unimplemented!("get_chunk does not read embeddings")
    }
}

#[async_trait]
impl MemoryCore for DetailDriver {
    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        taint: MemoryTaint,
    ) -> Result<(), MemoryError> {
        self.inner
            .store(namespace, key, content, category, session_id, taint)
            .await
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        self.inner.get(namespace, key).await
    }

    async fn forget(&self, namespace: &str, key: &str) -> Result<bool, MemoryError> {
        self.inner.forget(namespace, key).await
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        self.inner.list(namespace, category, session_id).await
    }

    async fn namespaces(&self) -> Result<Vec<NamespaceSummary>, MemoryError> {
        self.inner.namespaces().await
    }
}

#[async_trait]
impl MemoryRecall for DetailDriver {
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: &OwnedRecallOpts,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        self.inner.recall(query, limit, opts, scope).await
    }
}

#[async_trait]
impl MemoryPortability for DetailDriver {
    async fn export_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ExportPage, MemoryError> {
        self.inner.export_page(cursor, limit).await
    }

    async fn import_records(
        &self,
        records: Vec<ExportRecord>,
    ) -> Result<ImportOutcome, MemoryError> {
        self.inner.import_records(records).await
    }
}

#[async_trait]
impl MemoryProvider for DetailDriver {
    fn driver_id(&self) -> &str {
        "detail-double"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::all()
    }

    async fn health(&self) -> MemoryHealth {
        MemoryHealth::Ready
    }

    fn as_chunks(&self) -> Option<&dyn MemoryChunks> {
        Some(self)
    }
}

fn bind(cfg: &Config, detail: Option<ChunkDetail>) {
    binding::install_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Arc::new(DetailDriver::new(detail)) as Arc<dyn MemoryProvider>,
    );
}

fn detail(body: Option<&str>, content_path: Option<&str>) -> ChunkDetail {
    ChunkDetail {
        chunk: sample_chunk(),
        body: body.map(str::to_string),
        content_path: content_path.map(str::to_string),
        lifecycle_status: Some("admitted".into()),
        has_embedding: true,
    }
}

async fn fetch(cfg: &Config, id: &str) -> serde_json::Value {
    let outcome = get_chunk_rpc(cfg, GetChunkRequest { id: id.into() })
        .await
        .expect("get_chunk answers");
    serde_json::to_value(&outcome.value).expect("the response serialises")
}

/// The vault file exists: the full body and its path ride beside the chunk,
/// while `chunk.content` stays the stored preview (the existing contract).
#[tokio::test]
async fn get_chunk_returns_body_and_content_path_when_the_vault_file_exists() {
    let (_tmp, cfg) = test_config();
    let full = "the whole note, far longer than the stored preview";
    bind(
        &cfg,
        Some(detail(Some(full), Some("document/notion/plan/0.md"))),
    );

    let wire = fetch(&cfg, CHUNK_ID).await;

    assert_eq!(wire["body"], json!(full));
    assert_eq!(wire["content_path"], json!("document/notion/plan/0.md"));
    assert_eq!(wire["chunk"]["id"], json!(CHUNK_ID));
    assert_eq!(wire["chunk"]["content"], json!(PREVIEW));
    assert_eq!(wire["chunk"]["seq_in_source"], json!(0));
    assert_eq!(
        wire["chunk"]["metadata"]["source_ref"]["value"],
        json!("notion://page/plan")
    );
}

/// The vault read failed or the chunk has no vault body: both fields are
/// absent from the JSON (not `null`), and `chunk` is still returned intact.
#[tokio::test]
async fn get_chunk_omits_body_and_content_path_when_the_vault_file_is_missing() {
    let (_tmp, cfg) = test_config();
    bind(&cfg, Some(detail(None, None)));

    let wire = fetch(&cfg, CHUNK_ID).await;

    let object = wire.as_object().expect("an object response");
    assert!(!object.contains_key("body"), "body must be skipped: {wire}");
    assert!(
        !object.contains_key("content_path"),
        "content_path must be skipped: {wire}"
    );
    assert_eq!(wire["chunk"]["id"], json!(CHUNK_ID));
    assert_eq!(wire["chunk"]["content"], json!(PREVIEW));
}

/// An empty body is a legitimately empty note, distinct from a failed read.
#[tokio::test]
async fn get_chunk_keeps_an_empty_body_distinct_from_a_missing_one() {
    let (_tmp, cfg) = test_config();
    bind(
        &cfg,
        Some(detail(Some(""), Some("document/notion/plan/0.md"))),
    );

    let wire = fetch(&cfg, CHUNK_ID).await;

    assert_eq!(wire["body"], json!(""));
}

/// An id the store does not hold is `chunk: null` with no vault fields, the
/// answer this handler has always given.
#[tokio::test]
async fn get_chunk_for_an_unknown_id_reports_null_chunk_and_no_vault_fields() {
    let (_tmp, cfg) = test_config();
    bind(&cfg, Some(detail(Some("body"), Some("p.md"))));

    let wire = fetch(&cfg, "ffffffffffffffffffffffffffffffff").await;

    assert_eq!(wire["chunk"], json!(null));
    let object = wire.as_object().unwrap();
    assert!(!object.contains_key("body"));
    assert!(!object.contains_key("content_path"));
}

/// A driver with no chunk family degrades to the ordinary miss.
#[tokio::test]
async fn get_chunk_degrades_to_a_miss_when_the_driver_has_no_chunk_family() {
    let (_tmp, cfg) = test_config();
    binding::install_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Arc::new(NullMemoryProvider::new()) as Arc<dyn MemoryProvider>,
    );

    let wire = fetch(&cfg, CHUNK_ID).await;

    assert_eq!(wire["chunk"], json!(null));
    assert!(!wire.as_object().unwrap().contains_key("body"));
}

/// Old clients' payloads (no `body` / `content_path`) still deserialize.
#[test]
fn get_chunk_response_without_the_additive_fields_still_deserializes() {
    let parsed: super::rpc::GetChunkResponse =
        serde_json::from_value(json!({ "chunk": null })).expect("additive fields default");
    assert!(parsed.chunk.is_none());
    assert!(parsed.body.is_none());
    assert!(parsed.content_path.is_none());
}
