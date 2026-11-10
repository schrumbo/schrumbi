use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::ServerHandler;
use rmcp::model::{CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServiceExt};
use serde_json::{Value, json};

use super::{instance, tools};

const NOT_RUNNING: &str = "Schrumbi is not running, or its assistant is switched off. Ask the user to open Schrumbi and check Settings → KI (Einstellungen → KI).";
const READ_WAIT: Duration = Duration::from_secs(60);
/// A write waits for the user's answer in the app, which gives up after ten minutes.
const WRITE_WAIT: Duration = Duration::from_secs(11 * 60);

struct Bridge {
    session: String,
    forced: Option<u32>,
}

fn tool_list() -> Vec<Tool> {
    tools::definitions()
        .into_iter()
        .map(|t| {
            let schema = t.schema.as_object().cloned().unwrap_or_default();
            Tool::new(t.name, t.description, Arc::new(schema)).with_annotations(ToolAnnotations::new().read_only(!t.write).destructive(false).open_world(false))
        })
        .collect()
}

/// Sends one tool call to the running app and waits for its answer.
pub fn forward(session: &str, forced: Option<u32>, name: &str, args: &Value) -> Result<Value, String> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let target = instance::pick(&cwd, forced).ok_or(NOT_RUNNING)?;
    let addr = SocketAddr::from(([127, 0, 0, 1], target.port));
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|_| NOT_RUNNING.to_string())?;
    let _ = stream.set_read_timeout(Some(if tools::is_write(name) { WRITE_WAIT } else { READ_WAIT }));
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let body = json!({ "token": target.token, "session": session, "tool": name, "args": args });
    writeln!(writer, "{body}").and_then(|()| writer.flush()).map_err(|_| NOT_RUNNING.to_string())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(|_| "Schrumbi did not answer in time.".to_string())?;
    let answer: Value = serde_json::from_str(&line).map_err(|_| NOT_RUNNING.to_string())?;
    if answer["ok"].as_bool() == Some(true) { Ok(answer["value"].clone()) } else { Err(answer["error"].as_str().unwrap_or("Schrumbi refused the call.").to_string()) }
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(Implementation::new("schrumbi", env!("CARGO_PKG_VERSION"))).with_instructions(tools::INSTRUCTIONS)
    }

    async fn list_tools(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(tool_list()))
    }

    async fn call_tool(&self, request: CallToolRequestParams, _: RequestContext<RoleServer>) -> Result<CallToolResponse, McpError> {
        let name = request.name.to_string();
        if !tools::definitions().iter().any(|t| t.name == name) {
            return Err(McpError::invalid_params(format!("unknown tool {name}"), None));
        }
        let args = request.arguments.map_or_else(|| json!({}), Value::Object);
        let (session, forced) = (self.session.clone(), self.forced);
        let result = tokio::task::spawn_blocking(move || forward(&session, forced, &name, &args)).await.unwrap_or_else(|e| Err(e.to_string()));
        Ok(match result {
            Ok(value) => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error)]),
        }
        .into())
    }
}

fn forced_instance(args: &[String]) -> Result<Option<u32>, String> {
    match args.iter().position(|a| a == "--instance") {
        None => Ok(None),
        Some(i) => args.get(i + 1).and_then(|p| p.parse().ok()).map(Some).ok_or_else(|| "--instance needs a process id".to_string()),
    }
}

/// `schrumbi mcp [--instance <pid>]`: Schrumbi's tools over MCP on stdio, forwarded to the running app.
pub fn main(args: Vec<String>) -> i32 {
    let forced = match forced_instance(&args) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("schrumbi mcp: {e}");
            return 2;
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("schrumbi mcp: {e}");
            return 1;
        }
    };
    let session = std::env::var("SCHRUMBI_AI_SESSION").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    let bridge = Bridge { session, forced };
    runtime.block_on(async move {
        match bridge.serve(rmcp::transport::stdio()).await {
            Ok(running) => {
                let _ = running.waiting().await;
                0
            }
            Err(e) => {
                eprintln!("schrumbi mcp: {e}");
                1
            }
        }
    })
}
