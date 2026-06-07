//! API Gateway Management API event post-back.
//!
//! API Gateway WebSocket is **not** a persistent socket from the Lambda's point
//! of view: the function is invoked once per inbound frame and has no socket to
//! write to. To send events *back* to the connected client you call the API
//! Gateway **Management API** `PostToConnection`, addressing the client by its
//! `connectionId` against a callback endpoint derived from the event's
//! `domainName` + `stage` (`https://{domainName}/{stage}`).
//!
//! [`ConnectionPoster`] wraps that: build it once per invocation from the event
//! context, then `post` each protocol [`serde_json::Value`] event (the same
//! envelopes [`smooth_operator_agent_server::protocol`] builds) to the
//! connection. A `GoneException` (the client disconnected mid-turn) is treated
//! as a soft stop, not a hard error, so a streaming turn that outlives its
//! client doesn't blow up the invocation.

use anyhow::{anyhow, Result};
use aws_sdk_apigatewaymanagement::primitives::Blob;
use aws_sdk_apigatewaymanagement::Client as MgmtClient;
use serde_json::Value;

/// Posts protocol events back to a single WebSocket connection via the API
/// Gateway Management API.
#[derive(Clone)]
pub struct ConnectionPoster {
    client: MgmtClient,
    connection_id: String,
}

impl ConnectionPoster {
    /// Build a poster for `connection_id`, routing through the Management API
    /// callback endpoint `https://{domain_name}/{stage}`.
    ///
    /// The endpoint is the connection's own API Gateway domain + stage from the
    /// request context — NOT a configured value — so the same Lambda works
    /// across stages/custom-domains without redeploys.
    pub async fn new(domain_name: &str, stage: &str, connection_id: impl Into<String>) -> Self {
        let endpoint = format!("https://{domain_name}/{stage}");
        let shared = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let conf = aws_sdk_apigatewaymanagement::config::Builder::from(&shared)
            .endpoint_url(endpoint)
            .build();
        Self {
            client: MgmtClient::from_conf(conf),
            connection_id: connection_id.into(),
        }
    }

    /// Serialize `event` to JSON and post it to the connection.
    ///
    /// Returns `Ok(false)` if the connection is gone (`GoneException`) — the
    /// client disconnected — so callers can stop streaming gracefully. Other
    /// errors are returned.
    ///
    /// # Errors
    /// Returns an error if serialization fails or the Management API call fails
    /// for a reason other than the connection being gone.
    pub async fn post(&self, event: &Value) -> Result<bool> {
        let bytes = serde_json::to_vec(event).map_err(|e| anyhow!("serializing event: {e}"))?;
        let result = self
            .client
            .post_to_connection()
            .connection_id(&self.connection_id)
            .data(Blob::new(bytes))
            .send()
            .await;

        match result {
            Ok(_) => Ok(true),
            Err(e) => {
                // The client disconnected mid-turn — soft stop, not a failure.
                if let aws_sdk_apigatewaymanagement::error::SdkError::ServiceError(se) = &e {
                    if se.err().is_gone_exception() {
                        return Ok(false);
                    }
                }
                Err(anyhow!("post_to_connection: {e}"))
            }
        }
    }
}
