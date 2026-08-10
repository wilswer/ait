use std::collections::HashMap;
use std::fmt::Display;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::{borrow::Cow, fs::read_to_string, io};

use anyhow::{Context, Result, anyhow, bail};
#[cfg(not(target_os = "linux"))]
use arboard::Clipboard;
use genai::ModelSpec;
use genai::adapter::AdapterKind;
use genai::chat::ContentPart;
use syntect::highlighting::Theme;
use syntect::parsing::SyntaxSet;
use tokio::sync::mpsc;

use ratatui::{
    buffer::Buffer,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, ListState},
};
use ratatui_explorer::{File, FileExplorer, FileExplorerBuilder};
use ratatui_textarea::{TextArea, WrapMode};
use tiktoken_rs::cl100k_base;

use crate::config::ModelConfig;
use crate::models::{ModelItem, generate_model_spec, model_provider_from_spec};
use crate::ui::messages_to_lines;
use crate::{
    ai::MODELS,
    chats::ChatList,
    snippets::{EMBEDDED_THEME, SnippetItem, find_fenced_code_snippets, load_theme},
    storage::{
        create_db_conversation, delete_conversation, delete_message, get_cache_dir, insert_message,
        list_all_messages, list_conversations, touch_conversation,
    },
    ui::style_message,
};
use crate::{models::ModelList, snippets::SnippetList};

pub const RECACHE_COOLDOWN: u64 = 250;

/// Maximum number of conversations that may have an in-flight LLM stream at
/// once. Bounded to limit cost, memory, and provider rate limits.
pub const MAX_CONCURRENT_STREAMS: usize = 3;

/// Resolves the system prompt to send for a given model. OpenAI GPT models
/// get no system prompt (matching the original behaviour); everything else
/// uses the application's system prompt.
fn system_prompt_for_model(model: &ModelSpec, base: &str) -> Option<String> {
    match model {
        ModelSpec::Name(name) if name.starts_with("gpt") => None,
        ModelSpec::Iden(iden) if iden.adapter_kind == AdapterKind::OpenAI => None,
        _ => Some(base.to_string()),
    }
}

/// Per-conversation state for an in-flight (or about-to-start) LLM stream.
///
/// One entry exists in `App::streams` for each conversation that currently has
/// a pending/streaming request. It is removed when the stream completes, is
/// cancelled, or errors out.
#[derive(Debug, Clone)]
pub struct StreamState {
    /// `true` once the first response chunk has arrived.
    pub is_streaming: bool,
    /// `true` from submit until the first chunk arrives (request in flight but
    /// not yet streaming).
    pub is_waiting: bool,
    /// Accumulated partial assistant text, kept so it can be reattached to the
    /// view when the user browses away from and back to this conversation.
    pub partial: String,
    /// Cancel sender for the spawned streaming task. `None` once the task has
    /// been launched (and consumed by a cancel) or after the task finishes.
    pub cancel_tx: Option<mpsc::Sender<()>>,
    /// Snapshot of the conversation history sent with the request.
    pub messages: Vec<Message>,
    /// Model used for this request.
    pub selected_model: ModelSpec,
    /// Thinking effort used for this request.
    pub thinking_effort: ThinkingEffort,
    /// Resolved system prompt for this request.
    pub system_prompt: Option<String>,
}

/// Arguments needed to spawn a streaming task, extracted from a
/// [`StreamState`] so the task can own the data without borrowing `App`.
pub struct SpawnArgs {
    pub conversation_id: i64,
    pub messages: Vec<Message>,
    pub selected_model: ModelSpec,
    pub thinking_effort: ThinkingEffort,
    pub system_prompt: Option<String>,
    pub cancel_rx: mpsc::Receiver<()>,
}

/// Async actions reported back to the main event loop by background tasks.
#[derive(Debug, Clone)]
pub enum Action {
    StreamStart {
        conversation_id: i64,
    },
    StreamPartial {
        conversation_id: i64,
        content: String,
    },
    StreamComplete {
        conversation_id: i64,
        content: String,
    },
    StreamCancelled {
        conversation_id: i64,
        content: String,
    },
    Error {
        conversation_id: Option<i64>,
        message: String,
    },
    ModelsLoaded(Vec<(String, String)>),
    /// A single file was validated and its tokens estimated in the background.
    /// The file is added to the context with the estimated token count
    /// (`Some` for text files, `None` for recognized binary files).
    ContextFileAdded {
        file: File,
        est_tokens: Option<usize>,
    },
    /// Signals that a background context-add operation finished, switching
    /// the app to the given notification.
    ContextAddDone {
        notification: Notification,
    },
    /// An MCP server finished connecting and is ready for tool calls. The
    /// connection is moved into the app so it stays alive for the session.
    McpServerReady {
        id: String,
        display_name: String,
        tool_count: usize,
        bridge: mcp_genai_glue::McpToolBridge,
    },
    /// An MCP server failed to connect.
    McpServerFailed {
        id: String,
        display_name: String,
        error: String,
    },
    /// The user enabled a server in the management view; the main loop spawns
    /// a connect attempt for it. Carries the server id.
    McpEnableRequested {
        id: String,
    },
}

pub fn estimate_tokens(text: &str) -> AppResult<usize> {
    let bpe = cl100k_base()?;
    let base_count = bpe.encode_ordinary(text).len();
    Ok(base_count)
}

pub fn get_file_content(path: &PathBuf) -> io::Result<Cow<'_, str>> {
    read_to_string(path).map(Into::into)
}

/// Returns true for binary file types that are added to context as-is (no token
/// estimation is possible).
pub fn is_binary_file(name: &str) -> bool {
    [".pdf", ".jpg", ".png"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// Reads a file (if it is a text file) and estimates its token count.
///
/// Returns `Ok(Some(count))` for readable text files, `Ok(None)` for recognized
/// binary files (`pdf`/`jpg`/`png`), and `Err` for files that are neither valid
/// UTF-8 text nor a recognized binary type.
pub fn estimate_file_tokens(path: &PathBuf) -> AppResult<Option<usize>> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if is_binary_file(&name) {
        tracing::debug!(
            path = %path.display(),
            "skipped token estimation: recognized binary file"
        );
        return Ok(None);
    }
    match get_file_content(path) {
        Ok(content) => Ok(Some(estimate_tokens(content.as_ref())?)),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "skipped file during token estimation: could not read as UTF-8 text"
            );
            Err(anyhow!(
                "Could not read file \"{}\" as text: {}",
                path.display(),
                e
            ))
        }
    }
}

fn get_theme() -> ratatui_explorer::Theme {
    ratatui_explorer::Theme::default()
        .with_block(Block::default().borders(Borders::ALL))
        .with_dir_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .with_highlight_dir_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
                .bg(Color::DarkGray),
        )
        .with_scroll_padding(1)
}

