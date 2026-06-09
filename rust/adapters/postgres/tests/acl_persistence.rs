//! Document-level ACL **persistence** across the ingest→serve process boundary
//! (feature gap G3, Postgres backend), against a real pgvector container.
//!
//! The in-memory ACL side table is process-local: it cannot carry a document's
//! ACL from the ingestion process to the serving process. This test proves the
//! Postgres adapter persists the ACL in the `knowledge_vectors.acl` column and
//! enforces it **from the DB** at read — so a private-repo doc ingested by one
//! adapter instance is filtered for an unentitled requester when queried by a
//! **fresh** adapter instance (a different process, in production).
//!
//! TDD: before the `acl` column + the SQL ACL filter existed, `query_async`
//! returned every org-matching row regardless of the requester — the
//! `unentitled_query_from_fresh_adapter_does_not_leak` assertion below failed
//! (the private doc came back to a user with no entitlement).
//!
//! Skips (prints a notice, returns Ok) when Docker is unavailable, so CI without
//! a daemon stays green.

use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

use smooth_operator::access_control::{AccessContext, DocAcl};
use smooth_operator::adapter::StorageAdapter;
use smooth_operator_adapter_postgres::PostgresAdapter;
use smooth_operator_core::{Document, DocumentType};

/// The private-repo group ACL the ingestion pipeline stamps for a private repo.
const PRIVATE_GROUP: &str = "github:acme/secret";

/// Spin up a throwaway `pgvector/pgvector:pg16` container. `Ok(None)` if Docker
/// is unavailable (the caller then skips rather than fails).
async fn start_pgvector() -> anyhow::Result<Option<(ContainerAsync<GenericImage>, String)>> {
    let image = GenericImage::new("pgvector/pgvector", "pg16")
        .with_wait_for(WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_exposed_port(5432.tcp())
        .with_env_var("POSTGRES_USER", "postgres")
        .with_env_var("POSTGRES_PASSWORD", "postgres")
        .with_env_var("POSTGRES_DB", "postgres");

    match image.start().await {
        Ok(node) => {
            let host = node.get_host().await?;
            let port = node.get_host_port_ipv4(5432).await?;
            let conn_str =
                format!("host={host} port={port} user=postgres password=postgres dbname=postgres");
            Ok(Some((node, conn_str)))
        }
        Err(e) => {
            eprintln!("SKIP: could not start pgvector container (Docker unavailable?): {e}");
            Ok(None)
        }
    }
}

/// Ingest a public + a group-restricted doc through one adapter, then query both
/// with and without the entitlement through a **fresh** adapter (the same DB) —
/// proving the ACL is enforced from the persisted column, not in-memory state.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn acl_persists_and_is_enforced_from_db_across_a_fresh_adapter() -> anyhow::Result<()> {
    let Some((_node, conn_str)) = start_pgvector().await? else {
        return Ok(()); // Docker unavailable — skip, don't fail.
    };

    // --- INGEST PROCESS: ingest one org-public doc + one private-repo doc ---
    {
        let ingest_adapter = PostgresAdapter::connect(&conn_str).await?;
        let kb = ingest_adapter.knowledge();

        let mut public = Document::new(
            "The alpha office hours are open to the whole organization.",
            "handbook/hours.md",
            DocumentType::Documentation,
        );
        public.id = "doc-public".to_string();
        kb.ingest(public)?;

        let mut private = Document::new(
            "The alpha launch codes live in the private acme/secret repository.",
            "acme/secret/CODES.md",
            DocumentType::Documentation,
        );
        private.id = "doc-private".to_string();
        // Stamp the private-repo group ACL onto the document metadata, exactly
        // as the ingestion pipeline does for a private repo.
        let private = DocAcl::for_groups([PRIVATE_GROUP]).attach_to(private);
        kb.ingest(private)?;

        // Drop the ingest adapter — the ACL side table (if any) dies with it.
        // Only the DB row survives.
        drop(ingest_adapter);
    }

    // --- SERVE PROCESS: a FRESH adapter over the SAME database ---
    let serve_adapter = PostgresAdapter::connect(&conn_str).await?;

    // (a) An UNENTITLED requester (not in the private repo's group) must NOT see
    //     the private doc — enforced purely from the persisted `acl` column.
    let outsider = AccessContext::new(Some("random-user".to_string()), vec!["eng".to_string()]);
    let outsider_kb = serve_adapter.knowledge_for_access(&outsider);
    let outsider_ids: Vec<String> = outsider_kb
        .query("alpha", 10)?
        .into_iter()
        .map(|r| r.document_id)
        .collect();
    assert!(
        outsider_ids.contains(&"doc-public".to_string()),
        "public doc should be retrievable by anyone; got {outsider_ids:?}"
    );
    assert!(
        !outsider_ids.contains(&"doc-private".to_string()),
        "LEAK: private-repo doc retrievable by an unentitled requester from a FRESH adapter \
         (ACL not persisted/enforced from DB); got {outsider_ids:?}"
    );

    // (b) An ANONYMOUS requester (no token) — fail closed for ACL'd content.
    let anon_kb = serve_adapter.knowledge_for_access(&AccessContext::anonymous());
    let anon_ids: Vec<String> = anon_kb
        .query("alpha", 10)?
        .into_iter()
        .map(|r| r.document_id)
        .collect();
    assert!(
        anon_ids.contains(&"doc-public".to_string()),
        "anonymous should still see org-public knowledge; got {anon_ids:?}"
    );
    assert!(
        !anon_ids.contains(&"doc-private".to_string()),
        "LEAK: anonymous retrieved the private-repo doc; got {anon_ids:?}"
    );

    // (c) An ENTITLED requester (member of the private repo's group) DOES see it.
    let insider = AccessContext::new(
        Some("acme-dev".to_string()),
        vec![PRIVATE_GROUP.to_string()],
    );
    let insider_kb = serve_adapter.knowledge_for_access(&insider);
    let insider_ids: Vec<String> = insider_kb
        .query("alpha", 10)?
        .into_iter()
        .map(|r| r.document_id)
        .collect();
    assert!(
        insider_ids.contains(&"doc-private".to_string()),
        "entitled requester MUST retrieve the private-repo doc from a fresh adapter; got {insider_ids:?}"
    );
    assert!(
        insider_ids.contains(&"doc-public".to_string()),
        "entitled requester should also see org-public knowledge; got {insider_ids:?}"
    );

    // (d) The raw `knowledge()` handle (org isolation only, no access bound) is
    //     unaffected — it still returns both (used by ingest/admin paths).
    let raw_ids: Vec<String> = serve_adapter
        .knowledge()
        .query("alpha", 10)?
        .into_iter()
        .map(|r| r.document_id)
        .collect();
    assert!(raw_ids.contains(&"doc-public".to_string()));
    assert!(raw_ids.contains(&"doc-private".to_string()));

    println!(
        "POSTGRES ACL PERSISTENCE: ingest→serve ACL enforced from the knowledge_vectors.acl \
         column across a fresh adapter (outsider/anonymous denied, insider allowed)"
    );

    drop(serve_adapter);
    Ok(())
}
