use anyhow::{Context, Result, anyhow, bail};
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

    /// Recursively expand `${VAR}` / `${VAR:-default}` / `$VAR` references in
    /// every string field of this server config using the process environment.
    /// Called by the MCP loader right before connecting, so the on-disk config
    /// keeps the `${VAR}` placeholders (and [`Config::save`] never writes
    /// expanded secrets back to disk).
    ///
    /// An unset variable with no default is a hard error so missing secrets
    /// fail loudly instead of sending an empty string to a server.
    pub fn expand_env(&mut self) -> Result<()> {
        if let Some(name) = self.name.as_mut() {
            *name = crate::config::expand_env(name)?;
        }
        if let Some(command) = self.command.as_mut() {
            *command = crate::config::expand_env(command)?;
        }
        for arg in self.args.iter_mut() {
            *arg = crate::config::expand_env(arg)?;
        }
        for value in self.env.values_mut() {
            *value = crate::config::expand_env(value)?;
        }
        if let Some(url) = self.url.as_mut() {
            *url = crate::config::expand_env(url)?;
        }
        if let Some(api_key) = self.api_key.as_mut() {
            *api_key = crate::config::expand_env(api_key)?;
        }
        for value in self.headers.values_mut() {
            *value = crate::config::expand_env(value)?;
        }
        Ok(())
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

/// Expand environment-variable references in `input`.
///
/// Supported forms:
/// - `${VAR}` — replaced with the value of `VAR` (error if unset).
/// - `${VAR:-default}` — replaced with the value of `VAR`, or `default` if
///   `VAR` is unset.
/// - `$VAR` — replaced with the value of `VAR` (error if unset). `VAR` is a
///   run of `[A-Za-z_][A-Za-z0-9_]*`.
/// - `$$` — a literal `$` (escape).
///
/// An unset variable (with no `:-default`) is a hard error so missing secrets
/// fail loudly rather than silently sending an empty string to a server.
///
/// This is intentionally a pure function over `std::env`; it does not read any
/// files. It is applied to every string field of [`McpServerConfig`] by
/// [`McpServerConfig::expand_env`].
pub fn expand_env(input: &str) -> Result<String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(dollar) = rest.find('$') {
        // Copy everything before the `$`.
        out.push_str(&rest[..dollar]);
        let after = &rest[dollar + 1..];

        if after.is_empty() {
            // Trailing lone `$` — treat as literal.
            out.push('$');
            rest = "";
            break;
        }
        let next = after.as_bytes()[0];

        if next == b'$' {
            // `$$` → literal `$`.
            out.push('$');
            rest = &after[1..];
            continue;
        }

        if next == b'{' {
            // `${VAR}` or `${VAR:-default}`.
            let close = after
                .find('}')
                .ok_or_else(|| anyhow!("unterminated ${{...}} in `{input}`"))?;
            let inner = &after[1..close];
            let (name, default) = match inner.find(":-") {
                Some(idx) => (&inner[..idx], Some(&inner[idx + 2..])),
                None => (inner, None),
            };
            if name.is_empty() {
                bail!("empty variable name in `${{{inner}}}`");
            }
            if !is_valid_var_name(name) {
                bail!("invalid variable name `{name}` in `{input}`");
            }
            let value = match std::env::var(name).ok() {
                Some(v) => v,
                None => match default {
                    Some(d) => d.to_string(),
                    None => bail!(
                        "environment variable `{name}` is not set (referenced in `{input}`)"
                    ),
                },
            };
            out.push_str(&value);
            rest = &after[close + 1..];
        } else if next.is_ascii_alphabetic() || next == b'_' {
            // `$VAR` — name runs until a non-identifier byte.
            let end = after
                .as_bytes()
                .iter()
                .position(|&c| !(c.is_ascii_alphanumeric() || c == b'_'))
                .unwrap_or(after.len());
            let name = &after[..end];
            let value = std::env::var(name).with_context(|| {
                format!("environment variable `{name}` is not set (referenced in `{input}`)")
            })?;
            out.push_str(&value);
            rest = &after[end..];
        } else {
            // `$` followed by something that isn't a valid name start — keep
            // it literal (e.g. a `$` in a URL like `https://host/$path`).
            out.push('$');
            rest = after;
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// Whether `name` is a syntactically valid POSIX-ish env var name
/// (`[A-Za-z_][A-Za-z0-9_]*`).
fn is_valid_var_name(name: &str) -> bool {
    let mut bytes = name.as_bytes().iter();
    match bytes.next() {
        Some(&b) if b.is_ascii_alphabetic() || b == b'_' => {}
        _ => return false,
    }
    bytes.all(|&b| b.is_ascii_alphanumeric() || b == b'_')
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

    // --- expand_env ---

    fn set_env(key: &str, value: &str) {
        // SAFETY: tests run single-threaded within this module. Env vars are
        // process-global but that's acceptable for these isolated tests.
        unsafe { std::env::set_var(key, value) };
    }

    fn remove_env(key: &str) {
        // SAFETY: see `set_env`.
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn expand_braces() {
        set_env("AIT_TEST_KEY", "secret-value");
        assert_eq!(expand_env("${AIT_TEST_KEY}").unwrap(), "secret-value");
        assert_eq!(expand_env("key=${AIT_TEST_KEY}").unwrap(), "key=secret-value");
    }

    #[test]
    fn expand_bare_dollar_var() {
        set_env("AIT_TEST_BARE", "bareval");
        assert_eq!(expand_env("$AIT_TEST_BARE/end").unwrap(), "bareval/end");
        assert_eq!(expand_env("$AIT_TEST_BARE").unwrap(), "bareval");
    }

    #[test]
    fn expand_default_when_unset() {
        remove_env("AIT_TEST_UNSET_ONE");
        assert_eq!(
            expand_env("${AIT_TEST_UNSET_ONE:-fallback}").unwrap(),
            "fallback"
        );
        // default itself can contain arbitrary text
        assert_eq!(
            expand_env("${AIT_TEST_UNSET_ONE:-/usr/bin}").unwrap(),
            "/usr/bin"
        );
    }

    #[test]
    fn expand_unset_without_default_errors() {
        remove_env("AIT_TEST_UNSET_TWO");
        let err = expand_env("${AIT_TEST_UNSET_TWO}").unwrap_err();
        assert!(err.to_string().contains("AIT_TEST_UNSET_TWO"));
    }

    #[test]
    fn expand_bare_unset_errors() {
        remove_env("AIT_TEST_UNSET_THREE");
        let err = expand_env("$AIT_TEST_UNSET_THREE").unwrap_err();
        assert!(err.to_string().contains("AIT_TEST_UNSET_THREE"));
    }

    #[test]
    fn expand_double_dollar_is_literal() {
        set_env("AIT_TEST_NOT_EXPANDED", "should-not-appear");
        assert_eq!(expand_env("$$").unwrap(), "$");
        assert_eq!(expand_env("price: $$5").unwrap(), "price: $5");
        assert_eq!(expand_env("$${AIT_TEST_NOT_EXPANDED}").unwrap(), "${AIT_TEST_NOT_EXPANDED}");
    }

    #[test]
    fn expand_trailing_lone_dollar_is_literal() {
        assert_eq!(expand_env("abc$").unwrap(), "abc$");
    }

    #[test]
    fn expand_dollar_before_non_identifier_is_literal() {
        // `$1` and `$-` don't start a valid name → literal `$`.
        assert_eq!(expand_env("cost: $1.00").unwrap(), "cost: $1.00");
        assert_eq!(expand_env("$-").unwrap(), "$-");
    }

    #[test]
    fn expand_multiple_in_one_string() {
        set_env("AIT_TEST_A", "AAA");
        set_env("AIT_TEST_B", "BBB");
        assert_eq!(
            expand_env("${AIT_TEST_A}-$AIT_TEST_B-${AIT_TEST_A}").unwrap(),
            "AAA-BBB-AAA"
        );
    }

    #[test]
    fn expand_empty_braces_error() {
        let err = expand_env("${}").unwrap_err();
        assert!(err.to_string().contains("empty variable name"));
    }

    #[test]
    fn expand_invalid_name_in_braces_error() {
        let err = expand_env("${1BAD}").unwrap_err();
        assert!(err.to_string().contains("invalid variable name"));
    }

    #[test]
    fn expand_unterminated_braces_error() {
        let err = expand_env("${AIT_TEST_KEY").unwrap_err();
        assert!(err.to_string().contains("unterminated"));
    }

    #[test]
    fn mcp_server_config_expand_env_resolves_all_fields() {
        set_env("AIT_TEST_CMD", "echo-cmd");
        set_env("AIT_TEST_ARG", "argval");
        set_env("AIT_TEST_ENV", "envval");
        set_env("AIT_TEST_URL", "https://example.com/mcp");
        set_env("AIT_TEST_KEY", "apikey");
        set_env("AIT_TEST_HDR", "hdrval");
        set_env("AIT_TEST_NAME", "Display");

        let mut cfg = McpServerConfig {
            name: Some("${AIT_TEST_NAME}".into()),
            enabled: true,
            command: Some("${AIT_TEST_CMD}".into()),
            args: vec!["-y".into(), "$AIT_TEST_ARG".into()],
            env: [("K".to_string(), "${AIT_TEST_ENV}".into())].into(),
            url: Some("${AIT_TEST_URL}".into()),
            api_key: Some("$AIT_TEST_KEY".into()),
            headers: [("X-Custom".to_string(), "${AIT_TEST_HDR}".into())].into(),
        };
        cfg.expand_env().unwrap();

        assert_eq!(cfg.name.as_deref(), Some("Display"));
        assert_eq!(cfg.command.as_deref(), Some("echo-cmd"));
        assert_eq!(cfg.args, vec!["-y".to_string(), "argval".to_string()]);
        assert_eq!(cfg.env.get("K").map(|s| s.as_str()), Some("envval"));
        assert_eq!(cfg.url.as_deref(), Some("https://example.com/mcp"));
        assert_eq!(cfg.api_key.as_deref(), Some("apikey"));
        assert_eq!(cfg.headers.get("X-Custom").map(|s| s.as_str()), Some("hdrval"));
    }
}
