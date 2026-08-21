//! Typed Python tool discovery and execution via `uv`.
//!
//! User scripts are ordinary Python files. A small Python bridge, embedded in
//! this binary from `assets/python/bridge.py`, uses `inspect` and Pydantic to
//! derive JSON Schema from typed functions and to validate arguments before a
//! tool call is executed.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use genai::chat::{Tool, ToolCall, ToolName, ToolResponse};
use serde::Deserialize;
use serde_json::Value;
use tempfile::NamedTempFile;
use tokio::process::Command;
use tokio::time::timeout;

const BRIDGE_CODE: &str = include_str!("../assets/python/bridge.py");
const PYDANTIC_REQUIREMENT: &str = "pydantic>=2";
const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(30);

/// A user-configured Python tool script.
#[derive(Debug, Clone)]
pub struct PythonToolSource {
    pub id: String,
    pub display_name: String,
    pub script: PathBuf,
    /// Project directory passed to `uv --project`, when a pyproject.toml is
    /// located beside the tool script.
    pub project_dir: Option<PathBuf>,
    pub timeout: Duration,
    pub uv_command: String,
}

impl PythonToolSource {
    pub fn new(id: impl Into<String>, script: impl Into<PathBuf>) -> Self {
        let id = id.into();
        let script = script.into();
        let project_dir = adjacent_project_dir(&script);
        Self {
            display_name: id.clone(),
            id,
            script,
            project_dir,
            timeout: DEFAULT_TOOL_TIMEOUT,
            uv_command: "uv".to_string(),
        }
    }

    pub async fn discover(&self) -> Result<Vec<PythonToolDefinition>, PythonToolError> {
        let script = self.script_arg();
        let response = self.run_bridge(&["discover", &script]).await?;
        response
            .into_discovery_result()
            .map_err(|error| self.enrich_missing_dependency_error(error))
    }

    pub async fn execute(&self, call: &ToolCall) -> Result<ToolResponse, PythonToolError> {
        let arguments = serde_json::to_string(&call.fn_arguments).map_err(PythonToolError::Json)?;
        let script = self.script_arg();
        let response = self
            .run_bridge(&["execute", &script, &call.fn_name, &arguments])
            .await?;

        match response
            .into_execution_result()
            .map_err(|error| self.enrich_missing_dependency_error(error))
        {
            Ok(result) => {
                let content = format_tool_result(&result);
                Ok(ToolResponse::from_tool_call(call, content))
            }
            // A tool-level error is a normal ToolResponse: the model can
            // inspect it and retry, pick another tool, or explain the issue.
            Err(PythonToolError::Bridge(error)) => Ok(ToolResponse::from_tool_call(
                call,
                format!("Error: {}", error.for_model()),
            )),
            Err(error) => Err(error),
        }
    }

    /// Attach a hint to `ModuleNotFoundError` failures when this source has
    /// no adjacent `pyproject.toml`. Only the standard library plus AIT's
    /// injected Pydantic dependency are available in that case, so a missing
    /// third-party import is almost always solved by creating a project file
    /// beside the tool script.
    fn enrich_missing_dependency_error(&self, error: PythonToolError) -> PythonToolError {
        let PythonToolError::Bridge(mut bridge_error) = error else {
            return error;
        };
        let is_missing_module = bridge_error.kind == "module_load_error"
            && bridge_error.message.contains("ModuleNotFoundError");
        if self.project_dir.is_none() && is_missing_module {
            let dir = self
                .script
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| ".".to_string());
            bridge_error.hint = Some(format!(
                "This tool file has no adjacent pyproject.toml, so only the Python \
                 standard library (plus pydantic, provisioned by AIT) is available. \
                 Create `{dir}/pyproject.toml` and declare the missing dependency, e.g.:\n\n\
                 [project]\n\
                 name = \"ait-tools\"\n\
                 version = \"0.1.0\"\n\
                 dependencies = [\"<package-name>\"]"
            ));
        }
        PythonToolError::Bridge(bridge_error)
    }

    fn script_arg(&self) -> String {
        self.script.to_string_lossy().into_owned()
    }

    async fn run_bridge(&self, args: &[&str]) -> Result<BridgeResponse, PythonToolError> {
        let bridge = write_bridge_file()?;
        let bridge_path = bridge.path().to_path_buf();

        let mut command = Command::new(&self.uv_command);
        command.arg("run");
        if let Some(project_dir) = &self.project_dir {
            command.arg("--project").arg(project_dir);
        }
        command
            .arg("--with")
            .arg(PYDANTIC_REQUIREMENT)
            .arg(&bridge_path)
            .args(args)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let output = match timeout(self.timeout, command.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(PythonToolError::UvNotFound {
                    command: self.uv_command.clone(),
                });
            }
            Ok(Err(error)) => return Err(PythonToolError::Process(error)),
            Err(_) => {
                return Err(PythonToolError::Timeout {
                    tool_source: self.id.clone(),
                    timeout: self.timeout,
                });
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        // The bridge intentionally emits a structured error response before
        // exiting non-zero. Parse stdout first so AIT can show a useful
        // annotation/validation error instead of raw uv stderr.
        if !stdout.is_empty() {
            if let Ok(response) = serde_json::from_str::<BridgeResponse>(&stdout) {
                return Ok(response);
            }
        }

        if !output.status.success() {
            let message = if stderr.is_empty() {
                format!("uv exited with status {}", output.status)
            } else {
                stderr
            };
            return Err(PythonToolError::UvFailed(message));
        }

        if stdout.is_empty() {
            return Err(PythonToolError::Protocol(
                "bridge produced no JSON response".to_string(),
            ));
        }

        Err(PythonToolError::Protocol(format!(
            "bridge produced invalid JSON: {stdout}"
        )))
    }
}

