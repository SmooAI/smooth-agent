//! Headline acceptance test for the GitHub connector (feature gap G1, the
//! `examples/dev-support` knowledge agent's primary source).
//!
//! TDD contract (written before the implementation): stand up a **mock GitHub
//! API** with [`wiremock`] returning canned responses for the repo tree, a
//! README (prose), a source file (code), and an issue. Point `octocrab` at the
//! mock base URL via [`GithubConnectorConfig::base_uri`], run
//! [`GithubConnector::pull(None)`], and assert it produces correctly-shaped
//! [`RawDocument`]s:
//!
//! (a) one prose doc (README) — `metadata.kind = "prose"`, blob-URL source,
//! (b) one code doc — `metadata.kind = "code"`, `path` + `lang` metadata,
//! (c) one issue doc — `metadata.kind = "issue"`, issue-URL source, body +
//!     comments concatenated, state/labels in metadata,
//! (d) a **private**-repo config stamps a restricting `acl` on every doc, while
//!     a **public** config leaves them public (`acl == None`).
//!
//! Then it runs the full `ingest(github_connector, chunker, embedder,
//! knowledge)` over the mock and asserts a distinctive seeded term is
//! retrievable — the same chunk → embed → store → retrieve round-trip as
//! `ingestion_contract.rs`, reusing the in-memory adapter + `DeterministicEmbedder`.
//!
//! No live network, no credentials: every GitHub call is served by the local
//! wiremock server, so this runs on every PR.

use std::sync::Arc;

