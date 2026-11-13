# The cloud runner

A runner without a person at it: a container that holds repositories and carries out, for
one team-hub instance, the two kinds of work that need code on disk.

- **Agent runs** ("Implement in code", "Review the pull request", a custom task): claimed
  like the desktop app claims them, carried out as a thread in a working tree and branch of
  its own. Nobody sits at this machine, so its pushes and pull requests are decided **on the
  web**: the run shows the commits and the diff, a person approves, and the runner pushes
  exactly the commit that was shown, or has the server open the pull request.
- **Test jobs**: a project's e2e suite is checked out and run with recording on, and the
  report, the videos, the screenshots and the trace pages go back to the project's Tests page.

It is the desktop app's core (`apps/desktop/core`) started headless (`hub-runner`), with the
agent helper (`apps/desktop/sidecar`) beside it. It connects outwards only: to the team-hub
server, to GitHub, to the model provider. What it is built to is in
[`docs/cloud-runner.md`](../../docs/cloud-runner.md), [`docs/agent-runs.md`](../../docs/agent-runs.md)
§ Headless runners, [`docs/e2e-jobs.md`](../../docs/e2e-jobs.md) and [`docs/runners.md`](../../docs/runners.md).

## Variables

| Variable | Meaning |
|---|---|
| `TEAM_HUB_URL` | The server: its private address (`http://team-hub.railway.internal:8080`) or the public one |
| `TEAM_HUB_TOKEN` | An API token of the person the runner acts as (team-hub → Settings → API tokens) |
| `GITHUB_TOKEN` | Clones, fetches and pushes over HTTPS. Without it only public repositories can be cloned and nothing is pushed |
| `ANTHROPIC_API_KEY` | For agent runs. Without it the runner takes test jobs only |
| `RUNNER_NAME` | Shown on runs, jobs and the Runners list; default `Cloud runner` |
| `RUNNER_KINDS` | Agent run kinds it takes, comma-separated; default `implement,review,custom` |
| `RUNNER_PROJECTS` | Project keys it works for, comma-separated; default: every project the person can edit |
| `RUNNER_MAX_RUNNING` | Agent threads at once (1–16); default 2 |
| `RUNNER_TEST_JOBS` | `0` to take no test jobs |
| `RUNNER_ALLOW_SHELL` | `1` gives agent threads a shell where the sandbox does not work. Off by default; read Safety first |
| `RUNNER_DATA_DIR` | Clones, working trees, browsers, state; `/data` in the image. Mount a volume there |
| `RUNNER_JOB_USER` | The user test jobs run as when the runner is root; `job` in the image |
| `RUNNER_GIT_BASE` | Where `owner/name` is cloned from; default `https://github.com` (a GitHub Enterprise address, or a test's stand-in) |

With `TEAM_HUB_URL` or `TEAM_HUB_TOKEN` missing the runner stays up, says once a minute what
is missing and does nothing else. A value it cannot use (a number that is not one, a kind it
does not carry out) is replaced by the default and named in the log.

## On Railway, next to the server

1. In the project that runs team-hub: **New → GitHub Repo**, the same repository again. That
   is a second service; call it `runner`.
2. Service → Settings → Build: **Dockerfile path** `apps/runner/Dockerfile`. The root
   directory stays the repository's root: the build needs the whole workspace.
3. Service → **Volumes**: one volume mounted at `/data`. (The image declares none: Railway
   attaches volumes itself.) Without it every deploy clones again, downloads the browsers
   again, and forgets the threads it was working on.
4. Service → **Variables**: `TEAM_HUB_URL=http://${{team-hub.RAILWAY_PRIVATE_DOMAIN}}:8080`
   (the server's service name in place of `team-hub`; the port is the one the server listens
   on), `TEAM_HUB_TOKEN`, `GITHUB_TOKEN`, `ANTHROPIC_API_KEY`, and whatever else from the table.
   Private networking keeps the traffic inside the project; the public address works too.
5. No domain and no port: the runner listens on nothing. One replica: two would share
   nothing and each be a runner of its own, which is fine, but each then needs its own volume.

The token is a person's: the runner sees and may edit what that person may. An account made
for it, added to the projects it should work for, keeps that small. For GitHub, a
fine-grained token with *Contents: read and write* on the repositories is enough; pull
requests are opened by the server with its own token (Settings → GitHub), so the repository
has to be connected to the project there.

Anywhere else it is the same container: `docker build -f apps/runner/Dockerfile -t team-hub-runner .`
from the repository root, a volume at `/data`, the variables. The process ends within ten
seconds of `SIGTERM`; runs it was working on stay its own and continue when it is back (the
server gives them to another runner if it stays away for ten minutes).

### Without a container

```bash
(cd apps/desktop/sidecar && npm ci --omit=dev)
cargo build --release -p hub-desktop-core --bin hub-runner
TEAM_HUB_URL=http://localhost:8080 TEAM_HUB_TOKEN=th_… GITHUB_TOKEN=ghp_… ANTHROPIC_API_KEY=sk-ant-… \
RUNNER_DATA_DIR=$HOME/.team-hub-runner target/release/hub-runner
```

Needs git and Node 22.18 or newer on the PATH. Not being root, it runs test jobs as itself
and says so.

## Safety

- **An agent thread's file tools stay in its working tree.** Reading, searching and writing
  are refused outside it by the agent's own permission rules and again by the helper before
  each call: not `/proc`, not the runner's state, not another repository's clone or another
  thread's tree. A file named in an issue's text (`@/some/path`) is not attached either. No
  web tools, no sub-agents.
- **No shell unless it can be confined.** At start the runner tries bubblewrap. Where that
  works (the platform lets the container make namespaces), a thread's shell has no network
  and writes only in its tree; where it does not (most hosted platforms, Railway among them
  at the time of writing), threads work without a shell: they read and edit files and commit
  through the runner, and cannot run the project's tests. `RUNNER_ALLOW_SHELL=1` gives them
  an unconfined shell instead. Such a shell runs as the runner's user with the network on:
  whatever the agent reads (an issue's text, a file in the repository) could talk it into
  sending the model key, or the code, somewhere. Turn it on only for repositories and people
  you trust.
- **The helper holds the model key and none of the runner's tokens.** Its environment is
  made from nothing: where programs are, proxy and certificate settings, a home of its own,
  `ANTHROPIC_API_KEY`. The runner removes `TEAM_HUB_TOKEN`, `GITHUB_TOKEN` and
  `ANTHROPIC_API_KEY` from its own environment at start, also from what the kernel shows of
  it, and closes itself to inspection by its children.
- **The GitHub token goes to git for one command at a time**, through that command's
  environment, and is offered only to the host it is for. It is in no git config, no remote
  address, and no file under `/data`.
- **A test job runs a repository's own code**, so it is kept apart: a folder of its own with
  a repository that holds one commit and no way back to the runner's clone; an environment
  of `PATH`, a home in that folder, the browsers' folder, `CI=1` and what the project's test
  settings list (variables and secrets); a process group that is ended as a whole; and the
  user `job`, who can read neither the runner's state nor its clones and cannot gain rights.
  One job runs at a time. What the suite prints goes to the job's log with the settings'
  secret values masked.
- **What needs a second look is not pushed.** A push that changes CI or build files, adds
  something that reads like a secret, a binary, a link or an executable is refused with the
  reason on the run, where the diff is: apply it by hand if it is wanted.

## What it does not do

- It does not open pull requests itself: the server does, with its own GitHub token.
- No shell for agents on platforms without user namespaces, unless you switch it on (above).
- Jobs of different projects share the browsers' folder and the `job` user. A suite could
  leave something there for the next project's suite to run into: give projects that must not
  trust each other runners of their own (`RUNNER_PROJECTS`).
- It installs a suite's dependencies fresh for every job (`npm ci`); only browsers are kept.
- It runs `npx --no -- e2e run …`: the suite's own `e2e`, never one fetched from a registry.
  The report is read from `<directory>/.e2e/report.json`, or from the folder `--output` names
  in the project's run arguments; an `output` set only in the suite's config file is not seen.
- Nothing in the image was run against a real model, real GitHub or on Railway by its
  authors; the build steps were rehearsed outside a container. See the report with the change.