/// A tool discovered in a Python source file.
#[derive(Debug, Clone, Deserialize)]
pub struct PythonToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl PythonToolDefinition {
    pub fn into_genai_tool(self) -> Tool {
        Tool::new(ToolName::Custom(self.name))
            .with_description(self.description)
            .with_schema(self.input_schema)
    }
}

/// A machine-readable bridge error. `details` is retained for future TUI
/// diagnostics; `for_model` intentionally supplies a concise representation.
#[derive(Debug, Clone, Deserialize)]
pub struct PythonBridgeError {
    pub kind: String,
    pub message: String,
    pub file: Option<String>,
    pub tool: Option<String>,
    pub parameter: Option<String>,
    pub hint: Option<String>,
    pub details: Option<Value>,
}

impl PythonBridgeError {
    pub fn for_model(&self) -> String {
        let mut output = self.message.clone();
        if let Some(parameter) = &self.parameter {
            output = format!("{parameter}: {output}");
        }
        if let Some(details) = &self.details
            && self.kind == "argument_validation_error"
        {
            output.push_str(&format!(" Details: {details}"));
        }
        if let Some(hint) = &self.hint {
            output.push_str(&format!("\nHint: {hint}"));
        }
        output
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PythonToolError {
    #[error("`uv` command `{command}` was not found; install uv to enable Python tools")]
    UvNotFound { command: String },

    #[error("failed to start or communicate with uv: {0}")]
    Process(#[source] std::io::Error),

    #[error("uv failed: {0}")]
    UvFailed(String),

    #[error(
        "Python tool source `{tool_source}` exceeded its {}-second timeout",
        timeout.as_secs()
    )]
    Timeout {
        tool_source: String,
        timeout: Duration,
    },

    #[error("Python bridge protocol error: {0}")]
    Protocol(String),

    #[error("Python bridge error: {}", .0.message)]
    Bridge(PythonBridgeError),

    #[error("failed to encode or decode JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("failed to create embedded Python bridge file: {0}")]
    TempFile(#[source] std::io::Error),
}

#[derive(Debug, Deserialize)]
struct BridgeResponse {
    ok: bool,
    #[serde(default)]
    tools: Vec<PythonToolDefinition>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<PythonBridgeError>,
}

impl BridgeResponse {
    fn into_discovery_result(self) -> Result<Vec<PythonToolDefinition>, PythonToolError> {
        if self.ok {
            Ok(self.tools)
        } else {
            Err(PythonToolError::Bridge(self.error.ok_or_else(|| {
                PythonToolError::Protocol(
                    "bridge returned ok=false without an error object".to_string(),
                )
            })?))
        }
    }

    fn into_execution_result(self) -> Result<Value, PythonToolError> {
        if self.ok {
            self.result.ok_or_else(|| {
                PythonToolError::Protocol(
                    "bridge returned ok=true without a result value".to_string(),
                )
            })
        } else {
            Err(PythonToolError::Bridge(self.error.ok_or_else(|| {
                PythonToolError::Protocol(
                    "bridge returned ok=false without an error object".to_string(),
                )
            })?))
        }
    }
}

fn write_bridge_file() -> Result<NamedTempFile, PythonToolError> {
    let mut bridge = NamedTempFile::with_suffix(".py").map_err(PythonToolError::TempFile)?;
    std::io::Write::write_all(&mut bridge, BRIDGE_CODE.as_bytes())
        .map_err(PythonToolError::TempFile)?;
    Ok(bridge)
}

fn format_tool_result(result: &Value) -> String {
    match result {
        Value::String(value) => value.clone(),
        _ => serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string()),
    }
}

/// Verify that `uv` is present without loading any user-provided code.
pub async fn verify_uv(command: &str) -> Result<()> {
    let output = Command::new(command)
        .arg("--version")
        .output()
        .await
        .with_context(|| format!("could not execute `{command} --version`"))?;

    if !output.status.success() {
        bail!("`{command} --version` exited with status {}", output.status);
    }
    Ok(())
}

/// Convenience helper used by tests and early callers that only need a
/// single tool source and its `genai` schemas.
pub async fn load_tools(source: &PythonToolSource) -> Result<Vec<Tool>, PythonToolError> {
    source.discover().await.map(|definitions| {
        definitions
            .into_iter()
            .map(PythonToolDefinition::into_genai_tool)
            .collect()
    })
}

pub fn adjacent_project_dir(script: &Path) -> Option<PathBuf> {
    let parent = script.parent()?;
    parent
        .join("pyproject.toml")
        .is_file()
        .then(|| parent.to_path_buf())
}

pub fn validate_source_path(path: &Path) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(anyhow!(
            "Python tool file does not exist: {}",
            path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pydantic_discovery_schema_response() {
        // Regression: serde's internally-tagged-enum representation expects
        // string tag values, while the bridge correctly emits JSON booleans.
        // This is the real shape produced by Pydantic for Enum parameters.
        let response = r##"{
            "ok": true,
            "tools": [{
                "name": "get_weather",
                "description": "Return a demo temperature for a location.",
                "input_schema": {
                    "$defs": {
                        "Unit": {
                            "enum": ["celsius", "fahrenheit"],
                            "title": "Unit",
                            "type": "string"
                        }
                    },
                    "properties": {
                        "location": {"title": "Location", "type": "string"},
                        "unit": {"$ref": "#/$defs/Unit", "default": "celsius"}
                    },
                    "required": ["location"],
                    "title": "get_weather_arguments",
                    "type": "object",
                    "additionalProperties": false
                }
            }]
        }"##;

        let bridge_response: BridgeResponse = serde_json::from_str(response).unwrap();
        let tools = bridge_response.into_discovery_result().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "get_weather");
        assert_eq!(
            tools[0].input_schema["$defs"]["Unit"]["enum"],
            serde_json::json!(["celsius", "fahrenheit"])
        );
    }

