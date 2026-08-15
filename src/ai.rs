use std::collections::HashMap;

use futures::StreamExt;
use genai::adapter::AdapterKind;
use genai::chat::{
    CacheControl, ChatMessage, ChatOptions, ChatRequest, ChatRole, ChatStream, ChatStreamEvent,
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
            ProviderConfig::from_auth(AuthData::FromEnv(env_name.to_string()))
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
            Message::Assistant(m, _, _, _) => ChatMessage::assistant(m.clone()),
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
                Message::Assistant(m, None, None, None)
            } else {
                Message::Assistant("NO RESPONSE".to_string(), None, None, None)
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
            Message::Assistant(m, _, _, _) => ChatMessage::assistant(m.clone()),
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
///
/// Cache-affecting options set here:
/// - Request-level `CacheControl::Ephemeral` auto-applies a cache breakpoint
///   to the static tools+system prefix (Anthropic).
/// - A stable `prompt_cache_key` derived from the conversation id so OpenAI's
///   automatic prefix cache stays on the same shard across requests/turns.
///   genai ignores this on non-OpenAI adapters, so setting it unconditionally
///   is safe.
fn base_chat_opts(thinking_effort: ThinkingEffort, conversation_id: i64) -> ChatOptions {
    let base = ChatOptions::default()
        .with_cache_control(CacheControl::Ephemeral)
        .with_prompt_cache_key(format!("ait-conv-{conversation_id}"))
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

/// Build a round's [`ChatRequest`] from the current conversation history.
///
/// Places a **message-level** cache breakpoint on the last message so the
/// growing conversation prefix is cached (Anthropic), complementing the
/// request-level breakpoint on the static tools+system prefix. genai ignores
/// message-level cache control on non-Anthropic adapters (OpenAI/Gemini cache
/// prefixes transparently), so applying it unconditionally is safe.
fn build_round_request(
    system_prompt: &Option<String>,
    chat_messages: &[ChatMessage],
    tools: &[Tool],
) -> ChatRequest {
    let mut req = if let Some(sp) = system_prompt {
        ChatRequest::new(vec![ChatMessage::system(sp.clone())])
    } else {
        ChatRequest::new(vec![])
    };

    let msg_count = chat_messages.len();
    for (i, cm) in chat_messages.iter().enumerate() {
        let mut cm = cm.clone();
        // Mark the last message as the cache breakpoint so the full prefix up
        // to (and including) it is reusable by the next turn/round.
        if i == msg_count - 1 {
            cm = cm.with_options(CacheControl::Ephemeral);
        }
        req = req.append_message(cm);
    }

    if !tools.is_empty() {
        req = req.with_tools(tools.to_vec());
    }
    req
}

/// Strip display-only decorations (the `<think>` ... `</think>` thinking blocks and
/// tool-call/result lines) from a legacy assistant display string so it can
/// be sent to the provider as a clean `ChatMessage::assistant(text)` without
/// polluting the input with UI-only annotations.
///
/// This is only used as a fallback for messages persisted before structured
/// history was available. New messages carry `raw_messages` instead.
///
/// The format produced by [`StreamHelper::combined`] looks like:
///
/// ```text
/// <think>
/// reasoning + tool notes
/// </think>
/// assistant text
/// ```
///
/// This function removes everything between `<think>` and `</think>` (inclusive
/// of the tags) and any stray `> `/`> **TOOL OUTPUT**:\n` tool-note lines, keeping only the
/// assistant text portions.
fn strip_display_decorations(text: &str) -> String {
    let mut result = String::new();
    let mut in_thinking = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("<think>") {
            in_thinking = true;
            continue;
        }
        if trimmed.starts_with("</think>") {
            in_thinking = false;
            continue;
        }
        if in_thinking {
            continue;
        }
        // Defensive: skip any leftover tool-call/result annotations.
        if trimmed.starts_with("> ") || trimmed.starts_with("> **TOOL OUTPUT**:\n") {
            continue;
        }
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(line);
    }
    result.trim().to_string()
}

/// Build the final assistant [`ChatMessage`] from a [`StreamEnd`], preserving
/// thought signatures and reasoning content for cache-friendly replay on
/// subsequent turns.
///
/// Returns `None` when there is no captured content (e.g. the stream ended
/// without producing any text). In that case the caller should **not** push
/// anything into `raw_messages` — the display text path will be used for
/// legacy replay instead, which is better than sending an empty content
/// block (which some providers reject on replay).
fn build_final_assistant_message(end: &StreamEnd) -> Option<ChatMessage> {
    let content = end.captured_content.as_ref()?;
    if content.is_empty() {
        return None;
    }
    Some(
        ChatMessage {
            role: ChatRole::Assistant,
            content: content.clone(),
            options: None,
        }
        .with_reasoning_content(end.captured_reasoning_content.clone()),
    )
}

/// Finalize the `raw_messages` accumulator for persistence.
///
/// Returns `None` when `raw_messages` is empty or contains only tool-use
/// and tool-response messages without a closing assistant text message.
/// This happens on cancellation before the final text was captured, or
/// when the stream ended without an explicit `End` event. In these cases
/// the legacy display-text fallback (with decorations stripped) is safer
/// than persisting an incomplete structured turn that would be replayed as
/// a dangling tool-use without a result, or an empty assistant message that
/// some providers reject.
///
/// When `raw_messages` already contains a proper final assistant message
/// (pushed by `build_final_assistant_message`), it is returned as-is.
///
/// When `raw_messages` is non-empty but ends with a tool-response (i.e. the
/// round was interrupted after tool execution but before the next round
/// produced text), we synthesize a final assistant message from the
/// `StreamHelper`'s accumulated text so the turn is replayable.
fn finalize_raw_messages(
    raw_messages: &[ChatMessage],
    helper: &StreamHelper,
) -> Option<Vec<ChatMessage>> {
    if raw_messages.is_empty() {
        return None;
    }

    // Check if the last message is an assistant text message (the normal
    // success path — `build_final_assistant_message` already pushed it).
    if raw_messages
        .last()
        .is_some_and(|m| m.role == ChatRole::Assistant)
    {
        return Some(raw_messages.to_vec());
    }

    // The last message is a tool-response or something else — the assistant
    // turn was interrupted mid-loop. Try to synthesize a final assistant
    // message from the display helper's accumulated text so the structured
    // history is still replayable.
    let display_text = helper.combined();
    let clean = strip_display_decorations(&display_text);
    if clean.is_empty() {
        // No text at all — don't persist structured messages; the display
        // text (which may contain thinking blocks) will be used via the
        // legacy fallback.
        return None;
    }

    let mut result = raw_messages.to_vec();
    result.push(ChatMessage::assistant(clean));
    Some(result)
}

/// Resolve the union of tools across all connected MCP servers.
///
/// Returns the genai tool list to send to the model, plus a map from tool name
/// to the [`McpToolBridge`] that owns it (used to dispatch `tools/call`). On
/// name collisions across servers the last server seen wins and a warning is
/// logged; a future revision could namespace by server id.
async fn resolve_tools(bridges: &[McpToolBridge]) -> (Vec<Tool>, HashMap<String, McpToolBridge>) {
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
    // responses. When `raw_messages` is available (structured history), we
    // replay the exact ChatMessages the provider saw; otherwise we fall back
    // to the display text with decorations stripped.
    let mut chat_messages: Vec<ChatMessage> = Vec::new();
    for m in messages {
        match m {
            Message::User(m) => chat_messages.push(ChatMessage::user(m.clone())),
            Message::Assistant(_, _, _, Some(raw)) => chat_messages.extend(raw.iter().cloned()),
            // Legacy: strip display decorations and build a plain assistant
            // message so ` thinking` blocks and tool notes don't pollute the input.
            Message::Assistant(text, _, _, None) => {
                chat_messages.push(ChatMessage::assistant(strip_display_decorations(text)))
            }
        }
    }

    let clientbuilder = match &model {
        ModelSpec::Iden(iden) if iden.adapter_kind == AdapterKind::Ollama => init_clientbuilder(
            ollama_host_url.as_deref(),
            base_chat_opts(thinking_effort, conversation_id),
        ),
        _ => init_clientbuilder(None, base_chat_opts(thinking_effort, conversation_id)),
    };
    let client = clientbuilder.build();

    let _ = action_tx
        .send(Action::StreamStart { conversation_id })
        .await;

    let mut helper = StreamHelper::new();

    // Resolve the tool set **once**, before the round loop, and sort it
    // deterministically by name. Resolving per-round means a server
    // connecting mid-conversation changes the tool list (and its order),
    // which busts the provider's prefix cache on every Anthropic request.
    // Keeping it stable across rounds and turns is the cache-friendly
    // trade-off: servers that finish connecting mid-conversation are picked
    // up on the next user turn.
    let (mut tools, tool_map) = resolve_tools(&mcp_bridges).await;
    tools.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
    let tool_names = tools
        .iter()
        .map(|t| t.name.to_string())
        .collect::<Vec<String>>();
    tracing::info!("Available tools {:?}", tool_names);

    // Structured messages collected during this assistant turn for
    // cache-friendly history replay on subsequent turns.
    let mut raw_messages: Vec<ChatMessage> = Vec::new();

    for _round in 0..MAX_TOOL_ROUNDS {
        // Build this round's request from the (possibly extended) history,
        // with the stable tool set.
        let chat_req = build_round_request(&system_prompt, &chat_messages, &tools);

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
                            raw_messages: finalize_raw_messages(&raw_messages, &helper),
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
                                        chat_messages.push(msg.clone());
                                        raw_messages.push(msg);
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
                                        // `edit_file` gets a larger budget so a
                                        // small diff is fully visible.
                                        helper.push_tool_note(&format_tool_result_line(
                                            &content,
                                            result_char_budget(&fn_name),
                                        ));
                                        let _ = action_tx
                                            .send(Action::StreamPartial {
                                                conversation_id,
                                                content: helper.combined(),
                                            })
                                            .await;

                                        let resp = ToolResponse::from_tool_call(tc, content);
                                        let resp_msg = ChatMessage::from(resp);
                                        chat_messages.push(resp_msg.clone());
                                        raw_messages.push(resp_msg);
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
                                // Build the final assistant message from the
                                // captured content (includes thought signatures,
                                // text, etc.) for cache-friendly replay.
                                // If there's no captured content, skip pushing
                                // to raw_messages so the legacy display-text
                                // fallback is used instead (avoids sending
                                // empty content blocks some providers reject).
                                if let Some(final_msg) = build_final_assistant_message(&end) {
                                    raw_messages.push(final_msg);
                                }
                                let _ = action_tx
                                    .send(Action::StreamComplete {
                                        conversation_id,
                                        content: helper.combined(),
                                        raw_messages: finalize_raw_messages(&raw_messages, &helper),
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
                                    raw_messages: finalize_raw_messages(&raw_messages, &helper),
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
            raw_messages: finalize_raw_messages(&raw_messages, &helper),
        })
        .await;
    Ok(())
}

/// Borrowed view over the accumulating stream content, used to emit
/// `StreamPartial`/`StreamComplete`/`StreamCancelled` payloads consistently.
///
/// Content is tracked **per round** as a list of [`RoundContent`] entries,
/// so the display alternates thinking blocks and assistant text across
/// tool-calling rounds instead of one big block:
///
/// ```text
/// <think>
/// reasoning + tool notes for round 1
/// </think>
/// assistant text for round 1
///
/// <think>
/// reasoning + tool notes for round 2
/// </think>
/// assistant text for round 2
/// ```
struct StreamHelper {
    rounds: Vec<RoundContent>,
}

/// One tool-calling round's accumulated content: reasoning/tool notes go
/// in `thinking` (rendered inside a `<think>` block), assistant text goes in
/// `text` (rendered after the `<think>` block).
#[derive(Default)]
struct RoundContent {
    thinking: String,
    text: String,
}

impl StreamHelper {
    fn new() -> Self {
        Self {
            rounds: vec![RoundContent::default()],
        }
    }

    /// The current (latest) round, mutable.
    fn current(&mut self) -> &mut RoundContent {
        self.rounds.last_mut().expect("always at least one round")
    }

    /// Append an assistant text chunk to the current round.
    fn push_text(&mut self, content: &str) {
        self.current().text.push_str(content);
    }

    /// Append a reasoning text chunk to the current round.
    fn push_reasoning(&mut self, content: &str) {
        self.current().thinking.push_str(content);
    }

    /// Append a tool-call/result annotation as its own line in the
    /// current round's thinking buffer. Ensures a separating newline on
    /// both sides so it never merges with model reasoning text.
    fn push_tool_note(&mut self, note: &str) {
        let thinking = &mut self.current().thinking;
        if !thinking.is_empty() && !thinking.ends_with('\n') {
            thinking.push('\n');
        }
        thinking.push_str(note);
        thinking.push('\n');
    }

    /// Finalize the current round and start a fresh one. Called after all
    /// tool calls in a round have executed, right before looping into the
    /// next round. The next round's reasoning/text accumulate separately
    /// so they render as a new `<think>`/text block pair.
    fn end_round(&mut self) {
        self.rounds.push(RoundContent::default());
    }

    /// Render all rounds as alternating `<think>` thinking blocks and assistant
    /// text, joined by blank lines. Empty rounds and empty sections are
    /// skipped so the display stays clean.
    fn combined(&self) -> String {
        let mut blocks: Vec<String> = Vec::new();
        for round in &self.rounds {
            let thinking = round.thinking.trim_end();
            let text = round.text.trim_end();
            if thinking.is_empty() && text.is_empty() {
                continue;
            }
            let mut block = String::new();
            if !thinking.is_empty() {
                block.push_str("<think>\n");
                block.push_str(thinking);
                block.push_str("\n</think>");
            }
            if !text.is_empty() {
                if !block.is_empty() {
                    block.push('\n');
                }
                block.push_str(text);
            }
            blocks.push(block);
        }
        blocks.join("\n\n")
    }
}

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
        let cache_hit = cached as f64 / prompt as f64;
        tracing::info!(
            prompt_tokens = prompt,
            completion_tokens = completion,
            total_tokens = total,
            cached_tokens = cached,
            cache_creation_tokens = cache_creation,
            cache_hit_ratio = cache_hit,
            "stream completed - token usage"
        );
    } else {
        tracing::info!("stream completed - no token usage returned");
    }
}

