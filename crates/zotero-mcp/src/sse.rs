//! SSE (Server-Sent Events) transport for the MCP server.
//!
//! # MCP SSE Protocol
//!
//! - `GET /sse` — establish SSE stream, receive `endpoint` event with POST URL
//! - `POST /messages?session_id=<id>` — send JSON-RPC requests
//!
//! # Thread safety
//!
//! rusqlite::Connection is not Send+Sync, so we can't share it across
//! async tasks. Instead, we open a fresh read-only connection per request
//! inside `spawn_blocking`. Since SQLite reads take <1ms and the connection
//! open is ~0.5ms, this overhead is negligible for an SSE server that
//! handles maybe 1-5 requests/second.
//!
//! # Reference
//!
//! MCP SSE spec: <https://modelcontextprotocol.io/docs/concepts/transports#server-sent-events-sse>

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use crate::config::Config;
use crate::db::DbPool;
use crate::protocol::JsonRpcRequest;
use crate::server::ServerContext;

/// Per-session state: an SSE sender channel.
type SessionMap = Arc<RwLock<HashMap<String, mpsc::Sender<String>>>>;

/// Shared application state (thread-safe — no rusqlite in here).
#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    sessions: SessionMap,
}

/// Run the MCP server in SSE mode (async, multi-session).
pub fn run_sse(ctx: ServerContext, host: &str, port: u16) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let state = AppState {
            config: Arc::new(ctx.config),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        };

        let app = axum::Router::new()
            .route("/sse", get(handle_sse))
            .route("/messages", post(handle_message))
            .route("/messages/", post(handle_message))
            .with_state(state);

        let addr = format!("{host}:{port}");
        eprintln!("[zotero-mcp] SSE server listening on http://{addr}/sse");

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    })
}

/// GET /sse — establish SSE connection, return event stream.
async fn handle_sse(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel::<String>(64);

    state.sessions.write().await.insert(session_id.clone(), tx.clone());

    // Clean up session when client disconnects (sender channel closes)
    let tx_cleanup = tx.clone();
    let sessions_cleanup = state.sessions.clone();
    let session_id_cleanup = session_id.clone();
    tokio::spawn(async move {
        tx_cleanup.closed().await;
        sessions_cleanup.write().await.remove(&session_id_cleanup);
        eprintln!("[zotero-mcp] Session disconnected: {session_id_cleanup}");
    });

    eprintln!("[zotero-mcp] New SSE session: {session_id}");

    // Send endpoint event
    let endpoint_url = format!("/messages?session_id={session_id}");
    let _ = tx.send(format!("endpoint:{endpoint_url}")).await;

    let stream = ReceiverStream::new(rx).map(move |msg| {
        if let Some(url) = msg.strip_prefix("endpoint:") {
            Ok(Event::default().event("endpoint").data(url))
        } else {
            Ok(Event::default().event("message").data(msg))
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// POST /messages?session_id=<id> — receive JSON-RPC, respond via SSE.
async fn handle_message(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    body: String,
) -> impl IntoResponse {
    let session_id = match params.get("session_id") {
        Some(id) => id.clone(),
        None => return axum::http::StatusCode::BAD_REQUEST,
    };

    // Parse JSON-RPC request
    let request: JsonRpcRequest = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(_) => return axum::http::StatusCode::BAD_REQUEST,
    };

    let sessions = state.sessions.read().await;
    let tx = match sessions.get(&session_id) {
        Some(tx) => tx.clone(),
        None => return axum::http::StatusCode::NOT_FOUND,
    };
    drop(sessions);

    let is_notification = request.id.is_none();
    let config = state.config.clone();

    // Process in spawn_blocking (opens fresh SQLite connections per request
    // because rusqlite::Connection is not Send+Sync)
    let response = tokio::task::spawn_blocking(move || {
        let db = DbPool::open(&config.zotero_sqlite_path, &config.bbt_migrated_path);
        let ctx = ServerContext { db, config: (*config).clone() };
        // Reuse shared dispatch from server module
        crate::server::dispatch(&request, &ctx)
    })
    .await;

    if !is_notification
        && let Ok(Some(resp)) = response {
            let json = serde_json::to_string(&resp).unwrap_or_default();
            let _ = tx.send(json).await;
        }

    axum::http::StatusCode::ACCEPTED
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> ServerContext {
        ServerContext {
            db: DbPool::empty(),
            config: Config {
                zotero_sqlite_path: "/tmp/z.sqlite".into(),
                zotero_storage_path: "/tmp/storage".into(),
                bbt_migrated_path: "/tmp/bbt".into(),
                zotero_api_key: None,
                zotero_library_id: "1".into(),
                zotero_library_type: "user".into(),
                bbt_url: "http://localhost:23119".into(),
                log_level: crate::config::LogLevel::Quiet,
            },
        }
    }

    #[test]
    fn sse_dispatch_initialize() {
        let ctx = test_ctx();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "initialize".into(),
            params: serde_json::json!({}),
        };
        let resp = crate::server::dispatch(&req, &ctx).unwrap();
        let result = resp.result.unwrap();
        assert_eq!(result["serverInfo"]["name"], "zotero-mcp");
    }

    #[test]
    fn sse_dispatch_ping() {
        let ctx = test_ctx();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(2)),
            method: "ping".into(),
            params: serde_json::json!(null),
        };
        let resp = crate::server::dispatch(&req, &ctx).unwrap();
        assert!(resp.result.is_some());
    }

    #[test]
    fn sse_dispatch_notification_ignored() {
        let ctx = test_ctx();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: None,
            method: "notifications/initialized".into(),
            params: serde_json::json!(null),
        };
        assert!(crate::server::dispatch(&req, &ctx).is_none());
    }
}
