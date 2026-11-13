# reports

Agile reports, computed on request from the issues' recorded history (the `change` table).
Nothing is stored, so a report always agrees with what the history says today.

Crate `hub-mod-reports`, `pub struct ReportsModule;`, id `reports`, enabled by default.
No tables, no background work, no events.

## Routes

All under `/api/m/reports`, all `GET`, all for signed-in callers who may see the project
(`project_access`; a private project the caller is not a member of answers 404).
Errors have the usual shape `{ "error": { "code", "message" } }`.
Response types are exported to `web/src/bindings/Report*.ts`.

Dates are UTC days (`YYYY-MM-DD`); a day's values are measured at the end of that day.

### `GET /projects/{key}/burndown?sprint=<id>&unit=points|issues`

- `sprint`: defaults to the running sprint, else the sprint completed last. An id that is not
  a sprint of this project answers 404.
- `unit`: defaults to `points`, or to `issues` when nothing in the sprint was ever estimated.
  Anything else answers 400.

```ts
ReportBurndown = {
  sprint: Sprint | null,      // null when the project has no running or completed sprint
  unit: "points" | "issues",  // the unit actually used
  committed: number,          // scope at the end of the sprint's first day
  days: ReportBurndownDay[],
}
ReportBurndownDay = {
  date: string,
  remaining: number,  // in the sprint and not done
  ideal: number,      // straight line from `committed` on the start date to 0 on the end date
  completed: number,  // in the sprint and done (a level, not a per-day amount)
  added: number,      // scope that entered the sprint during this day; 0 on the first day
  removed: number,    // scope that left during this day
}
```

`remaining + completed` is the sprint's scope that day, and
`scope(day) = scope(day - 1) + added - removed`. A change of an issue's points counts as scope
added or removed by the difference.

One entry per day from the sprint's start date: up to today for a running sprint (also past
its end date, with `ideal` 0), up to the day of completion for a completed one. That last day
is measured at the moment of completion, before the unfinished issues were moved out, so the
work a sprint hands on is still `remaining` there. A sprint that has not started has no days.
To draw the ideal line beyond today, use `committed` with `sprint.startDate`/`sprint.endDate`.

### `GET /projects/{key}/velocity?sprints=6`

`sprints`: how many completed sprints, 1 to 50.

```ts
ReportVelocity = {
  sprints: ReportVelocitySprint[],   // completed sprints, oldest first
  averageCompletedPoints: number,    // mean of completedPoints; 0 without sprints
}
ReportVelocitySprint = {
  sprint: Sprint,
  committedPoints: number, completedPoints: number,
  committedIssues: number, completedIssues: number,
}
```

Committed is what was in the sprint at the end of its start date; completed is what was in it
and done at the moment it was completed.

### `GET /projects/{key}/cumulative-flow?days=30`

`days`: 1 to 366, ending today.

```ts
ReportCumulativeFlow = { days: ReportFlowDay[] }
ReportFlowDay = { date: string, todo: number, inProgress: number, done: number }
```

### `GET /projects/{key}/created-vs-resolved?days=30`

```ts
ReportCreatedResolved = { days: ReportCreatedResolvedDay[], created: number, resolved: number }
ReportCreatedResolvedDay = {
  date: string, created: number, resolved: number,
  createdTotal: number, resolvedTotal: number,   // running totals from the first day listed
}
```

An issue is resolved once: on the day it entered the done statuses it is still in. Reopening
forgets the earlier resolution.

### `GET /projects/{key}/cycle-time?days=60`

```ts
ReportCycleTime = { issues: ReportCycleIssue[], lead: ReportPercentiles, cycle: ReportPercentiles }
ReportCycleIssue = {
  key: string, summary: string, resolvedAt: string,   // RFC 3339
  leadDays: number,           // created → resolved
  cycleDays: number | null,   // first move into an in-progress status → resolved;
                              // null when the issue was never in progress
}
ReportPercentiles = { p50: number | null, p85: number | null, p95: number | null }
```

Issues resolved in the last `days` days, oldest resolution first. Durations are days with two
decimals. Percentiles are nearest-rank (always one of the listed values) and null when there
is nothing to measure; `cycle` ignores issues without `cycleDays`.

### `GET /projects/{key}/workload`

```ts
ReportWorkload = { assignees: ReportWorkloadRow[] }
ReportWorkloadRow = {
  assigneeId: number | null,   // null for the "Unassigned" row
  name: string,
  openIssues: number, openPoints: number, inProgress: number,
}
```

The current state, not history. People with open issues, busiest first; the unassigned row,
when there is one, comes last.

## What counts

- Epics are never counted: they group work, they are not work.
- Points are counted on stories, tasks and bugs only (an epic's or a subtask's would count the
  same work twice). Issue counts include subtasks.
- "Done", "in progress" and "to do" are the categories of the project's statuses as they are
  now. Time an issue spent in a status that has been deleted since counts as to do.

## How the past is reconstructed

- **Sprint**: from the issue's `sprint` changes (`old_ref`/`new_ref`). Creating an issue
  writes no change, so before its first change it was in that change's `old_ref`; an issue
  without sprint changes has been in its current sprint since it was created.
- **Status**: from `status` changes, the same way. When a status is deleted its issues are
  moved without a change row; that move is dated by the issue's `status_since`.
- **Points**: from `points` changes (`old_value`/`new_value`), the same way.
- A change that names something without an id (a `sprint`/`status` row with a value but no
  ref) is ignored.

## Code

- `history.rs`: plain structs (`IssueHistory`, `ProjectHistory`) and "value before an instant".
- `compute.rs`: one pure function per report; they fan out over days, issues and sprints with
  Rayon. Unit tests with handcrafted histories live here.
- `load.rs`: the SQL that fills the structs.
- `routes.rs`: thin handlers.
- `tests/demo.rs`: the reports on the demo data, with numbers derived from `hub_app::seed`.