#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub start: Option<(u16, u16)>, // (column, row)
    pub end: Option<(u16, u16)>,
}

impl Selection {
    pub fn get_selected_text(&self, buffer: &Buffer) -> Option<String> {
        // Need both start and end points to make a selection
        let (start, end) = match (self.start, self.end) {
            (Some(start), Some(end)) => (start, end),
            _ => return None,
        };

        // Calculate bounds (handles selection in any direction)
        let start_row = start.1.min(end.1);
        let end_row = start.1.max(end.1);
        let start_col = start.0.min(end.0);
        let end_col = start.0.max(end.0);

        let mut selected_text = String::new();

        for row in start_row..=end_row {
            // Add newline between rows, but not before first row
            if row > start_row {
                selected_text.push('\n');
            }

            for col in start_col..=end_col {
                let cell = buffer.cell((col, row));
                if let Some(cell) = cell {
                    selected_text.push_str(cell.symbol());
                }
            }
        }

        Some(selected_text)
    }

    pub fn iter_selected_cells(&self) -> Option<impl Iterator<Item = (u16, u16)>> {
        let (start, end) = match (self.start, self.end) {
            (Some(start), Some(end)) => (start, end),
            _ => return None,
        };

        let start_row = start.1.min(end.1);
        let end_row = start.1.max(end.1);
        let start_col = start.0.min(end.0);
        let end_col = start.0.max(end.0);

        Some(
            (start_row..=end_row)
                .flat_map(move |row| (start_col..=end_col).map(move |col| (col, row))),
        )
    }
}

#[derive(Debug, Clone)]
pub enum UserContent {
    Input,
    Context,
}

/**
 * A chat message.
 *
 * Assistant messages additionally carry the `model` and `provider` that
 * produced them (`None` for messages created before this was tracked, or
 * for synthetic placeholder messages).
 */
#[derive(Debug, Clone)]
pub enum Message {
    User(Vec<ContentPart>),
    Assistant(String, Option<String>, Option<String>),
}

#[derive(Debug, Clone)]
pub enum PartialMessage {
    Start,
    Continue(String),
    End,
}

pub fn partial_messages_to_string(partial_messages: Vec<PartialMessage>) -> String {
    let mut result = String::new();

    for message in partial_messages {
        match message {
            PartialMessage::Start => (), // Do nothing for Start
            PartialMessage::Continue(s) => result.push_str(&s),
            PartialMessage::End => (), // Do nothing for End
        }
    }

    result
}

impl From<String> for Message {
    fn from(message: String) -> Self {
        Message::User(vec![ContentPart::from_text(message)])
    }
}

impl From<&str> for Message {
    fn from(message: &str) -> Self {
        Message::User(vec![ContentPart::from_text(message)])
    }
}

impl Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Message::User(content) => {
                for part in content {
                    if let ContentPart::Text(text) = part {
                        write!(f, "{}", text)?;
                    }
                }
                Ok(())
            }
            Message::Assistant(text, _, _) => write!(f, "{}", text),
        }
    }
}

/// Application result type.
pub type AppResult<T> = Result<T>;

pub const THINKING_EFFORTS: [&str; 6] = ["None", "Low", "Medium", "High", "XHigh", "Max"];

#[derive(Debug, Clone, PartialEq, Default)]
pub enum ThinkingEffort {
    None,
    Low,
    #[default]
    Medium,
    High,
    XHigh,
    Max,
}

impl ThinkingEffort {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThinkingEffort::None => "None",
            ThinkingEffort::Low => "Low",
            ThinkingEffort::Medium => "Medium",
            ThinkingEffort::High => "High",
            ThinkingEffort::XHigh => "XHigh",
            ThinkingEffort::Max => "Max",
        }
    }

    pub fn from_index(i: usize) -> Self {
        match i {
            1 => ThinkingEffort::Low,
            2 => ThinkingEffort::Medium,
            3 => ThinkingEffort::High,
            4 => ThinkingEffort::XHigh,
            5 => ThinkingEffort::Max,
            _ => ThinkingEffort::None,
        }
    }

    pub fn to_index(&self) -> usize {
        match self {
            ThinkingEffort::None => 0,
            ThinkingEffort::Low => 1,
            ThinkingEffort::Medium => 2,
            ThinkingEffort::High => 3,
            ThinkingEffort::XHigh => 4,
            ThinkingEffort::Max => 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    Editing,
    ModelSelection,
    FilterModels,
    ThinkingEffortSelection,
    SnippetSelection,
    ShowHistory,
    FilterHistory,
    ExploreFiles,
    ShowContext,
    ServerManagement,
    Help,
    Notify { notification: Notification },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Notification {
    TokenEstimate((Option<usize>, String)),
    Info(String),
    Error(String),
}

#[derive(Debug, Clone, Copy)]
pub struct TerminalSize {
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone)]
pub struct ContextContent {
    pub file: File,
    pub est_tokens: Option<usize>,
}

/// App holds the state of the application
pub struct App<'a> {
    /// Input text area
    pub input_textarea: TextArea<'a>,
    /// Position of cursor in the editor area.
    pub app_mode: AppMode,
    /// Conversation ID for chat database.
    pub conversation_id: Option<i64>,
    /// In-flight (pending or streaming) conversations, keyed by conversation id.
    /// A conversation appears here from `submit_message` until the stream
    /// completes, is cancelled, or errors.
    pub streams: HashMap<i64, StreamState>,
    /// System prompt
    pub system_prompt: &'a str,
    /// History of recorded messages
    pub messages: Vec<Message>,
    /// Vertical scroll
    pub vertical_scroll: usize,
    /// Help text scroll
    pub help_scroll: usize,
    /// Is the application running?
    pub running: bool,
    /// System clipboard.
    /// Not enabled on Linux because of an issue with the `arboard` crate,
    /// see <https://github.com/1Password/arboard/issues/153>
    #[cfg(not(target_os = "linux"))]
    pub clipboard: Clipboard,
    /// List of models
    pub model_list: ModelList,
    /// Selected model
    pub selected_model: ModelSpec,
    /// Discovered snippets
    pub snippet_list: SnippetList,
    /// List of chats
    pub chat_list: ChatList,
    /// Selected text
    pub selection: Selection,
    /// Highlighting theme index
    pub theme_index: usize,
    /// Highlighting theme
    pub theme: Theme,
    /// Terminal size
    pub size: Option<TerminalSize>,
    /// Cached highlighted lines
    pub cached_lines: Vec<Line<'a>>,
    /// Does the app need to recache the syntax highlighting?
    pub needs_recache: bool,
    /// Time of last recaching of syntax highlighting
    pub last_recache: Instant,
    /// Spinner animation frame counter
    pub spinner_frame: usize,
    /// File explorer
    pub file_explorer: FileExplorer,
    /// Current context
    pub current_context: Option<Vec<ContextContent>>,
    /// Search bar.
    pub search_bar: TextArea<'a>,
    /// Toggle for syntax highlighting.
    pub do_highlight: bool,
    /// Selected thinking effort
    pub thinking_effort: ThinkingEffort,
    /// List state for thinking effort selection
    pub thinking_effort_state: ListState,
    /// Is the app loading available models?
    pub is_loading_models: bool,
    /// Per-server status for the MCP footer (connecting/ready/failed), one
    /// entry per `enabled` server in the config. Drives the status counts.
    pub mcp_statuses: Vec<crate::mcp::McpServerStatus>,
    /// Bridges of currently-connected MCP servers. Snapshot by streaming
    /// tasks so they can resolve/execute tools.
    pub mcp_bridges: Vec<mcp_genai_glue::McpToolBridge>,
    /// Owning `McpConnection`s, keyed by server id. Keeping these alive keeps
    /// the underlying `RunningService` (transport) running. Removed when the
    /// user disables a server.
    pub mcp_connections: std::collections::HashMap<String, crate::mcp::McpConnection>,
    /// User intent for this session: which server ids should be connected.
    /// Initialized from config (`enabled = true`); toggled in the server
    /// management view. Independent of live connection status.
    pub mcp_enabled: std::collections::HashSet<String>,
    /// List selection state for the server management view.
    pub mcp_server_state: ratatui::widgets::ListState,
}

