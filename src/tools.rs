//! Unified model-tool registry.
//!
//! A tool may be backed by an MCP server or by a typed Python function. Both
//! sources expose the same `genai::chat::Tool` schema to the model and accept
//! the same `genai::chat::ToolCall` at execution time.

use std::collections::HashMap;

use anyhow::{Result, anyhow, bail};
use genai::chat::{Tool, ToolCall, ToolResponse};
use mcp_genai_glue::McpToolBridge;

use crate::python_tools::{PythonToolDefinition, PythonToolSource};

#[derive(Clone)]
pub enum ToolExecutor {
    Mcp(McpToolBridge),
    Python(PythonToolSource),
}

impl ToolExecutor {
    pub async fn execute(&self, call: &ToolCall) -> Result<ToolResponse> {
        match self {
            Self::Mcp(bridge) => Ok(bridge.execute(call).await?),
            Self::Python(source) => Ok(source.execute(call).await?),
        }
    }

    fn source_description(&self) -> String {
        match self {
            Self::Mcp(_) => "an MCP server".to_string(),
            Self::Python(source) => format!("Python source `{}`", source.display_name),
        }
    }
}

/// An immutable, request-safe snapshot of all tools available to the model.
///
/// Build a new registry when MCP/Python sources change, and clone the snapshot
/// into each assistant stream. This keeps schemas and executors stable across
/// every tool-calling round of an in-flight request.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: Vec<Tool>,
    executors: HashMap<String, ToolExecutor>,
}

impl ToolRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    pub fn names(&self) -> Vec<String> {
        self.tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub async fn execute(&self, call: &ToolCall) -> Result<ToolResponse> {
        let executor = self.executors.get(&call.fn_name).ok_or_else(|| {
            anyhow!(
                "no enabled tool source provides `{}`; reload tool sources and try again",
                call.fn_name
            )
        })?;
        executor.execute(call).await
    }

    pub fn builder() -> ToolRegistryBuilder {
        ToolRegistryBuilder::default()
    }
}

#[derive(Default)]
pub struct ToolRegistryBuilder {
    tools: Vec<Tool>,
    executors: HashMap<String, ToolExecutor>,
}

impl ToolRegistryBuilder {
    pub fn add_mcp_tools(
        &mut self,
        bridge: McpToolBridge,
        tools: impl IntoIterator<Item = Tool>,
    ) -> Result<()> {
        for tool in tools {
            self.insert(tool, ToolExecutor::Mcp(bridge.clone()))?;
        }
        Ok(())
    }

    pub fn add_python_tools(
        &mut self,
        source: PythonToolSource,
        definitions: impl IntoIterator<Item = PythonToolDefinition>,
    ) -> Result<()> {
        for definition in definitions {
            self.insert(
                definition.into_genai_tool(),
                ToolExecutor::Python(source.clone()),
            )?;
        }
        Ok(())
    }

    pub fn build(mut self) -> ToolRegistry {
        self.tools
            .sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
        ToolRegistry {
            tools: self.tools,
            executors: self.executors,
        }
    }

    fn insert(&mut self, tool: Tool, executor: ToolExecutor) -> Result<()> {
        let name = tool.name.as_str().to_string();
        if let Some(existing) = self.executors.get(&name) {
            bail!(
                "tool name collision for `{name}` between {} and {}",
                existing.source_description(),
                executor.source_description(),
            );
        }
        self.executors.insert(name, executor);
        self.tools.push(tool);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn definition(name: &str) -> PythonToolDefinition {
        PythonToolDefinition {
            name: name.to_string(),
            description: format!("Description for {name}"),
            input_schema: json!({ "type": "object" }),
        }
    }

    fn source(id: &str) -> PythonToolSource {
        PythonToolSource::new(id, format!("/tmp/{id}.py"))
    }

    #[test]
    fn builder_sorts_python_tools_by_name() {
        let mut builder = ToolRegistry::builder();
        builder
            .add_python_tools(source("tools"), [definition("zebra"), definition("alpha")])
            .unwrap();

        let registry = builder.build();
        assert_eq!(registry.names(), vec!["alpha", "zebra"]);
    }

    #[test]
    fn builder_rejects_duplicate_python_tool_names() {
        let mut builder = ToolRegistry::builder();
        builder
            .add_python_tools(source("first"), [definition("lookup")])
            .unwrap();

        let error = builder
            .add_python_tools(source("second"), [definition("lookup")])
            .unwrap_err();

        assert!(error.to_string().contains("tool name collision"));
        assert!(error.to_string().contains("lookup"));
    }

    #[test]
    fn empty_registry_has_no_tools() {
        let registry = ToolRegistry::empty();
        assert!(registry.is_empty());
        assert!(registry.names().is_empty());
    }
}
