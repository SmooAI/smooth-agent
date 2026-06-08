//! Server + per-connection state.
//!
//! [`AppState`] is shared across every connection (cloneable `Arc` handles): the
//! storage adapter, the resolved [`ServerConfig`], and the session registry.
//!
//! Sessions live in an in-memory map keyed by `sessionId` so `get_session` and
//! reconnects work across connections (mirrors the protocol's "connection →
//! session" / "session → connections" state model, simplified for the reference
//! single-process server). On AWS this map would be DynamoDB; on k8s, Redis or
//! Postgres.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use smooth_operator::adapter::StorageAdapter;
use smooth_operator::domain::Session;

use crate::config::ServerConfig;

/// Shared, cloneable application state handed to every WebSocket connection.
#[derive(Clone)]
pub struct AppState {
    /// The single storage seam (conversations / participants / messages /
    /// sessions / checkpoints / knowledge).
    pub storage: Arc<dyn StorageAdapter>,
    /// Resolved server configuration (gateway, model, limits).
    pub config: Arc<ServerConfig>,
    /// Session registry: `sessionId` → session blob. Shared across connections.
    sessions: Arc<RwLock<HashMap<String, Session>>>,
}

impl AppState {
    /// Construct shared state over a storage adapter and config.
    #[must_use]
    pub fn new(storage: Arc<dyn StorageAdapter>, config: ServerConfig) -> Self {
        Self {
            storage,
            config: Arc::new(config),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a freshly created session.
    pub fn insert_session(&self, session: Session) {
        if let Ok(mut map) = self.sessions.write() {
            map.insert(session.session_id.clone(), session);
        }
    }

    /// Look up a session by id.
    #[must_use]
    pub fn get_session(&self, session_id: &str) -> Option<Session> {
        self.sessions.read().ok()?.get(session_id).cloned()
    }
}
