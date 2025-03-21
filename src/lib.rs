#![doc = include_str!("../README.md")]

/// Application.
pub mod app;

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

/// Model selector.
pub mod models;

/// Snippets finder.
pub mod snippets;

/// Command line interface.
pub mod cli;

///Chat conversations storage.
pub mod storage;

/// Chat list.
pub mod chats;
