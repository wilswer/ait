//! MCP server loader.
//!
//! Turns the `[mcp.servers.*]` entries from the config file into live
//! [`McpToolBridge`] instances backed by a connected `rmcp` client service.
//!
//! Each server uses either the **stdio** transport (a spawned child process,
//! configured via `command`/`args`/`env`) or the **http** streamable transport
//! (a remote `url`, optionally authenticated with `api_key`/`headers`).
//!
//! [`McpConnection`] keeps the underlying [`RunningService`] alive for the
//! lifetime of the bridge — dropping it cancels the connection.

use std::collections::HashMap;
use std::process::Stdio;

use anyhow::{Context, Result, anyhow};
use http::{HeaderName, HeaderValue};
use mcp_genai_glue::McpToolBridge;
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::TokioChildProcess;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use tokio::io::AsyncBufReadExt;
use tracing::{info, warn};

use crate::config::{McpConfig, McpServerConfig, McpTransportKind};

/// A live MCP server connection.
///
/// Holds the running client service (which drives the transport) together
/// with the genai-facing [`McpToolBridge`]. The `service` must outlive the
/// `bridge`; this struct enforces that by owning both.
pub struct McpConnection {
    /// Stable server id (the TOML table key).
    pub id: String,
    /// Human-readable display name.
    pub display_name: String,
    /// genai-facing bridge over this server's tools.
    pub bridge: McpToolBridge,
    /// The running client service. Kept alive so the transport keeps working.
    #[allow(dead_code)]
    service: RunningService<RoleClient, ()>,
}

impl McpConnection {
    /// The number of tools this server currently exposes (fetched live).
    pub async fn tool_count(&self) -> usize {
        self.bridge.tools().await.map(|t| t.len()).unwrap_or(0)
    }
}

/// Connect to every `enabled` MCP server defined in the config.
///
/// Servers are connected **concurrently**; each server that fails to connect
/// is logged and skipped — one bad server does not abort the others. Returns
/// the connections that succeeded, in the config's (sorted) order.
pub async fn connect_all(config: &McpConfig) -> Vec<McpConnection> {
    let mut tasks = Vec::new();
    for (id, server_cfg) in &config.servers {
        if !server_cfg.enabled {
            info!(mcp.server = id, "MCP server disabled, skipping");
            continue;
        }
        let id = id.clone();
        let server_cfg = server_cfg.clone();
        tasks.push(tokio::spawn(async move {
            match connect_one(id.clone(), server_cfg).await {
                Ok(conn) => Some(conn),
                Err(e) => {
                    warn!(mcp.server = %id, error = %e, "failed to connect MCP server");
                    None
                }
            }
        }));
    }

    let mut connections = Vec::new();
    for task in tasks {
        match task.await {
            Ok(Some(conn)) => connections.push(conn),
            Ok(None) => {}
            Err(e) => warn!(error = %e, "MCP loader task panicked"),
        }
    }
    connections
}

/// Connect to a single MCP server.
async fn connect_one(id: String, cfg: McpServerConfig) -> Result<McpConnection> {
    let display_name = cfg.display_name(&id).to_string();
    let kind = cfg
        .transport_kind()
        .map_err(|msg| anyhow!("server `{id}`: {msg}"))?;

    let service = match kind {
        McpTransportKind::Stdio => connect_stdio(&id, &cfg).await?,
        McpTransportKind::Http => connect_http(&id, &cfg).await?,
    };

    let bridge = McpToolBridge::from_running_service(&service);
    info!(mcp.server = %id, "MCP server connected");
    Ok(McpConnection {
        id,
        display_name,
        bridge,
        service,
    })
}

/// Spawn a stdio MCP server as a child process and serve it.
async fn connect_stdio(id: &str, cfg: &McpServerConfig) -> Result<RunningService<RoleClient, ()>> {
    let command = cfg
        .command
        .as_ref()
        .ok_or_else(|| anyhow!("server `{id}`: missing `command`"))?;

    let mut cmd = tokio::process::Command::new(command);
    cmd.args(&cfg.args);
    cmd.envs(&cfg.env);

    // Pipe all three std streams so the child never touches the terminal:
    //  - stdin/stdout are the JSON-RPC transport (rmcp pipes them itself).
    //  - stderr is captured and routed into our logger.
    let (child, stderr) = TokioChildProcess::builder(cmd)
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("server `{id}`: failed to spawn `{command}`"))?;

    if let Some(stderr) = stderr {
        let id = id.to_string();
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim_end();
                        if !trimmed.is_empty() {
                            info!(mcp.server = %id, "{}", trimmed);
                        }
                    }
                    Err(e) => {
                        warn!(mcp.server = %id, error = %e, "error reading server stderr");
                        break;
                    }
                }
            }
        });
    }

    let service = ()
        .serve(child)
        .await
        .with_context(|| format!("server `{id}`: MCP initialize handshake failed"))?;
    Ok(service)
}

/// Connect to an http/streamable MCP server.
async fn connect_http(id: &str, cfg: &McpServerConfig) -> Result<RunningService<RoleClient, ()>> {
    let url = cfg
        .url
        .as_ref()
        .ok_or_else(|| anyhow!("server `{id}`: missing `url`"))?;

    let mut http_cfg = StreamableHttpClientTransportConfig::with_uri(url.as_str());
    if let Some(api_key) = &cfg.api_key {
        http_cfg = http_cfg.auth_header(api_key.as_str());
    }
    if !cfg.headers.is_empty() {
        let mut headers = HashMap::with_capacity(cfg.headers.len());
        for (name, value) in &cfg.headers {
            let name = HeaderName::try_from(name.as_str())
                .with_context(|| format!("server `{id}`: invalid header name `{name}`"))?;
            let value = HeaderValue::try_from(value.as_str())
                .with_context(|| format!("server `{id}`: invalid header value for `{name}`"))?;
            headers.insert(name, value);
        }
        http_cfg = http_cfg.custom_headers(headers);
    }

    let transport = StreamableHttpClientTransport::from_config(http_cfg);
    let service = ()
        .serve(transport)
        .await
        .with_context(|| format!("server `{id}`: MCP initialize handshake failed"))?;
    Ok(service)
}
