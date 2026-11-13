# agents

An MCP (Model Context Protocol) server, so coding agents such as Claude Code can read and
change issues from inside a repository.

Crate `hub-mod-agents`, `pub struct AgentsModule;`, id `agents`, enabled by default.
No tables, no background work.

Callers authenticate like everywhere else (`AuthUser`): an agent sends an API token as
`Authorization: Bearer th_…`, and every tool then runs as that person, with their permissions.
Changes go through `hub_app::services`, so history, notifications and events look the same as
when the person does it in the web interface.

## Routes

All under `/api/m/agents`. Without valid credentials they answer 401.

### `GET /info`

```ts
AgentInfo = {
  endpoint: string,        // "<base_url>/api/m/agents/mcp"
  exampleCommand: string,  // claude mcp add --transport http team-hub <endpoint> --header "Authorization: Bearer <token>"
}
```

`<token>` is a placeholder for an API token (`POST /api/me/tokens`).

### `GET /tools`

Every tool an agent can call right now: the built-in ones, then those of each enabled module.

```ts
AgentToolInfo[] = [{
  name: string,
  description: string,
  readOnly: boolean,      // calling it changes nothing
  inputSchema: JsonValue, // JSON Schema of the arguments object
  module: string,         // "agents" for built-in tools, else the id of the module offering it
}]
```

### `POST /mcp`

The MCP endpoint: Streamable HTTP in its stateless form. The body is one JSON-RPC 2.0 message;
the answer is one `application/json` body. No SSE, no session id. `GET` and `DELETE` answer 405.

| Request | Answer |
|---|---|
| `initialize` | `{ protocolVersion, capabilities: { tools: { listChanged: false } }, serverInfo: { name: "team-hub", version }, instructions }`. `protocolVersion` echoes the client's when it is `2025-11-25`, `2025-06-18`, `2025-03-26` or `2024-11-05`, else `2025-11-25`. |
| `ping` | `{}` |
| `tools/list` | `{ tools: [{ name, description, inputSchema, annotations: { readOnlyHint } }] }` |
| `tools/call` with `{ name, arguments? }` | `{ content: [{ type: "text", text }], isError }` |
| a notification (no `id`), or a client's response | HTTP 202, empty body |

`text` is the tool's output as pretty-printed JSON (or as it is, when the tool returns a string).
A tool that fails (not found, forbidden, invalid input, unknown name of a status or person)
still answers `tools/call` successfully, with `isError: true` and the message as `text`.

JSON-RPC errors:

| Code | When | HTTP |
|---|---|---|
| `-32700` | the body is not JSON | 400 |
| `-32600` | not a JSON-RPC 2.0 request, a batch (array), or an `MCP-Protocol-Version` header this server does not speak | 400 |
| `-32601` | unknown method | 200 |
| `-32602` | `tools/call` without a name, with arguments that are not an object, or naming a tool that does not exist | 200 |
| `-32603` | the tool crashed | 200 |

## Built-in tools

Tools take and return names, not ids. A name nothing answers to is a tool error that lists the
valid ones, e.g. `Unknown status "Shipped" (valid: To Do, In Progress, In Review, Done)`.
`?` marks optional arguments.

| Tool | Arguments | Returns |
|---|---|---|
| `list_projects` | none | `[{ key, name, description, boardMode, role, openIssues, totalIssues, archived }]` |
| `get_project` | `key` | `{ key, name, description, boardMode, lead, strictWorkflow, statuses: [{ name, category }], issueTypes: [{ name, level }], priorities, openSprints: [{ name, state, startDate, endDate, goal }], versions: [{ name, released, releaseDate }], components, labels, people }` |
| `search_issues` | `query` (TQL or plain words), `limit?` (20, at most 100) | `{ total, issues: [row] }` |
| `get_issue` | `key` | `{ key, url, summary, type, status, statusCategory, priority, assignee, reporter, sprint, version, points, dueDate, labels, components, parent: { key, summary } \| null, estimate, timeSpent, createdAt, updatedAt, resolvedAt, description, subtasks: [{ key, summary, status, assignee }], links: [{ relation, key, summary, status }], comments: [{ author, createdAt, body }], panels }` |
| `create_issue` | `projectKey`, `summary`, `description?`, `type?`, `priority?`, `assignee?`, `labels?`, `parentKey?`, `points?` | row |
| `update_issue` | `key`, `summary?`, `description?`, `priority?`, `assignee?`, `labels?`, `points?`, `dueDate?`, `sprint?`, `version?` | row plus `changed: [field]` |
| `transition_issue` | `key`, `status` | row |
| `add_comment` | `key`, `body` | `{ key, commentId, createdAt, comments }` |
| `log_work` | `key`, `spent` (`"1h 30m"`), `note?` | `{ key, logged, totalSpent }` |
| `my_work` | none | `{ inProgress: [row], todo: [row] }` |

- row = `{ key, summary, status, type, priority, assignee, sprint, points }`.
- `priority` is `Highest`, `High`, `Medium`, `Low` or `Lowest` (a number from 1 to 5 is accepted too).
- `assignee` is a full name, an e-mail address or `"me"`.
- `sprint` is the name of a sprint that is not completed, or `"active"`.
- In `update_issue`, `assignee`, `points`, `dueDate`, `sprint` and `version` can be `null` to clear the field.
- `status` in `transition_issue` goes through `services::issues::status_by_name`: a status name,
  or a word such as `start`, `done`, `reopen`.
- `estimate`, `timeSpent`, `logged`, `totalSpent` are durations such as `"1h 30m"`.
- `relation` is `blocks`, `is blocked by`, `duplicates`, `is duplicated by` or `relates to`.
- `panels` is what other modules contribute to the issue page, keyed by module id.
- `my_work` lists up to 100 open issues assigned to the caller.

## Tools of other modules

`tools/list` and `GET /tools` append `Module::tools()` of every enabled module, and `tools/call`
falls through to `Module::call_tool(app, &user, name, arguments)`. When two tools share a name,
the built-in one wins, then the module loaded first. A module that is switched off takes its
tools with it.

Tool work runs on the Rayon pool (`app.run`).

## Code

- `lib.rs`: the module, the three routes, the MCP methods.
- `rpc.rs`: telling requests, notifications and invalid messages apart.
- `catalog.rs`: built-in tools plus the modules' tools.
- `builtin.rs`: the built-in tools.
- `names.rs`: names ↔ ids.
- `tests/mcp.rs`: everything through the module's router.
