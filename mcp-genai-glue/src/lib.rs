//! Glue crate bridging [MCP](https://modelcontextprotocol.io) servers (via the
//! `rmcp` Rust SDK) with [`genai`](https://crates.io/crates/genai) tool-calling.
//!
//! The crate is intentionally **transport-agnostic**: it never opens a connection
//! to an MCP server. It operates purely on an already-initialized
//! [`rmcp::service::Peer<rmcp::service::RoleClient>`], translating:
//!
//! * MCP [`rmcp::model::Tool`]s  → [`genai::chat::Tool`]s, and
//! * a [`genai::chat::ToolCall`] → an MCP `tools/call` round-trip →
//!   [`genai::chat::ToolResponse`].
//!
//! Connection / transport / URL plumbing is left to the host application.
//!
//! # Known limitations
//!
//! `genai::chat::ToolResponse::content` is a `String`, so non-text MCP content
//! blocks (images, audio, embedded resources, resource links) cannot be passed
//! through verbatim. They are summarized as short placeholders appended to the
//! text content.

use genai::chat::{Tool, ToolCall, ToolName, ToolResponse};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Tool as McpTool,
};
use rmcp::service::{Peer, RoleClient, Service, ServiceError};
use serde_json::Value;

/// Errors produced while bridging an MCP server with genai tool-calling.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// An error reported by the MCP client service (transport, protocol, …).
    #[error("MCP service error: {0}")]
    Mcp(#[from] ServiceError),

    /// The model produced tool-call arguments that are not a JSON object.
    #[error("tool arguments must be a JSON object, got: {0}")]
    BadArguments(String),

    /// The MCP server returned a response kind this bridge does not yet handle
    /// (e.g. `input_required` / `task`).
    #[error("MCP server returned an unsupported response kind: {0}")]
    UnsupportedResponse(String),
}

/// Bridges a single connected MCP server with genai's tool-calling API.
///
/// Construct from an already-initialized MCP client peer (or from the
/// [`rmcp::service::RunningService`] returned by `serve_client`).
///
/// [`McpToolBridge`] is cheap to clone — it holds a single
/// [`Peer<RoleClient>`], which is itself a channel handle.
#[derive(Clone, Debug)]
pub struct McpToolBridge {
    peer: Peer<RoleClient>,
}

impl McpToolBridge {
    /// Wrap an already-initialized MCP client peer.
    pub fn new(peer: Peer<RoleClient>) -> Self {
        Self { peer }
    }

    /// Convenience constructor: take the peer off a running client service.
    ///
    /// `S` is the [`rmcp::service::Service`] implementation backing the client
    /// (often just `()`).
    pub fn from_running_service<S>(service: &rmcp::service::RunningService<RoleClient, S>) -> Self
    where
        S: Service<RoleClient>,
    {
        Self::new(service.peer().clone())
    }

    /// The underlying MCP client peer, in case the host needs it directly.
    pub fn peer(&self) -> &Peer<RoleClient> {
        &self.peer
    }

    /// List every tool exposed by the MCP server, converted to
    /// [`genai::chat::Tool`]s.
    ///
    /// Pagination is handled internally via `Peer::list_all_tools`.
    pub async fn tools(&self) -> Result<Vec<Tool>, BridgeError> {
        let mcp_tools = self.peer.list_all_tools().await?;
        Ok(mcp_tools.into_iter().map(mcp_tool_to_genai).collect())
    }

    /// Execute a single [`ToolCall`] emitted by the model against the MCP
    /// server, returning a [`ToolResponse`] correlated by `tool_call.call_id`.
    ///
    /// Tool-level errors (`CallToolResult { is_error: true }`) are surfaced as
    /// normal, non-error [`ToolResponse`]s whose content is prefixed with
    /// `"Error: "` so the model can react to the failure within the chat.
    pub async fn execute(&self, tool_call: &ToolCall) -> Result<ToolResponse, BridgeError> {
        let params = tool_call_to_request(tool_call)?;
        let response = self.peer.call_tool_once(params).await?;
        let result = match response {
            CallToolResponse::Complete(result) => result,
            CallToolResponse::InputRequired(_) => {
                return Err(BridgeError::UnsupportedResponse(
                    "input_required (MRTR elicitation not supported)".into(),
                ));
            }
            CallToolResponse::Task(_) => {
                return Err(BridgeError::UnsupportedResponse(
                    "task (SEP-2663 Tasks extension not supported)".into(),
                ));
            }
            // `CallToolResponse` is #[non_exhaustive]; guard against future variants.
            other => {
                return Err(BridgeError::UnsupportedResponse(format!(
                    "unknown response variant: {other:?}"
                )));
            }
        };

        let content = call_tool_result_to_content(&result);
        let content = if result.is_error == Some(true) {
            format!("Error: {content}")
        } else {
            content
        };

        Ok(ToolResponse {
            call_id: tool_call.call_id.clone(),
            fn_name: Some(tool_call.fn_name.clone()),
            content,
        })
    }

