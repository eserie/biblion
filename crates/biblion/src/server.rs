//! MCP server — stdio JSON-RPC event loop.
//!
//! # Architecture
//!
//! This is a synchronous, blocking server that reads JSON-RPC requests
//! line-by-line from stdin and writes responses to stdout. This is the
//! same pattern as ox-mcp (oxymake) and msgvault (Go).
//!
//! Why synchronous? Because our hot path is SQLite reads, which are
//! inherently blocking and complete in <1ms. Adding an async runtime
//! would add complexity for zero benefit on the common path. The few
//! tools that need network (BBT RPC, Zotero Web API, PDF resolver)
//! use `tokio::runtime::Builder::new_current_thread()` on demand.
//!
//! # Flow
//!
//! ```text
//! stdin → read_line → parse JSON-RPC → dispatch → tool handler → JSON-RPC → stdout
//!                                         │
//!                           ┌──────────────┼──────────────┐
//!                           │              │              │
//!                      initialize    tools/list     tools/call
//!                                                        │
//!                                                  tools::handle()
//! ```

use std::io::{BufRead, Write};

use anyhow::Result;
use serde_json::json;

use crate::config::{Config, LogLevel};
use crate::db::DbPool;
use crate::protocol::{
    InitializeResult, JsonRpcRequest, JsonRpcResponse, ServerCapabilities, ServerInfo,
    ToolCallParams, ToolsCapability, ToolsListResult,
};
use crate::tools;

/// Runtime context shared across all tool calls.
///
/// Holds the database connections and configuration. Created once at
/// startup and passed by reference to every tool handler.
pub struct ServerContext {
    pub db: DbPool,
    pub config: Config,
}

/// Run the MCP server over stdio (blocking).
///
/// Reads JSON-RPC requests line-by-line from stdin, dispatches them,
/// and writes responses to stdout. EOF on stdin causes a clean exit.
pub fn run_stdio(ctx: &ServerContext) -> Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    log(
        ctx,
        LogLevel::Info,
        "Zotero MCP server started (Rust, stdio)",
    );

    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            log(ctx, LogLevel::Info, "Client disconnected (EOF)");
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        log(ctx, LogLevel::Debug, &format!("< {trimmed}"));

        let request: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse::error(None, -32700, format!("Parse error: {e}"));
                write_response(&mut writer, &resp, ctx)?;
                continue;
            }
        };

        let is_notification = request.id.is_none();
        let response = dispatch(&request, ctx);

        if !is_notification && let Some(resp) = response {
            write_response(&mut writer, &resp, ctx)?;
        }
    }

    Ok(())
}

/// Dispatch a JSON-RPC request to the appropriate handler.
/// Shared between stdio and SSE transports.
pub(crate) fn dispatch(request: &JsonRpcRequest, ctx: &ServerContext) -> Option<JsonRpcResponse> {
    match request.method.as_str() {
        "initialize" => Some(handle_initialize(request)),
        "notifications/initialized" => {
            log(ctx, LogLevel::Debug, "Client initialized");
            None
        }
        "tools/list" => Some(handle_tools_list(request)),
        "tools/call" => Some(handle_tools_call(request, ctx)),
        "ping" => Some(JsonRpcResponse::success(request.id.clone(), json!({}))),
        _ => {
            if request.id.is_some() {
                Some(JsonRpcResponse::method_not_found(
                    request.id.clone(),
                    &request.method,
                ))
            } else {
                None
            }
        }
    }
}

fn handle_initialize(request: &JsonRpcRequest) -> JsonRpcResponse {
    let result = InitializeResult {
        protocol_version: "2024-11-05".into(),
        capabilities: ServerCapabilities {
            tools: ToolsCapability {
                list_changed: Some(false),
            },
        },
        server_info: ServerInfo {
            name: "biblion".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    };

    JsonRpcResponse::success(
        request.id.clone(),
        serde_json::to_value(result).unwrap_or_default(),
    )
}

fn handle_tools_list(request: &JsonRpcRequest) -> JsonRpcResponse {
    let catalog = tools::tool_catalog();
    let result = ToolsListResult { tools: catalog };
    JsonRpcResponse::success(
        request.id.clone(),
        serde_json::to_value(result).unwrap_or_default(),
    )
}

fn handle_tools_call(request: &JsonRpcRequest, ctx: &ServerContext) -> JsonRpcResponse {
    let params: ToolCallParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::error(
                request.id.clone(),
                -32602,
                format!("Invalid params: {e}"),
            );
        }
    };

    log(
        ctx,
        LogLevel::Debug,
        &format!("Tool call: {} args={}", params.name, params.arguments),
    );

    let result = tools::handle_tool_call(&params.name, &params.arguments, ctx);

    JsonRpcResponse::success(
        request.id.clone(),
        serde_json::to_value(result).unwrap_or_default(),
    )
}

