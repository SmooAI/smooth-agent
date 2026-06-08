//! The admin HTTP API (Phase 12, increment 1).
//!
//! A REST surface, mounted under `/admin`, that the Next.js management console
//! (increment 2) consumes: whoami, chat history, indexing status, and document
//! sets. Everything except `/admin/health` is gated by [`require_role`] and
//! org-scoped to the caller's [`Principal`].
//!
//! ## Routes + role gates
//!
//! | route | min role | scope |
//! | --- | --- | --- |
//! | `GET /admin/health` | — (public) | liveness only |
//! | `GET /admin/me` | Basic | the caller's own principal |
//! | `GET /admin/conversations` | Basic | Admin/Curator: org-wide; Basic: own only |
//! | `GET /admin/conversations/{id}/messages` | Basic | role-scoped (Basic must own the convo) |
//! | `GET /admin/indexing/runs` | Curator | org connectors |
//! | `GET /admin/document-sets` | Curator | org document sets |
//!
//! ## Org-scoping + "Basic sees own"
//!
//! Every read filters to `principal.org_id` (the storage adapter's
//! `list_conversations_by_org`). For a **Basic** caller, the result is further
//! narrowed to conversations the caller *owns*: a conversation is owned when one
//! of its `User` participants carries `external_id == principal.user_id`. An
//! Admin or Curator sees the whole org. This mirrors the document-level
//! [`AccessContext`](smooth_operator::access_control::AccessContext) model RBAC
//! sits on top of.
//!
//! ## Errors
//!
//! Auth failures map to clean status codes (401 unauthenticated / invalid token /
//! missing role; 403 insufficient role) with the protocol's `error` envelope
//! shape (`{ code, message }`) reused for the body. Never leaks a token.

use axum::extract::{Path, Query, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use smooth_operator::auth::{AuthError, Principal, Role};
use smooth_operator::domain::ParticipantType;

use crate::protocol;
use crate::state::AppState;

/// Build the `/admin` router over the shared [`AppState`].
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/health", get(health))
        .route("/admin/me", get(me))
        .route("/admin/conversations", get(list_conversations))
        .route(
            "/admin/conversations/{id}/messages",
            get(conversation_messages),
        )
        .route("/admin/indexing/runs", get(indexing_runs))
        .route("/admin/document-sets", get(document_sets))
}

// ---------------------------------------------------------------------------
// Auth extractor — `require_role`
// ---------------------------------------------------------------------------

/// An authenticated [`Principal`] guaranteed to hold at least role `MIN`.
///
/// Used as an axum extractor: it reads `Authorization: Bearer <token>`, verifies
/// it via the configured [`AuthVerifier`](smooth_operator::auth::AuthVerifier) in
/// [`AppState`], and rejects with 401/403 if the token is missing/invalid or the
/// role is insufficient — *before* the handler body runs. `MIN` is a const role
/// rank: `0 = Basic`, `1 = Curator`, `2 = Admin`.
pub struct RequireRole<const MIN: u8>(pub Principal);

/// Map a [`Role`] to the const rank used by [`RequireRole`].
const fn role_rank(role: Role) -> u8 {
    match role {
        Role::Basic => 0,
        Role::Curator => 1,
        Role::Admin => 2,
    }
}

/// The minimum [`Role`] a const rank denotes (for error messages).
const fn rank_role(min: u8) -> Role {
    match min {
        0 => Role::Basic,
        1 => Role::Curator,
        _ => Role::Admin,
    }
}

impl<const MIN: u8> axum::extract::FromRequestParts<AppState> for RequireRole<MIN> {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).ok_or(AuthRejection(AuthError::Unauthenticated))?;
        let principal = state.auth.verify(&token).map_err(AuthRejection)?;
        if role_rank(principal.role) < MIN {
            return Err(AuthRejection(AuthError::Forbidden {
                required: rank_role(MIN),
                actual: principal.role,
            }));
        }
        Ok(RequireRole(principal))
    }
}