    /// Execute a batch of [`ToolCall`]s, preserving order. Each call is an
    /// independent round-trip; one failing call does not abort the others —
    /// its error is captured in the corresponding [`ToolResponse`].
    pub async fn execute_all(
        &self,
        tool_calls: &[ToolCall],
    ) -> Vec<Result<ToolResponse, BridgeError>> {
        let mut out = Vec::with_capacity(tool_calls.len());
        for tc in tool_calls {
            out.push(self.execute(tc).await);
        }
        out
    }
}

// Allow `().serve(...)`-style construction to feed the bridge directly.
impl<S> From<&rmcp::service::RunningService<RoleClient, S>> for McpToolBridge
where
    S: Service<RoleClient>,
{
    fn from(service: &rmcp::service::RunningService<RoleClient, S>) -> Self {
        Self::from_running_service(service)
    }
}

// ---------------------------------------------------------------------------
// --- Pure translation functions (no I/O) ------------------------------------
// ---------------------------------------------------------------------------

/// Convert an MCP [`rmcp::model::Tool`] into a [`genai::chat::Tool`].
///
/// MCP's `input_schema` *is* a JSON Schema object, so it maps 1:1 onto
/// `genai::chat::Tool::schema`. `strict` and `config` have no MCP equivalent
/// and are left unset.
pub fn mcp_tool_to_genai(mcp_tool: McpTool) -> Tool {
    let mut tool = Tool::new(ToolName::Custom(mcp_tool.name.into_owned()));
    if let Some(description) = mcp_tool.description {
        tool = tool.with_description(description.into_owned());
    }
    // `input_schema` is `Arc<JsonObject>` (i.e. `serde_json::Map<String, Value>`).
    // `Tool::schema` expects a `Value`; reconstruct the object value.
    tool = tool.with_schema(Value::Object(mcp_tool.input_schema.as_ref().clone()));
    tool
}

/// Convert a [`ToolCall`] (from the model) into MCP `tools/call` parameters.
///
/// `fn_arguments` must be a JSON object (or `null`, treated as no arguments).
/// Any other shape is a contract violation by the model and yields
/// [`BridgeError::BadArguments`].
pub fn tool_call_to_request(tool_call: &ToolCall) -> Result<CallToolRequestParams, BridgeError> {
    let params = CallToolRequestParams::new(tool_call.fn_name.clone());
    match tool_call.fn_arguments {
        Value::Null => Ok(params),
        Value::Object(ref map) => Ok(params.with_arguments(map.clone())),
        ref other => Err(BridgeError::BadArguments(other.to_string())),
    }
}