fn write_response(
    writer: &mut impl Write,
    response: &JsonRpcResponse,
    ctx: &ServerContext,
) -> Result<()> {
    let json = serde_json::to_string(response)?;
    log(ctx, LogLevel::Debug, &format!("> {json}"));
    writeln!(writer, "{json}")?;
    writer.flush()?;
    Ok(())
}

pub fn log(ctx: &ServerContext, level: LogLevel, message: &str) {
    if level_enabled(ctx.config.log_level, level) {
        let prefix = match level {
            LogLevel::Quiet => "",
            LogLevel::Info => "[biblion] ",
            LogLevel::Debug => "[biblion:debug] ",
        };
        eprintln!("{prefix}{message}");
    }
}

fn level_enabled(configured: LogLevel, requested: LogLevel) -> bool {
    match configured {
        LogLevel::Quiet => false,
        LogLevel::Info => matches!(requested, LogLevel::Info),
        LogLevel::Debug => matches!(requested, LogLevel::Info | LogLevel::Debug),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::JsonRpcRequest;

    fn test_ctx() -> ServerContext {
        ServerContext {
            db: DbPool::empty(),
            config: Config {
                zotero_sqlite_path: "/tmp/nonexistent.sqlite".into(),
                zotero_storage_path: "/tmp/storage".into(),
                bbt_migrated_path: "/tmp/nonexistent.migrated".into(),
                zotero_api_key: None,
                zotero_library_id: "1".into(),
                zotero_library_type: "user".into(),
                bbt_url: "http://localhost:23119/better-bibtex/json-rpc".into(),
                log_level: LogLevel::Quiet,
                writes_enabled: false,
                resolver: paper_resolver::ResolverConfig::default(),
            },
        }
    }

    #[test]
    fn dispatch_initialize() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "initialize".into(),
            params: json!({}),
        };
        let ctx = test_ctx();
        let resp = dispatch(&req, &ctx).unwrap();
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "biblion");
    }

    #[test]
    fn dispatch_ping() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(2)),
            method: "ping".into(),
            params: json!(null),
        };
        let ctx = test_ctx();
        let resp = dispatch(&req, &ctx).unwrap();
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn dispatch_tools_list() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(3)),
            method: "tools/list".into(),
            params: json!({}),
        };
        let ctx = test_ctx();
        let resp = dispatch(&req, &ctx).unwrap();
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        // Should have at least the read tools
        assert!(!tools.is_empty());
    }

    #[test]
    fn dispatch_notification_returns_none() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: None,
            method: "notifications/initialized".into(),
            params: json!(null),
        };
        let ctx = test_ctx();
        assert!(dispatch(&req, &ctx).is_none());
    }

    #[test]
    fn dispatch_unknown_method_returns_error() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(99)),
            method: "bogus/method".into(),
            params: json!(null),
        };
        let ctx = test_ctx();
        let resp = dispatch(&req, &ctx).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    #[test]
    fn dispatch_unknown_notification_ignored() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: None,
            method: "bogus/notification".into(),
            params: json!(null),
        };
        let ctx = test_ctx();
        assert!(dispatch(&req, &ctx).is_none());
    }

    #[test]
    fn dispatch_tools_call_invalid_params() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(10)),
            method: "tools/call".into(),
            params: json!("not an object"),
        };
        let ctx = test_ctx();
        let resp = dispatch(&req, &ctx).unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[test]
    fn level_quiet_blocks_all() {
        assert!(!level_enabled(LogLevel::Quiet, LogLevel::Info));
        assert!(!level_enabled(LogLevel::Quiet, LogLevel::Debug));
    }

    #[test]
    fn level_info_passes_info_only() {
        assert!(level_enabled(LogLevel::Info, LogLevel::Info));
        assert!(!level_enabled(LogLevel::Info, LogLevel::Debug));
    }

    #[test]
    fn level_debug_passes_both() {
        assert!(level_enabled(LogLevel::Debug, LogLevel::Info));
        assert!(level_enabled(LogLevel::Debug, LogLevel::Debug));
    }
}
