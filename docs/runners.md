# Runners: where the work is carried out

Two kinds of work need a repository on disk: **agent runs** (`docs/agent-runs.md`) and
**test jobs** (`docs/e2e-jobs.md`). The server never does either. A **runner** does: a
machine that has signed in with somebody's API token, holds the repositories, and asks the
server for work.

| Runner | What it is | `interactive` | Whose resources |
|---|---|---|---|
| The desktop app | Somebody's computer, with the person at it | `true` | That computer's CPU, memory and disk; the person's own model sign-in |
| The cloud runner (`docs/cloud-runner.md`) | A container, e.g. a service next to the server | `false` | The container's; the model key set on it |

A runner has one stable id (per installation) and uses it for both kinds of work. The web
interface and the desktop app show the same list and the same runs and jobs: both read the
server, and the server announces every change.

## Being there

| Work | How a runner says it is there |
|---|---|
| Agent runs | `POST /api/m/agents/runners/heartbeat` (as before) with `interactive`, `pinnedOnly` |
| Test jobs | Every `POST /api/m/e2e/jobs/claim` counts; it carries `runnerId`, `runnerName`, `interactive`, `version`, `pinnedOnly` |

`pinnedOnly: true` means "give me only what was sent to me by name". The desktop app takes
test jobs this way unless its owner turns on taking the team's queue: running a suite is
running a repository's code on that person's computer, so it happens when they asked for it.

A runner is **online** while it was heard from in the last two minutes.

## The list

`GET /api/runners` (any signed-in person): your own runners, and other people's while they
are online. Modules contribute what they know (`Module::runners`); the core merges by id.

```ts
type Runner = {
  id: string; name: string; userId: number;
  interactive: boolean;         // somebody's computer (true) or a service (false)
  online: boolean; lastSeenAt: string; version: string | null;
  work: RunnerWork[];           // one entry per module that knows it
};
type RunnerWork = {
  module: string;               // "agents" | "e2e"
  what: string[];               // agents: the run kinds it takes; e2e: ["tests"]
  projects: string[] | null;    // project keys it works for; null: any it may edit
  pinnedOnly: boolean;
  online: boolean; lastSeenAt: string;
  active: { title: string; href: string }[];   // what it is carrying out now, with a page to open
};
```

## Sending work to one runner

Both `POST /api/m/agents/issues/{key}/runs` and `POST /api/m/e2e/projects/{key}/jobs` take an
optional **`onRunner`**: a runner's id. The run or job then waits for that runner and is
offered to nobody else; without it, any runner that fits takes it, as before.

- The runner must be known to that module and take that work in that project, else 400.
- A person's computer (`interactive`) takes work by name only from its owner: 403 for
  anyone else. A service takes it from anyone who may start that work in the project.
- The runner being offline is not an error: the work waits, and the page says for whom.
- A run or job pinned to a runner stays pinned when the runner stops answering and it is
  queued again. To move it, cancel it and start another.

`AgentRun.onRunner` / `E2eJob.onRunner` hold the id (`null` when not pinned), and
`onRunnerName` the name it had when the work was sent.

This is how a person chooses whose resources are used: "this computer" in the desktop app
pins to the app's own id; "Cloud runner" pins to the service; "any" leaves it to the queue.

## What the desktop app does with this

- **Runs started in the app are carried out by the app**, on the computer: an agent thread
  is created already claimed by the app's runner id (as before), and a test run is a job
  pinned to it. Nothing of that uses the server's or the cloud runner's CPU; the server
  only stores what comes back (events, the report, videos, screenshots).
- The app can also hand either to another runner: the same `onRunner`, picked from the
  list. It then follows the run or job like the web does.
- **Runners** shows `GET /api/runners`, with this computer marked.
- **Tests** shows the project's jobs and starts them; a finished job opens its session
  (`docs/e2e-jobs.md`) in the browser, where the recording is.
- The work queue shows runs wherever they are carried out, each with its runner's name.
