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

Missing `TEAM_HUB_URL` or `TEAM_HUB_TOKEN`: the runner says so once a minute and does nothing
else, so a half-configured service stays up and readable instead of crash-looping.

## Repositories

A run or a job names its repository as `owner/name`. The runner keeps one clone per
repository under `RUNNER_DATA_DIR/repos/<owner>/<name>` (cloned on first use, fetched before
each use) and maps it to the project by itself. The token is the one that came with the
claimed work (a shared runner: the requester's own) or `GITHUB_TOKEN` (a runner that is not
shared). It is passed to git for that one command and is in no git config, no remote
address, no file and no child process's environment. When the fetch fails with the
requester's token the work fails with the reason: an older clone is not used instead.

## What leaves the machine

The runner announces itself with `interactive: false`. Its `push` and `pull_request`
proposals are then decided on the web (`docs/agent-runs.md` → Headless runners): the person
sees the commits and the diff the runner uploaded, approves, and the runner pushes exactly
the commit that was shown, or opens the pull request through the server
(`POST /api/m/github/issues/{key}/pull-request`), and reports how it went.

## Safety

- An agent thread's tools are confined to its working tree: no read or write outside it
  (`/proc`, the runner's state, other clones). No web tools, no sub-agents.
- No shell for agent threads unless the operating system's sandbox works in the container
  (bubblewrap) or `RUNNER_ALLOW_SHELL=1`. With a shell and no sandbox, anything the agent
  reads (an issue's text, a file in the repository) could talk it into sending the model
  key somewhere: turn it on only for repositories and people you trust.
- A test job runs a repository's own code. It runs as a separate unprivileged user, with an
  environment that holds only what the project's test settings list, in a directory of its
  own; it cannot read the runner's tokens or state.
