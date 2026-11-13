# Test jobs: running a repository's e2e suite from team-hub

The `e2e` module so far only received reports. A **job** asks a runner to produce one: check
a repository out, run its [e2e](https://github.com/e2e-inc/e2e) suite with recording on, and
send back the report with its videos and screenshots, which the module then shows as a
session a person can watch. All routes are under `/api/m/e2e`; shapes are camelCase JSON.

## Project settings

`GET /projects/{key}/settings` (anyone who sees the project; secrets by name only),
`PUT /projects/{key}/settings` (project administrators):

```ts
type E2eSettings = {
  repo: string | null;          // "owner/name"; null: no jobs for this project
  ref: string;                  // branch, tag or commit run by default; "main"
  directory: string;            // where the suite's package.json is, relative to the repository root; "" for the root
  installCommand: string;       // default "npm ci"
  runArgs: string;              // extra arguments for `e2e run`, e.g. "--workers 2"
  video: 'on' | 'retain-on-failure' | 'on-first-retry' | 'off';   // default "on"
  trace: 'on' | 'retain-on-failure' | 'off';                       // default "on"
  env: Record<string, string>;  // plain variables for the suite, e.g. the address of the app under test
  secretNames: string[];        // names of stored secrets; values are write-only
  timeoutMinutes: number;       // a job is stopped after this long; default 30, at most 240
};
// PUT takes the same, with `secrets?: Record<string, string | null>` (null removes one) instead of secretNames.
```

A field `PUT` leaves out keeps what is stored (`repo: null` takes the repository away), and a
secret it does not name is left alone. Names of variables and secrets are what a shell can
export (letters, digits, `_`), and one name is not both.

Secret values are encrypted at rest with the instance key and leave the server in one place
only: the answer to the runner that claims a job.

## Jobs, for people

```ts
type E2eJobState = 'queued' | 'claimed' | 'running' | 'succeeded' | 'failed' | 'cancelled';
// succeeded: the suite ran and a report arrived (its tests may have failed: see the run).
// failed: the suite could not be run (clone, install, no report, timeout).
type E2eJob = {
  id: number; projectKey: string; state: E2eJobState;
  repo: string; ref: string; commit: string | null;
  filter: string | null;        // test files or a --grep pattern, as typed
  video: string; trace: string;
  requestedBy: number;
  onRunner: string | null;      // the runner the job was sent to by name (docs/runners.md); null: any that fits
  onRunnerName: string | null;  // the name that runner had then
  runnerName: string | null;
  runId: number | null;         // the stored run, once the report has arrived
  error: string | null; cancelRequested: boolean;
  step: string | null;          // what the runner is doing: "cloning", "installing", "running", "uploading"
  createdAt: string; startedAt: string | null; finishedAt: string | null;
};
type E2eJobDetail = { job: E2eJob; log: { seq: number; text: string }[]; logTruncated: boolean };
// logTruncated: the job's first lines are no longer kept (only the newest 2000 are).
```

| Request | Answer |
|---|---|
| `POST /projects/{key}/jobs` `{ ref?, filter?, video?, trace?, taskId?, onRunner? }` | `E2eJob` (members; 409 when the project has no `repo`). With `onRunner` the job waits for that runner (`docs/runners.md` → "Sending work to one runner": 400 for a runner this module has not heard from or one that does not work for the project, 403 for somebody else's computer) |
| `GET /projects/{key}/jobs?limit=` | `E2eJob[]`, newest first |
| `GET /jobs/{id}?after=<seq>` | `E2eJobDetail`; `log` holds the lines after `after` (the last 2000 lines are kept) |
| `POST /jobs/{id}/cancel` | `E2eJob` (members); a queued job is cancelled at once, a running one when the runner next looks |

A job a runner holds is only marked (`cancelRequested`) by a cancel. It becomes `cancelled`
with the runner's next `PATCH` (whose answer then says `state: "cancelled"`), or with its
`finish`, whichever comes first.

Every change announces `e2e.job` `{ jobId, state }` with the project, so pages follow a job
live without polling; new log lines announce `e2e.job.log` `{ jobId, seq }` at most once a
second (the last lines of a burst are announced when that second has passed). An uploaded
file announces `e2e.artifact` `{ runId, artifactId, path }`, saved settings `e2e.settings`.

## Jobs, for runners

A runner is whoever's API token it sends; it must be able to edit the project.

| Request | Answer |
|---|---|
| `POST /jobs/claim` `{ runnerId, runnerName, projects?, interactive?, version?, pinnedOnly?, shared? }` | The job the caller takes next, now `claimed`, as `{ job: E2eJob, settings: E2eSettings & { secrets: Record<string, string> }, credentials?: { githubToken?: string } }`; `null` when there is none |
| `PATCH /jobs/{id}` `{ state?: 'running' \| 'failed', step?, commit?, error? }` | `E2eJob`; read `cancelRequested` from it |
| `POST /jobs/{id}/log` `{ lines: string[] }` | `{ seq }`; lines are cut at 2000 characters |
| `POST /projects/{key}/runs?job={id}` (the report, as today) | `E2eIngestResult`; the run is tied to the job |
| `PUT /runs/{runId}/artifacts?path=<path>` (raw body, `content-type` set) | `{ id }` — `path` is the artifact's path exactly as the report names it |
| `POST /jobs/{id}/finish` `{ error? }` | `E2eJob`: `succeeded` when a run is tied to it and no error is given, else `failed` (`cancelled` when a cancel was asked for). A job that has already ended is answered as it ended, so a runner may always finish |

Every claim also says "this runner is here" (`docs/runners.md`): `interactive` (default
true), `version` (default `""`) and `pinnedOnly` (default false) are kept with the runner's
id, name and `projects`, and shown in `GET /api/runners`. A runner id belongs to the person
whose token used it first: 409 for anybody else. A runner is offered the jobs sent to it by
name before older ones from the queue, never a job sent to another runner, and with
`pinnedOnly` nothing from the queue at all.

`shared: true` (default false) announces the team's runner (`docs/runners.md` → "Shared
runners and people's own keys"). It is accepted only from an instance administrator's
token and with `interactive: false`: 403 "Only an administrator's runner can be shared"
otherwise. The answer to a shared runner's claim carries `credentials`, the stored keys of
the person who asked for the job: `githubToken` (their personal key `github_token`, absent
when they have stored none), for that job's clone and fetch only. A runner that is not
shared gets no `credentials` field. The token is read when the claim is answered; it is
not kept on the job and no other route returns it.

A claimed or running job whose runner has not been heard from for ten minutes fails with
"the runner stopped answering" (any write to the job counts as an answer: a `PATCH`, log
lines, the report, an upload to its run). Only the runner that claimed a job may write to
it: 403 for any other token. `PATCH` and `log` on a job that has ended answer 409, which
tells a runner that came back late to stop. A log line is stored without terminal escape
sequences.

Artifacts: at most 200 MB each and 2 GB per run; kinds by extension (`.webm`/`.mp4` video,
`.png`/`.jpg` screenshot, `.md`/`.txt`/`.json`/`.log` text, anything else a download).
`GET /artifacts/{id}` serves one to whoever can see the project, with `Range` support so a
video can be seeked, `content-disposition: inline` for media and `attachment` plus
`x-content-type-options: nosniff` for the rest. Deleting a run deletes its files.

- `path` is relative, with `/` between its parts: the report's artifact paths are, being
  relative to `<output>/results`. One that is absolute, has `..` or a `\` is refused (400);
  it names the file and is never a place on the server's disk.
- The media type is decided by the extension, not by the `content-type` sent.
- Too large is 413, for the file and for the run. Uploading a path again replaces the file
  under a new id.
- **Trace pages** are not among the report's artifacts. The runner uploads each test's
  `trace.md` under the path it has below `<output>/results`, e.g.
  `checkout-applies-the-coupon-1a2b3c4d5e6f7a8b/trace.md`; the module finds the result by
  the end of the directory's name, which is the start of the report's result id.

## What a run remembers for the session view

From the report, per test result and attempt: the steps (index, api, label, source line,
status, start and duration, error, the engine actions and navigations under it, capped),
the app log lines with their step, the error and the screen text at failure, and the
artifacts the report names (kind, path, `startedAt` for recordings). `GET /results/{id}`
returns that as `E2eSession`; each artifact in it carries the id of the uploaded file when
it has arrived.

```ts
type E2eSession = {
  run: E2eRun; result: E2eRunResult; tags: string[];
  attempts: E2eSessionAttempt[];        // empty for a JUnit run, or a test that never started
  traceArtifactId: number | null;       // the uploaded trace.md
  jobId: number | null; jobActive: boolean;   // a file not here yet may still arrive
  canEdit: boolean; bugTypeId: number | null;
};
type E2eSessionAttempt = {
  index: number; status: string; startedAt: string | null; durationMs: number;
  error: E2eSessionError | null; alsoFailed: E2eSessionError[];
  steps: E2eSessionStep[]; moreSteps: number;         // more…: how many the report had beyond those kept
  appLog: { level: string; text: string; at: string | null; step: number | null }[]; moreAppLog: number;
  failure: { url, screenshot, screen, nodes, candidates } | null;   // screenshot and screen: an artifact's `ref`
  artifacts: { ref: string; kind: string; path: string | null; url: string | null; startedAt: string | null;
               artifactId: number | null; size: number | null }[];
  environment: Record<string, string>;
};
type E2eSessionStep = {
  index: number; api: string; label: string; status: string; startedAt: string | null; durationMs: number;
  file: string | null; line: number | null; phase: string | null; error: E2eSessionError | null;
  explanation: string | null;
  events: { kind: 'engine' | 'navigation' | 'poll'; detail: string; failed: boolean; durationMs: number }[]; moreEvents: number;
  location: string | null; changes: string[]; moreChanges: number;
  artifacts: string[];                  // `ref`s of what the step produced, such as a screenshot
};
type E2eSessionError = { code, message, locator, expected, observed, file, line };
```

A step is at `step.startedAt − artifact.startedAt` in a recording. What a result keeps is
bounded (about 1 MB): past that, what steps did and saw goes first, then the later steps.
`E2eRunResult` says, for lists, how many `videos` and `screenshots` the report names and
whether the result `hasSession`; `E2eRunDetail.jobId` is the job that produced the run.

## What the runner does with a job

1. Clone or fetch `repo`, check `ref` out into a directory of its own; report `commit`.
2. In `directory`: `installCommand`, then the browser the suite's engine wants
   (`npx e2e-web install chromium`), with a browser cache that survives between jobs.
3. `npx e2e run <filter> --video=<video> --trace=<trace> <runArgs>` with the settings' `env`
   and `secrets` and nothing else of the runner's environment, as an unprivileged user,
   stopped after `timeoutMinutes`. Its output goes to the job's log as it comes.
4. Send `.e2e/report.json`, then every artifact the report names that is on disk and every
   `.e2e/results/*/trace.md` (see Artifacts), then `finish`. No report: `finish` with the
   reason.
