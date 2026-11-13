# Agent runs: the contract

An **agent run** is a piece of work on an issue handed to an AI agent. The web interface
starts runs and shows them; **runners** carry them out: the server itself for work that
needs no repository, the desktop app (or any other client with an API token) for work in
code. This file is the contract between the three; the `agents` module implements the server
side under `/api/m/agents`.

The `agents` module implements this; where it had to decide something this file leaves
open (state changes around proposals, who counts as a runner, what cancel does), its
[README](../modules/agents/README.md) is the reference.

The rule that holds everywhere: **an agent proposes, a person decides.** A run can read
whatever its requester can read, but changes to issues are *proposals* that take effect when
a person approves them (or at once, for the kinds a project has chosen to trust).

## Types

```ts
type RunKind =
  | 'refine'      // improve the description, add acceptance criteria   → update_fields proposal
  | 'breakdown'   // split into subtasks                                  → create_subtasks proposal
  | 'summarize'   // summarise description, comments and code state       → a message, no proposal
  | 'triage'      // suggest priority, labels, duplicates                 → update_fields + comment proposals
  | 'implement'   // make the change in a repository: branch, commits, pull request
  | 'review'      // review the issue's pull request
  | 'custom';     // free instructions

type RunState = 'queued' | 'claimed' | 'running' | 'waiting' | 'succeeded' | 'failed' | 'cancelled';
// waiting = the run needs a person: a question to answer or a proposal to decide.

type AgentRun = {
  id: number;
  issueKey: string;
  issueSummary: string;
  projectKey: string;
  kind: RunKind;
  instructions: string;        // extra guidance from the requester; may be empty
  state: RunState;
  requestedBy: number;         // user id
  shared: boolean;             // false: only the requester's runners may claim it
  runnerId: string | null;     // who carries it out: "server", or a runner's id
  runnerName: string | null;   // "Server", "Desktop · Bartek's MacBook"
  model: string | null;
  repo: string | null;         // "owner/name", for work in code
  branch: string | null;
  pullRequestUrl: string | null;
  summary: string | null;      // the result in a few sentences (Markdown)
  inputTokens: number | null;
  outputTokens: number | null;
  costUsd: number | null;
  cancelRequested: boolean;
  pendingProposals: number;
  pendingQuestion: boolean;
  createdAt: string;
  startedAt: string | null;
  finishedAt: string | null;
};

type RunEvent = {
  id: number;
  runId: number;
  kind: 'message' | 'tool' | 'status' | 'question' | 'answer' | 'error';
  text: string;                // Markdown for message/question/answer; one line for tool/status
  data: unknown;               // optional details (a command, a file list)
  actorId: number | null;      // the person, for answers
  at: string;
};

type ProposalKind = 'comment' | 'transition' | 'update_fields' | 'create_subtasks' | 'create_issue' | 'log_work'
  | 'push' | 'pull_request';   // decided in the runner's app, see below

type AgentProposal = {
  id: number;
  runId: number;
  kind: ProposalKind;
  payload: unknown;            // see below
  state: 'pending' | 'approved' | 'rejected' | 'failed';
  decidedBy: number | null;
  decidedAt: string | null;
  note: string | null;         // the person's reason, or why applying it failed
  result: unknown;             // what applying it produced, e.g. { keys: ["WEB-31", "WEB-32"] }
};

type AgentRunDetail = { run: AgentRun; events: RunEvent[]; proposals: AgentProposal[] };

type AgentRunner = {
  id: string;                  // stable per installation
  name: string;
  userId: number;
  kinds: RunKind[];
  projects: string[] | null;   // project keys it has repositories for; null = any
  version: string;
  lastSeenAt: string;
  online: boolean;             // seen in the last two minutes
};
```

Proposal payloads:

| Kind | Payload |
|---|---|
| `comment` | `{ body }` |
| `transition` | `{ status }` (a status name of the issue's project) |
| `update_fields` | any of `{ summary, description, priority, labels, points, assignee, dueDate }` (names, not ids) |
| `create_subtasks` | `{ items: [{ summary, description? }] }` |
| `create_issue` | `{ projectKey, summary, description?, type?, parentKey? }` |
| `log_work` | `{ spent, note? }` (`spent` as "1h 30m") |
| `push` | `{ branch, note? }`; result `{ commit }` |
| `pull_request` | `{ title, body?, head?, base? }`; result `{ url }` |

`push` and `pull_request` are different in kind: they happen on the computer that has the
code, after a person has looked at the changes there. A runner (never the server's own run)
records them so that the run shows what is waiting; nobody decides them here (`decide`
answers 409), no project can trust them in advance, and the runner says how it went with
`POST /proposals/{id}/settle`.

An approved proposal is applied through the ordinary services, as the approving person, with
the label "AI agent" in the history.

## For people (the web interface)

| Request | Answer |
|---|---|
| `POST /issues/{key}/runs` `{ kind, instructions?, model?, repo?, shared? }` | `AgentRun` (needs edit access to the project) |
| `GET /issues/{key}/runs` | `AgentRun[]`, newest first |
| `GET /runs?state=&mine=true&project=&limit=` | `AgentRun[]` the caller may see |
| `GET /runs/{id}` | `AgentRunDetail` |
| `POST /runs/{id}/cancel` | `AgentRun` (sets `cancelRequested`; a queued run is cancelled at once) |
| `POST /runs/{id}/answer` `{ text }` | `AgentRunDetail` (adds an `answer` event; `waiting` → `running`) |
| `POST /proposals/{id}/decide` `{ approve, note? }` | `AgentRunDetail` (applies the proposal when approved) |
| `GET /runners` | `AgentRunner[]` the caller may see (their own, and shared ones) |
| `GET /settings`, `PUT /settings` (admin) | model provider key (hint only), default model, which run kinds the server carries out |

## For runners

A runner authenticates with an API token and acts as that person.

| Request | Answer |
|---|---|
| `POST /runners/heartbeat` `{ runnerId, name, kinds, projects?, version }` | `{ ok: true }`; call every 30–60 s |
| `POST /runs/claim` `{ runnerId, kinds, projects?, runId? }` | `AgentRunDetail` of the oldest matching queued run, now `claimed` by this runner; `null` when there is none. With `runId`, that run or nothing. A run is offered to the requester's own runners, or to anyone's when it is `shared`. |
| `POST /issues/{key}/runs` `{ …, runnerId }` | `AgentRun` created already `claimed` by the caller's own runner (which must have sent a heartbeat), for a runner that starts its own work: no other runner can take it in between. |
| `POST /runs/{id}/events` `{ kind, text, data? }` | `{ id }` — a `question` puts the run in `waiting`. An `answer` is what the person said to the agent in the runner's own app, recorded as said by them; it answers no pending question (409 then: use `POST /runs/{id}/answer`). |
| `POST /runs/{id}/proposals` `{ kind, payload }` | `AgentProposal` |
| `PATCH /runs/{id}` `{ state?, summary?, branch?, pullRequestUrl?, model?, inputTokens?, outputTokens?, costUsd? }` | `AgentRun` — read `cancelRequested` from the answer and stop when it is set |
| `POST /proposals/{id}/settle` `{ approve, note?, result? }` | `AgentRunDetail` — for a `push` or `pull_request` of a run this runner holds: done (with `{ commit }` or `{ url }`) or declined, recorded as the person's decision. Allowed after the run has ended. |
| `GET /runs/{id}` | `AgentRunDetail` — where answers and decisions show up |

Only the runner that claimed a run may write to it. A claimed run whose runner has not been
heard from for ten minutes goes back to `queued`. Once a run has ended, writes answer 409,
with one exception: its runner may still `PATCH` `branch` and `pullRequestUrl`, because the
push and the pull request usually happen after a person has looked at the result.

## Live

Every change to a run announces `agent.run` on the live stream (`GET /api/stream`) with
`{ runId, state }`, the issue and the project, so open pages and runners need not poll.

## What a runner is given

`GET /runs/{id}` is enough to start: the issue key, the kind and the instructions. The issue
itself (description, comments, links, code state, linked pages) is read through the ordinary
API or through the MCP tools (`get_issue`, `get_issue_development`, `search_pages`, …), which
a coding agent can use directly.

## Headless runners

A runner that has nobody at it (a container, a server) says so on every heartbeat:
`{ runnerId, name, kinds, projects?, version, interactive: false }` (`interactive` defaults
to true: the desktop app). `AgentRunner.interactive` reports it.

For a run held by a headless runner, the web is where things are decided:

- **Changes.** The runner uploads what the thread has committed, so a person can look
  before anything is pushed: `PUT /runs/{id}/changes`
  `{ base, head, commits: [{ sha, subject, author? }], files: [{ path, status, additions, deletions }], patch, truncated }`
  (`patch` is a unified diff of at most 2 MB; `truncated` says it was cut; `status` is
  `added | modified | deleted | renamed`). `GET /runs/{id}/changes` returns it (404 when there
  is none) to anyone who can see the run; `AgentRun.hasChanges` says whether there is one.
- **`push` and `pull_request` proposals** are decided with the ordinary
  `POST /proposals/{id}/decide`. Approving does not apply anything on the server: the
  proposal becomes `approved` with `result: null`, which means "approved, the runner is
  carrying it out". A `push` proposal's payload carries the `commit` that was shown; the
  runner pushes that commit and no later one.
- The runner reports with `POST /proposals/{id}/result`: `{ result }` (`{ commit }` for a
  push, `{ url }` for a pull request) or `{ error }`, which turns the proposal `failed` with
  the error as its note. Only the run's runner may, only for these two kinds, only while the
  proposal is `approved` with no result yet; also after the run has ended.
- `settle` (the desktop app's call) is refused for a headless runner's run, and `decide` is
  refused for an interactive runner's `push` / `pull_request`, as before.
