# Writing a module

Everything beyond the tracker's core lives in a module: one folder under `modules/<id>/`
holding a Rust crate (`Cargo.toml`, `src/`) and, when it has screens, a `web/` folder.
A module can be switched off at runtime (Settings → Modules); while off, its routes answer
404 and it receives no events.

## The crate

- Package name `hub-mod-<id>`; depends on `hub-core` and `hub-app` (workspace dependencies).
- `src/lib.rs` exports one unit struct, e.g. `pub struct WikiModule;`, implementing
  `hub_app::Module` (`crates/hub-app/src/module.rs` documents every hook).
- `hub-server/src/lib.rs::modules()` lists the modules compiled into the build.

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
| Background work | `fn start(&App)`: spawn a plain thread; never block the Rayon pool on the network. |
| Demo data | `fn seed_demo(&App)`. |

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

`modules/<id>/web/index.tsx` exports one `WebModule` (the type, with a comment on every
field, is in `web/src/modules.ts`, where the module is also listed). It only shows up where
it has something: project navigation, full pages, badges on issue cards, a panel on the issue
page, a section of "My work", tabs in project and instance settings.
`modules/github/web/index.tsx` uses every one of these and is the reference.

- **Imports**: `react`, and the application through the `@/` alias (`web/src`). Nothing else
  resolves from a module folder, so no extra packages: charts are hand-drawn SVG.
  - `@/api.ts`: `api.get/post/patch/put/del/upload`, `query()`, `errorMessage()`.
  - `@/session.tsx`: `useSession()` (user, people, `moduleEnabled`), `useRemote(load, deps, live?)`
    (data that reloads when the server announces a change the predicate accepts),
    `useProject()` inside project pages, `useStored()` for per-browser interface state.
  - `@/components/ui.tsx`: `Button`, `Dialog`, `ConfirmDialog`, `Picker`, `Menu`, `Popover`,
    `Tabs`, `Tag`, `Field`, `Empty`, `Spinner`, `ErrorNote`, `ProgressBar`, `Avatar`,
    `InlineEdit`, `attempt()`, `useBusy()`, `toast()`, `copyText()`.
  - `@/components/icon.tsx` (`Icon`, the names are the keys of its table),
    `@/components/issue-bits.tsx` (`IssueLine`, `IssueKey`, pickers for issue fields),
    `@/components/markdown.tsx` (`Markdown`, `MarkdownEditor`), `@/router.tsx` (`Link`,
    `navigate`, `useSearchParams`, `setParam`), `@/views/settings.tsx` (`SettingsCard`).
  - `@/bindings/*`: the types generated from Rust. Never write those shapes by hand.
- **Routes**: a pattern starting with `/p/:key/` renders inside the project (so `useProject()`
  works); anything else is a top-level page.
- **Text**: English in place through `t('…', { name })`; `modules/<id>/web/pl.ts` holds the
  Polish for every phrase the module uses (the key is the English text). Dates and durations
  come from `@/i18n/index.ts` (`formatDate`, `formatDateTime`, `timeAgo`, `countOf`).
- **Looks**: reuse the application's classes (`view`, `view-head`, `panel`, `panel-head`,
  `table`, `tag`, `form`, `inline-form`, `muted`, …) and its CSS variables (`--surface`,
  `--sunk`, `--ink`, `--muted`, `--line`, `--accent`, `--green`, `--amber`, `--red`,
  `--blue`, `--purple` and their `-soft` variants), which follow the light and dark themes.
  A module's own rules go in `modules/<id>/web/styles.css`, imported from its `index.tsx`,
  with class names prefixed by the module id.
- **Check**: `cd web && npx tsc --noEmit && npm run build`, then look at it in a browser in
  both themes and both languages.