pub fn styled_textarea(title: &'static str) -> TextArea<'static> {
    let mut input_textarea = TextArea::default();
    input_textarea.set_block(Block::bordered().title(title));
    input_textarea.set_style(Style::default().fg(Color::Yellow));
    input_textarea.set_cursor_line_style(Style::default().not_underlined());
    input_textarea.set_cursor_style(Style::default().bg(Color::DarkGray));
    input_textarea.set_placeholder_text("Start typing...");
    input_textarea.set_placeholder_style(Style::default().fg(Color::DarkGray).italic().dim());
    input_textarea.set_wrap_mode(WrapMode::WordOrGlyph);
    input_textarea
}

impl Default for App<'_> {
    fn default() -> Self {
        Self {
            input_textarea: styled_textarea("Input"),
            app_mode: AppMode::Normal,
            system_prompt: "You are a helpful, friendly assistant.",
            conversation_id: None,
            streams: HashMap::new(),
            messages: Vec::new(),
            // user_messages: Vec::new(),
            // assistant_messages: Vec::new(),
            vertical_scroll: 0,
            help_scroll: 0,
            running: true,
            #[cfg(not(target_os = "linux"))]
            clipboard: Clipboard::new().unwrap(),
            model_list: ModelList::from_iter(MODELS.map(|(provider, model)| {
                if model == "gemini-3.1-pro-preview" {
                    (provider, model, true)
                } else {
                    (provider, model, false)
                }
            })),
            selected_model: "gemini-3.1-pro-preview".into(),
            snippet_list: SnippetList::from_iter([].iter().map(|&snippet| (snippet, false, None))),
            chat_list: ChatList::from_iter([].iter().map(|&chat| (chat, "".to_string(), false))),
            selection: Selection::default(),
            theme_index: 0,
            theme: load_theme(0),
            size: None,
            cached_lines: Vec::new(),
            needs_recache: false,
            last_recache: Instant::now() - Duration::from_secs(1),
            spinner_frame: 0,
            file_explorer: FileExplorerBuilder::default()
                .show_hidden(true)
                .theme(get_theme())
                .build()
                .expect("Could not construct file explorer."),
            current_context: None,
            search_bar: styled_textarea("Search"),
            do_highlight: true,
            thinking_effort: ThinkingEffort::Medium,
            thinking_effort_state: {
                let mut s = ListState::default();
                s.select_first();
                s
            },
            is_loading_models: true,
            mcp_statuses: Vec::new(),
            mcp_bridges: Vec::new(),
            mcp_connections: std::collections::HashMap::new(),
            mcp_enabled: std::collections::HashSet::new(),
            mcp_server_state: ratatui::widgets::ListState::default(),
        }
    }
}

impl<'a> App<'a> {
    pub fn new(system_prompt: &'a str, default_model: ModelConfig) -> Self {
        let model_list = ModelList::from_iter(MODELS.map(|(provider, name)| {
            if name == default_model.name {
                (provider, name, true)
            } else {
                (provider, name, false)
            }
        }));
        Self {
            system_prompt,
            selected_model: generate_model_spec(
                default_model.name.as_str(),
                default_model.provider.as_str(),
            ),
            model_list,
            ..Default::default()
        }
    }

    /// Handles the tick event of the terminal.
    pub fn tick(&mut self) {
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
    }

    pub fn set_app_mode(&mut self, new_app_mode: AppMode) {
        self.app_mode = new_app_mode;
    }

    /// Returns true if the conversation currently being viewed has a stream
    /// that is actively producing chunks. Streaming-related UI (live partial,
    /// non-cached rendering) should only be shown when this is true.
    pub fn is_view_streaming(&self) -> bool {
        self.conversation_id
            .is_some_and(|id| self.streams.get(&id).is_some_and(|s| s.is_streaming))
    }

    /// Returns true if the current view should show the "waiting for response"
    /// indicator (the request is in flight but no chunks have arrived yet).
    pub fn is_view_waiting(&self) -> bool {
        self.conversation_id
            .is_some_and(|id| self.streams.get(&id).is_some_and(|s| s.is_waiting))
    }

    /// Returns true if the currently viewed conversation has any in-flight
    /// stream (either waiting for the first chunk or actively streaming).
    pub fn current_has_stream(&self) -> bool {
        self.conversation_id
            .is_some_and(|id| self.streams.contains_key(&id))
    }

    /// Number of conversations with an in-flight stream.
    pub fn active_stream_count(&self) -> usize {
        self.streams.len()
    }

    // --- MCP status ---

    /// Mark an MCP server as connected, store its owning connection, and
    /// register its bridge for tool calls. Replaces any prior status entry
    /// with the same id.
    ///
    /// `tool_count` is computed by the caller (it requires an async query);
    /// we keep this method sync since `App` methods run on the sync UI loop.
    pub fn mcp_server_ready(
        &mut self,
        conn: crate::mcp::McpConnection,
        tool_count: usize,
    ) {
        let id = conn.id.clone();
        let display_name = conn.display_name.clone();
        tracing::info!(mcp.server = %id, tools = tool_count, "MCP server ready");
        self.update_mcp_status(crate::mcp::McpServerStatus::Ready {
            id: id.clone(),
            display_name,
            tool_count,
        });
        // Replace any prior connection for this id (avoids duplicate bridges
        // on reconnect), then rebuild the bridges list.
        self.mcp_connections.insert(id, conn);
        self.mcp_bridges = self
            .mcp_connections
            .values()
            .map(|c| c.bridge.clone())
            .collect();
    }