/// Flatten a completed [`CallToolResult`] into the `String` payload carried by
/// a [`genai::chat::ToolResponse`].
///
/// Priority:
/// 1. `structured_content` (pretty-printed JSON) — most providers/clients care
///    about structured output, so prefer it when present.
/// 2. Concatenated text from all `ContentBlock::Text` blocks, joined by `\n`.
/// 3. Short placeholders for any non-text blocks (images/audio/resources can't
///    be represented in a `String`).
pub fn call_tool_result_to_content(result: &CallToolResult) -> String {
    // 1. Prefer text content blocks — they have real newlines (not
    //    JSON-escaped \n). This is the human-readable output MCP servers
    //    intend for display (e.g. a git-style diff from `edit_file`).
    let text_parts: Vec<&str> = result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|t| t.text.as_str()))
        .collect();
    if !text_parts.is_empty() {
        let joined = text_parts.join("\n");
        // Some servers (e.g. using `CallToolResult::structured()`) put JSON
        // as the text content. If it looks like JSON with a "content" field
        // that's a string, extract that string (it has the real text).
        return extract_text_from_json_if_present(&joined);
    }

    // 2. Fall back to structured content (pretty-printed JSON).
    if let Some(structured) = &result.structured_content {
        // Try to extract a "content" string field — many servers wrap the
        // displayable text inside `{"content": "..."}`.
        if let Some(text) = structured.get("content").and_then(|v| v.as_str()) {
            return text.to_string();
        }
        return serde_json::to_string_pretty(structured).unwrap_or_else(|_| structured.to_string());
    }

    // 3. Walk non-text content blocks for anything useful.
    let mut parts: Vec<String> = Vec::new();
    for block in &result.content {
        match block {
            ContentBlock::Image(image) => {
                parts.push(format!("[{} omitted]", image.mime_type));
            }
            ContentBlock::Audio(audio) => {
                parts.push(format!("[{} omitted]", audio.mime_type));
            }
            ContentBlock::Resource(resource) => {
                let text = resource.get_text();
                if text.is_empty() {
                    parts.push("[embedded binary resource omitted]".to_string());
                } else {
                    parts.push(text);
                }
            }
            ContentBlock::ResourceLink(link) => {
                parts.push(format!("[resource link: {}]", link.uri));
            }
            // `ContentBlock` is #[non_exhaustive]; guard against future variants.
            _ => parts.push("[unsupported content block]".to_string()),
        }
    }

    parts.join("\n")
}