    #[test]
    fn parses_bridge_error_response() {
        let response = r#"{
            "ok": false,
            "error": {
                "kind": "missing_annotation",
                "message": "Parameter 'location' must have a type annotation.",
                "tool": "get_weather",
                "parameter": "location"
            }
        }"#;
        let bridge_response: BridgeResponse = serde_json::from_str(response).unwrap();
        let error = bridge_response.into_discovery_result().unwrap_err();
        assert!(matches!(error, PythonToolError::Bridge(_)));
    }

    #[test]
    fn finds_only_adjacent_pyproject() {
        let temp = tempfile::tempdir().unwrap();
        let tools_dir = temp.path().join("tools");
        std::fs::create_dir(&tools_dir).unwrap();
        let script = tools_dir.join("tools.py");
        std::fs::write(&script, "").unwrap();

        assert_eq!(adjacent_project_dir(&script), None);
        std::fs::write(
            tools_dir.join("pyproject.toml"),
            "[project]\nname='tools'\n",
        )
        .unwrap();
        assert_eq!(adjacent_project_dir(&script), Some(tools_dir));
    }

    #[test]
    fn does_not_use_parent_project_without_adjacent_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path();
        let tools_dir = project.join("nested");
        std::fs::create_dir(&tools_dir).unwrap();
        std::fs::write(project.join("pyproject.toml"), "[project]\nname='parent'\n").unwrap();
        let script = tools_dir.join("tools.py");
        std::fs::write(&script, "").unwrap();

        assert_eq!(adjacent_project_dir(&script), None);
    }

    #[test]
    fn formats_string_result_without_json_quotes() {
        assert_eq!(format_tool_result(&Value::String("hello".into())), "hello");
    }

    #[test]
    fn formats_structured_result_as_pretty_json() {
        assert_eq!(
            format_tool_result(&serde_json::json!({ "temperature": 22.5 })),
            "{\n  \"temperature\": 22.5\n}"
        );
    }

    #[test]
    fn model_error_includes_parameter_and_validation_details() {
        let error = PythonBridgeError {
            kind: "argument_validation_error".to_string(),
            message: "Invalid arguments for 'get_weather'.".to_string(),
            file: None,
            tool: Some("get_weather".to_string()),
            parameter: Some("unit".to_string()),
            hint: None,
            details: Some(serde_json::json!([{ "msg": "invalid unit" }])),
        };

        let output = error.for_model();
        assert!(output.starts_with("unit: Invalid arguments"));
        assert!(output.contains("invalid unit"));
    }

    #[test]
    fn validates_existing_and_missing_source_paths() {
        let existing = std::env::current_dir()
            .unwrap()
            .join("assets")
            .join("python")
            .join("bridge.py");
        assert!(validate_source_path(&existing).is_ok());
        assert!(validate_source_path(Path::new("/definitely/missing/ait-tools.py")).is_err());
    }
}