    /// Mark an MCP server as failed. Replaces any prior status entry with the
    /// same id.
    pub fn mcp_server_failed(&mut self, id: String, display_name: String, error: String) {
        tracing::warn!(mcp.server = %id, error = %error, "MCP server failed to connect");
        self.update_mcp_status(crate::mcp::McpServerStatus::Failed {
            id,
            display_name,
            error,
        });
    }

    /// Replace the status entry matching `id`, or push a new one.
    fn update_mcp_status(&mut self, status: crate::mcp::McpServerStatus) {
        let id = status.id().to_string();
        if let Some(slot) = self.mcp_statuses.iter_mut().find(|s| s.id() == id) {
            *slot = status;
        } else {
            self.mcp_statuses.push(status);
        }
    }

    /// Counts of MCP servers by state, for the footer summary. Returns
    /// `(ready, ready_tool_total, connecting, failed)`.
    pub fn mcp_status_counts(&self) -> (usize, usize, usize, usize) {
        let mut ready = 0;
        let mut ready_tools = 0;
        let mut connecting = 0;
        let mut failed = 0;
        for s in &self.mcp_statuses {
            match s {
                crate::mcp::McpServerStatus::Ready { tool_count, .. } => {
                    ready += 1;
                    ready_tools += *tool_count;
                }
                crate::mcp::McpServerStatus::Connecting { .. } => connecting += 1,
                crate::mcp::McpServerStatus::Failed { .. } => failed += 1,
                // Disabled servers are omitted from the footer counts.
                crate::mcp::McpServerStatus::Disabled { .. } => {}
            }
        }
        (ready, ready_tools, connecting, failed)
    }

    // --- Server management (enable/disable) ---

    /// User intent: should this server be connected right now?
    pub fn mcp_is_enabled(&self, id: &str) -> bool {
        self.mcp_enabled.contains(id)
    }

    /// Disconnect a server: drop its owning connection (cancels the
    /// transport), remove its bridge, and mark its status `Disabled`.
    pub fn mcp_disable(&mut self, id: &str) {
        if !self.mcp_enabled.remove(id) {
            // Already disabled; nothing to do.
            return;
        }
        // Drop the owning connection (cancels the running service).
        self.mcp_connections.remove(id);
        // Rebuild the bridges list from remaining connections (the one we
        // just removed is gone, so its bridge drops out too).
        self.mcp_bridges = self
            .mcp_connections
            .values()
            .map(|c| c.bridge.clone())
            .collect();
        // Update status to Disabled, preserving the display name.
        let display_name = self
            .mcp_statuses
            .iter()
            .find(|s| s.id() == id)
            .map(|s| s.display_name().to_string())
            .unwrap_or_else(|| id.to_string());
        self.update_mcp_status(crate::mcp::McpServerStatus::Disabled {
            id: id.to_string(),
            display_name,
        });
        tracing::info!(mcp.server = id, "MCP server disabled");
    }

    /// Mark a server as user-enabled and set its status to `Connecting`.
    /// The caller is responsible for spawning the actual connect attempt.
    pub fn mcp_enable(&mut self, id: &str) {
        if self.mcp_enabled.insert(id.to_string()) {
            let display_name = self
                .mcp_statuses
                .iter()
                .find(|s| s.id() == id)
                .map(|s| s.display_name().to_string())
                .unwrap_or_else(|| id.to_string());
            self.update_mcp_status(crate::mcp::McpServerStatus::Connecting {
                id: id.to_string(),
                display_name,
            });
            tracing::info!(mcp.server = id, "MCP server enabled");
        }
    }

    /// The server ids in the management list order (sorted, matching
    /// `mcp_statuses`).
    pub fn mcp_server_ids(&self) -> Vec<String> {
        self.mcp_statuses
            .iter()
            .map(|s| s.id().to_string())
            .collect()
    }

    /// The currently selected server id in the management view, if any.
    pub fn mcp_selected_id(&self) -> Option<String> {
        let idx = self.mcp_server_state.selected()?;
        self.mcp_server_ids().get(idx).cloned()
    }

    /// Cancels the in-flight stream for the currently viewed conversation, if
    /// any. The streaming task will emit a `StreamCancelled` action with
    /// whatever partial content was produced.
    pub fn cancel_current_stream(&mut self) {
        if let Some(id) = self.conversation_id
            && let Some(state) = self.streams.get_mut(&id)
            && let Some(tx) = state.cancel_tx.take()
        {
            let _ = tx.try_send(());
        }
    }

    /// Extracts the set of streams that have been submitted but not yet
    /// spawned into a streaming task. For each, a cancel channel is created
    /// and stored on the stream state; the returned [`SpawnArgs`] carry the
    /// receiving end plus the request data needed to spawn the task.
    pub fn take_pending_spawns(&mut self) -> Vec<SpawnArgs> {
        let mut pending = Vec::new();
        for (&conv_id, state) in self.streams.iter_mut() {
            if state.is_waiting && state.cancel_tx.is_none() {
                let (cancel_tx, cancel_rx) = mpsc::channel::<()>(1);
                state.cancel_tx = Some(cancel_tx);
                pending.push(SpawnArgs {
                    conversation_id: conv_id,
                    messages: state.messages.clone(),
                    selected_model: state.selected_model.clone(),
                    thinking_effort: state.thinking_effort.clone(),
                    system_prompt: state.system_prompt.clone(),
                    cancel_rx,
                });
            }
        }
        pending
    }

    pub fn create_conversation(&mut self) -> AppResult<i64> {
        let conv_id = create_db_conversation(self.system_prompt)
            .context("Failed to create conversation in db")?;
        self.conversation_id = Some(conv_id);
        Ok(conv_id)
    }

    pub fn set_terminal_size(&mut self, width: u16, height: u16) {
        self.size = Some(TerminalSize { width, height });
    }

    pub fn add_cached_lines(&mut self, message: Message) {
        if let Some(TerminalSize { width, height: _ }) = self.size {
            // line width inside the chat block: terminal width minus the outer
            // layout margins (2) and the chat block borders (2).
            self.cached_lines.extend(style_message(
                message,
                width.saturating_sub(4) as usize,
                self.theme.clone(),
            ));
        }
    }

    /// Returns an estimate for token usage of all messages sent and receved from the LLM.
    pub fn estimate_messages_tokens(&self) -> usize {
        let count: usize = self
            .messages
            .iter()
            .map(|m| estimate_tokens(&m.to_string()).unwrap_or(0))
            .sum();
        count
    }

