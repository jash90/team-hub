# The schedule

Recurring work for a project: an agent that triages yesterday's new issues every morning,
the e2e suite run after work, a summary on demand. The idea and the rules are work-hub's:
two daily slots and "on demand", catch-up rather than cron, and a failed attempt never
blanks the last good result.

The tracker's core owns the schedule and its page; **modules own the kinds of task**. A
module says what it can run on a schedule and starts it when asked; nothing here knows what
an agent run or a test job is.

## Tasks

```ts
type TaskSlot = 'morning' | 'afternoon' | 'on_demand';
type TaskState = 'idle' | 'running' | 'ok' | 'failed';
type ScheduledTask = {
  id: number; projectKey: string;
  kind: string;                 // "<module>.<name>", e.g. "agents.run", "e2e.job"
  name: string;
  config: Record<string, unknown>;   // what the kind's form holds
  slot: TaskSlot; enabled: boolean;
  runAs: number;                // the person who made it, or last saved its settings: it runs with their permissions
  state: TaskState;
  lastStartedAt: string | null; lastFinishedAt: string | null;
  lastError: string | null;     // why the last attempt failed
  progress: string | null;      // what the module said when it answered `Running`; null otherwise
  // The last attempt that worked, kept when a later one fails:
  result: { at: string; summary: string; link: string | null } | null;
  kindAvailable: boolean;       // false when the module is off or gone: shown, never run
};
type TaskKind = {
  id: string; module: string; name: string; description: string;
  fields: TaskField[];          // the form for `config`
};
type TaskField = {
  name: string; label: string; hint?: string; required: boolean;
  type: 'text' | 'textarea' | 'query' | 'select' | 'boolean' | 'number';
  options: { value: string; label: string }[];    // for select; empty otherwise
  default?: unknown;
};
type ScheduleSettings = {
  morning: string; afternoon: string;   // "HH:MM", default "10:00" and "16:00"
  timezone: string;                     // IANA name, default "Europe/Warsaw"
  weekdaysOnly: boolean;                // default true
};
```

| Request | Answer |
|---|---|
| `GET /api/task-kinds` | `TaskKind[]` of the enabled modules |
| `GET /api/projects/{key}/tasks` | `ScheduledTask[]` (kinds of modules switched off in the project are `kindAvailable: false`) |
| `POST /api/projects/{key}/tasks` `{ kind, name, config, slot, enabled? }` | `ScheduledTask` (project administrators) |
| `PATCH /api/tasks/{id}` `{ name?, config?, slot?, enabled? }` | `ScheduledTask` (project administrators) |
| `DELETE /api/tasks/{id}` | `{ ok: true }` (project administrators) |
| `POST /api/tasks/{id}/run` | `ScheduledTask`, now `running` (members who can edit; 409 while it is running) |
| `POST /api/projects/{key}/tasks/run` `{ slot }` | `ScheduledTask[]`: every enabled task of the slot started |
| `GET /api/schedule`, `PUT /api/admin/schedule` | `ScheduleSettings` |

Every change to a task announces `task.updated` `{ taskId, state, name, change }` with the
project. `change` is `"created"`, `"changed"` or `"deleted"` for what a person did to the
task, which is what the activity feed shows, and null for a run starting or ending. Saved
settings are announced as `schedule.settings`, to everyone.

What is saved is checked against the kind's form: required fields, the right type, one of
a select's options, and for a `query` field that the tracker understands the text. A
`PATCH` that carries `config` makes the caller the person the task runs as: they are the
one who last decided what it does, and it is how a task outlives the person who made it.
A task whose kind is not available cannot be run (409) and its `config` cannot be changed
(400); it can still be renamed, moved, switched off and deleted. A task that is switched
off is left out when its group runs, and still runs from its own button.

## When a slot runs

Catch-up, not cron. A slot is **due** when its time has passed today, in the schedule's
time zone, and it has not been run for today yet. A server that was down from nine to two
runs the morning slot once when it comes back, instead of losing it; one that was down all
day runs nothing for that day the next morning. With `weekdaysOnly`, Saturday and Sunday are
never due. A minute-by-minute check inside the server does this; there is nothing to
install. `on_demand` tasks run only from their button.

Tasks of a slot start one after another, in the order they were made. A task that is still
running from the slot before is left alone.

Each project remembers the day each slot last ran for, and the check writes that down
before it starts the slot's tasks: a slot never runs twice for a day, and a task or a
project added after the slot's time waits for the next day. Archived projects run nothing.
A demo instance starts as if the slots already due had run, so making one in the afternoon
does not run its morning.

## What a module does

```rust
fn task_kinds(&self) -> Vec<TaskKind>;                 // what it can run
fn start_task(&self, app: &App, task: &ScheduledTask, actor: &AuthUser)
    -> Option<ApiResult<TaskStarted>>;                 // None: not this module's kind
```

`actor` is the person the task runs as, acting through the module, so the usual access
checks apply. `TaskStarted` is either `Done { summary, link }` (the work is over, or was
handed on and needs no follow-up) or `Running { summary }`: the module then calls
`hub_app::services::schedule::finish(tx, task_id, Ok(Outcome { summary, link }) | Err(reason))`
when the thing it started ends. A task left `running` for six hours is marked failed, and
so is one that the server's stopping cut off before its module could answer.

`start_task` is never called on the worker pool: the two "run" routes answer with the
tasks marked `running` and start them on a thread of their own, and the check has its
thread. How a run ends is announced as `task.updated`.

The task runs as `runAs` whoever presses the button. When that person has been deactivated
or can no longer see the project, the attempt fails with a reason that says so, and the
module is not asked.

A failure (from `start_task` or from `finish`) sets `state: failed` and `lastError`, and
leaves `result` as it was: the page then shows the older result under a note saying when it
is from and why the newer attempt failed.

## The page

`/p/:key/schedule`, "Schedule" in the project's navigation: three groups (morning with its
time, after work with its time, on demand), each task a card with its kind, what it does,
its last result with a link, how long ago, the stale note when the last attempt failed, a
switch, **Run now**, and edit / delete for administrators; a **Run this group now** button
per group; "New task" opens the kind's form. Administrators of the instance set the two
times, the time zone and weekdays-only from the same page.
