//! Every tool an agent can call: the tracker's own, then those of each enabled module.

use hub_app::{ApiResult, App, AuthUser, ToolSpec};
use serde::Serialize;
use serde_json::{json, Value};
use ts_rs::TS;

use crate::{builtin, runner_tools};

pub const OWN_MODULE: &str = "agents";

/// A tool as the web interface lists it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentToolInfo {
    pub name: String,
    pub description: String,
    /// True when calling the tool changes nothing.
    pub read_only: bool,
    /// JSON Schema of the tool's arguments.
    pub input_schema: Value,
    /// The id of the module the tool comes from; "agents" for the tracker's own.
    pub module: String,
}

/// The tools with the module each comes from. A module's tool that has the name of one
/// already listed is left out: the tracker's own tools win, then the module loaded first.
fn tools(app: &App) -> Vec<(ToolSpec, &'static str)> {
    let mut tools: Vec<(ToolSpec, &'static str)> = builtin::specs().into_iter().chain(runner_tools::specs()).map(|spec| (spec, OWN_MODULE)).collect();

    for module in app.enabled_modules() {
        for spec in module.tools() {
            if tools.iter().all(|(listed, _)| listed.name != spec.name) {
                tools.push((spec, module.meta().id));
            }
        }
    }

    tools
}

pub fn infos(app: &App) -> Vec<AgentToolInfo> {
    tools(app)
        .into_iter()
        .map(|(spec, module)| AgentToolInfo {
            name: spec.name.to_string(),
            description: spec.description.to_string(),
            read_only: spec.read_only,
            input_schema: spec.input_schema,
            module: module.to_string(),
        })
        .collect()
}

/// The tools as MCP's `tools/list` describes them.
pub fn listing(app: &App) -> Vec<Value> {
    tools(app)
        .into_iter()
        .map(|(spec, _)| {
            json!({
                "name": spec.name,
                "description": spec.description,
                "inputSchema": spec.input_schema,
                "annotations": { "readOnlyHint": spec.read_only },
            })
        })
        .collect()
}

/// Runs a tool as `user`; none when nothing has a tool of that name.
pub fn call(app: &App, user: &AuthUser, name: &str, arguments: &Value) -> Option<ApiResult<Value>> {
    builtin::call(app, user, name, arguments)
        .or_else(|| runner_tools::call(app, user, name, arguments))
        .or_else(|| app.enabled_modules().find_map(|module| module.call_tool(app, user, name, arguments)))
}