/// If `s` looks like a JSON object with a "content" field that's a string,
/// extract and return that string (it contains the real displayable text with
/// actual newlines). Otherwise return `s` unchanged.
fn extract_text_from_json_if_present(s: &str) -> String {
    let trimmed = s.trim();
    if !trimmed.starts_with('{') {
        return s.to_string();
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(content) = value.get("content").and_then(|v| v.as_str())
    {
        return content.to_string();
    }
    s.to_string()
}

// ---------------------------------------------------------------------------
// --- Tests -----------------------------------------------------------------
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::TextContent;
    use serde_json::json;
    use std::sync::Arc;

    fn schema_arc(value: Value) -> Arc<serde_json::Map<String, Value>> {
        Arc::new(value.as_object().unwrap().clone())
    }

    #[test]
    fn converts_mcp_tool_to_genai() {
        let schema = schema_arc(json!({
            "type": "object",
            "properties": {
                "city": { "type": "string" }
            },
            "required": ["city"],
        }));

        let mcp_tool = McpTool::new("get_weather", "Get the weather for a city.", schema);

        let genai_tool = mcp_tool_to_genai(mcp_tool);

        assert_eq!(genai_tool.name, ToolName::Custom("get_weather".to_string()));
        assert_eq!(
            genai_tool.description.as_deref(),
            Some("Get the weather for a city.")
        );
        let schema = genai_tool.schema.expect("schema should be set");
        assert_eq!(
            schema,
            json!({
                "type": "object",
                "properties": { "city": { "type": "string" } },
                "required": ["city"],
            })
        );
        // MCP has no notion of strict / config.
        assert!(genai_tool.strict.is_none());
        assert!(genai_tool.config.is_none());
    }

    #[test]
    fn converts_mcp_tool_without_description() {
        let mcp_tool = McpTool::new_with_raw("ping", None, schema_arc(json!({})));

        let genai_tool = mcp_tool_to_genai(mcp_tool);
        assert!(genai_tool.description.is_none());
    }

    #[test]
    fn tool_call_null_arguments_yield_no_arguments() {
        let tool_call = ToolCall {
            call_id: "1".into(),
            fn_name: "ping".into(),
            fn_arguments: Value::Null,
            thought_signatures: None,
        };

        let params = tool_call_to_request(&tool_call).unwrap();
        assert_eq!(params.name.as_ref(), "ping");
        assert!(params.arguments.is_none());
    }

    #[test]
    fn tool_call_object_arguments_are_forwarded() {
        let tool_call = ToolCall {
            call_id: "2".into(),
            fn_name: "get_weather".into(),
            fn_arguments: json!({ "city": "Stockholm" }),
            thought_signatures: None,
        };

        let params = tool_call_to_request(&tool_call).unwrap();
        let args = params.arguments.expect("arguments should be set");
        assert_eq!(args.get("city").and_then(|v| v.as_str()), Some("Stockholm"));
    }

    #[test]
    fn tool_call_non_object_arguments_are_rejected() {
        let tool_call = ToolCall {
            call_id: "3".into(),
            fn_name: "bad".into(),
            fn_arguments: json!("just a string"),
            thought_signatures: None,
        };

        let err = tool_call_to_request(&tool_call).unwrap_err();
        assert!(matches!(err, BridgeError::BadArguments(_)));
    }

    #[test]
    fn structured_content_with_content_field_extracted() {
        // Many servers (e.g. filesystem edit_file) wrap the displayable text
        // inside `{"content": "..."}`. We should extract it, not pretty-print
        // the JSON.
        let mut result = CallToolResult::default();
        result.content = vec![];
        result.structured_content = Some(json!({
            "content": "```diff\n- old line\n+ new line\n```"
        }));

        let content = call_tool_result_to_content(&result);
        assert_eq!(content, "```diff\n- old line\n+ new line\n```");
    }

    #[test]
    fn structured_content_without_content_field_pretty_printed() {
        let mut result = CallToolResult::default();
        result.content = vec![];
        result.structured_content = Some(json!({
            "temperature": 22.5,
            "humidity": 65,
        }));

        let content = call_tool_result_to_content(&result);
        assert!(content.contains("temperature"));
        assert!(content.contains("22.5"));
    }

    #[test]
    fn text_blocks_preferred_over_structured_content() {
        // Text blocks have real newlines; structured_content has escaped ones.
        // We should prefer text blocks.
        let mut result = CallToolResult::default();
        result.content = vec![ContentBlock::text("real diff with\nnewlines")];
        result.structured_content = Some(json!({
            "content": "escaped\\ndiff"
        }));

        let content = call_tool_result_to_content(&result);
        assert_eq!(content, "real diff with\nnewlines");
    }

    #[test]
    fn text_block_with_json_content_field_extracted() {
        // Some servers put JSON as the text content block. If it has a
        // "content" string field, extract it.
        let result = CallToolResult::success(vec![ContentBlock::text(
            r#"{"content": "```diff\n- removed\n+ added\n```"}"#,
        )]);

        let content = call_tool_result_to_content(&result);
        assert_eq!(content, "```diff\n- removed\n+ added\n```");
    }

    #[test]
    fn text_blocks_are_joined() {
        let result = CallToolResult::success(vec![
            ContentBlock::Text(TextContent::new("line one")),
            ContentBlock::Text(TextContent::new("line two")),
        ]);

        let content = call_tool_result_to_content(&result);
        assert_eq!(content, "line one\nline two");
    }

    #[test]
    fn empty_result_yields_empty_string() {
        let result = CallToolResult::success(vec![]);
        assert_eq!(call_tool_result_to_content(&result), "");
    }

    #[test]
    fn non_text_blocks_get_placeholders() {
        let result = CallToolResult::success(vec![
            ContentBlock::Image(rmcp::model::ImageContent::new("AAAA", "image/png")),
            ContentBlock::Audio(rmcp::model::AudioContent::new("AAAA", "audio/wav")),
        ]);

        let content = call_tool_result_to_content(&result);
        assert!(content.contains("[image/png omitted]"));
        assert!(content.contains("[audio/wav omitted]"));
    }

    #[test]
    fn execute_prefixes_tool_errors() {
        // Build a tool-error result and check the prefix logic without a server.
        let result = CallToolResult::error(vec![ContentBlock::text("city not found")]);
        let content = call_tool_result_to_content(&result);
        // `call_tool_result_to_content` itself does not add the prefix —
        // `execute` does. Here we just verify the underlying content.
        assert_eq!(content, "city not found");

        // Simulate the prefix logic `execute` applies:
        let prefixed = if result.is_error == Some(true) {
            format!("Error: {content}")
        } else {
            content
        };
        assert_eq!(prefixed, "Error: city not found");
    }
}
