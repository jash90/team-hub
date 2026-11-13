# Writing a module

Everything beyond the tracker's core lives in a module: one folder under `modules/<id>/`
holding a Rust crate (`Cargo.toml`, `src/`) and, when it has screens, a `web/` folder.
A module can be switched off at runtime (Settings → Modules); while off, its routes answer
404 and it receives no events. A module that is on can also be switched off in one project
(Project settings → Modules); see "Per project" below.

`scripts/new-module.sh <id> "<Name>"` creates the skeleton described here (a crate with one
tested route, a `web/` folder with one page) and prints the lines that register it.

## Registering a module

A module is named in two places, once for each half; nothing else in the workspace knows it.

- **The server**, `crates/hub-server`: a Cargo feature named by the module's id, the
  optional dependency behind it, and its line in `modules()`.

  ```toml
  [features]
  default = ["github", "wiki", …, "notes"]        # to have it in the usual build
  notes = ["dep:hub-mod-notes"]                    # plus the ids it depends on: ["dep:…", "wiki"]

  [dependencies]
  hub-mod-notes = { path = "../../modules/notes", optional = true }
  ```

  ```rust
  #[cfg(feature = "notes")]
  modules.push(Arc::new(hub_mod_notes::NotesModule));
  ```

- **The interface**, `web/src/modules.ts`: one line in `SCREENS`.

  ```ts
  notes: () => import('../../modules/notes/web/index.tsx'),
  ```

A module folder that is not registered fails `cargo test -p hub-server` and `npm run build`,
each with the line to add.

**Choosing modules at build time.** `cargo build -p hub-server` has every module of `default`;
`--no-default-features` builds the tracker alone, `--no-default-features --features github,wiki`
picks. A build tells the interface which modules it has (the bootstrap lists them), and the
interface fetches the screens of those only, so one built `web/dist` serves any selection.

## The crate

- Package name `hub-mod-<id>`; depends on `hub-core` and `hub-app` (workspace dependencies).
- `src/lib.rs` exports one unit struct, e.g. `pub struct WikiModule;`, implementing
  `hub_app::Module` (`crates/hub-app/src/module.rs` documents every hook).
- `hub-server/src/lib.rs::modules()` lists the modules compiled into the build, each under
  its Cargo feature (see "Registering a module").
- `ModuleMeta::depends_on` names the modules this one needs, written as a plain list
  (`depends_on: &["wiki"]`): the boundary test reads it from the source, so that it can check
  a module the build leaves out.

## What the host gives a module

| Need | Use |
|---|---|
| Tables | `fn migrations()`: SQL strings, applied once each, in order. Prefix tables with the module id. Only ever append. |
| Routes for signed-in people | `fn routes() -> Router<App>`, mounted at `/api/m/<id>`. Take `AuthUser` as an extractor. |
| Routes other systems call | `fn hooks()`, mounted at `/hooks/<id>`. The module authenticates the caller (signature, token). |
| Reacting to changes | `fn on_event(&App, &Event)`, called after the transaction committed. |
| Badges on issue cards and rows | `fn decorate_issues`: put a JSON value into `row.ext["<id>"]`. |
| A panel on the issue page | `fn issue_panel` → JSON, delivered as `issue.panels["<id>"]`. |
| Query language fields | `fn query_fields` (for the editor) and `fn compile_field` (SQL on the issue aliased `i`). |
| Global quick search | `fn quick_search` → `QuickHit`s. |
| A section of "My work" | `fn dashboard`. |
| Tools for AI agents (MCP) | `fn tools` + `fn call_tool`. |
| Work on a schedule | `fn task_kinds` (what it can run, with the form for it) and `fn start_task` (run one). See below. |
| Background work | `fn start(&App)`: spawn a plain thread; never block the Rayon pool on the network. |
| Demo data | `fn seed_demo(&App, DemoLanguage)`. See below. |

### Work on a schedule

