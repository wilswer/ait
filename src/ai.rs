use std::collections::HashMap;

use futures::StreamExt;
use genai::adapter::AdapterKind;
use genai::chat::{
    CacheControl, ChatMessage, ChatOptions, ChatRequest, ChatStream, ChatStreamEvent,
    ReasoningEffort, StreamChunk, StreamEnd, Tool, ToolCall, ToolResponse,
};
use genai::resolver::{AuthData, Endpoint, ProviderConfig, ServiceTargetResolver};
use genai::{ClientBuilder, ClientConfig, ModelIden, ModelSpec, ServiceTarget};
use mcp_genai_glue::McpToolBridge;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::app::{Action, AppResult, Message, ThinkingEffort};

pub const MODELS: [(&str, &str); 1] = [("Gemini", "gemini-3.1-pro-preview")];

fn get_api_key_name(kind: &AdapterKind) -> &'static str {
    match kind {
        AdapterKind::OpenAI | AdapterKind::OpenAIResp => "OPENAI_API_KEY",
        AdapterKind::Ollama => "",
        AdapterKind::Gemini => "GEMINI_API_KEY",
        AdapterKind::Anthropic => "ANTHROPIC_API_KEY",
        AdapterKind::Groq => "GROQ_API_KEY",
        AdapterKind::Cohere => "COHERE_API_KEY",
        AdapterKind::Xai => "XAI_API_KEY",
        AdapterKind::DeepSeek => "DEEPSEEK_API_KEY",
        AdapterKind::Fireworks => "FIREWORKS_API_KEY",
        AdapterKind::Together => "TOGETHER_API_KEY",
        AdapterKind::Nebius => "NEBIUS_API_KEY",
        AdapterKind::Zai => "ZAI_API_KEY",
        AdapterKind::BigModel => "BIGMODEL_API_KEY",
        AdapterKind::Mimo => "MIMO_API_KEY",
        AdapterKind::OpenRouter => "OPENROUTER_API_KEY",
        _ => todo!(),
    }
}

fn init_clientbuilder(ollama_host_url: Option<&str>, chat_options: ChatOptions) -> ClientBuilder {
    let client_config = if let Some(host) = ollama_host_url {
        let host = host.to_string();
        let resolver = ServiceTargetResolver::from_resolver_fn(
            move |service_target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
                if service_target.model.adapter_kind == AdapterKind::Ollama {
                    let endpoint = Endpoint::from_owned(host.clone());
                    let auth = AuthData::from_single("ollama");
                    let model =
                        ModelIden::new(AdapterKind::Ollama, service_target.model.model_name);
                    Ok(ServiceTarget {
                        endpoint,
                        auth,
                        model,
                    })
                } else {
                    Ok(service_target)
                }
            },
        );
        ClientConfig::default()
            .with_chat_options(chat_options)
            .with_service_target_resolver(resolver)
    } else {
        ClientConfig::default().with_chat_options(chat_options)
    };
    ClientBuilder::default().with_config(client_config)
}

pub async fn get_models(ollama_host_url: Option<&str>) -> AppResult<Vec<(String, String)>> {
    const KINDS: &[AdapterKind] = &[
        AdapterKind::OpenAI,
        AdapterKind::Ollama,
        AdapterKind::Gemini,
        AdapterKind::Anthropic,
        AdapterKind::Groq,
        AdapterKind::Cohere,
        AdapterKind::Xai,
        AdapterKind::DeepSeek,
        AdapterKind::OpenRouter,
    ];

    let client = init_clientbuilder(ollama_host_url, ChatOptions::default()).build();
    let mut models = Vec::new();
    for &kind in KINDS {
        let env_name = get_api_key_name(&kind);
        if !env_name.is_empty() && std::env::var(env_name).is_err() {
            continue;
        }
        let provider_config = if let Some(host_url) = ollama_host_url
            && kind == AdapterKind::Ollama
        {
            let endpoint = Endpoint::from_owned(host_url);
            ProviderConfig::from_endpoint(endpoint)
        } else {
            ProviderConfig::default()
        };
        let models_provider_res = client.all_model_names(kind, provider_config).await;
        let models_provider = match models_provider_res {
            Ok(m) => m
                .into_iter()
                .map(|m| (kind.as_str().to_string(), m))
                .collect::<Vec<(String, String)>>(),
            Err(_) => Vec::new(),
        };
        models.extend(models_provider);
    }
    for (p, m) in MODELS {
        if !models.contains(&(p.to_string(), m.to_string())) {
            models.push((p.to_string(), m.to_string()));
        }
    }
    models.sort();
    Ok(models)
}