    /// Estimate the tokens for the provided text.
    pub fn estimate_tokens(&self, text: &str) -> usize {
        estimate_tokens(text).unwrap_or(0)
    }

    pub fn next_theme(&mut self) {
        if self.theme_index == EMBEDDED_THEME.len() - 1 {
            self.theme_index = 0;
        } else {
            self.theme_index += 1;
        }
        self.theme = load_theme(self.theme_index);
    }

    pub fn previous_theme(&mut self) {
        if self.theme_index == 0 {
            self.theme_index = EMBEDDED_THEME.len() - 1;
        } else {
            self.theme_index -= 1;
        }
        self.theme = load_theme(self.theme_index);
    }

    pub fn toggle_highlighting(&mut self) {
        self.do_highlight = !self.do_highlight;
    }

    pub fn recache_lines(&mut self, messages: Vec<Message>) {
        self.cached_lines.clear();
        if let Some(TerminalSize { width, height: _ }) = self.size {
            for message in messages {
                self.cached_lines.extend(style_message(
                    message,
                    width.saturating_sub(4) as usize,
                    self.theme.clone(),
                ));
            }
        }
    }

    fn write_chat_log(&self) -> AppResult<()> {
        let mut chat_log = String::new();
        for message in self.messages.iter() {
            match message {
                Message::User(_) => {
                    chat_log.push_str(&format!("User: {}\n", message));
                }
                Message::Assistant(message, model, provider) => {
                    let model = model.as_deref().unwrap_or("unknown");
                    let provider = provider.as_deref().unwrap_or("unknown");
                    chat_log.push_str(&format!("Assistant ({model} -- {provider}): {message}\n"));
                }
            }
        }
        let cache_dir = get_cache_dir()?;
        fs::create_dir_all(&cache_dir).context("Could not create cache directory")?;
        let mut path = cache_dir;
        path.push("latest-chat.log");
        fs::write(&path, chat_log).context("Unable to write chat log")?;
        Ok(())
    }

    pub fn add_to_context(&mut self, new_context: File, est_tokens: Option<usize>) {
        if let Some(mut current_context) = self.current_context.clone() {
            if !current_context
                .iter()
                .map(|c| c.file.to_owned())
                .collect::<Vec<File>>()
                .contains(&new_context)
            {
                current_context.push(ContextContent {
                    file: new_context,
                    est_tokens,
                });
                self.current_context = Some(current_context)
            }
        } else {
            self.current_context = Some(vec![ContextContent {
                file: new_context,
                est_tokens,
            }]);
        }
    }

    pub fn remove_from_context(&mut self, context: &File) {
        if let Some(mut current_context) = self.current_context.clone()
            && let Some(idx) = current_context.iter().position(|f| &f.file == context)
        {
            current_context.remove(idx);
            self.current_context = Some(current_context)
        };
    }

    fn get_max_scroll(&self) -> AppResult<usize> {
        let TerminalSize { width, height } =
            self.size.ok_or(anyhow!("Could not get terminal size"))?;
        // Bubble lines are pre-wrapped to fit the chat block, and the chat
        // paragraph is rendered without wrapping, so the line count is simply
        // the number of generated lines.
        let total_lines = if !self.is_view_streaming() && self.do_highlight {
            self.cached_lines.len()
        } else {
            messages_to_lines(&self.messages, width.saturating_sub(4) as usize).len()
        };
        let sub = if self.is_view_streaming() {
            (height - 4) as usize
        } else if self.is_view_waiting() {
            (height - 8) as usize
        } else {
            2
        };
        Ok(total_lines.saturating_sub(sub))
    }

    pub fn increment_vertical_scroll(&mut self) -> AppResult<()> {
        let max_scroll = self.get_max_scroll().context("Unable to get max scroll")?;
        if self.vertical_scroll < max_scroll {
            self.vertical_scroll += 1;
        }
        Ok(())
    }

    pub fn decrement_vertical_scroll(&mut self) -> AppResult<()> {
        self.vertical_scroll = self.vertical_scroll.saturating_sub(1);
        Ok(())
    }

    pub fn scroll_to_top(&mut self) {
        self.vertical_scroll = 0;
    }

    pub fn scroll_to_bottom(&mut self) -> AppResult<()> {
        self.vertical_scroll = self.get_max_scroll().context("Unable to get max scroll")?;
        Ok(())
    }

    pub fn increment_help_scroll(&mut self, max_scroll: usize) {
        if self.help_scroll < max_scroll {
            self.help_scroll += 1;
        }
    }

    pub fn decrement_help_scroll(&mut self) {
        self.help_scroll = self.help_scroll.saturating_sub(1);
    }

    pub fn reset_help_scroll(&mut self) {
        self.help_scroll = 0;
    }

    pub fn submit_message(&mut self) -> AppResult<()> {
        let text = self.input_textarea.lines().join("\n");
        if text.is_empty() {
            return Ok(());
        }
        if self.current_has_stream() {
            bail!("A response is already in flight for this chat.");
        }
        if self.streams.len() >= MAX_CONCURRENT_STREAMS {
            bail!(
                "Too many concurrent streams (max {}). Wait for one to finish.",
                MAX_CONCURRENT_STREAMS
            );
        }
        let mut content_parts = Vec::new();
        if let Some(context) = &self.current_context {
            let ps = SyntaxSet::load_defaults_newlines();
            let additional_context = "<context>\nINFO FOR LLMs\nThe user provided the following context, please use it (if relevant) when providing an answer:".to_string();
            content_parts.push(ContentPart::from_text(additional_context));
            for c in context {
                let extension = if let Some((_, extension)) = c.file.name.split_once(".") {
                    extension
                } else {
                    ""
                };
                match extension {
                    "pdf" | "jpg" | "png" => {
                        content_parts.push(ContentPart::from_text(format!(
                            "\n---\nFile name: {}\nContent:\n<binary file>",
                            &c.file.name
                        )));
                        content_parts.push(ContentPart::from_binary_file(c.file.path.clone())?);
                    }
                    _ => {
                        let syntax_name =
                            if let Some(syntax) = ps.find_syntax_by_extension(extension) {
                                syntax.name.to_string()
                            } else {
                                "Plain Text".to_string()
                            };
                        if c.file.is_file() {
                            let context_str = get_file_content(&c.file.path)?;
                            content_parts.push(ContentPart::from_text(format!(
                                "\n---\nFile name: {}\nContent:\n```{}\n{}\n```",
                                &c.file.name,
                                syntax_name.to_lowercase(),
                                context_str
                            )));
                        }
                    }
                }
            }
            self.current_context = None;
            content_parts.push(ContentPart::from_text("\n</context>\n"));
        }
        content_parts.push(ContentPart::from_text(&text));
        let n_user_messages = self
            .messages
            .iter()
            .filter(|m| matches!(m, Message::User(_)))
            .count();
        let n_assistant_messages = self
            .messages
            .iter()
            .filter(|m| matches!(m, Message::Assistant(..)))
            .count();
        if n_user_messages != n_assistant_messages {
            return Ok(());
        }

        self.reset_input_textarea();
        self.set_app_mode(AppMode::Normal);
        self.write_chat_log()
            .context("Unable to write submitted message to chat log")?;
        let message = Message::User(content_parts);
        if let Some(id) = self.conversation_id {
            insert_message(id, &message)?;
        } else {
            let id = self.create_conversation()?;
            insert_message(id, &message)?;
        }
        // Record an in-flight stream for this conversation so the upcoming
        // response is routed here, and so the spawn loop can launch it.
        let conv_id = self
            .conversation_id
            .ok_or_else(|| anyhow!("No conversation id after submit"))?;
        self.add_cached_lines(message.clone());
        self.messages.push(message);
        // Record an in-flight stream for this conversation so the upcoming
        // response is routed here, and so the spawn loop can launch it. The
        // message history (including the message just submitted) is snapshotted
        // here so the streaming task is independent of the current view.
        self.streams.insert(
            conv_id,
            StreamState {
                is_streaming: false,
                is_waiting: true,
                partial: String::new(),
                cancel_tx: None,
                messages: self.messages.clone(),
                selected_model: self.selected_model.clone(),
                thinking_effort: self.thinking_effort.clone(),
                system_prompt: system_prompt_for_model(&self.selected_model, self.system_prompt),
            },
        );

        self.scroll_to_bottom()
            .context("Scrolling to bottom failed.")?;

        Ok(())
    }