/// Format a tool-call announcement for the thinking trace.
///
/// Tries a filesystem-specific presentation first (e.g.
/// `> reading `src/main.rs``), falling back to a generic
/// `> calling `name` with arguments ...` line for everything else.
fn format_tool_call_line(name: &str, args: &Value) -> String {
    if let Some(fs_line) = format_filesystem_call_line(name, args) {
        return fs_line;
    }
    format_generic_call_line(name, args)
}

/// Generic fallback: `> calling `name` with arguments k="v", n=123`.
fn format_generic_call_line(name: &str, args: &Value) -> String {
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

/// Default char budget for a result line in the thinking trace.
const DEFAULT_RESULT_BUDGET: Option<usize> = Some(300);
/// Larger budget for `edit_file` so a small diff is fully visible.
const DIFF_RESULT_BUDGET: Option<usize> = None;

/// Char budget for the result line of a given tool. `edit_file` gets a larger
/// budget so git-style diffs are not truncated as aggressively.
fn result_char_budget(name: &str) -> Option<usize> {
    match name {
        "edit_file" => DIFF_RESULT_BUDGET,
        _ => DEFAULT_RESULT_BUDGET,
    }
}

/// Format a (truncated) tool-result line for the thinking trace, e.g.
/// `> **TOOL OUTPUT**:\n18C, partly cloudy`. Long results are truncated to `max_chars` so the
/// bubble stays readable; the full result is still sent to the model.
fn format_tool_result_line(content: &str, max_chars: Option<usize>) -> String {
    let trimmed = content.trim();
    let mut out = String::from("> **TOOL OUTPUT**:\n");
    let char_count = trimmed.chars().count();
    if let Some(max_char_count) = max_chars
        && char_count > max_char_count
    {
        out.extend(trimmed.chars().take(max_char_count));
        out.push('…');
    } else {
        out.push_str(trimmed);
    }
    out
}

// region:    --- Filesystem special cases ---

/// Make a path string relative to the current working directory when it is
/// cleanly inside it. Absolute paths outside the cwd are left as-is, and
/// already-relative paths are returned unchanged.
fn relative_path_display(path: &str) -> String {
    use std::path::Path;
    let p = Path::new(path);
    if !p.is_absolute() {
        return path.to_string();
    }
    let Ok(cwd) = std::env::current_dir() else {
        return path.to_string();
    };
    if let Some(rel) = pathdiff::diff_paths(p, &cwd) {
        let rel_str = rel.to_string_lossy().into_owned();
        // Only use the relative form if it does not escape the cwd.
        if !rel_str.starts_with("..") {
            return rel_str;
        }
    }
    path.to_string()
}

/// Extract a string argument from a JSON value by key.
fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

/// Filesystem-specific call-line formatting for
/// `@modelcontextprotocol/server-filesystem`. Returns `None` for unrecognized
/// tool names so the caller falls back to the generic formatter.
fn format_filesystem_call_line(name: &str, args: &Value) -> Option<String> {
    let line = match name {
        "read_file" | "read_text_file" | "read_media_file" => {
            let p = arg_str(args, "path").map(relative_path_display)?;
            format!("> reading `{p}`")
        }
        "read_multiple_files" => {
            let n = args
                .get("paths")
                .and_then(|v| v.as_array())
                .map(|a| a.len())?;
            format!("> reading {n} files")
        }
        "write_file" => {
            let p = arg_str(args, "path").map(relative_path_display)?;
            format!("> writing `{p}`")
        }
        "edit_file" => {
            let p = arg_str(args, "path").map(relative_path_display)?;
            format!("> editing `{p}`")
        }
        "move_file" => {
            let src = arg_str(args, "source").map(relative_path_display);
            let dst = arg_str(args, "destination").map(relative_path_display);
            let s = src?;
            let d = dst?;
            format!("> moving `{s}` to `{d}`")
        }
        "create_directory" => {
            let p = arg_str(args, "path").map(relative_path_display)?;
            format!("> creating directory `{p}`")
        }
        "list_directory" | "list_directory_with_sizes" | "directory_tree" => {
            let p = arg_str(args, "path").map(relative_path_display)?;
            format!("> listing `{p}`")
        }
        "search_files" => {
            let path = arg_str(args, "path").map(relative_path_display);
            let pattern = arg_str(args, "pattern");
            let p = path?;
            match pattern {
                Some(pat) => format!("> searching `{pat}` in `{p}`"),
                None => format!("> searching in `{p}`"),
            }
        }
        "get_file_info" => {
            let p = arg_str(args, "path").map(relative_path_display)?;
            format!("> info `{p}`")
        }
        "list_allowed_directories" => "> allowed directories".to_string(),
        _ => return None,
    };
    Some(line)
}

// endregion:    --- Filesystem special cases ---

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
        let line = format_tool_result_line("18°C, partly cloudy", Some(300));
        assert_eq!(line, "> **TOOL OUTPUT**:\n18°C, partly cloudy");
    }

    #[test]
    fn format_tool_result_truncates_long() {
        let long = "x".repeat(500);
        let line = format_tool_result_line(&long, Some(300));
        assert!(line.starts_with("> **TOOL OUTPUT**:\n"));
        assert!(line.ends_with('…'));
        // 300 chars of content + "> **TOOL OUTPUT**:\n" (19) + "…" (1)
        assert_eq!(line.chars().count(), 300 + 20);
    }

    #[test]
    fn format_tool_result_trims_whitespace() {
        let line = format_tool_result_line("  \n  hello  \n", Some(300));
        assert_eq!(line, "> **TOOL OUTPUT**:\nhello");
    }

    #[test]
    fn format_tool_result_respects_unicode_boundary() {
        let s = "é".repeat(350); // each é is 2 bytes but 1 char
        let line = format_tool_result_line(&s, Some(300));
        // Should not panic on char boundary; truncated to 300 chars + ellipsis.
        assert_eq!(line.chars().count(), 300 + 20);
    }

    // --- per-round combined() ---

    #[test]
    fn combined_single_round_text_only() {
        let mut h = StreamHelper::new();
        h.push_text("Hello world.");
        assert_eq!(h.combined(), "Hello world.");
    }

    #[test]
    fn combined_single_round_thinking_and_text() {
        let mut h = StreamHelper::new();
        h.push_reasoning("Let me think...");
        h.push_text("The answer is 42.");
        assert_eq!(
            h.combined(),
            "<think>\nLet me think...\n</think>\nThe answer is 42."
        );
    }

    #[test]
    fn combined_thinking_only_no_text() {
        let mut h = StreamHelper::new();
        h.push_reasoning("Just reasoning.");
        assert_eq!(h.combined(), "<think>\nJust reasoning.\n</think>");
    }

    #[test]
    fn combined_empty_round_skipped() {
        let mut h = StreamHelper::new();
        h.push_text("round 1 text");
        h.end_round(); // start round 2 (empty so far)
        h.end_round(); // start round 3
        h.push_text("round 3 text");
        assert_eq!(h.combined(), "round 1 text\n\nround 3 text");
    }

    #[test]
    fn combined_multi_round_with_tool_notes() {
        let mut h = StreamHelper::new();
        // Round 1: reasoning + tool call (no text)
        h.push_reasoning("let me check the weather.");
        h.push_tool_note("> calling `get_weather`");
        h.push_tool_note("> **TOOL OUTPUT**:\n18C");
        h.end_round();
        // Round 2: reasoning + tool call + text response
        h.push_reasoning("now check the time.");
        h.push_tool_note("> calling `search`");
        h.push_tool_note("> **TOOL OUTPUT**:\n14:00");
        h.push_text("the time is 14:00.");
        let out = h.combined();
        let expected = concat!(
            "<think>\n",
            "let me check the weather.\n",
            "> calling `get_weather`\n> **TOOL OUTPUT**:\n18C\n",
            "</think>\n\n",
            "<think>\n",
            "now check the time.\n",
            "> calling `search`\n> **TOOL OUTPUT**:\n14:00\n",
            "</think>\n",
            "the time is 14:00.",
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn end_round_starts_new_round() {
        let mut h = StreamHelper::new();
        h.push_text("round 1");
        h.end_round();
        h.push_text("round 2");
        assert_eq!(h.combined(), "round 1\n\nround 2");
    }

    #[test]
    fn push_tool_note_separates_from_reasoning() {
        let mut h = StreamHelper::new();
        h.push_reasoning("some reasoning");
        h.push_tool_note("> calling `echo`");
        // The tool note should be on its own line, after the reasoning.
        assert_eq!(h.current().thinking, "some reasoning\n> calling `echo`\n");
    }

    // --- Filesystem special cases ---

    #[test]
    fn fs_read_text_file() {
        let line = format_tool_call_line("read_text_file", &json!({ "path": "/tmp/src/main.rs" }));
        assert!(line.starts_with("> reading `"));
    }

    #[test]
    fn fs_read_file_deprecated() {
        let line = format_tool_call_line("read_file", &json!({ "path": "src/main.rs" }));
        assert_eq!(line, "> reading `src/main.rs`");
    }

    #[test]
    fn fs_read_multiple_files() {
        let line = format_tool_call_line(
            "read_multiple_files",
            &json!({ "paths": ["/tmp/a.rs", "/tmp/b.rs", "/tmp/c.rs"] }),
        );
        assert_eq!(line, "> reading 3 files");
    }

    #[test]
    fn fs_write_file() {
        let line = format_tool_call_line(
            "write_file",
            &json!({ "path": "src/new.rs", "content": "fn main() {}" }),
        );
        assert_eq!(line, "> writing `src/new.rs`");
    }

    #[test]
    fn fs_edit_file() {
        let line = format_tool_call_line(
            "edit_file",
            &json!({ "path": "src/main.rs", "edits": [{ "oldText": "a", "newText": "b" }] }),
        );
        assert_eq!(line, "> editing `src/main.rs`");
    }

    #[test]
    fn fs_move_file() {
        let line = format_tool_call_line(
            "move_file",
            &json!({ "source": "src/old.rs", "destination": "src/new.rs" }),
        );
        assert_eq!(line, "> moving `src/old.rs` to `src/new.rs`");
    }

    #[test]
    fn fs_create_directory() {
        let line = format_tool_call_line("create_directory", &json!({ "path": "src/new_dir" }));
        assert_eq!(line, "> creating directory `src/new_dir`");
    }

    #[test]
    fn fs_list_directory() {
        let line = format_tool_call_line("list_directory", &json!({ "path": "src" }));
        assert_eq!(line, "> listing `src`");
    }

    #[test]
    fn fs_search_files_with_pattern() {
        let line =
            format_tool_call_line("search_files", &json!({ "path": "src", "pattern": "*.rs" }));
        assert_eq!(line, "> searching `*.rs` in `src`");
    }

    #[test]
    fn fs_search_files_no_pattern() {
        let line = format_tool_call_line("search_files", &json!({ "path": "src" }));
        assert_eq!(line, "> searching in `src`");
    }

    #[test]
    fn fs_get_file_info() {
        let line = format_tool_call_line("get_file_info", &json!({ "path": "src/main.rs" }));
        assert_eq!(line, "> info `src/main.rs`");
    }

    #[test]
    fn fs_list_allowed_directories() {
        let line = format_tool_call_line("list_allowed_directories", &json!({}));
        assert_eq!(line, "> allowed directories");
    }

    #[test]
    fn fs_unknown_tool_falls_back_to_generic() {
        let line = format_tool_call_line("some_custom_tool", &json!({ "path": "src/main.rs" }));
        assert!(line.starts_with("> calling `some_custom_tool`"));
    }

    #[test]
    fn fs_missing_path_arg_falls_back() {
        let line = format_tool_call_line("read_text_file", &json!({}));
        assert!(line.starts_with("> calling `read_text_file`"));
    }

    #[test]
    fn result_budget_edit_file_is_larger() {
        assert_eq!(result_char_budget("edit_file"), None);
        assert_eq!(result_char_budget("read_file"), Some(300));
        assert_eq!(result_char_budget("write_file"), Some(300));
    }

    #[test]
    fn format_result_with_custom_budget() {
        let s = "x".repeat(600);
        let line = format_tool_result_line(&s, Some(600));
        assert!(!line.ends_with("\u{2026}"));
        let line2 = format_tool_result_line(&s, Some(300));
        assert!(line2.ends_with("\u{2026}"));
    }

    #[test]
    fn strip_decorations_simple_text_unchanged() {
        assert_eq!(strip_display_decorations("Hello."), "Hello.");
    }

    #[test]
    fn strip_decorations_removes_thinking_block() {
        let combined = "<think>\nLet me think carefully.\n</think>\nThe answer is 42.";
        assert_eq!(strip_display_decorations(combined), "The answer is 42.");
    }

    #[test]
    fn strip_decorations_removes_tool_notes() {
        let combined = concat!(
            "<think>\n",
            "let me check the weather.\n",
            "> calling `get_weather`\n",
            "> **TOOL OUTPUT**:\n18C\n",
            "</think>\n",
            "It is 18C outside.",
        );
        assert_eq!(strip_display_decorations(combined), "It is 18C outside.");
    }

    #[test]
    fn strip_decorations_multi_round_keeps_text_only() {
        let combined = concat!(
            "<think>\n",
            "reasoning round 1\n",
            "> calling `search`\n",
            "> **TOOL OUTPUT**:\nresult\n",
            "</think>\n\n",
            "<think>\n",
            "reasoning round 2\n",
            "</think>\n",
            "final text with thinking and multi-round content.",
        );
        assert_eq!(
            strip_display_decorations(combined),
            "final text with thinking and multi-round content."
        );
    }

    #[test]
    fn strip_decorations_thinking_only_yields_empty() {
        let combined = "<think>\nJust reasoning.\n</think>";
        assert_eq!(strip_display_decorations(combined), "");
    }

    #[test]
    fn strip_decorations_matches_combined_output() {
        // Verify against the real StreamHelper::combined() output format.
        let mut h = StreamHelper::new();
        h.push_reasoning("Let me think...");
        h.push_tool_note("> calling `get_weather`");
        h.push_tool_note("> **TOOL OUTPUT**:\n18C");
        h.end_round();
        h.push_text("It is 18C outside.");
        let combined = h.combined();
        assert_eq!(strip_display_decorations(&combined), "It is 18C outside.");
    }

    #[test]
    fn finalize_raw_messages_empty_uses_legacy_fallback() {
        let helper = StreamHelper::new();
        assert!(finalize_raw_messages(&[], &helper).is_none());
    }

    #[test]
    fn finalize_raw_messages_adds_partial_text_after_tool_messages() {
        let mut helper = StreamHelper::new();
        helper.push_text("Partial final answer.");
        let raw = vec![ChatMessage::from(ToolResponse::new(
            "call-id".to_string(),
            "tool result".to_string(),
        ))];
        let final_messages = finalize_raw_messages(&raw, &helper).expect("structured history");
        assert_eq!(final_messages.len(), 2);
        assert_eq!(final_messages[1].role, ChatRole::Assistant);
    }

    #[test]
    fn relative_path_keeps_relative_as_is() {
        assert_eq!(relative_path_display("src/main.rs"), "src/main.rs");
        assert_eq!(relative_path_display("./src/main.rs"), "./src/main.rs");
    }
}
