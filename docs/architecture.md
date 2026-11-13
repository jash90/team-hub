# Architecture

```
crates/hub-core     rules and types: no network, no disk
crates/hub-app      SQLite, sign-in, the services that change things, the module host
crates/hub-server   the HTTP API (axum), the live stream, the built web interface
modules/<id>/       one optional feature each: a Rust crate (src/) and its screens (web/)
web/                the interface: React, TypeScript, Vite
integrations/       packages for other tools (the e2e reporter)
e2e/                end-to-end tests of team-hub itself
scripts/            new-module.sh: the skeleton of a module
```

## Small units

Each layer is cut by what it is about, one file per thing, behind a stable entry point:

- `hub-server/src/api/`: one file per resource (`issues`, `comments`, `sprints`,
  `project_config`, …), each exporting `routes()`; `api/mod.rs` merges them and
  `api/shared.rs` holds what several of them use.
- `web/src/components/ui/` and `components/issue-bits/`: one file per component or tight
  family. `ui.tsx` and `issue-bits.tsx` re-export them, and that is what views and modules
  import.
- `hub-app/src/services/issues/`: one file per thing that happens to an issue (`create`,
  `update`, `moving`, `comments`, `links`, `worklogs`, `attachments`, `import`, …);
  `issues/mod.rs` re-exports what is public, so callers write `services::issues::create_issue`.
- `web/src/views/<feature>/` for the large views (the issue page, search, project settings):
  the frame, and a file per part or tab. `views/<feature>.tsx` re-exports the view.

Dependencies point one way: `hub-core` ← `hub-app` ← modules ← `hub-server`, and in the
interface `web/src` ← module screens, with `web/src/modules.ts` the only place that names a
module. Tests and a build step enforce this (see "Boundaries" in [modules.md](modules.md)).

## What is built, what is fetched

- **The server.** Each module is a Cargo feature of `hub-server`, all on by default.
  `cargo build -p hub-server --no-default-features` is the tracker alone;
  `--no-default-features --features github,wiki` picks modules. A module that is left out
  has no tables, no routes and no entry in Settings.
- **The interface** is built once, with every module, and split into chunks:
  - the shell (`index`): React, the building blocks, sign-in, navigation;
  - one chunk per core view (`board`, `backlog`, `issue`, `search`, …; `views/lazy.ts`);
  - one chunk per module (`module-<id>`), holding its screens, Polish and styles;
  - the Markdown renderer, shared by the views and modules that show formatted text.

  At the start the browser fetches the shell, then, side by side, the bootstrap and the view
  in the address, then the chunks of the modules that are switched on; it shows the shell
  when those are here, so the first paint is complete. The remaining core views are fetched
  when the browser is idle, so moving between pages does not wait. A module that is off, or
  not in the build, is never fetched; one switched on in Settings is fetched at that moment.

## One core, shared

`hub-core` holds every rule as a function of its arguments: the query language parser, issue
keys and how they are found in commits and branches, ranks for ordering, workflow checks,
commit-message commands, drift between an issue's status and its code, text diff, release
notes. The server, every module and the tests use the same definitions, and the types that
cross the API are declared here once; `cargo test` generates their TypeScript counterparts
into `web/src/bindings/`, so the interface cannot drift from the server.

## Where work runs

- **Rayon is the worker pool.** A request handler does nothing itself: `App::run` moves the
  work onto the Rayon pool and awaits the answer, so the async runtime only does I/O.
- **Reads are parallel.** Every thread has its own read-only SQLite connection (WAL), so
  independent reads of one request fan out with `rayon::join` / `par_iter`: an issue page
  reads comments, history, links, children, attachments and module panels at once; the
  backlog reads each sprint at once; "My work" its four lists; search counts while it
  fetches.
- **Batch computation is parallel**: keys extracted from every commit of a push, drift for
  every issue of a project, report series, pages converted during an import.
- **Writes are serial.** One writing connection; `App::write` runs a transaction on behalf
  of an actor (a person, or a system such as "GitHub") and collects events.
- **Calls to other systems never run on the pool**; they block, so they get a thread.
- **The schedule has a thread of its own.** Once a minute it asks which slots are due and
  starts their tasks; a task started from its button gets a thread too. What a module does
  to start a task is its own business, so it never happens on the pool.

## Events

Every change emits an event inside its transaction (`issue.transitioned`, `comment.created`,
`sprint.completed`, `github.pr`, …). The `event` table is an append-only journal. After the
commit the events go to the enabled modules (`on_event`) and to the live stream
(`GET /api/stream`, server-sent events), which the interface uses to refresh what is on
screen. A person only receives events of projects they may see.

## Data

SQLite in one file under the data directory, with FTS5 for text search (text is folded, so
`zolta` finds `żółta`). Each issue change is recorded twice: as what a person reads (a status
name) and as the id behind it, which reports use. Relations between kinds of things that the
core does not know about (page ↔ issue) live in one `link` table, so a new kind of relation
needs no new table.

## Sign-in and safety

- Browser sessions are an HttpOnly cookie plus a CSRF token required on every write; API
  tokens are Bearer tokens, stored hashed and shown once.
- Passwords are hashed with Argon2; sign-in attempts are rate limited.
- Tokens of other systems (GitHub, Confluence) are encrypted at rest with the instance key and
  never returned by any route; forms show a hint and treat a blank field as "keep it".
- Webhooks are only read when signed with the configured secret, and each delivery once.
- A private project is indistinguishable from a missing one for people outside it.
- Attachments are served so that nothing in them can run script.

## Modules

See [modules.md](modules.md). A module adds tables, routes, webhooks, query-language fields,
badges on cards, a panel on the issue page, a section of "My work", tools for AI agents,
kinds of task for [the schedule](schedule.md) and its own screens, and can be switched off at runtime: for the whole instance, or for one
project (the `project_module_off` table lists what is off where; the host then leaves the
module's badges and panel out of that project's issues, and the interface its navigation).
