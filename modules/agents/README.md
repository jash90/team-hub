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

## Agent runs

The server side of `docs/agent-runs.md`: people hand work on an issue to an agent, runners
carry it out, and what the agent wants to change comes back as proposals a person decides.
Tables `agent_run`, `agent_run_event`, `agent_proposal`, `agent_runner`. Every change to a
run announces `agent.run` (`{ runId, state, key }`, with project and issue) on the live stream.

Types are the contract's with an `Agent` prefix in `web/src/bindings/` (`AgentRun`,
`AgentRunDetail`, `AgentRunEvent`, `AgentProposal`, `AgentRunner`, `AgentRunKind`, …).

### For people

| Request | Answer |
|---|---|
| `POST /issues/{key}/runs` `{ kind, instructions?, model?, repo?, shared? }` | `AgentRun`; needs edit access |
| `GET /issues/{key}/runs` | `AgentRun[]`, newest first |
| `GET /runs?state=a,b&mine=true&project=KEY&limit=` | `AgentRun[]` in projects the caller sees |
| `GET /runs/{id}` | `AgentRunDetail` |
| `POST /runs/{id}/cancel` | `AgentRun`; needs edit access |
| `POST /runs/{id}/answer` `{ text }` | `AgentRunDetail`; needs edit access |
| `POST /proposals/{id}/decide` `{ approve, note?, payload? }` | `AgentRunDetail`; needs edit access |
| `GET /runners` | `AgentRunner[]`: the caller's own, and other people's that are online |
| `GET /settings` | `AgentSettings` (everyone; the key's hint only for administrators) |
| `PUT /settings` `{ apiKey?, clearApiKey?, apiUrl?, defaultModel?, serverKinds?, monthlyCapUsd? }` | `AgentSettings`; administrators |
| `GET /projects/{key}/trust` | `AgentTrust` `{ kinds: ProposalKind[] }` |
| `PUT /projects/{key}/trust` `{ kinds }` | `AgentTrust`; project administrators |

```ts
AgentSettings = {
  keyConfigured: boolean, keyHint: string | null, apiUrl: string,
  defaultModel: string, models: { id, name, inputPrice, outputPrice }[],  // USD per million tokens
  serverKinds: RunKind[],              // what the server carries out itself
  monthlyCapUsd: number | null, spentThisMonthUsd: number,
  serverStatus: 'ready' | 'no_key' | 'cap_reached',
}
```

### For runners

`POST /runners/heartbeat`, `POST /runs/claim`, `POST /runs/{id}/events`,
`POST /runs/{id}/proposals`, `PATCH /runs/{id}`, `GET /runs/{id}`: bodies and answers as in the
contract. A runner is whoever's API token it sends; only the person whose runner claimed a run
may write to it (403 otherwise, 409 once the run has ended).

### How it behaves (where the contract leaves room)

- **States.** A `question` puts the run in `waiting`; the answer puts it back in `running`.
  A proposal from a runner elsewhere puts the run in `waiting` too; the runner's next event, or
  the last decision, puts it back in `running`. The server's own run stays `running` until the
  model is through, then is `waiting` while proposals are undecided and `succeeded` after the
  last decision. `PATCH` accepts `running`, `waiting`, `succeeded`, `failed`, `cancelled`.
  Proposals can still be decided after a run has ended (not after it was cancelled).
- **Cancel.** A queued run, or a server run waiting for a person, is cancelled at once; any
  other gets `cancelRequested` and the runner stops. Cancelling rejects undecided proposals.
- **Claim** is one transaction (transactions are serialised), offers the oldest queued run of
  a wanted kind in a project the caller can see, requested by the caller or `shared`.
- **Silence.** A `claimed` or `running` run whose runner has neither written to it nor sent a
  heartbeat for ten minutes returns to `queued` (a sweep every 30 s). A run that waits for a
  person is left alone. Runs the server was working on when it stopped are requeued at start.
- **Proposals** are checked on arrival against the issue's project (unknown status, person,
  type, priority → 400 with the valid names) and stored tidied. Each carries `preview`
  (not in the contract): `{ changes: [{ field, before, after }], descriptionDiff: [{ kind, text }] | null, allowed, issueType }`,
  computed against the issue as it is now while undecided, frozen at the decision.
  Approval applies through `hub_app::services` as the approver with `Actor::user_via(id, "AI agent")`,
  in one transaction: if anything fails nothing changes and the proposal is `failed` with the
  reason in `note`. `decide` may carry `payload`, the proposal as the person edited it.
  `result`: `{ changed, previousDescription? }`, `{ keys }`, `{ key }`, `{ commentId }`, `{ worklogId }`.
  New subtasks join their parent's sprint.
- **Trust.** A proposal of a kind the project trusts is applied at once, as the run's requester.
- **Notifications** (kind `agent`) go to the requester: a question, each proposal, finished, failed.

### The server as a runner

For `refine`, `breakdown`, `summarize`, `triage`, and `custom` without a repository, when a key
is configured, the kind is switched on and this month's cap is not spent. One worker thread
(`worker.rs`, never the Rayon pool) takes the oldest such run, reads the issue through the same
data `get_issue` and `get_project` return (as the requester), and asks the model
(`POST {apiUrl}/v1/messages`, `anthropic-version: 2026-10-21`, tool use). The model's text
becomes `message` events, its tool calls proposals; a refused proposal goes back to the model
as an error to correct. Limits: 6 steps, 8,000 output tokens per answer, 400,000 tokens per
run. 429, 529 and 5xx are retried four times with growing pauses (or `retry-after`). Cost is
computed from the reported usage and the price list in `settings.rs`.

The model can do nothing except through proposals and events. Tracker text reaches it inside
one delimited block it is told not to take instructions from; the block cannot be closed from
inside (`<` is escaped).

### MCP tools for runners

`list_my_runs`, `get_run { runId }`, `report_progress { runId, text }`,
`ask_question { runId, text }`, `propose { runId, kind, payload }`,
`finish_run { runId, summary, state?, branch?, pullRequestUrl? }`. The first write to a queued
run the caller may take claims it (runner id `mcp-<userId>`).

### What other screens get

- `issue.ext.agents = { working, waiting }` on rows with a run in progress or waiting for a person.
- `issue.panels.agents = { runs: AgentRun[] }`.
- `dashboard.ext.agents = { waiting: AgentRun[] }`: the viewer's runs that need them.

## Web

`web/index.tsx`: the issue panel "AI agent" (`panel.tsx`), the page `/agents` (`page.tsx`), the
run and proposal views shared by both (`runs.tsx`), the settings tab and the project settings
tab "AI agent" (`settings.tsx`), badges and the "My work" section.

## Code

- `runs.rs`: runs, events, runners, claim, the sweep for silent runners.
- `proposals.rs`: checking, previewing, deciding and applying proposals; project trust.
- `worker.rs`, `model.rs`, `settings.rs`: the server as a runner, the model client and prompts, its settings.
- `routes.rs`, `runner_tools.rs`: the HTTP routes and the MCP tools over the same functions.
- `tests/runs.rs`: all of the above against a model endpoint faked on `127.0.0.1`.
- `lib.rs`: the module, its hooks, the MCP endpoint and its methods.
- `rpc.rs`: telling requests, notifications and invalid messages apart.
- `catalog.rs`: built-in tools plus the modules' tools.
- `builtin.rs`: the built-in tools.
- `names.rs`: names ↔ ids.
- `tests/mcp.rs`: everything through the module's router.
