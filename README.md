# team-hub

A self-hosted issue tracker for software teams: the core of Jira, tied to the code and to
the knowledge base instead of bolted onto them.

- **The tracker**: projects, issue types, epics and subtasks, configurable workflows, scrum
  and kanban boards with drag and drop, a backlog with sprint planning, releases with
  release notes, comments with mentions, attachments, links, time tracking, watchers and
  notifications, a query language (`project = WEB AND status = "In Review" AND pr = open`),
  saved filters, bulk changes, an activity feed, and live updates in every open browser.
- **The schedule**: recurring work of a project, run every morning, every afternoon or on
  demand (an agent that triages yesterday's new issues, say). It catches up after downtime
  instead of losing a run, and a failed attempt keeps the last good result on show. What
  can be scheduled comes from the modules; see [docs/schedule.md](docs/schedule.md).
- **Code** (module `github`): branches, commits and pull requests appear on the issue they
  mention; a branch starts the issue, a pull request puts it in review, the last merge
  finishes it; commit messages can comment, log time and move issues; and *drift* shows
  where an issue's status and its code disagree ("Done, but a pull request is still open").
- **Knowledge** (modules `wiki`, `confluence`): a Markdown knowledge base with page trees,
  history and diff, pages and issues linked both ways, issues created from a page's task
  list, and import of Confluence spaces as a mirror.
- **Timeline** (module `timeline`): a project's epics as bars on a time axis, from start
  date to due date, with sprints and releases behind them; drag a bar to plan, and an epic
  nobody has dated shows the span its issues suggest.
- **Reports** (module `reports`): burndown, velocity, cumulative flow, cycle time, workload,
  all reconstructed from issue history.
- **Tests** (module `e2e`): runs of the [e2e](https://github.com/e2e-inc/e2e) testing
  framework arrive through a reporter, or are started from team-hub: a *test job* has a
  runner check the project's repository out, run its suite with recording on, and send
  everything back ([docs/e2e-jobs.md](docs/e2e-jobs.md)). A run opens as a session a person
  can watch: the recording with each step tied to its moment, the screenshots, the error,
  the trace. Each test is linked to the issues it verifies, so an issue shows whether what
  it describes actually works.
- **Runners** ([docs/runners.md](docs/runners.md)): agent runs in code and test jobs need a
  repository on disk, and a runner has one: the desktop app on somebody's computer, using
  that computer, or the [cloud runner](docs/cloud-runner.md), a container next to the
  server that the whole team shares. One page lists them all; work can be left to any
  runner that fits or sent to one by name. A shared runner holds no keys: each person
  stores their own model key and GitHub token (Settings → My keys), and their work runs
  with those.
- **AI agents** (module `agents`): work on an issue can be handed to an agent from the
  issue's page (refine the description, break it into subtasks, review the pull request,
  implement it in code, or a task in your own words). A run shows what the agent did, asks
  when it needs a decision, and ends in *proposals*: a comment, a status change, subtasks.
  **An agent proposes, a person decides**; nothing changes until someone approves it, unless
  the project's administrator chose to trust a kind of change. The server carries out the
  runs that need no code (with your own model provider key); runs that need the repository
  go to a *runner*: the desktop app, the `hub` command line, or anything that speaks
  [the run contract](docs/agent-runs.md). The module is also an MCP server, so a coding agent
  working in the repository can read its issue, comment, move it and look things up in the
  knowledge base, with the permissions of the person whose token it uses.
- **Import from Jira** (module `jira`): Jira Cloud and Server/Data Center over their API, or
  a CSV export, with the issue keys kept, history, comments, worklogs, sprints, versions and
  links; statuses, types and people are mapped in a preview before anything is written, and
  an imported project can be brought up to date again later.
- **Desktop app** ([apps/desktop](apps/desktop/README.md)): a developer's cockpit for macOS,
  Windows and Linux. The issues assigned to you become agent threads: an agent works on an
  issue in a working tree and branch of its own in your local clone, and whatever should
  leave your computer (a comment, a push, a pull request) waits for your click. The same app
  is a runner for work delegated on the web.

Everything beyond the tracker is a module that can be switched off, for the whole instance
or for one project, or left out of the build, and new ones plug in the same way: see
[docs/modules.md](docs/modules.md).

## Running it

Requirements: Rust (stable) and Node 22.

```bash
cargo build --release
```

```bash
(cd web && npm install && npm run build)
```

```bash
./target/release/team-hub
```

Open <http://localhost:8080>. The first visit creates the administrator account, optionally
with example projects, a team and a few weeks of history to look around in. The example data
comes in English and in Polish: it is written in the language the setup form is shown in
(the form follows the browser and has a switch). Over the API, `POST /api/setup` takes
`"demo": true` and `"demoLanguage": "en"` or `"pl"`; without a language the data is Polish.

Every module is in that build. To leave some out, name the ones to keep (`confluence` needs
`wiki`); the interface is built the same way either way and fetches only the screens of the
modules the server has:

```bash
cargo build --release -p hub-server --no-default-features --features github,wiki
```

A module in the build can still be switched off: for the whole instance in **Settings →
Modules**, or for one project in the project's settings. Its data stays; it shows nothing
and receives nothing until it is switched on again.

The desktop app is built separately, from `apps/desktop`; see
[its README](apps/desktop/README.md).

| Variable | Default | Meaning |
|---|---|---|
| `TEAM_HUB_PORT` | `$PORT`, else `8080` | Port to listen on |
| `TEAM_HUB_BIND` | `127.0.0.1` | Address to bind; put a TLS-terminating proxy in front before exposing it |
| `TEAM_HUB_BASE_URL` | `http://localhost:<port>` | The address people use; appears in webhook URLs and makes the session cookie `Secure` when it is `https://` |
| `TEAM_HUB_DATA_DIR` | `data` | Database, attachments, and the generated instance key |
| `TEAM_HUB_WEB_DIR` | `web/dist` | The built interface |
| `TEAM_HUB_LANDING` | on | The page about team-hub that a visitor who is not signed in sees at `/`, with the way to the sign-in and to the desktop app ([`landing/`](landing/README.md)). `off`: such a visitor gets the sign-in right away |
| `TEAM_HUB_LANDING_DIR` | `landing` | Where that page is |
| `TEAM_HUB_SECRET_KEY` | generated into the data directory | 64 hex characters; encrypts stored integration tokens |
| `TEAM_HUB_SETUP_TOKEN` | none | When set, creating the first account asks for it. Set it wherever the address is reachable by others before you have set the instance up. |

### In a container

```bash
docker build -t team-hub .
```

```bash
docker run -p 8080:8080 -v team-hub-data:/data -e TEAM_HUB_SETUP_TOKEN=choose-one team-hub
```

The image holds the server, the built interface and the landing page, listens on all interfaces, and keeps
everything in `/data`. On [Railway](https://railway.com) the repository deploys as it is:
connect it to a service, add a volume mounted at `/data`, a public domain, the variables
`TEAM_HUB_BASE_URL=https://${{RAILWAY_PUBLIC_DOMAIN}}` and `TEAM_HUB_SETUP_TOKEN`, and set
the service's healthcheck path to `/api/bootstrap`.

Back up the data directory. Without the instance key the stored integration tokens cannot be
read and have to be entered again; everything else is in `team-hub.db`.

## Development

```bash
cargo test --workspace
```

```bash
(cd web && npm run dev)
```

The dev server proxies `/api` to a team-hub running on port 8080. Tests use no network:
other systems are faked on a local port. `cargo test` also regenerates the TypeScript types
in `web/src/bindings/` from the Rust ones.

## Layout

| Path | What it is |
|---|---|
| `crates/hub-core` | Rules and types; no network, no disk. Shared by everything. |
| `crates/hub-app` | SQLite store, sign-in, services, the module host. |
| `crates/hub-server` | HTTP API, live stream, serves the interface. The `team-hub` binary. |
| `modules/<id>` | One optional feature each: Rust crate in `src/`, screens in `web/`. |
| `web` | The interface: React, TypeScript, Vite. Polish and English. |
| `apps/desktop` | The desktop app: `core` (no window; also the `hub` command line), `src-tauri` (the window), `ui`, `sidecar` (runs the agent). |
| `integrations/e2e-reporter` | Reporter that sends e2e test runs to team-hub. |
| `e2e` | End-to-end tests of team-hub itself, written with the e2e framework. |

How the pieces fit, where work runs (Rayon as the worker pool, parallel reads, serial
writes) and the safety rules are in [docs/architecture.md](docs/architecture.md).

## The query language

```
project = WEB AND sprint in openSprints() AND assignee = currentUser()
type = Bug AND priority >= High AND created >= -7d ORDER BY priority DESC
text ~ "kod pocztowy" AND category != done
category = done AND pr = open            -- with the github module
drift is not empty                        -- status and code disagree
```

Fields: `project`, `key`, `type`, `status`, `category`, `priority`, `assignee`, `reporter`,
`watcher`, `label`, `component`, `sprint`, `version`, `parent`/`epic`, `points`, `text`,
`summary`, `description`, `comment`, `created`, `updated`, `resolved`, `due`, plus whatever
enabled modules add. Plain words search the text; a bare key opens the issue.

## What it does not do yet

Each of these is meant to arrive as a module: e-mail notifications, custom fields,
automation rules beyond the code-driven ones, GitLab as a second code host (the drift rules
and badges are host-neutral already), single sign-on.

Known gaps in what is there:

- Tested only against local fakes, never against the real thing: the Jira and Confluence
  imports, the GitHub API calls, and the server's own agent runs (a fake model). The
  desktop app has run on Linux only; its macOS and Windows sandboxes, the tray,
  notifications and installers are unverified.
- A shared runner signs in with an administrator's API token: it has no identity of its
  own with narrower rights. Recordings were verified with videos made by hand in the
  framework's format, not with a real recorded run; the cloud runner has never run on a
  real host.
- The server writes English. The interface puts its errors, notifications and status lines
  into Polish as they arrive, from a dictionary the build checks against the server's
  sources. The descriptions of agent tools, which the agent itself reads, stay English.

## Licence

MIT; see [LICENSE](LICENSE).