    pub fn set_models(&mut self, models: Vec<(String, String)>) {
        self.model_list = ModelList::from_iter(models.into_iter().map(|(provider, model)| {
            if model == "gpt-4o-mini" {
                (provider, model, true)
            } else {
                (provider, model, false)
            }
        }));
    }

    pub async fn receive_message(
        &mut self,
        conversation_id: i64,
        message: Message,
    ) -> AppResult<()> {
        // The stream is resolved; drop its state. If the state is already gone
        // (e.g. the conversation was deleted mid-stream), there is nothing to
        // persist and nothing to update in the view.
        let stream_state = match self.streams.remove(&conversation_id) {
            Some(state) => state,
            None => return Ok(()),
        };

        // Record which model produced this assistant message, taken from the
        // in-flight stream state (the source of truth for the request that
        // just completed). The incoming message from the streaming action
        // carries no model info, so we attach it here before persisting /
        // updating the view.
        let message = match message {
            Message::Assistant(text, model, provider) => {
                let (m, p) = model_provider_from_spec(&stream_state.selected_model);
                Message::Assistant(text, model.or(m), provider.or(p))
            }
            other => other,
        };

        let is_current = self.conversation_id == Some(conversation_id);

        // Only mutate the in-memory view when the user is still looking at the
        // conversation that the stream belongs to. Otherwise persist silently
        // to the database; the message will appear when the user reopens that
        // chat (which reloads from the database).
        if is_current {
            if let Some(Message::Assistant(..)) = self.messages.last() {
                self.messages.pop();
            }

            let message_content = message.to_string();
            let discovered_snippets = find_fenced_code_snippets(
                message_content.split('\n').map(|s| s.to_string()).collect(),
            );
            let snippet_items: Vec<SnippetItem> = discovered_snippets
                .into_iter()
                .map(|snippet| snippet.into())
                .collect();
            self.snippet_list.items.extend(snippet_items);

            self.write_chat_log()
                .context("Unable to write received message to chat log")?;
        }

        insert_message(conversation_id, &message)?;
        touch_conversation(conversation_id)?;

        if is_current {
            self.add_cached_lines(message.clone());
            self.messages.push(message);
            // The view was rendering the live partial from `messages_to_lines`
            // during streaming; force a rebuild of the cached (highlighted)
            // lines so it matches the final message list.
            self.needs_recache = true;
        }

        Ok(())
    }

