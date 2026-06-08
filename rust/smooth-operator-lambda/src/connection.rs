//! Connection bookkeeping for `$connect` / `$disconnect`.
//!
//! API Gateway WebSocket fires `$connect` when a client opens the socket and
//! `$disconnect` when it closes. Because the Lambda holds no in-process state
//! across invocations, the connection registry lives in the same DynamoDB
//! table as everything else: a tiny item per live connection keyed
//! `PK=WSCONN#<connectionId>, SK=WSCONN#`. `$connect` writes it (with a TTL so
//! stale rows self-expire if a `$disconnect` is ever missed); `$disconnect`
//! deletes it. Sessions are keyed independently, so message handling never
//! depends on this registry — it's pure operational bookkeeping (who is
//! connected right now) layered onto the single table.
//!
//! These writes use the concrete [`DynamoDbAdapter`]'s exposed client + table
//! name rather than adding a method to the storage trait, keeping the trait
//! focused on the protocol's domain entities.

use anyhow::{anyhow, Result};
use aws_sdk_dynamodb::types::AttributeValue;
use smooth_operator_adapter_dynamodb::DynamoDbAdapter;

/// PK/SK helpers for the connection item (kept local — it's not a protocol
/// domain entity, just operational state).
fn conn_pk(connection_id: &str) -> String {
    format!("WSCONN#{connection_id}")
}
const CONN_SK: &str = "WSCONN#";

/// TTL for a connection row: a generous ceiling so a missed `$disconnect`
/// doesn't leave the registry growing unbounded. API Gateway's own idle/socket
/// timeout is far shorter, so this only guards the pathological case.
const CONN_TTL_SECS: i64 = 24 * 60 * 60;

/// Record a freshly opened connection. Idempotent (a plain put overwrites).
///
/// # Errors
/// Returns an error if the DynamoDB put fails.
pub async fn record_connect(adapter: &DynamoDbAdapter, connection_id: &str) -> Result<()> {
    let now = chrono::Utc::now();
    let ttl = now.timestamp() + CONN_TTL_SECS;
    adapter
        .client()
        .put_item()
        .table_name(adapter.table_name())
        .item("pk", AttributeValue::S(conn_pk(connection_id)))
        .item("sk", AttributeValue::S(CONN_SK.to_string()))
        .item("entity", AttributeValue::S("ws-connection".to_string()))
        .item("connectionId", AttributeValue::S(connection_id.to_string()))
        .item("connectedAt", AttributeValue::S(now.to_rfc3339()))
        .item("ttl", AttributeValue::N(ttl.to_string()))
        .send()
        .await
        .map_err(|e| anyhow!("record_connect put_item: {e}"))?;
    Ok(())
}

/// Remove a closed connection's record. Missing rows are fine (idempotent).
///
/// # Errors
/// Returns an error if the DynamoDB delete fails.
pub async fn record_disconnect(adapter: &DynamoDbAdapter, connection_id: &str) -> Result<()> {
    adapter
        .client()
        .delete_item()
        .table_name(adapter.table_name())
        .key("pk", AttributeValue::S(conn_pk(connection_id)))
        .key("sk", AttributeValue::S(CONN_SK.to_string()))
        .send()
        .await
        .map_err(|e| anyhow!("record_disconnect delete_item: {e}"))?;
    Ok(())
}