/// Extract the raw bearer token (without the `Bearer ` prefix) from the
/// `Authorization` header. Returns `None` when absent or not a bearer scheme.
fn bearer_token(parts: &Parts) -> Option<String> {
    let header = parts.headers.get(axum::http::header::AUTHORIZATION)?;
    let value = header.to_str().ok()?;
    let rest = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// An auth/authorization rejection rendered as the protocol's `error` envelope
/// with the right HTTP status.
pub struct AuthRejection(AuthError);

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        let (status, code) = match &self.0 {
            AuthError::Unauthenticated => (StatusCode::UNAUTHORIZED, "UNAUTHENTICATED"),
            AuthError::InvalidToken(_) => (StatusCode::UNAUTHORIZED, "INVALID_TOKEN"),
            AuthError::MissingRole(_) => (StatusCode::UNAUTHORIZED, "MISSING_ROLE"),
            AuthError::Forbidden { .. } => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            // A misconfigured verifier is a server error, surfaced as 500 with a
            // non-leaking message.
            AuthError::Misconfigured(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "AUTH_MISCONFIGURED")
            }
        };
        let body = protocol::error(None, code, &self.0.to_string());
        (status, Json(body)).into_response()
    }
}

/// An error from a handler body (storage failure, etc.) rendered as a 500 with
/// the protocol error shape.
struct AdminError(StatusCode, String, &'static str);

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        let body = protocol::error(None, self.2, &self.1);
        (self.0, Json(body)).into_response()
    }
}

