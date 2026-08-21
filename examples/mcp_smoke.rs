// Smoke test: spawn the official `@modelcontextprotocol/server-everything`
// MCP server, list its tools, call `echo`, and print everything. Exercises the
// real ait::mcp loader + mcp-genai-glue bridge without the TUI.
//
// Run: cargo run --example mcp_smoke
// (needs `npx` on PATH; first run downloads the npm package, can take ~20s)

use ait::config::McpServerConfig;
use genai::chat::ToolCall;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = ait::logger::init_logging()?;

    let cfg = McpServerConfig {
        name: Some("Everything (test)".into()),
        enabled: true,
        command: Some("npx".into()),
        args: vec![
            "-y".into(),
            "@modelcontextprotocol/server-everything".into(),
        ],
        env: Default::default(),
        url: None,
        api_key: None,
        headers: Default::default(),
    };

    println!("Spawning @modelcontextprotocol/server-everything via npx ...");
    let conn = ait::mcp::connect_one("everything".into(), cfg).await?;

    println!("\n=== connected: {} ===", conn.display_name);
    let tools = conn.bridge.tools().await?;
    println!("Tool count: {}", tools.len());
    for t in &tools {
        println!(
            "  - {} : {}",
            t.name.as_ref(),
            t.description.as_deref().unwrap_or("")
        );
    }

    // Call `echo` via the glue bridge, using a synthetic ToolCall.
    let echo_call = ToolCall {
        call_id: "smoke-1".into(),
        fn_name: "echo".into(),
        fn_arguments: json!({ "message": "hello from ait mcp smoke test" }),
        thought_signatures: None,
    };
    println!("\n=== calling tool `echo` ===");
    let resp = conn.bridge.execute(&echo_call).await?;
    println!("ToolResponse content:\n{}", resp.content);

    Ok(())
}