    pub async fn receive_incomplete_message(
        &mut self,
        conversation_id: i64,
        captured_content: &str,
    ) -> AppResult<()> {
        let is_current = self.conversation_id == Some(conversation_id);

        // Update the per-conversation partial buffer. If the stream state is
        // gone (e.g. conversation deleted), ignore the chunk entirely.
        //
        // An empty `captured_content` corresponds to the `StreamStart` signal
        // (the request has been sent but no chunks have arrived yet). We must
        // NOT flip `is_waiting` -> `is_streaming` here, otherwise the
        // "Processing user query..." spinner disappears before any real
        // content arrives. The transition happens only on the first non-empty
        // chunk.
        let mut just_started = false;
        if let Some(state) = self.streams.get_mut(&conversation_id) {
            if !captured_content.is_empty() {
                if !state.is_streaming {
                    // First real chunk: waiting -> streaming transition.
                    just_started = true;
                }
                state.is_waiting = false;
                state.is_streaming = true;
                state.partial = captured_content.to_string();
            }
        } else {
            return Ok(());
        }

        // Only touch the in-memory view for the conversation the user is
        // currently looking at, and only once there is actual content to
        // show (the waiting bubble covers the no-content-yet phase).
        if is_current && !captured_content.is_empty() {
            // On the first chunk we force a scroll to the bottom: the
            // max-scroll formula changes between the waiting and streaming
            // phases, so the "are we already at the bottom" check would
            // otherwise fail and auto-scroll would never latch on. On
            // subsequent chunks we only follow if the user is already at the
            // bottom.
            let do_scroll = just_started
                || self.vertical_scroll
                    == self.get_max_scroll().context("Could not get max scroll.")?;
            // Ensure there is an assistant message at the tail to update. This
            // is also needed when the user browses away and back to the
            // streaming conversation mid-stream (the partial message would
            // have been dropped when reloading from the database).
            if !matches!(self.messages.last(), Some(Message::Assistant(..))) {
                // Seed a placeholder assistant message carrying the model /
                // provider from the in-flight stream so the bubble header
                // shows which model is responding even mid-stream.
                let (model, provider) = match self.streams.get(&conversation_id) {
                    Some(s) => model_provider_from_spec(&s.selected_model),
                    None => (None, None),
                };
                self.messages
                    .push(Message::Assistant(String::new(), model, provider));
            }
            if let Some(Message::Assistant(last, _, _)) = self.messages.last_mut() {
                *last = captured_content.to_string();
            }
            if do_scroll {
                self.scroll_to_bottom()
                    .context("Could not set max scroll in incomplete message.")?;
            }
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn paste_to_input_textarea(&mut self) {
        if let Ok(clipboard_content) = self.clipboard.get_text() {
            self.input_textarea.insert_str(clipboard_content);
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn yank_latest_assistant_message(&mut self) {
        let mut assistant_messages = self.messages.iter().filter_map(|m| match m {
            Message::Assistant(message, _, _) => Some(message),
            _ => None,
        });
        if let Some(message) = assistant_messages.next_back() {
            self.clipboard.set_text(message).unwrap();
        }
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn select_no_model(&mut self) {
        self.model_list.state.select(None);
    }

    pub fn select_next_model(&mut self) {
        self.model_list.state.select_next();
    }
    pub fn select_previous_model(&mut self) {
        self.model_list.state.select_previous();
    }

    pub fn select_first_model(&mut self) {
        self.model_list.state.select_first();
    }

    pub fn select_last_model(&mut self) {
        self.model_list.state.select_last();
    }

    pub fn select_next_thinking_effort(&mut self) {
        self.thinking_effort_state.select_next();
    }

    pub fn select_previous_thinking_effort(&mut self) {
        self.thinking_effort_state.select_previous();
    }

    pub fn select_first_thinking_effort(&mut self) {
        self.thinking_effort_state.select_first();
    }

    pub fn select_last_thinking_effort(&mut self) {
        self.thinking_effort_state
            .select(Some(THINKING_EFFORTS.len() - 1));
    }

    pub fn set_thinking_effort(&mut self) {
        if let Some(i) = self.thinking_effort_state.selected() {
            self.thinking_effort = ThinkingEffort::from_index(i);
        }
    }

    /// Returns indices into `model_list.items` that match the current search bar
    /// query. When the query is empty, returns all indices.
    pub fn filtered_model_indices(&self) -> Vec<usize> {
        let query = self.search_bar.lines().first().cloned().unwrap_or_default();
        let query_lower = query.to_lowercase();
        if query_lower.is_empty() {
            return (0..self.model_list.items.len()).collect();
        }
        self.model_list
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                format!("{}: {}", item.provider, item.name)
                    .to_lowercase()
                    .contains(&query_lower)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Changes the status of the selected list item
    pub fn set_model(&mut self) {
        let indices = self.filtered_model_indices();
        if let Some(sel) = self.model_list.state.selected()
            && let Some(&actual_idx) = indices.get(sel)
        {
            for item in self.model_list.items.iter_mut() {
                item.selected = false;
            }
            self.model_list.items[actual_idx].selected = true;
            let ModelItem { provider, name, .. } = self.model_list.items[actual_idx].clone();
            let model_spec = generate_model_spec(name.as_str(), provider.as_str());
            self.selected_model = model_spec;
        }
    }

    pub fn select_no_snippet(&mut self) {
        self.snippet_list.state.select(None);
    }

    pub fn select_next_snippet(&mut self) {
        self.snippet_list.state.select_next();
    }
    pub fn select_previous_snippet(&mut self) {
        self.snippet_list.state.select_previous();
    }

    pub fn select_first_snippet(&mut self) {
        self.snippet_list.state.select_first();
    }

    pub fn select_last_snippet(&mut self) {
        self.snippet_list.state.select_last();
    }

    pub fn get_snippet(&self) -> Option<&SnippetItem> {
        self.snippet_list
            .state
            .selected()
            .map(|i| &self.snippet_list.items[i])
    }

    #[cfg(not(target_os = "linux"))]
    /// Copy the selected snippet to the clipboard.
    pub fn copy_snippet(&mut self) -> AppResult<()> {
        if let Some(i) = self.snippet_list.state.selected() {
            for item in self.snippet_list.items.iter_mut() {
                item.selected = false;
            }
            self.snippet_list.items[i].selected = true;
            self.clipboard
                .set_text(&self.snippet_list.items[i].text)
                .context("Unable to copy snippet to clipboard")?;
        }
        Ok(())
    }

    pub fn select_no_chat(&mut self) {
        self.chat_list.state.select(None);
    }

    pub fn select_next_chat(&mut self) {
        self.chat_list.state.select_next();
    }
    pub fn select_previous_chat(&mut self) {
        self.chat_list.state.select_previous();
    }

    pub fn select_first_chat(&mut self) {
        self.chat_list.state.select_first();
    }

    pub fn select_last_chat(&mut self) {
        self.chat_list.state.select_last();
    }

    pub fn set_chat_list(&mut self, query_filter: Option<String>) -> AppResult<()> {
        let chats = list_conversations(query_filter)?;
        let chats = chats
            .into_iter()
            .map(|(id, started_at)| {
                if Some(&id) == self.get_selected_chat_id() {
                    (id, started_at, true)
                } else {
                    (id, started_at, false)
                }
            })
            .collect::<Vec<(i64, String, bool)>>();
        self.chat_list = ChatList::from_iter(chats);
        Ok(())
    }

    pub fn delete_selected_chat(&mut self) -> AppResult<()> {
        if let Some(i) = self.chat_list.state.selected() {
            let chat_id = self.chat_list.items[i].chat_id;
            // Cancel any in-flight stream for this conversation and discard its
            // state so late actions from the task are ignored (the conversation
            // will no longer exist in the database).
            if let Some(state) = self.streams.remove(&chat_id)
                && let Some(tx) = state.cancel_tx
            {
                let _ = tx.try_send(());
            }
            delete_conversation(chat_id)?;
            self.chat_list.items.remove(i);
            let new_chat_index = if i >= self.chat_list.items.len() {
                i - 1
            } else {
                i
            };
            self.chat_list.items[new_chat_index].selected = true;
            self.chat_list.state.select(Some(new_chat_index));
            let new_chat_id = self.chat_list.items[new_chat_index].chat_id;
            self.messages.clear();
            self.cached_lines.clear();
            self.messages = list_all_messages(new_chat_id)?;
            self.conversation_id = Some(new_chat_id);
            self.needs_recache = true;
        }
        Ok(())
    }

    pub fn delete_chat_by_id(&mut self, id: i64) -> AppResult<()> {
        if let Some(state) = self.streams.remove(&id)
            && let Some(tx) = state.cancel_tx
        {
            let _ = tx.try_send(());
        }
        delete_conversation(id)?;
        Ok(())
    }

    pub fn new_chat(&mut self) {
        if !self.messages.is_empty() {
            self.messages.clear();
            self.cached_lines.clear();
            self.conversation_id = None;
            self.snippet_list = SnippetList::new();
            // NOTE: `streams` is intentionally left untouched here. It tracks
            // in-flight requests that may belong to other conversations; a
            // freshly created (empty) chat has no stream of its own. The view
            // only shows streaming state for the conversation that owns a
            // stream (see `is_view_streaming` / `is_view_waiting`).
        }
    }

    pub fn reset_searchbar(&mut self) {
        self.search_bar = styled_textarea("Search")
    }

    pub fn reset_input_textarea(&mut self) {
        self.input_textarea = styled_textarea("Input")
    }

    pub fn redo_last_message(&mut self) -> AppResult<()> {
        while let Some(m) = self.messages.pop() {
            if let Some(chat_id) = self.conversation_id {
                delete_message(chat_id, &m)?;
            }
            match m {
                Message::User(_) => {
                    self.reset_input_textarea();
                    let message_text = m.to_string();
                    // TODO: A bit fugly, should be a better way to do this.
                    if let Some((_, user_input)) = message_text.split_once("\n</context>\n") {
                        self.input_textarea.insert_str(user_input);
                    } else {
                        self.input_textarea.insert_str(m.to_string());
                    }
                    break;
                }
                _ => {
                    continue;
                }
            }
        }
        self.needs_recache = true;

        // Clear snippet list and find fenced code snippets
        self.snippet_list.clear();
        for message in self.messages.iter() {
            let message_content = message.to_string();
            let discovered_snippets = find_fenced_code_snippets(
                message_content.split('\n').map(|s| s.to_string()).collect(),
            );
            let snippet_items: Vec<SnippetItem> = discovered_snippets
                .into_iter()
                .map(|snippet| snippet.into())
                .collect();
            self.snippet_list.items.extend(snippet_items);
        }
        Ok(())
    }

    pub fn get_selected_chat_id(&self) -> Option<&i64> {
        if self.chat_list.items.is_empty() {
            return None;
        }
        self.chat_list
            .state
            .selected()
            .map(|i| &self.chat_list.items[i].chat_id)
    }

    pub fn set_chat(&mut self) -> AppResult<()> {
        if let Some(i) = self.chat_list.state.selected() {
            self.reset_searchbar();
            for item in self.chat_list.items.iter_mut() {
                item.selected = false;
            }
            self.chat_list.items[i].selected = true;
            let conv_id = self.chat_list.items[i].chat_id;
            self.conversation_id = Some(conv_id);
            self.messages.clear();
            self.messages = list_all_messages(conv_id)?;
            self.snippet_list.clear();
            for message in self.messages.iter_mut() {
                let message_content = message.to_string();
                let discovered_snippets = find_fenced_code_snippets(
                    message_content.split('\n').map(|s| s.to_string()).collect(),
                );
                let snippet_items: Vec<SnippetItem> = discovered_snippets
                    .into_iter()
                    .map(|snippet| snippet.into())
                    .collect();
                self.snippet_list.items.extend(snippet_items);
            }
            // Reattach the live partial if this conversation has an in-flight
            // stream, so the user immediately sees progress when browsing back
            // to it mid-stream.
            if let Some(state) = self.streams.get(&conv_id)
                && !state.partial.is_empty()
            {
                let (model, provider) = model_provider_from_spec(&state.selected_model);
                self.messages
                    .push(Message::Assistant(state.partial.clone(), model, provider));
            }
            self.needs_recache = true;
            self.vertical_scroll = 0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpServerStatus;

    /// Build an `App` with seeded MCP statuses and enabled intent (no live
    /// connections — we only test the intent/status bookkeeping, which is the
    /// pure logic that doesn't need a running service).
    fn app_with_statuses(statuses: Vec<McpServerStatus>, enabled: &[&str]) -> App<'static> {
        let mut app = App::default();
        app.mcp_statuses = statuses;
        for id in enabled {
            app.mcp_enabled.insert(id.to_string());
        }
        app
    }

    #[test]
    fn enable_adds_to_intent_and_sets_connecting() {
        let mut app = app_with_statuses(
            vec![McpServerStatus::Disabled {
                id: "kagi".into(),
                display_name: "Kagi".into(),
            }],
            &[],
        );

        assert!(!app.mcp_is_enabled("kagi"));
        app.mcp_enable("kagi");
        assert!(app.mcp_is_enabled("kagi"));
        assert!(matches!(
            app.mcp_statuses[0],
            McpServerStatus::Connecting { .. }
        ));
    }

    #[test]
    fn enable_is_idempotent() {
        let mut app = app_with_statuses(
            vec![McpServerStatus::Connecting {
                id: "kagi".into(),
                display_name: "Kagi".into(),
            }],
            &["kagi"],
        );
        // Already enabled — calling again is a no-op.
        app.mcp_enable("kagi");
        assert_eq!(app.mcp_enabled.len(), 1);
    }

    #[test]
    fn disable_removes_intent_and_sets_disabled() {
        let mut app = app_with_statuses(
            vec![McpServerStatus::Connecting {
                id: "kagi".into(),
                display_name: "Kagi".into(),
            }],
            &["kagi"],
        );

        assert!(app.mcp_is_enabled("kagi"));
        app.mcp_disable("kagi");
        assert!(!app.mcp_is_enabled("kagi"));
        assert!(matches!(
            app.mcp_statuses[0],
            McpServerStatus::Disabled { .. }
        ));
        // No live connection was present, so bridges stay empty.
        assert!(app.mcp_bridges.is_empty());
    }

    #[test]
    fn disable_is_noop_when_already_disabled() {
        let mut app = app_with_statuses(
            vec![McpServerStatus::Disabled {
                id: "kagi".into(),
                display_name: "Kagi".into(),
            }],
            &[],
        );
        app.mcp_disable("kagi");
        assert!(!app.mcp_is_enabled("kagi"));
        assert!(matches!(
            app.mcp_statuses[0],
            McpServerStatus::Disabled { .. }
        ));
    }

    #[test]
    fn status_counts_exclude_disabled() {
        let app = app_with_statuses(
            vec![
                McpServerStatus::Ready {
                    id: "fs".into(),
                    display_name: "FS".into(),
                    tool_count: 5,
                },
                McpServerStatus::Disabled {
                    id: "kagi".into(),
                    display_name: "Kagi".into(),
                },
                McpServerStatus::Connecting {
                    id: "weather".into(),
                    display_name: "Weather".into(),
                },
            ],
            &["fs", "weather"],
        );
        let (ready, tools, connecting, failed) = app.mcp_status_counts();
        assert_eq!(ready, 1);
        assert_eq!(tools, 5);
        assert_eq!(connecting, 1);
        assert_eq!(failed, 0);
    }

    #[test]
    fn server_ids_preserve_status_order() {
        let app = app_with_statuses(
            vec![
                McpServerStatus::Disabled {
                    id: "alpha".into(),
                    display_name: "Alpha".into(),
                },
                McpServerStatus::Ready {
                    id: "beta".into(),
                    display_name: "Beta".into(),
                    tool_count: 2,
                },
            ],
            &["beta"],
        );
        assert_eq!(app.mcp_server_ids(), vec!["alpha", "beta"]);
    }
}
