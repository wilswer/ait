#![doc = include_str!("../README.md")]

/// Application.
pub mod app;

/// Application logging.
pub mod logger;

/// Terminal events handler.
pub mod event;

/// Widget renderer.
pub mod ui;

/// Terminal user interface.
pub mod tui;

/// Event handler.
pub mod handler;

/// GenAI chat client.
pub mod ai;

/// Typed Python tool discovery and execution via uv.
pub mod python_tools;

/// Unified MCP and Python tool registry.
pub mod tools;

/// MCP server loader / bridge wiring.
pub mod mcp;

/// Model selector.
pub mod models;

/// Snippets finder.
pub mod snippets;

/// Command line interface.
pub mod cli;

/// Configuration.
pub mod config;

pub mod observability;
///Chat conversations storage.
pub mod storage;

/// Chat list.
pub mod chats;

/// Messages selector.
pub mod message_list;