The core owns the schedule ([schedule.md](schedule.md)): tasks, the two daily slots, the
page. A module owns the *kinds* of task. `task_kinds()` lists them: each a `TaskKind` with
the id `"<module id>.<name>"`, `module` set to the module's id (the host leaves out a kind
that is not named after its module), a name and a description, and the `fields` of its
form. The host draws the form, checks what is saved against it (required, the right type,
one of the options, a query the tracker understands) and stores it as the task's `config`;
what the values mean is the module's to check when the task runs.

`start_task(app, task, actor)` runs one: answer `None` for a kind that is not yours. It is
called outside any transaction, on a thread of the schedule's, never while the module is
off for the instance or for the task's project. `actor` is the person the task runs as,
already acting through the module, so `project_access`, `issue_access` and
`visible_projects` say what they may do; scope the work to `task.project_key` yourself,
whatever the task's settings say. Answer quickly:

- `Ok(TaskStarted::Done { summary, link })` when the work is over or handed on;
- `Ok(TaskStarted::Running { summary })` when it goes on. The task then stays `running`
  until the module calls `hub_app::services::schedule::finish(tx, task.id, outcome)` in the
  transaction in which the work ends: `Ok(Outcome { summary, link })`, or `Err(reason)`.
  Keep `task.id` with whatever you started (a column on your job's row). `finish` does
  nothing for a task that was deleted meanwhile, and a task nobody finishes is marked
  failed after six hours;
- `Err(…)` when it cannot start; the error's message is what the page shows.

Summaries, reasons, and the names, labels and hints of a kind are English, like everything
the server says, with their Polish in `web/src/i18n/server.pl.ts`. The build finds the
texts written in place (`TaskKind { name: "…" }`, `TaskStarted::Done { summary: format!(…) }`,
an `ApiError`'s message) and fails when one has no Polish; for a text that reaches `finish`
some other way, add the entry all the same. `modules/agents/src/scheduled.rs` is the
reference: one kind, its form, its start, and its example tasks for the demo.

### Demo data

The example data of a new instance comes in English and in Polish, and whoever sets the
instance up gets it in the language of their interface. It is one data set: the same
projects, issue keys, statuses, people's roles, dates, numbers and commit SHAs in both, so
only what a person reads differs. `seed_demo(app, language)` therefore keeps one structure
and writes every text as a pair, `DemoText::new("Password reset", "Reset hasła")`
(`hub_core::demo`), picked with `.pick(language)`; `DemoText::same("seo")` is a text that
reads the same in both. Long texts (a page's body) are written out in full in each language.

Names that seeds share have to agree in each language: a branch a test run names is the
branch of the demo's pull request, a `[[link]]` in a page names the title of the page in the
same language. An id nobody reads (a commit's SHA, a test's id) is made from one of the two
texts, always the same one, so it does not depend on the language.
`crates/hub-server/tests/demo_languages.rs` seeds both and compares them, and fails when the
English demo has a Polish word in it: when a seed gets a new text, add the places it is
shown to the walk in that test if they are not there yet.

## Per project

A project administrator can switch a module off for their project
(`PUT /api/projects/{key}/modules/{id}` with `{ "enabled": false }`); the project then lists
it in `disabledModules`. The module is not asked and does not have to know:

- the host drops the module's `ext` entry from that project's issue rows and does not call
  `issue_panel` for its issues;
- a request inside the module's routes (`/api/m/<id>/…`) does not see the project: the user
  it is given carries the module (`AuthUser::through`), and `access::project_access`,
  `issue_access` and `visible_projects` treat a project that has switched the module off
  as one that is not there. The same user is handed to `call_tool`, `dashboard` and
  `quick_search`, so whatever they look up with the usual access checks leaves the project
  out too;
- the module's query-language fields match nothing in that project;
- `on_event` is not called for events about that project;
- the interface leaves the module out of that project: its project navigation, its pages
  under `/p/:key/…`, its tab in the project's settings, its panel and its badges.

All of this follows from using the host's access checks, which a module does anyway. What
the host cannot see is left to the module: a query over its own tables that never asks
about access, work it starts itself (`start`), and calls from other systems (`hooks`),
which carry no user. There the module adds
`hub_app::services::project_modules::on_in_project("<id>", "<project id column>")` to its
query, or reads `project_modules::disabled_modules(connection, project_id)`: the code
module does both, so a project that switched it off takes no webhook deliveries and is not
polled. A module that authenticates callers itself wraps the user in `.through("<id>")`.

Every open browser learns of a switch at once: it is announced as `project.updated`, and the
interface then asks for the list of projects again.

## Rules

- **Handlers are thin.** `app.run(move |app| …).await` moves the work onto the Rayon pool;
  inside, `app.read(|connection| …)` reads (any number at once, each thread has its own
  connection) and `app.write(actor, |tx| …)` runs one transaction. `tx` dereferences to the
  connection and `tx.emit(kind, project_id, issue_id, payload)` announces what happened.
- **Inside `app.write`, read through `tx`**, never through `app.read` (it would not see the
  write), and do not start Rayon work there.
- **Rayon is for computation** (`par_iter`, `rayon::join` over independent reads). Calls to
  other systems are blocking I/O: run them with `tokio::task::spawn_blocking` or on a thread.
- **Access**: `hub_app::access::project_access(connection, &user, key)` returns the caller's
  role or "not found"; call `.require_edit()` / `.require_admin()` before changing things.
  Instance-wide settings need `user.require_admin()`.
- **Secrets** go through `hub_app::secrets` (encrypted at rest) and are never returned by a
  route; show `secret_hint` instead. A blank field in a settings form means "keep what is stored".
- **Changing issues** goes through `hub_app::services::issues` (`create_issue`, `update_issue`,
  `add_comment`, …), so history, notifications, search and events stay right. A system acting
  on its own uses `Actor::system("GitHub")`; a matched person uses `Actor::user_via(id, label)`.
- **Relations between kinds of things** (page ↔ issue, pull request ↔ issue) go into the
  shared `link` table (`from_kind, from_id, to_kind, to_id, relation`).
- **Types that cross the API** derive `serde::Serialize` with `rename_all = "camelCase"` and
  `ts_rs::TS` with `#[ts(export)]`; `cargo test` writes them to `web/src/bindings/`.
- **Tests use no network**: build the app with `App::new(Config::for_tests(dir), vec![…])`,
  serve fake upstreams from `127.0.0.1:0`.

## The web half

The default export of `modules/<id>/web/index.tsx` is one `WebModule` (the type, with a
comment on every field, is in `web/src/modules.ts`, where the module is also listed). It only
shows up where it has something: project navigation, full pages, badges on issue cards, a
panel on the issue page, a section of "My work", tabs in project and instance settings.
`modules/github/web/index.tsx` uses every one of these and is the reference.

A module's screens, its Polish and its styles are a chunk of their own
(`assets/module-<id>-….js` and `.css`). The browser fetches the chunks of the modules that
are switched on, side by side, right after the bootstrap and before it shows anything; a
module that is off, or not in the build, costs the browser nothing, and one switched on later
is fetched then, without a reload. What `index.tsx` imports at the top is therefore what
every page waits for: keep a heavy part that only one page needs behind `lazyView` from
`@/lazy.tsx`.

- **Imports**: `react`, and the application through the `@/` alias (`web/src`). Nothing else
  resolves from a module folder, so no extra packages: charts are hand-drawn SVG.
  - `@/api.ts`: `api.get/post/patch/put/del/upload`, `query()`, `errorMessage()`.
  - `@/session.tsx`: `useSession()` (user, people, `moduleEnabled`), `useRemote(load, deps, live?)`
    (data that reloads when the server announces a change the predicate accepts),
    `useProject()` inside project pages, `useStored()` for per-browser interface state.
  - `@/shell.ts`: `useShellProject(key)`, for a page outside `/p/:key/…` that belongs to a
    project all the same (a knowledge-base space bound to one): the shell then shows that
    project's navigation instead of the last one used.
  - `@/components/ui.tsx`: `Button`, `Dialog`, `ConfirmDialog`, `Picker`, `Menu`, `Popover`,
    `Tabs`, `Tag`, `Field`, `Empty`, `Spinner`, `ErrorNote`, `ProgressBar`, `Avatar`,
    `InlineEdit`, `SettingsCard`, `attempt()`, `useBusy()`, `toast()`, `copyText()`. Each lives
    in its own file under `components/ui/`; import them from `ui.tsx`, which is what stays
    stable.
  - `@/components/icon.tsx` (`Icon`, the names are the keys of its table),
    `@/components/issue-bits.tsx` (`IssueLine`, `IssueKey`, pickers for issue fields),
    `@/components/markdown.tsx` (`Markdown`, `MarkdownEditor`), `@/router.tsx` (`Link`,
    `navigate`, `useSearchParams`, `setParam`).
  - `@/modules.ts`: the `WebModule` type, and `useModules(project?)` for the rare module that
    renders what other modules contribute.
  - `@/bindings/*`: the types generated from Rust. Never write those shapes by hand.
- **Routes**: a pattern starting with `/p/:key/` renders inside the project (so `useProject()`
  works); anything else is a top-level page, which can still name its project with
  `useShellProject()`.
- **Text**: English in place through `t('…', { name })`; `modules/<id>/web/pl.ts` holds the
  Polish for every phrase the module uses (the key is the English text). Dates and durations
  come from `@/i18n/index.ts` (`formatDate`, `formatDateTime`, `timeAgo`, `countOf`).
- **What the server says**: a module's Rust half writes English too (error messages,
  notification texts, status lines). The interface translates them on arrival with
  `translateServer` from `@/i18n/server.ts`; errors from `api` already are. The Polish for
  them is in `web/src/i18n/server.pl.ts`, and `npm run build` fails when a message in the
  Rust sources has no entry there, or an entry has no message. Wrap in `translateServer`
  only what the server wrote, never what a person did.
- **Looks**: reuse the application's classes (`view`, `view-head`, `panel`, `panel-head`,
  `table`, `tag`, `form`, `inline-form`, `muted`, …) and its CSS variables (`--surface`,
  `--sunk`, `--ink`, `--muted`, `--line`, `--accent`, `--green`, `--amber`, `--red`,
  `--blue`, `--purple` and their `-soft` variants), which follow the light and dark themes.
  A module's own rules go in `modules/<id>/web/styles.css`, imported from its `index.tsx`,
  with class names prefixed by the module id.
- **Check**: `cd web && npx tsc --noEmit && npm run build`, then look at it in a browser in
  both themes and both languages.

## Boundaries

The rules about who may depend on whom are checked, not just written down:

- `cargo test -p hub-server --test boundaries` reads the manifests and the sources of every
  folder under `modules/`, whether or not the build has that module. A module may depend on
  another module's crate only when its `ModuleMeta::depends_on` names it; no module depends
  on `hub-server`; every module is a feature of `hub-server` that switches on its crate and
  the features of what it depends on; `hub-app` depends on no module; `hub-core` depends on
  nothing that touches the network, the disk or a runtime.
- `npm run check:boundaries` (part of `npm run build`) reads the imports. A file under
  `modules/<id>/web/` imports `react`, `@/…` and its own files, never another module's folder
  and never a package of its own; under `web/src/` only `modules.ts` names a module's folder,
  it lists every module that has screens, and it fetches each with `import()`.

A module that needs something from another one declares it in `depends_on` and uses its
crate on the server; in the interface it goes through what the host offers (`useModules()`,
the issue's `panels` and `ext`), not through the other module's files.