use serde_json::json;
use wiremock::matchers::{method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use smooth_operator::adapter::StorageAdapter;
use smooth_operator_adapter_memory::InMemoryStorageAdapter;

use smooth_operator_ingestion::connectors::github::{
    GithubAuth, GithubConnector, GithubConnectorConfig, GithubVisibility,
};
use smooth_operator_ingestion::{ingest, Chunker, Connector, DeterministicEmbedder, IngestOptions};

/// base64 of a body, the way the GitHub contents API encodes blob content.
fn b64(body: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(body.as_bytes())
}

/// Stand up a mock GitHub API for repo `octocat/hello`:
/// - the recursive git tree (one README, one source file),
/// - the README contents blob (prose with a distinctive term),
/// - the source-file contents blob (code with a distinctive term),
/// - one issue + its comments.
async fn mock_github() -> MockServer {
    let server = MockServer::start().await;

    // GET /repos/{owner}/{repo} — repo metadata (default_branch).
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1,
            "name": "hello",
            "full_name": "octocat/hello",
            "private": false,
            "default_branch": "main",
        })))
        .mount(&server)
        .await;

    // GET /repos/{owner}/{repo}/git/trees/main?recursive=1 — the file tree.
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello/git/trees/main"))
        .and(query_param("recursive", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sha": "treesha",
            "truncated": false,
            "tree": [
                { "path": "README.md", "type": "blob", "sha": "rsha", "size": 64 },
                { "path": "src/lib.rs", "type": "blob", "sha": "csha", "size": 80 },
                // A vendored path that MUST be skipped by the connector's filter.
                { "path": "node_modules/dep/index.js", "type": "blob", "sha": "vsha", "size": 10 },
                // A binary/asset extension that MUST be skipped.
                { "path": "logo.png", "type": "blob", "sha": "psha", "size": 999 },
                // A directory entry (type=tree) — not a blob, must be ignored.
                { "path": "src", "type": "tree", "sha": "dsha" }
            ]
        })))
        .mount(&server)
        .await;

    // GET /repos/{owner}/{repo}/contents/README.md?ref=main — prose blob.
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello/contents/README.md"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "README.md",
            "path": "README.md",
            "sha": "rsha",
            "size": 64,
            "type": "file",
            "encoding": "base64",
            "content": b64("# Hello Project\n\nThe quibbleton subsystem powers onboarding."),
            "html_url": "https://github.com/octocat/hello/blob/main/README.md",
        })))
        .mount(&server)
        .await;

    // GET /repos/{owner}/{repo}/contents/src/lib.rs?ref=main — code blob.
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello/contents/src/lib.rs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "lib.rs",
            "path": "src/lib.rs",
            "sha": "csha",
            "size": 80,
            "type": "file",
            "encoding": "base64",
            "content": b64("pub fn zorblax() -> u32 {\n    42 // the answer\n}"),
            "html_url": "https://github.com/octocat/hello/blob/main/src/lib.rs",
        })))
        .mount(&server)
        .await;

    // GET /repos/{owner}/{repo}/issues?... — one issue (not a PR).
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "number": 7,
                "title": "Flooble fails on empty input",
                "body": "When the input is empty the flooble panics.",
                "state": "open",
                "html_url": "https://github.com/octocat/hello/issues/7",
                "comments": 1,
                "labels": [ { "name": "bug" }, { "name": "p1" } ],
                "updated_at": "2026-06-01T12:00:00Z",
                "user": { "login": "octocat" }
            }
        ])))
        .mount(&server)
        .await;

    // GET /repos/{owner}/{repo}/issues/7/comments — the issue's comments.
    Mock::given(method("GET"))
        .and(path("/repos/octocat/hello/issues/7/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "body": "Confirmed — guard the empty case.", "user": { "login": "ferris" } }
        ])))
        .mount(&server)
        .await;

    // Any unmatched GET returns an empty array so optional calls don't 404.
    Mock::given(method("GET"))
        .and(path_regex(".*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    server
}

fn config_for(server: &MockServer, visibility: GithubVisibility) -> GithubConnectorConfig {
    let mut cfg = GithubConnectorConfig::new("octocat", "hello", GithubAuth::Unauthenticated)
        .base_uri(server.uri())
        .visibility(visibility);
    if matches!(visibility, GithubVisibility::Private) {
        cfg = cfg.acl_groups(["eng-team"]);
    }
    cfg
}

#[tokio::test]
async fn pull_shapes_prose_code_and_issue_documents() {
    let server = mock_github().await;
    let connector = GithubConnector::new(config_for(&server, GithubVisibility::Public));

    assert_eq!(connector.name(), "github");

    let docs = connector.pull(None).await.expect("pull from mock GitHub");

    // ---- (a) prose: README -------------------------------------------------
    let prose = docs
        .iter()
        .find(|d| d.metadata.get("kind").map(String::as_str) == Some("prose"))
        .expect("a prose RawDocument for the README");
    assert!(
        prose.content.contains("quibbleton"),
        "prose content missing seeded term: {}",
        prose.content
    );
    assert_eq!(
        prose.source, "https://github.com/octocat/hello/blob/main/README.md",
        "prose source should be the GitHub blob URL"
    );
    assert_eq!(
        prose.metadata.get("repo").map(String::as_str),
        Some("octocat/hello")
    );
    assert_eq!(
        prose.metadata.get("path").map(String::as_str),
        Some("README.md")
    );

    // ---- (b) code: src/lib.rs ----------------------------------------------
    let code = docs
        .iter()
        .find(|d| d.metadata.get("kind").map(String::as_str) == Some("code"))
        .expect("a code RawDocument for src/lib.rs");
    assert!(
        code.content.contains("zorblax"),
        "code content missing seeded term: {}",
        code.content
    );
    assert_eq!(
        code.metadata.get("path").map(String::as_str),
        Some("src/lib.rs")
    );
    assert_eq!(
        code.metadata.get("lang").map(String::as_str),
        Some("rust"),
        "code lang should be derived from the .rs extension"
    );
    assert_eq!(
        code.source, "https://github.com/octocat/hello/blob/main/src/lib.rs",
        "code source should be the GitHub blob URL"
    );

    // Vendored + binary paths must NOT appear as documents.
    assert!(
        !docs.iter().any(|d| d.content.contains("node_modules")
            || d.metadata.get("path").map(String::as_str) == Some("node_modules/dep/index.js")),
        "vendored node_modules file leaked into documents"
    );
    assert!(
        !docs
            .iter()
            .any(|d| d.metadata.get("path").map(String::as_str) == Some("logo.png")),
        "binary asset leaked into documents"
    );

    // ---- (c) issue ---------------------------------------------------------
    let issue = docs
        .iter()
        .find(|d| d.metadata.get("kind").map(String::as_str) == Some("issue"))
        .expect("an issue RawDocument");
    assert!(
        issue.content.contains("flooble panics"),
        "issue body missing: {}",
        issue.content
    );
    assert!(
        issue.content.contains("guard the empty case"),
        "issue should concatenate top comments: {}",
        issue.content
    );
    assert_eq!(
        issue.source, "https://github.com/octocat/hello/issues/7",
        "issue source should be the GitHub issue URL"
    );
    assert_eq!(
        issue.metadata.get("state").map(String::as_str),
        Some("open")
    );
    assert!(
        issue
            .metadata
            .get("labels")
            .map(String::as_str)
            .unwrap_or("")
            .contains("bug"),
        "issue labels should be in metadata: {:?}",
        issue.metadata.get("labels")
    );

    // ---- (d) public repo → no ACL stamped ----------------------------------
    assert!(
        docs.iter().all(|d| d.acl.is_none()),
        "public-repo documents must be org-public (no ACL)"
    );
}

#[tokio::test]
async fn private_repo_stamps_a_restricting_acl() {
    let server = mock_github().await;
    let connector = GithubConnector::new(config_for(&server, GithubVisibility::Private));

    let docs = connector.pull(None).await.expect("pull from mock GitHub");
    assert!(!docs.is_empty(), "expected documents from the private repo");
    for doc in &docs {
        let acl = doc
            .acl
            .as_ref()
            .unwrap_or_else(|| panic!("private-repo doc {} must carry an ACL", doc.id));
        assert!(
            acl.iter().any(|g| g == "eng-team"),
            "private-repo ACL must scope to the configured group, got {acl:?}"
        );
    }
}

/// Configurable ACL group naming: when an operator sets `acl_groups` to *their
/// own* IdP/SSO entitlement group names (e.g. Okta groups `TS-Eng-Pricing` /
/// `TS-Eng-Orders`), the connector stamps **those exact strings** on every
/// document as the repo's `DocAcl` groups — no translation layer. This is what
/// lets a customer's SSO groups gate a repo's documents directly.
#[tokio::test]
async fn custom_acl_groups_are_stamped_verbatim() {
    let server = mock_github().await;
    let cfg = GithubConnectorConfig::new("octocat", "hello", GithubAuth::Unauthenticated)
        .base_uri(server.uri())
        .visibility(GithubVisibility::Private)
        .acl_groups(["TS-Eng-Pricing", "TS-Eng-Orders"]);
    let connector = GithubConnector::new(cfg);

    let docs = connector.pull(None).await.expect("pull from mock GitHub");
    assert!(!docs.is_empty(), "expected documents from the private repo");
    for doc in &docs {
        let acl = doc
            .acl
            .as_ref()
            .unwrap_or_else(|| panic!("doc {} must carry an ACL", doc.id));
        // EXACTLY the operator's groups — verbatim, no synthetic repo scope.
        assert_eq!(
            acl,
            &vec!["TS-Eng-Pricing".to_string(), "TS-Eng-Orders".to_string()],
            "custom acl_groups must be stamped verbatim, got {acl:?}"
        );
    }
}

/// Backward compatibility: a private repo with **no** `acl_groups` configured
/// falls back to the synthetic per-repo scope `github:owner/repo` (today's
/// behavior, unchanged), and a public repo stays org-public (`acl == None`) in
/// both the custom and default cases.
#[tokio::test]
async fn default_private_repo_uses_repo_scope_and_public_stays_open() {
    let server = mock_github().await;

    // Private, no acl_groups ⇒ synthetic `github:owner/repo` scope (unchanged).
    let private = GithubConnector::new(
        GithubConnectorConfig::new("octocat", "hello", GithubAuth::Unauthenticated)
            .base_uri(server.uri())
            .visibility(GithubVisibility::Private),
    );
    let docs = private.pull(None).await.expect("pull private");
    assert!(!docs.is_empty());
    for doc in &docs {
        let acl = doc.acl.as_ref().unwrap_or_else(|| {
            panic!(
                "private-repo doc {} must carry the default repo-scope ACL",
                doc.id
            )
        });
        assert_eq!(
            acl,
            &vec!["github:octocat/hello".to_string()],
            "default private-repo ACL must be the synthetic repo scope, got {acl:?}"
        );
    }

    // Public ⇒ no ACL stamped, regardless of whether acl_groups is set.
    let public_default = GithubConnector::new(config_for(&server, GithubVisibility::Public));
    let pub_docs = public_default.pull(None).await.expect("pull public");
    assert!(!pub_docs.is_empty());
    assert!(
        pub_docs.iter().all(|d| d.acl.is_none()),
        "public-repo documents must be org-public (no ACL)"
    );

    // A public repo with acl_groups set still stays org-public — acl_groups only
    // changes WHICH groups gate a *restricted* repo; it never makes a public repo
    // private.
    let public_with_groups = GithubConnector::new(
        GithubConnectorConfig::new("octocat", "hello", GithubAuth::Unauthenticated)
            .base_uri(server.uri())
            .visibility(GithubVisibility::Public)
            .acl_groups(["TS-Eng-Pricing"]),
    );
    let pwg_docs = public_with_groups
        .pull(None)
        .await
        .expect("pull public+groups");
    assert!(
        pwg_docs.iter().all(|d| d.acl.is_none()),
        "acl_groups must not make a public repo private"
    );
}

/// End-to-end: a document stamped by the connector with a **custom** SSO group
/// (`TS-Eng-Pricing`) is retrievable by an `AccessContext` carrying that group
/// and is NOT retrievable by one without it — proving a customer's own IdP group
/// gates retrieval exactly like the default `github:owner/repo` scope would. This
/// drives the full real chain: connector `acl_groups` → pipeline `DocAcl` →
/// `AclKnowledgeStore` side table → `AclReader` enforcement.
#[tokio::test]
async fn custom_group_gates_retrieval_end_to_end() {
    use smooth_operator::access_control::{AccessContext, AclKnowledgeStore};

    let server = mock_github().await;
    let connector = GithubConnector::new(
        GithubConnectorConfig::new("octocat", "hello", GithubAuth::Unauthenticated)
            .base_uri(server.uri())
            .visibility(GithubVisibility::Private)
            .acl_groups(["TS-Eng-Pricing"]),
    );

    // Wrap the in-memory KB so the ACL the connector stamps is recorded at ingest
    // and enforced at read.
    let storage: Arc<dyn StorageAdapter> = Arc::new(InMemoryStorageAdapter::new());
    let acl_store = AclKnowledgeStore::new(storage.knowledge());
    let report = ingest(
        &connector,
        &Chunker::default(),
        &DeterministicEmbedder::new(),
        acl_store.ingest_handle(),
        IngestOptions::for_org("org-topstep"),
    )
    .await
    .expect("ingest the private repo through the ACL store");
    assert!(
        report.chunks_stored >= 1,
        "expected stored chunks, got {}",
        report.chunks_stored
    );

    // A carrier of the custom Okta group reads the repo's docs.
    let carrier = acl_store.reader(AccessContext::new(
        Some("alice".into()),
        vec!["TS-Eng-Pricing".into()],
    ));
    let hits = carrier.query("quibbleton", 5).expect("carrier query");
    assert!(
        hits.iter()
            .any(|h| h.chunk.to_lowercase().contains("quibbleton")),
        "a user carrying TS-Eng-Pricing must be able to read the repo's docs"
    );

    // A user WITHOUT the group (different group, and the default org-anonymous)
    // sees nothing from the restricted repo.
    let outsider = acl_store.reader(AccessContext::new(
        Some("bob".into()),
        vec!["TS-Eng-Billing".into()],
    ));
    let blocked = outsider.query("quibbleton", 5).expect("outsider query");
    assert!(
        blocked.is_empty(),
        "a user WITHOUT TS-Eng-Pricing must not read the repo's docs, got {} hits",
        blocked.len()
    );
    let anon = acl_store.reader(AccessContext::anonymous());
    assert!(
        anon.query("quibbleton", 5).expect("anon query").is_empty(),
        "an anonymous requester must not read the restricted repo's docs"
    );
}

#[tokio::test]
async fn ingest_over_github_connector_is_retrievable() {
    let server = mock_github().await;
    let connector = GithubConnector::new(config_for(&server, GithubVisibility::Public));

    let storage: Arc<dyn StorageAdapter> = Arc::new(InMemoryStorageAdapter::new());
    let report = ingest(
        &connector,
        &Chunker::default(),
        &DeterministicEmbedder::new(),
        storage.knowledge(),
        IngestOptions::for_org("org-acme"),
    )
    .await
    .expect("ingest over the GitHub connector");

    // README (prose) + lib.rs (code) + issue #7 = 3 source docs at minimum.
    assert!(
        report.documents_pulled >= 3,
        "expected >=3 pulled docs (prose+code+issue), got {}",
        report.documents_pulled
    );
    assert!(
        report.chunks_stored >= 3,
        "expected chunks stored, got {}",
        report.chunks_stored
    );

    // The distinctive prose term is retrievable end-to-end.
    let kb = storage.knowledge();
    let hits = kb.query("quibbleton", 5).expect("query knowledge base");
    assert!(!hits.is_empty(), "quibbleton query returned nothing");
    assert!(
        hits[0].chunk.to_lowercase().contains("quibbleton"),
        "top hit should be the README chunk, got: {}",
        hits[0].chunk
    );

    // The distinctive code term is retrievable too.
    let code_hits = kb.query("zorblax", 5).expect("query knowledge base");
    assert!(
        code_hits.iter().any(|h| h.chunk.contains("zorblax")),
        "code term zorblax not retrievable"
    );
}