pub async fn assistant_response(
    messages: &[Message],
    model: &str,
    system_prompt: Option<String>,
    ollama_host_url: Option<&str>,
) -> AppResult<Message> {
    let chat_messages = messages
        .iter()
        .map(|m| match m {
            Message::User(m) => ChatMessage::user(m.clone()),
            Message::Assistant(m, _, _) => ChatMessage::assistant(m.clone()),
        })
        .collect::<Vec<ChatMessage>>();
    let mut chat_req = if let Some(system_prompt) = system_prompt {
        ChatRequest::new(vec![ChatMessage::system(system_prompt)])
    } else {
        ChatRequest::new(vec![])
    };

    for chat_message in chat_messages {
        chat_req = chat_req.append_message(chat_message);
    }
    let client = init_clientbuilder(ollama_host_url, ChatOptions::default()).build();
    match client.exec_chat(model, chat_req, None).await {
        Ok(res) => {
            let chat_res = if let Some(m) = res.into_first_text() {
                Message::Assistant(m, None, None)
            } else {
                Message::Assistant("NO RESPONSE".to_string(), None, None)
            };
            Ok(chat_res)
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn assistant_response_streaming(
    messages: &[Message],
    model: ModelSpec,
    system_prompt: Option<String>,
    thinking_effort: ThinkingEffort,
    ollama_host_url: Option<String>,
) -> AppResult<ChatStream> {
    let chat_messages = messages
        .iter()
        .map(|m| match m {
            Message::User(m) => ChatMessage::user(m.clone()),
            Message::Assistant(m, _, _) => ChatMessage::assistant(m.clone()),
        })
        .collect::<Vec<ChatMessage>>();
    let mut chat_req = if let Some(system_prompt) = system_prompt {
        ChatRequest::new(vec![ChatMessage::system(system_prompt)])
    } else {
        ChatRequest::new(vec![])
    };

    for chat_message in chat_messages {
        chat_req = chat_req.append_message(chat_message);
    }

    let base_chat_opts = ChatOptions::default().with_cache_control(CacheControl::Ephemeral);

    let chat_opts = match thinking_effort {
        ThinkingEffort::None => base_chat_opts.with_reasoning_effort(ReasoningEffort::None),
        ThinkingEffort::Low => base_chat_opts.with_reasoning_effort(ReasoningEffort::Low),
        ThinkingEffort::Medium => base_chat_opts.with_reasoning_effort(ReasoningEffort::Medium),
        ThinkingEffort::High => base_chat_opts.with_reasoning_effort(ReasoningEffort::High),
        ThinkingEffort::XHigh => base_chat_opts.with_reasoning_effort(ReasoningEffort::XHigh),
        ThinkingEffort::Max => base_chat_opts.with_reasoning_effort(ReasoningEffort::Max),
    };

    let clientbuilder = match &model {
        ModelSpec::Iden(iden) if iden.adapter_kind == AdapterKind::Ollama => {
            init_clientbuilder(ollama_host_url.as_deref(), chat_opts)
        }
        _ => init_clientbuilder(None, chat_opts),
    };

    let client = clientbuilder.build();
    let chat_res = client.exec_chat_stream(model, chat_req, None).await?;
    Ok(chat_res.stream)
}

// region:    --- MCP tool-calling loop ---

/// Hard cap on how many tool-calling rounds a single assistant response may
/// run, to avoid runaway loops (the model keeps asking for tools forever).
const MAX_TOOL_ROUNDS: usize = 12;

/// Build the per-request [`ChatOptions`] with the streaming captures enabled
/// (text, reasoning, usage, and tool calls) plus the given thinking effort.
fn base_chat_opts(thinking_effort: ThinkingEffort) -> ChatOptions {
    let base = ChatOptions::default()
        .with_cache_control(CacheControl::Ephemeral)
        .with_capture_content(true)
        .with_capture_reasoning_content(true)
        .with_capture_usage(true)
        .with_capture_tool_calls(true);

    match thinking_effort {
        ThinkingEffort::None => base.with_reasoning_effort(ReasoningEffort::None),
        ThinkingEffort::Low => base.with_reasoning_effort(ReasoningEffort::Low),
        ThinkingEffort::Medium => base.with_reasoning_effort(ReasoningEffort::Medium),
        ThinkingEffort::High => base.with_reasoning_effort(ReasoningEffort::High),
        ThinkingEffort::XHigh => base.with_reasoning_effort(ReasoningEffort::XHigh),
        ThinkingEffort::Max => base.with_reasoning_effort(ReasoningEffort::Max),
    }
}

/// Resolve the union of tools across all connected MCP servers.
///
/// Returns the genai tool list to send to the model, plus a map from tool name
/// to the [`McpToolBridge`] that owns it (used to dispatch `tools/call`). On
/// name collisions across servers the last server seen wins and a warning is
/// logged; a future revision could namespace by server id.
async fn resolve_tools(
    bridges: &[McpToolBridge],
) -> (Vec<Tool>, HashMap<String, McpToolBridge>) {
    let mut tools = Vec::new();
    let mut by_name = HashMap::new();
    for bridge in bridges {
        match bridge.tools().await {
            Ok(server_tools) => {
                for tool in server_tools {
                    let name = tool.name.as_str().to_string();
                    if by_name.contains_key(&name) {
                        tracing::warn!(
                            mcp.tool = %name,
                            "tool name collision across MCP servers; last definition wins"
                        );
                    }
                    by_name.insert(name, bridge.clone());
                    tools.push(tool);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to list tools from an MCP server; skipping");
            }
        }
    }
    (tools, by_name)
}

/// Run the full assistant response stream, including the agentic tool-calling
/// loop.
///
/// The model is allowed to call MCP tools (resolved from `mcp_bridges`); each
/// tool-call round is executed against the owning server and the result is
/// fed back as a tool-response turn, up to [`MAX_TOOL_ROUNDS`] rounds. Text and
/// reasoning chunks are streamed back to the UI via `action_tx` as
/// [`Action::StreamPartial`] updates; the final message is sent as
/// [`Action::StreamComplete`] (or [`Action::StreamCancelled`] if the user
/// cancels).
///
/// This owns the entire multi-round conversation so the caller (`main.rs`)
/// only has to spawn it once per submitted message.
#[allow(clippy::too_many_arguments)]
pub async fn run_assistant_stream(
    messages: &[Message],
    model: ModelSpec,
    system_prompt: Option<String>,
    thinking_effort: ThinkingEffort,
    ollama_host_url: Option<String>,
    mcp_bridges: Vec<McpToolBridge>,
    conversation_id: i64,
    action_tx: mpsc::Sender<Action>,
    mut cancel_rx: mpsc::Receiver<()>,
) -> AppResult<()> {
    // Genai chat messages for the conversation history. This grows across
    // tool-calling rounds as we append assistant tool-use turns and tool
    // responses.
    let mut chat_messages: Vec<ChatMessage> = messages
        .iter()
        .map(|m| match m {
            Message::User(m) => ChatMessage::user(m.clone()),
            Message::Assistant(m, _, _) => ChatMessage::assistant(m.clone()),
        })
        .collect();

    let clientbuilder = match &model {
        ModelSpec::Iden(iden) if iden.adapter_kind == AdapterKind::Ollama => {
            init_clientbuilder(ollama_host_url.as_deref(), base_chat_opts(thinking_effort))
        }
        _ => init_clientbuilder(None, base_chat_opts(thinking_effort)),
    };
    let client = clientbuilder.build();

    let mut full_content = String::new();
    let mut full_thinking_content = String::new();

    let _ = action_tx
        .send(Action::StreamStart { conversation_id })
        .await;

    let mut helper = StreamHelper {
        full_content: &mut full_content,
        full_thinking_content: &mut full_thinking_content,
    };

    for _round in 0..MAX_TOOL_ROUNDS {
        // Re-resolve the tool set each round so servers that finish connecting
        // mid-conversation become available without re-sending the prompt.
        let (tools, tool_map) = resolve_tools(&mcp_bridges).await;
        let has_tools = !tools.is_empty();

        // Build this round's request from the (possibly extended) history.
        let mut chat_req = if let Some(ref sp) = system_prompt {
            ChatRequest::new(vec![ChatMessage::system(sp.clone())])
        } else {
            ChatRequest::new(vec![])
        };
        for cm in &chat_messages {
            chat_req = chat_req.append_message(cm.clone());
        }
        if has_tools {
            chat_req = chat_req.with_tools(tools.clone());
        }

        let mut stream = match client.exec_chat_stream(model.clone(), chat_req, None).await {
            Ok(res) => res.stream,
            Err(e) => {
                let _ = action_tx
                    .send(Action::Error {
                        conversation_id: Some(conversation_id),
                        message: format!("API Error: {e}"),
                    })
                    .await;
                return Ok(());
            }
        };

        // Drive this round's stream until it ends. If the model emitted tool
        // calls, execute them and loop into the next round; otherwise finalize.
        loop {
            tokio::select! {
                _ = cancel_rx.recv() => {
                    let _ = action_tx
                        .send(Action::StreamCancelled {
                            conversation_id,
                            content: helper.combined(),
                        })
                        .await;
                    return Ok(());
                }
                result_opt = stream.next() => {
                    match result_opt {
                        Some(Ok(event)) => match event {
                            ChatStreamEvent::ReasoningChunk(StreamChunk { content })
                                if !content.is_empty() =>
                            {
                                helper.push_reasoning(&content);
                                let combined = helper.combined();
                                let _ = action_tx
                                    .send(Action::StreamPartial {
                                        conversation_id,
                                        content: combined,
                                    })
                                    .await;
                            }
                            ChatStreamEvent::Chunk(StreamChunk { content }) if !content.is_empty() => {
                                helper.push_text(&content);
                                let combined = helper.combined();
                                let _ = action_tx
                                    .send(Action::StreamPartial {
                                        conversation_id,
                                        content: combined,
                                    })
                                    .await;
                            }
                            ChatStreamEvent::End(end) => {
                                log_usage(&end);
                                let tool_calls: Vec<ToolCall> = end
                                    .captured_tool_calls()
                                    .map(|c| c.into_iter().cloned().collect())
                                    .unwrap_or_default();

                                if !tool_calls.is_empty() && !tool_map.is_empty() {
                                    // Append the assistant tool-use turn (thought
                                    // signatures included, ordered before calls).
                                    if let Some(msg) = end.into_assistant_message_for_tool_use() {
                                        chat_messages.push(msg);
                                    }
                                    // Execute each tool call against its owning server.
                                    for tc in &tool_calls {
                                        let fn_name = tc.fn_name.clone();

                                        // Announce the call in the thinking trace
                                        // so the user sees what's happening live.
                                        helper.push_tool_note(&format_tool_call_line(
                                            &fn_name,
                                            &tc.fn_arguments,
                                        ));
                                        let _ = action_tx
                                            .send(Action::StreamPartial {
                                                conversation_id,
                                                content: helper.combined(),
                                            })
                                            .await;

                                        let content = match tool_map.get(&fn_name) {
                                            Some(bridge) => match bridge.execute(tc).await {
                                                Ok(resp) => resp.content,
                                                Err(e) => format!("Error: {e}"),
                                            },
                                            None => format!(
                                                "Error: no MCP server provides tool `{fn_name}`"
                                            ),
                                        };
                                        tracing::info!(
                                            mcp.tool = %fn_name,
                                            result_len = content.len(),
                                            "executed MCP tool"
                                        );

                                        // Append a truncated result line to the
                                        // thinking trace, then refresh the UI.
                                        helper.push_tool_note(&format_tool_result_line(&content));
                                        let _ = action_tx
                                            .send(Action::StreamPartial {
                                                conversation_id,
                                                content: helper.combined(),
                                            })
                                            .await;

                                        let resp = ToolResponse::from_tool_call(tc, content);
                                        chat_messages.push(ChatMessage::from(resp));
                                    }
                                    // Separate this round's text/notes from the
                                    // next round's so they don't concatenate.
                                    helper.end_round();
                                    let _ = action_tx
                                        .send(Action::StreamPartial {
                                            conversation_id,
                                            content: helper.combined(),
                                        })
                                        .await;
                                    // Break the inner loop; the outer loop runs the next round.
                                    break;
                                }

                                // No (executable) tool calls: this is the final turn.
                                let _ = action_tx
                                    .send(Action::StreamComplete {
                                        conversation_id,
                                        content: helper.combined(),
                                    })
                                    .await;
                                return Ok(());
                            }
                            _ => {}
                        },
                        Some(Err(e)) => {
                            let _ = action_tx
                                .send(Action::Error {
                                    conversation_id: Some(conversation_id),
                                    message: format!("Stream error: {e}"),
                                })
                                .await;
                            return Ok(());
                        }
                        None => {
                            // Stream ended without an explicit `End` event.
                            let _ = action_tx
                                .send(Action::StreamComplete {
                                    conversation_id,
                                    content: helper.combined(),
                                })
                                .await;
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    // Loop cap reached: finalize with whatever we have so the conversation stays usable.
    tracing::warn!(
        rounds = MAX_TOOL_ROUNDS,
        "MCP tool-calling loop hit the round cap; finalizing"
    );
    let _ = action_tx
        .send(Action::StreamComplete {
            conversation_id,
            content: helper.combined(),
        })
        .await;
    Ok(())
}

/// Borrowed view over the accumulating stream content, used to emit
/// `StreamPartial`/`StreamComplete`/`StreamCancelled` payloads consistently.
/// Owns the mutation methods so the mutable borrows stay in one place.
struct StreamHelper<'a> {
    full_content: &'a mut String,
    full_thinking_content: &'a mut String,
}

impl<'a> StreamHelper<'a> {
    /// Append an assistant text chunk.
    fn push_text(&mut self, content: &str) {
        self.full_content.push_str(content);
    }

    /// Append a reasoning text chunk.
    fn push_reasoning(&mut self, content: &str) {
        self.full_thinking_content.push_str(content);
    }

    /// Append a tool-call/result annotation as its own line in the thinking
    /// trace (inside the `<think>` block). Ensures a separating newline on
    /// both sides so it never merges with model reasoning text.
    fn push_tool_note(&mut self, note: &str) {
        if !self.full_thinking_content.is_empty()
            && !self.full_thinking_content.ends_with('\n')
        {
            self.full_thinking_content.push('\n');
        }
        self.full_thinking_content.push_str(note);
        self.full_thinking_content.push('\n');
    }

    /// Insert a separator between tool-calling rounds so text from one round
    /// doesn't concatenate with the next round's text, and the thinking trace
    /// gets a blank line before the next round's reasoning.
    ///
    /// Called after all tool calls in a round have executed, right before
    /// looping into the next round.
    fn end_round(&mut self) {
        // Ensure assistant text ends with a newline so the next round's text
        // starts on a fresh line instead of concatenating mid-line.
        if !self.full_content.is_empty() && !self.full_content.ends_with('\n') {
            self.full_content.push('\n');
        }
        // Add a blank line in the thinking trace to visually separate this
        // round's tool notes from the next round's reasoning.
        if !self.full_thinking_content.is_empty()
            && !self.full_thinking_content.ends_with("\n\n")
        {
            if !self.full_thinking_content.ends_with('\n') {
                self.full_thinking_content.push('\n');
            }
            self.full_thinking_content.push('\n');
        }
    }

    /// Combine the accumulated reasoning + assistant text into the display
    /// payload, matching the format the UI already expects.
    fn combined(&self) -> String {
        if self.full_thinking_content.is_empty() {
            self.full_content.clone()
        } else {
            format!(
                "<think>\n{}\n</think>\n{}",
                self.full_thinking_content, self.full_content
            )
        }
    }
}

/// Log the token-usage info carried by a stream end, if present.
fn log_usage(end: &StreamEnd) {
    if let Some(u) = &end.captured_usage {
        let prompt = u.prompt_tokens.unwrap_or(0);
        let completion = u.completion_tokens.unwrap_or(0);
        let total = u.total_tokens.unwrap_or(0);
        let cached = u
            .prompt_tokens_details
            .as_ref()
            .and_then(|d| d.cached_tokens)
            .unwrap_or(0);
        let cache_creation = u
            .prompt_tokens_details
            .as_ref()
            .and_then(|d| d.cache_creation_tokens)
            .unwrap_or(0);
        tracing::info!(
            prompt_tokens = prompt,
            completion_tokens = completion,
            total_tokens = total,
            cached_tokens = cached,
            cache_creation_tokens = cache_creation,
            "stream completed - token usage"
        );
    } else {
        tracing::info!("stream completed - no token usage returned");
    }
}

/// Format a tool-call announcement for the thinking trace, e.g.
/// `> calling `get_weather` with arguments location="stockholm", unit="celsius"`.
fn format_tool_call_line(name: &str, args: &Value) -> String {
    let args_str = match args {
        Value::Null => String::new(),
        Value::Object(map) => {
            let pairs: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let v_str = match v {
                        Value::String(s) => format!("\"{s}\""),
                        other => other.to_string(),
                    };
                    format!("{k}={v_str}")
                })
                .collect();
            pairs.join(", ")
        }
        other => other.to_string(),
    };
    if args_str.is_empty() {
        format!("> calling `{name}`")
    } else {
        format!("> calling `{name}` with arguments {args_str}")
    }
}

/// Format a (truncated) tool-result line for the thinking trace, e.g.
/// `< 18°C, partly cloudy`. Long results are truncated to a char budget so
/// the bubble stays readable; the full result is still sent to the model.
fn format_tool_result_line(content: &str) -> String {
    const MAX_CHARS: usize = 300;
    let trimmed = content.trim();
    let mut out = String::from("< ");
    let char_count = trimmed.chars().count();
    if char_count <= MAX_CHARS {
        out.push_str(trimmed);
    } else {
        out.extend(trimmed.chars().take(MAX_CHARS));
        out.push('…');
    }
    out
}

// endregion:    --- MCP tool-calling loop ---

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn format_tool_call_with_object_args() {
        let line = format_tool_call_line(
            "get_weather",
            &json!({ "location": "stockholm", "unit": "celsius" }),
        );
        // Order from a JSON object is insertion order (serde_json preserves it).
        assert_eq!(
            line,
            "> calling `get_weather` with arguments location=\"stockholm\", unit=\"celsius\""
        );
    }

    #[test]
    fn format_tool_call_with_no_args() {
        let line = format_tool_call_line("ping", &Value::Null);
        assert_eq!(line, "> calling `ping`");
    }

    #[test]
    fn format_tool_call_with_non_string_args() {
        let line = format_tool_call_line("add", &json!({ "a": 2, "b": 3 }));
        assert_eq!(line, "> calling `add` with arguments a=2, b=3");
    }

    #[test]
    fn format_tool_call_with_non_object_args() {
        let line = format_tool_call_line("echo", &json!("just a string"));
        assert_eq!(line, "> calling `echo` with arguments \"just a string\"");
    }

    #[test]
    fn format_tool_result_short() {
        let line = format_tool_result_line("18°C, partly cloudy");
        assert_eq!(line, "< 18°C, partly cloudy");
    }

    #[test]
    fn format_tool_result_truncates_long() {
        let long = "x".repeat(500);
        let line = format_tool_result_line(&long);
        assert!(line.starts_with("< "));
        assert!(line.ends_with('…'));
        // 300 chars of content + "< " (2) + "…" (1)
        assert_eq!(line.chars().count(), 300 + 3);
    }

    #[test]
    fn format_tool_result_trims_whitespace() {
        let line = format_tool_result_line("  \n  hello  \n");
        assert_eq!(line, "< hello");
    }

    #[test]
    fn format_tool_result_respects_unicode_boundary() {
        let s = "é".repeat(350); // each é is 2 bytes but 1 char
        let line = format_tool_result_line(&s);
        // Should not panic on char boundary; truncated to 300 chars + ellipsis.
        assert_eq!(line.chars().count(), 300 + 3);
    }

    // --- end_round separator ---

    #[test]
    fn end_round_adds_newline_to_text() {
        let mut text = String::from("Let me check.");
        let mut thinking = String::new();
        let mut helper = StreamHelper {
            full_content: &mut text,
            full_thinking_content: &mut thinking,
        };
        helper.end_round();
        assert_eq!(text, "Let me check.\n");
    }

    #[test]
    fn end_round_noop_if_text_already_ends_with_newline() {
        let mut text = String::from("Hello\n");
        let mut thinking = String::new();
        let mut helper = StreamHelper {
            full_content: &mut text,
            full_thinking_content: &mut thinking,
        };
        helper.end_round();
        assert_eq!(text, "Hello\n");
    }

    #[test]
    fn end_round_noop_if_text_empty() {
        let mut text = String::new();
        let mut thinking = String::new();
        let mut helper = StreamHelper {
            full_content: &mut text,
            full_thinking_content: &mut thinking,
        };
        helper.end_round();
        assert_eq!(text, "");
    }

    #[test]
    fn end_round_adds_blank_line_to_thinking_after_tool_note() {
        let mut text = String::new();
        let mut thinking = String::from("> calling `echo`\n< result\n");
        let mut helper = StreamHelper {
            full_content: &mut text,
            full_thinking_content: &mut thinking,
        };
        helper.end_round();
        // Should end with double newline (blank line separator).
        assert_eq!(thinking, "> calling `echo`\n< result\n\n");
    }

    #[test]
    fn end_round_thinking_noop_if_already_double_newline() {
        let mut text = String::new();
        let mut thinking = String::from("> calling `echo`\n< result\n\n");
        let mut helper = StreamHelper {
            full_content: &mut text,
            full_thinking_content: &mut thinking,
        };
        helper.end_round();
        assert_eq!(thinking, "> calling `echo`\n< result\n\n");
    }

    #[test]
    fn end_round_separates_text_across_rounds() {
        // Simulate: round 1 text + tool note, end_round, round 2 text.
        let mut text = String::from("Let me check.");
        let mut thinking = String::from("> calling `echo`\n< ok\n");
        let mut helper = StreamHelper {
            full_content: &mut text,
            full_thinking_content: &mut thinking,
        };
        helper.end_round();
        // Round 2 text arrives:
        helper.push_text("The result is ok.");
        assert_eq!(text, "Let me check.\nThe result is ok.");
    }
}
