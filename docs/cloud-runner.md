# The cloud runner

A runner without a person at it: a container that holds repositories and carries out, for a
team-hub instance, the two kinds of work that need code on disk.

- **Agent runs** (`docs/agent-runs.md`): it claims runs like the desktop app does, works in a
  working tree and branch of its own, and proposes. Nobody sits at it, so what leaves the
  machine is decided on the web (below).
- **Test jobs** (`docs/e2e-jobs.md`): it checks a repository out, runs its e2e suite with
  recording on, and sends the report, the videos and the screenshots back.

It is the desktop app's core (`apps/desktop/core`) run headless, packaged by
`apps/runner/Dockerfile`. It connects outwards only: to the team-hub server, to GitHub, to
the model provider.

## Configuration (environment)

| Variable | Meaning |
|---|---|
| `TEAM_HUB_URL` | The server, e.g. `http://team-hub.railway.internal:8080` or the public address |
| `TEAM_HUB_TOKEN` | An API token the runner signs in with (Settings → API tokens). For a shared runner: an administrator's, made for the runner alone |
| `RUNNER_SHARED` | `1` (the default when no key below is set): the runner is the team's and works with each person's own keys (`docs/runners.md` → Shared runners). `0`: it is its owner's and uses the two keys below |
| `GITHUB_TOKEN` | Only for a runner that is not shared: clones, fetches and pushes over HTTPS. Never given to an agent or to a test job |
| `ANTHROPIC_API_KEY` | Only for a runner that is not shared: the model key for agent runs; without it such a runner takes test jobs only |
| `RUNNER_NAME` | Shown on runs and jobs; default `Cloud runner` |
| `RUNNER_KINDS` | Agent run kinds it takes, comma-separated; default `implement,review,custom` |
| `RUNNER_PROJECTS` | Project keys it works for, comma-separated; default: every project the person can edit |
| `RUNNER_MAX_RUNNING` | Agent threads at once; default 2 |
| `RUNNER_TEST_JOBS` | `0` to take no test jobs |
| `RUNNER_ALLOW_SHELL` | `1` lets agent threads use a shell where no OS sandbox is available. Off by default: see Safety |
| `RUNNER_DATA_DIR` | Clones, working trees, state; default `/data` (mount a volume) |
| `RUNNER_JOB_USER` | The user test jobs run as when the runner is root; default `job` (the image has it). Root without such a user runs no jobs |
| `RUNNER_GIT_BASE` | Where `owner/name` is cloned from; default `https://github.com` |

Missing `TEAM_HUB_URL` or `TEAM_HUB_TOKEN`: the runner says so once a minute and does nothing
else, so a half-configured service stays up and readable instead of crash-looping. A value
that cannot be used is replaced by its default and named in the log. A shared runner whose
token is not an administrator's is refused by the server (403): it says so and asks again
with each heartbeat.

The runner's id is made once and kept in `RUNNER_DATA_DIR/state`, so a runner is the same
one after a restart on the same volume, and two runners with a volume each are two.

## Repositories

A run or a job names its repository as `owner/name`. The runner keeps one clone per
repository under `RUNNER_DATA_DIR/repos/<owner>/<name>` (cloned on first use, fetched before
each use) and maps it to the project by itself. The token is the one that came with the
claimed work (a shared runner: the requester's own) or `GITHUB_TOKEN` (a runner that is not
shared). It is passed to git for that one command and is in no git config, no remote
address, no file and no child process's environment. When the fetch fails with the
requester's token the work fails with the reason: an older clone is not used instead.

A shared runner keeps the keys that came with a run in memory, for as long as the run has
something left to do here, and writes them nowhere. When the runner restarts they are gone:
a run it was working on fails with "the runner restarted and no longer holds your keys",
and is started again by its requester. A runner with keys of its own instead continues its
threads after a restart, in the same session.

## What leaves the machine

The runner announces itself with `interactive: false`. Its `push` and `pull_request`
proposals are then decided on the web (`docs/agent-runs.md` → Headless runners): the person
sees the commits and the diff the runner uploaded, approves, and the runner pushes exactly
the commit that was shown, or opens the pull request through the server
(`POST /api/m/github/issues/{key}/pull-request`), and reports how it went.

- A push that the desktop app would ask a second confirmation for (CI or build files,
  something that reads like a secret, a binary, a link, an executable) is not pushed: the
  proposal fails with the reason, and the diff stays on the run.
- A pull request approved before its branch's push is decided waits for the push.
- Told to stop (`SIGTERM`), the runner ends its helpers and jobs and leaves within ten
  seconds without telling the server its runs are over: they stay its own. A job that was
  running is finished as failed, with the reason.

## Safety

- An agent thread's tools are confined to its working tree: no read or write outside it
  (`/proc`, the runner's state, other clones). No web tools, no sub-agents. The allow list
  scopes `Read`, `Edit` and `Write` to the tree, and the helper checks every path of a file
  tool itself before the tool runs (`apps/desktop/sidecar/src/confine.ts`); a file named in
  the prompt as `@path` is not attached.
- The helper's environment is made from nothing: `PATH`, proxy and certificate settings, a
  home of its own under the data folder, and the model key of that run. The runner takes
  its own secrets out of its environment at start (also out of `/proc/<pid>/environ`).
- No shell for agent threads unless the operating system's sandbox works in the container
  (bubblewrap, tried at start) or `RUNNER_ALLOW_SHELL=1`. With a shell and no sandbox, anything the agent
  reads (an issue's text, a file in the repository) could talk it into sending the model
  key somewhere: turn it on only for repositories and people you trust.
- A test job runs a repository's own code. It runs as a separate unprivileged user, with an
  environment that holds only what the project's test settings list, in a directory of its
  own; it cannot read the runner's tokens or state. The directory holds a repository with
  the one commit and no way back to the runner's clone. A runner that is not root (a
  developer's machine) runs jobs as itself and says that they are kept apart by their
  environment only. The suite is run as `npx --no -- e2e run …`: its own `e2e`, never one
  fetched from a registry.
