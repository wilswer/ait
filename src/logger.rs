//! Application logging.
//!
//! Logs are written to a file inside the project's data directory (resolved
//! via the [`directories`] crate, just like the chat database) so that they do
//! not interfere with the terminal user interface.
//!
//! The returned [`WorkerGuard`] must be kept alive for the whole lifetime of
//! the program: dropping it flushes and tears down the background writer. If
//! initialization fails, logging is silently disabled and the rest of the
//! application keeps running normally.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Returns the directory used to store log files.
///
/// This lives under the project's data directory (alongside `chats.db`) in a
/// `logs` sub-directory.
pub fn get_log_dir() -> Result<PathBuf> {
    let project_dirs = ProjectDirs::from("", "", "ait")
        .context("Could not determine project directories")?;
    Ok(project_dirs.data_dir().join("logs"))
}

/// Initialize the global tracing subscriber writing to a log file.
///
/// Returns the [`WorkerGuard`] that must be held until shutdown.
pub fn init_logging() -> Result<WorkerGuard> {
    let log_dir = get_log_dir()?;
    fs::create_dir_all(&log_dir).context("Could not create log directory")?;

    // `never` keeps a single, ever-growing log file (appended across runs).
    let file_appender = tracing_appender::rolling::never(&log_dir, "ait.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Allow overriding via `RUST_LOG`; otherwise default to INFO for deps and
    // DEBUG for our own crate so the skip reasons from token estimation show up.
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,ait=debug"));

    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false));

    // `try_init` returns an error if a global subscriber is already installed,
    // which is harmless for our purposes.
    let _ = subscriber.try_init();

    tracing::info!(log_dir = %log_dir.display(), "logging initialized");
    Ok(guard)
}
