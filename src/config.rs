use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Config {
    pub ollama_host: Option<String>,
    pub system_prompt: Option<String>,
    pub default_model: Option<ModelConfig>,
    /// MCP (Model Context Protocol) server configuration.
    #[serde(default)]
    pub mcp: McpConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ModelConfig {
    pub name: String,
    pub provider: String,
}

impl ModelConfig {
    pub fn new(name: String, provider: String) -> Self {
        Self { name, provider }
    }
}

/// Top-level MCP configuration section (`[mcp]`).
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct McpConfig {
    /// MCP servers, keyed by a stable id (the TOML table key).
    ///
    /// A [`BTreeMap`] is used so iteration order is deterministic (sorted by
    /// id), which keeps the in-app server list stable across reloads.
    #[serde(default)]
    pub servers: BTreeMap<String, McpServerConfig>,
}

/// Configuration for a single MCP server.
///
/// A server uses the **stdio** transport when `command` is set, or the
/// **http** (streamable) transport when `url` is set. Setting both — or
/// neither — is a configuration error and is rejected at load time.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct McpServerConfig {
    /// Optional human-readable display name. Defaults to the server id when
    /// `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Whether this server is connected automatically on startup. The user
    /// can toggle servers on/off in-app later. Defaults to `true` when
    /// omitted.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    // --- stdio transport -------------------------------------------------
    /// Command to spawn for a stdio MCP server (e.g. `"npx"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Arguments passed to `command`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Extra environment variables for the spawned process. This is also
    /// where stdio servers should receive their secrets (e.g. API keys).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,

    // --- http transport ---------------------------------------------------
    /// URL of a remote/streamable-http MCP server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// API key sent as `Authorization: Bearer <api_key>` on every request to
    /// an http server. Ignored for stdio servers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Extra HTTP headers sent on every request to an http server (e.g.
    /// `X-API-Key`). Ignored for stdio servers.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
}

/// Serde default for [`McpServerConfig::enabled`] — servers are on unless
/// the user explicitly writes `enabled = false`.
fn default_enabled() -> bool {
    true
}

/// Which transport a server config selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransportKind {
    Stdio,
    Http,
}

impl McpServerConfig {
    /// Effective display name: the configured `name`, or the server id.
    pub fn display_name<'a>(&'a self, id: &'a str) -> &'a str {
        self.name.as_deref().unwrap_or(id)
    }

    /// Resolve which transport this config selects, validating that exactly
    /// one of `command`/`url` is present.
    pub fn transport_kind(&self) -> Result<McpTransportKind, String> {
        match (&self.command, &self.url) {
            (Some(_), None) => Ok(McpTransportKind::Stdio),
            (None, Some(_)) => Ok(McpTransportKind::Http),
            (Some(_), Some(_)) => Err(
                "server config has both `command` (stdio) and `url` (http); pick one"
                    .to_string(),
            ),
            (None, None) => Err(
                "server config has neither `command` (stdio) nor `url` (http); set one"
                    .to_string(),
            ),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::get_config_path()?;
        if !config_path.exists() {
            return Ok(Config::default());
        }

        let content = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file at {:?}", config_path))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file at {:?}", config_path))?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let config_path = Self::get_config_path()?;
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory {:?}", parent))?;
        }

        let content = toml::to_string_pretty(self).with_context(|| "Failed to serialize config")?;
        fs::write(&config_path, content)
            .with_context(|| format!("Failed to write config file at {:?}", config_path))?;
        Ok(())
    }

    fn get_config_path() -> Result<PathBuf> {
        let proj_dirs = ProjectDirs::from("", "", "ait")
            .with_context(|| "Could not determine config directory")?;
        let config_dir = proj_dirs.config_dir();
        Ok(config_dir.join("config.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stdio_and_http_servers() {
        let toml = r#"
[mcp.servers.filesystem]
name = "Filesystem"
enabled = true
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[mcp.servers.weather]
url = "https://weather.example.com/mcp"
api_key = "secret"
headers = { "X-Custom" = "value" }
"#;
        let config: Config = toml::from_str(toml).unwrap();

        let fs = config.mcp.servers.get("filesystem").unwrap();
        assert_eq!(fs.transport_kind().unwrap(), McpTransportKind::Stdio);
        assert!(fs.enabled);
        assert_eq!(fs.display_name("filesystem"), "Filesystem");
        assert_eq!(fs.command.as_deref(), Some("npx"));
        assert_eq!(fs.url, None);

        let weather = config.mcp.servers.get("weather").unwrap();
        assert_eq!(weather.transport_kind().unwrap(), McpTransportKind::Http);
        // `enabled` defaults to true when omitted.
        assert!(weather.enabled);
        assert_eq!(weather.display_name("weather"), "weather");
        assert_eq!(weather.api_key.as_deref(), Some("secret"));
        assert_eq!(weather.headers.get("X-Custom").map(|s| s.as_str()), Some("value"));
    }

    #[test]
    fn enabled_can_be_disabled() {
        let toml = r#"
[mcp.servers.off]
command = "echo"
enabled = false
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(!config.mcp.servers["off"].enabled);
    }

    #[test]
    fn rejects_both_transports() {
        let toml = r#"
[mcp.servers.bad]
command = "echo"
url = "https://example.com/mcp"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let err = config.mcp.servers["bad"].transport_kind().unwrap_err();
        assert!(err.contains("both"));
    }

    #[test]
    fn rejects_no_transport() {
        let toml = r#"
[mcp.servers.bad]
name = "Bad"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let err = config.mcp.servers["bad"].transport_kind().unwrap_err();
        assert!(err.contains("neither"));
    }

    #[test]
    fn no_mcp_section_is_fine() {
        let toml = r#"system_prompt = "hi""#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.mcp.servers.is_empty());
    }
}
