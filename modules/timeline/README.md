# timeline

A project's epics as bars on a time axis: Jira's timeline (roadmap).

A bar runs from the epic's **start date** to its **due date**. The due date is the tracker's
own (`issue.due_date`) and is only ever changed through `services::issues::update_issue`, so
it lands in the issue's history and is announced like any other change of it. The start date
is this module's: one table,

```sql
timeline_span (issue_id PRIMARY KEY REFERENCES issue ON DELETE CASCADE, start_date TEXT NOT NULL)
```

Crate `hub-mod-timeline`, `pub struct TimelineModule;`, id `timeline`, enabled by default, no
dependencies, no background work. Dates are days, `YYYY-MM-DD`.

## Routes

All under `/api/m/timeline`, for signed-in callers. Access is the host's (`project_access`,
`issue_access`): a private project the caller is not a member of answers 404, and so does a
project that has switched the module off. Errors have the usual shape
`{ "error": { "code", "message" } }`. Response types are exported to
`web/src/bindings/Timeline*.ts`.

### `GET /projects/{key}`

Everything the page shows, in one answer (its four reads run side by side).

```ts
TimelineProject = {
  projectKey: string,
  epics: TimelineEpic[],        // issues whose type is at the epic level, in rank order
  sprints: TimelineSprint[],    // every sprint of the project, in its order
  versions: TimelineVersion[],  // versions that are not archived
  canEdit: boolean,             // viewers read, members edit
}
TimelineEpic = {
  id: number, key: string, summary: string,
  status: { id: number, name: string, category: "todo" | "in_progress" | "done" },
  assigneeId: number | null,
  start: string | null, due: string | null,
  derived: { start: boolean, due: boolean },   // which of the two is an estimate
  counts: { total, todo, inProgress, done },   // the children by status category
  pointsDone: number, pointsTotal: number,     // story points of the children
  children: TimelineChild[],                   // in rank order
}
TimelineChild = { id, key, summary, status, assigneeId, start: string | null, due: string | null }
TimelineSprint = { id, name, state: "future" | "active" | "closed", start: string | null, end: string | null }
TimelineVersion = { id, name, released: boolean, releaseDate: string | null }
```

A sprint completed without an end date ends on the day it was completed; a version released
without a release date has the day it was released. A child's dates are its own, never
estimated.

### `PUT /issues/{key}/span`

```json
{ "start": "2026-11-02", "due": null }
```

Sets both ends; `null` clears one. Both fields must be in the body: a date that was left out
is not taken for one to clear (422). Answers with the issue's row as in the `GET`
(`TimelineEpic`; for an issue that is not an epic, the children are what is under it).

- 400 for something that is not a date, and for a start after the due date (the same day is fine).
- 403 for a viewer, 404 for an issue the caller cannot see.
- Any issue may have a span, not only an epic: a child with a start of its own gets a thin
  bar under its epic.
- When something changed, the event `timeline.updated` is emitted with the project and the
  issue (payload `{ key, start, due }`), and open pages reload. A changed due date also
  produces the tracker's `issue.updated` and a `due` entry in the issue's history. The start
  date is not part of the issue's history.

## Estimated spans

An epic that lacks a start or a due date gets the missing end from its children, and
`derived` says which end that is. Set dates are never overridden.

- **Start**: the earliest start of the children's sprints; without a dated sprint, the first
  day one of the children was in an in-progress status (its first recorded move into one;
  for a child created in such a status, the day it got there); while nothing has begun, the
  day the first child was created.
- **Due**: the latest of the children's due dates, the ends of their sprints, and the days
  they were resolved.
- A span never runs backwards: an estimated start is no later than a due date that is set,
  and an estimated due date no earlier than the start.
- Without children, or without anything to go by, the end stays `null` and is not marked.

Days are UTC days of the recorded instants. Status categories are those of the project's
statuses as they are now.

## What else the module adds

- **Issue panel** (`issue.panels.timeline`): `{ start, due, derived: { start, due } }` for an
  epic, and for any issue that has a start of its own; nothing for other issues.
- **Query language**: a date field `start` (alias `startDate`), with the operators and
  values of the core's `due`: `start >= 2026-01-01`, `start < -7d`, `start <= 2w`,
  `start >= startOfMonth()`, `start is empty`. Only a start that was set matches, never an
  estimated one. `field.rs` reads dates the way `hub_app::services::search` does (that code
  is not exposed by the host, so the rules are repeated there, with tests).
- **Tool for agents**: `get_timeline { project }`, read-only: the same data as the `GET`,
  compact (status by name, `estimated: ["start", "due"]` only where something is estimated).
- **Demo data**: the shop's three epics get dates around today, and three issues that
  already have a due date get a start. The API project's epic is left alone, so its row
  shows an estimated span.

## Code

- `model.rs`: the SQL that reads rows and their children, and the pure functions that
  estimate a span and count progress (unit tests with handcrafted children).
- `span.rs`: `set_span`, used by the route and by the demo data.
- `field.rs`: the `start` field.
- `routes.rs`: thin handlers. `lib.rs`: the module, its panel and its tool.
- `tests/api.rs`: the routes, estimates, validation, permissions, the per-project switch,
  the query field, the tool, the history of the due date, and the demo.

## Web

`web/index.tsx` adds the project page `/p/:key/timeline` (navigation entry "Timeline") and
the panel on the issue page. Polish in `web/pl.ts`, styles in `web/styles.css`.

- **The page** (`page.tsx`, `chart.tsx`, `dates.ts`): epics down the left (key, summary,
  status, progress "4 / 10"; a row opens to list its issues), time across, scrolling
  sideways under a header of months and weeks (quarters and months when zoomed out). Zoom
  is Weeks / Months / Quarters and is remembered per browser. Sprints are bands behind the
  rows with their names in the header, versions are flags on their release date, today is a
  line; "Today" scrolls to it. The legend names every mark.
- **Bars**: solid from start to due, filled by the share of children that are done. An
  estimated span is hatched with a dashed outline and captioned ("estimated", "start
  estimated", "due date estimated"). A span with only one end fades out towards the other
  and says which is missing. No dates at all: a quiet "Set dates" button in the row.
- **Editing**, for people who may edit: drag a bar to move it, drag an end to change one
  date (pointer events, so mouse, pen and touch; snapping to days; the dates show while
  dragging). The new dates show at once; a refusal takes them back and says why. Without a
  pointer: the row's menu has "Set dates…" (two date fields, "Clear") and "Clear dates";
  children have the same menu. In the dialog an estimate is a hint under the field, not its
  value, so saving does not turn an estimate into a plan; dragging an estimated bar does.
- **Live**: the page reloads on `timeline.updated` and on the tracker's events that change
  what it shows (`issue.created`, `issue.updated`, `issue.transitioned`, `issue.ranked`,
  `issue.deleted`, `sprint.*`, `version.*`) of its project.
- **No epics**: the page explains what an epic is for here and, for people who may edit,
  opens the "Create issue" dialog with the epic type chosen.
- **The issue panel** (`panel.tsx`): "Start date" with a date field (disabled for viewers),
  the due date, what is estimated, and a link to the project's timeline.

All of a day's arithmetic is done on day numbers in `dates.ts`, so no time zone moves a bar.
The column of issues narrows at 1100 px and again at 760 px, where key and summary stack
and the status stays on the issue itself.