impl AdminError {
    fn internal(msg: impl Into<String>) -> Self {
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            msg.into(),
            "INTERNAL_ERROR",
        )
    }

    fn forbidden(msg: impl Into<String>) -> Self {
        Self(StatusCode::FORBIDDEN, msg.into(), "FORBIDDEN")
    }

    fn not_found(msg: impl Into<String>) -> Self {
        Self(StatusCode::NOT_FOUND, msg.into(), "NOT_FOUND")
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /admin/health` — unauthenticated liveness probe.
async fn health() -> Json<Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

/// `GET /admin/me` — whoami. Returns the authenticated principal (any role).
async fn me(RequireRole::<0>(principal): RequireRole<0>) -> Json<Principal> {
    Json(principal)
}

/// Query params for `GET /admin/conversations`.
#[derive(Debug, Deserialize)]
struct ConversationsQuery {
    /// Max conversations to return (defaults to 50, capped at 200).
    limit: Option<usize>,
    /// Opaque cursor: the index to start from (simple offset paging over the
    /// org-scoped, newest-first list). `None` ⇒ start at the beginning.
    cursor: Option<usize>,
}

/// A conversation row in the admin list response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversationRow {
    id: String,
    name: String,
    platform: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

/// The `GET /admin/conversations` response envelope.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversationsResponse {
    conversations: Vec<ConversationRow>,
    /// Opaque cursor for the next page, or `null` when exhausted.
    next_cursor: Option<usize>,
}

/// `GET /admin/conversations` — chat history, org-scoped. Admin/Curator see the
/// whole org; Basic sees only conversations they own.
async fn list_conversations(
    RequireRole::<0>(principal): RequireRole<0>,
    State(state): State<AppState>,
    Query(q): Query<ConversationsQuery>,
) -> Result<Json<ConversationsResponse>, AdminError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.cursor.unwrap_or(0);

    let all = state
        .storage
        .list_conversations_by_org(&principal.org_id)
        .await
        .map_err(|e| AdminError::internal(format!("list conversations failed: {e}")))?;

    // Basic callers only see conversations they own.
    let visible: Vec<_> = if principal.role >= Role::Curator {
        all
    } else {
        let mut owned = Vec::new();
        for conv in all {
            if conversation_owned_by(&state, &conv.id, &principal.user_id).await {
                owned.push(conv);
            }
        }
        owned
    };

    let total = visible.len();
    let page: Vec<ConversationRow> = visible
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|c| ConversationRow {
            id: c.id,
            name: c.name,
            platform: format!("{:?}", c.platform).to_lowercase(),
            created_at: c.created_at,
            updated_at: c.updated_at,
        })
        .collect();

    let next = offset + page.len();
    let next_cursor = if next < total { Some(next) } else { None };

    Ok(Json(ConversationsResponse {
        conversations: page,
        next_cursor,
    }))
}

/// `GET /admin/conversations/{id}/messages` — messages for one conversation,
/// role-scoped (a Basic caller must own the conversation).
async fn conversation_messages(
    RequireRole::<0>(principal): RequireRole<0>,
    State(state): State<AppState>,
    Path(conversation_id): Path<String>,
) -> Result<Json<Value>, AdminError> {
    // The conversation must exist + belong to the caller's org.
    let conv = state
        .storage
        .get_conversation(&conversation_id)
        .await
        .map_err(|e| AdminError::internal(format!("get conversation failed: {e}")))?
        .ok_or_else(|| {
            AdminError::not_found(format!("conversation '{conversation_id}' not found"))
        })?;

    if conv.organization_id != principal.org_id {
        // Don't leak existence across orgs — 404, not 403.
        return Err(AdminError::not_found(format!(
            "conversation '{conversation_id}' not found"
        )));
    }

    // Basic callers may only read conversations they own.
    if principal.role < Role::Curator
        && !conversation_owned_by(&state, &conversation_id, &principal.user_id).await
    {
        return Err(AdminError::forbidden(
            "you do not have access to this conversation",
        ));
    }

    let query = smooth_operator::adapter::MessageQuery::new(&conversation_id, 200);
    let page = state
        .storage
        .list_messages_by_conversation(query)
        .await
        .map_err(|e| AdminError::internal(format!("list messages failed: {e}")))?;

    Ok(Json(serde_json::json!({
        "conversationId": conversation_id,
        "messages": page.messages,
        "nextCursor": page.next_cursor,
    })))
}

/// `GET /admin/indexing/runs` — indexing-run status across the org's connectors.
/// Curator+ only.
async fn indexing_runs(
    RequireRole::<1>(_principal): RequireRole<1>,
    State(state): State<AppState>,
) -> Json<Value> {
    let mut runs = Vec::new();
    for connector in state.connectors() {
        for run in state.indexing.list_runs(&connector) {
            runs.push(serde_json::json!({
                "id": run.id,
                "connectorName": run.connector_name,
                "status": format!("{:?}", run.status).to_lowercase(),
                "startedAt": run.started_at,
                "finishedAt": run.finished_at,
                "documentsSeen": run.documents_seen,
                "chunksIndexed": run.chunks_indexed,
                "documentsSkipped": run.documents_skipped,
                "cursor": run.cursor,
                "error": run.error,
            }));
        }
    }
    Json(serde_json::json!({ "runs": runs }))
}

/// A document-set row.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentSetRow {
    name: String,
    document_count: usize,
}

/// `GET /admin/document-sets` — distinct document-set names + doc counts.
/// Curator+ only.
async fn document_sets(
    RequireRole::<1>(_principal): RequireRole<1>,
    State(state): State<AppState>,
) -> Json<Value> {
    let sets: Vec<DocumentSetRow> = state
        .document_sets()
        .into_iter()
        .map(|(name, document_count)| DocumentSetRow {
            name,
            document_count,
        })
        .collect();
    Json(serde_json::json!({ "documentSets": sets }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Whether `user_id` owns the conversation — true when a `User` participant in
/// the conversation carries `external_id == user_id`.
async fn conversation_owned_by(state: &AppState, conversation_id: &str, user_id: &str) -> bool {
    match state
        .storage
        .list_participants_by_conversation(conversation_id)
        .await
    {
        Ok(parts) => parts.iter().any(|p| {
            p.participant_type == ParticipantType::User && p.external_id.as_deref() == Some(user_id)
        }),
        Err(_) => false,
    }
}
