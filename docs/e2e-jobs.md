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
  requestedBy: number; runnerName: string | null;
  runId: number | null;         // the stored run, once the report has arrived
  error: string | null; cancelRequested: boolean;
  step: string | null;          // what the runner is doing: "cloning", "installing", "running", "uploading"
  createdAt: string; startedAt: string | null; finishedAt: string | null;
};
type E2eJobDetail = { job: E2eJob; log: { seq: number; text: string }[]; logTruncated: boolean };
```

| Request | Answer |
|---|---|
| `POST /projects/{key}/jobs` `{ ref?, filter?, video?, trace?, taskId? }` | `E2eJob` (members; 409 when the project has no `repo`) |
| `GET /projects/{key}/jobs?limit=` | `E2eJob[]`, newest first |
| `GET /jobs/{id}?after=<seq>` | `E2eJobDetail`; `log` holds the lines after `after` (the last 2000 lines are kept) |
| `POST /jobs/{id}/cancel` | `E2eJob`; a queued job is cancelled at once, a running one when the runner next looks |

Every change announces `e2e.job` `{ jobId, state }` with the project, so pages follow a job
live without polling; new log lines announce `e2e.job.log` `{ jobId, seq }` at most once a
second.

## Jobs, for runners

A runner is whoever's API token it sends; it must be able to edit the project.

| Request | Answer |
|---|---|
| `POST /jobs/claim` `{ runnerId, runnerName, projects? }` | The oldest queued job the caller may run, now `claimed`, as `{ job: E2eJob, settings: E2eSettings & { secrets: Record<string, string> } }`; `null` when there is none |
| `PATCH /jobs/{id}` `{ state?: 'running' \| 'failed', step?, commit?, error? }` | `E2eJob`; read `cancelRequested` from it |
| `POST /jobs/{id}/log` `{ lines: string[] }` | `{ seq }`; lines are cut at 2000 characters |
| `POST /projects/{key}/runs?job={id}` (the report, as today) | `E2eIngestResult`; the run is tied to the job |
| `PUT /runs/{runId}/artifacts?path=<path>` (raw body, `content-type` set) | `{ id }` — `path` is the artifact's path exactly as the report names it |
| `POST /jobs/{id}/finish` `{ error? }` | `E2eJob`: `succeeded` when a run is tied to it and no error is given, else `failed` |

A claimed or running job whose runner has not been heard from for ten minutes fails with
"the runner stopped answering". Only the runner that claimed a job may write to it.

Artifacts: at most 200 MB each and 2 GB per run; kinds by extension (`.webm`/`.mp4` video,
`.png`/`.jpg` screenshot, `.md`/`.txt`/`.json`/`.log` text, anything else a download).
`GET /artifacts/{id}` serves one to whoever can see the project, with `Range` support so a
video can be seeked, `content-disposition: inline` for media and `attachment` plus
`x-content-type-options: nosniff` for the rest. Deleting a run deletes its files.

## What a run remembers for the session view

From the report, per test result and attempt: the steps (index, api, label, source line,
status, start and duration, error, the engine actions and navigations under it, capped),
the app log lines with their step, the error and the screen text at failure, and the
artifacts the report names (kind, path, `startedAt` for recordings). `GET /results/{id}`
returns that as `E2eSession`; each artifact in it carries the id of the uploaded file when
it has arrived.

## What the runner does with a job

1. Clone or fetch `repo`, check `ref` out into a directory of its own; report `commit`.
2. In `directory`: `installCommand`, then the browser the suite's engine wants
   (`npx e2e-web install chromium`), with a browser cache that survives between jobs.
3. `npx e2e run <filter> --video=<video> --trace=<trace> <runArgs>` with the settings' `env`
   and `secrets` and nothing else of the runner's environment, as an unprivileged user,
   stopped after `timeoutMinutes`. Its output goes to the job's log as it comes.
4. Send `.e2e/report.json`, then every artifact the report names that is on disk, then
   `finish`. No report: `finish` with the reason.
