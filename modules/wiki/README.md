# wiki — Knowledge base

Spaces of Markdown pages in a tree, with history, full-text search and links to issues.
Crate `hub-mod-wiki`, module id `wiki`, enabled by default.

All routes are under `/api/m/wiki` and need a signed-in user. JSON is camelCase; the types
named below are generated into `web/src/bindings/` (`WikiSpace.ts`, …). Errors have the usual
shape `{ "error": { "code", "message" } }`.

## Access

| Space | See | Create, edit, move, delete pages | Delete the space, change its project |
|---|---|---|---|
| bound to a project | whoever sees the project | whoever may edit the project (not viewers) | project administrators |
| not bound | every signed-in user | every signed-in user | instance administrators |

A space (or page) the caller may not see answers 404. `WikiSpace.canEdit` / `canAdmin` say
what the caller may do.

## Routes

### Spaces

| Route | Request | Response |
|---|---|---|
| `GET /spaces` | | `WikiSpace[]`, visible ones, by name |
| `POST /spaces` | `WikiNewSpace` `{ key, name, description?, projectKey? }` | `WikiSpace`. The key has the shape of a project key (2–10 letters/digits, upper-cased). Binding to a project needs edit access to it. 409 when the key is taken |
| `GET /spaces/{key}` | | `WikiSpace` |
| `PATCH /spaces/{key}` | `WikiSpacePatch` `{ name?, description?, projectKey? }` (`projectKey: null` unbinds) | `WikiSpace`. Changing the binding needs admin rights on the space and on the new project; unbinding needs an instance administrator |
| `DELETE /spaces/{key}` | | `null`. Deletes pages, history and links |
| `GET /spaces/{key}/tree` | | `WikiTreeNode[]` `{ id, parentId, title, position, updatedAt, locked }`, flat, in reading order (each page is followed by its subtree) |

`WikiSpace`: `{ id, key, name, description, projectId, projectKey, source, createdAt, pageCount, canEdit, canAdmin }`;
`source` is `local` or `confluence`.

### Pages

| Route | Request | Response |
|---|---|---|
| `POST /spaces/{key}/pages` | `WikiNewPage` `{ title, body?, parentId?, templateId? }` | `WikiPageDetail`. Appended after its siblings. Without `body`, `templateId` supplies it |
| `GET /pages/{id}` | | `WikiPageDetail` |
| `PATCH /pages/{id}` | `WikiPagePatch` `{ title?, body?, baseVersion, message? }` | `WikiPageDetail`. **409** when `baseVersion` is not the current version ("Someone else saved version N…") and when the page is `locked` ("…detach it to edit"). A save that changes nothing creates no version |
| `POST /pages/{id}/move` | `WikiMove` `{ parentId, position }` (`parentId: null` = top level; `position` = index among the new siblings) | `WikiPageDetail`. 400 for a parent in another space or under the page itself; 409 for a locked page |
| `DELETE /pages/{id}?recursive=true\|false` | | `{ deleted: n }`. 409 when the page has children and `recursive` is not `true`. A locked page can be deleted; the next import brings it back unless it is excluded there |
| `POST /pages/{id}/detach` | | `WikiPageDetail`. Clears `locked`, `sourceId`, `sourceVersion`, `sourceUrl`; the importer never touches the page again |

`WikiPageDetail` is the page (`WikiPage`: `{ id, spaceId, spaceKey, parentId, title, body, position, version, authorId, updatedBy, createdAt, updatedAt, sourceId, sourceVersion, sourceUrl, locked }`) plus:

- `space`: `WikiSpace`
- `breadcrumbs`: `{ id, title }[]`, outermost ancestor first
- `children`: `WikiTreeNode[]`
- `issues`: `{ relation: "mentions" | "documents", issue: IssueRow }[]`, only issues the caller may see; an issue linked both ways is listed once, as `documents`
- `backlinks`: `{ pageId, spaceKey, title }[]`, pages linking here with `[[…]]`

### History

| Route | Request | Response |
|---|---|---|
| `GET /pages/{id}/revisions` | | `WikiRevisionInfo[]` `{ version, title, authorId, authorLabel, message, createdAt }`, newest first |
| `GET /pages/{id}/revisions/{version}` | | `WikiRevision` (the info plus `body`) |
| `GET /pages/{id}/diff?from=&to=` | defaults: `to` = current version, `from` = `to − 1` | `WikiDiff` `{ from, to, fromTitle, toTitle, lines: DiffLine[] }` |
| `POST /pages/{id}/restore` | `WikiRestore` `{ version }` | `WikiPageDetail`; the old content becomes a new version. 409 for a locked page |

### Issues

| Route | Request | Response |
|---|---|---|
| `POST /pages/{id}/issues` | `WikiTasksRequest` `{ projectKey, typeId?, lines, baseVersion? }` | `WikiTasksResult` `{ keys, page }` |
| `PUT /pages/{id}/issues/{issueKey}` | | `WikiPageDetail`; adds the explicit `documents` link |
| `DELETE /pages/{id}/issues/{issueKey}` | | `WikiPageDetail`; removes it |

`lines` are 0-based indexes into the body split on `\n`. Each must be an open task
(`- [ ] text`; `*`, `+` and `1.` bullets and indentation are fine) that does not already start
with an issue key. Every line becomes an issue (summary = the text, reporter = the caller, the
description links back to the page) and is rewritten to `- [ ] KEY text`; the page gets one new
version. All or nothing: one bad line and no issue is created. Needs edit access to the space
and to the project; 409 for a locked page or a stale `baseVersion`.

### Search and templates

| Route | Response |
|---|---|
| `GET /search?q=&space=` | `WikiSearchHit[]` `{ pageId, spaceKey, title, snippet }`, best first, at most 30. Every word must match, as a prefix; case and diacritics do not matter. `snippet` is an excerpt of the body as plain text in which each matched word stands between `<mark>` and `</mark>` (split on those; do not insert as HTML) |
| `GET /templates` | `WikiTemplate[]` `{ id, name, description, body }`: `decision-record`, `meeting-notes`, `runbook`, `technical-spec`, `retrospective` |

## Links

On every save the body is read for

- issue keys of existing projects (`WEB-12`, exact case) → `link` rows page → issue, relation `mentions`;
- `[[Title]]`, `[[SPACE:Title]]`, `[[Title|shown text]]` → `link` rows page → page, relation `links`.
  Titles match without regard to case and diacritics; a link to a page that does not exist
  yet starts working when the page is created or renamed to that title.

Explicit page → issue links use relation `documents`. In the `link` table a page is
`kind = 'page'`, `id` = the page id as text.

## What the module adds elsewhere

- **Issue page**: `issue.panels.wiki` = `WikiIssuePanel` `{ pages: { pageId, spaceKey, title, relation, updatedAt }[] }`, pages the viewer may see, newest first.
- **Quick search**: `QuickHit { kind: "page", id, title, subtitle: space name, url: "/wiki/<spaceKey>/<pageId>" }`.
- **Events** (project = the space's project, may be none): `page.created` and `page.deleted` with `{ pageId, spaceKey, title }` (`page.deleted` also `pages`, the number removed), `page.updated` with an extra `change`: `edited`, `restored`, `moved` or `detached`.
- **Agent tools**: `search_pages { query, space? }`, `get_page { id }`, `create_page { spaceKey, title, body, parentId? }`, `update_page { id, body, baseVersion }`; they run with the access of the calling user and return the same shapes as the routes.
- **Demo data**: space `WEB` ("Sklep — baza wiedzy", bound to project WEB) with eleven pages, and the unbound space `TEAM` ("Zespół") with two.

## For other modules

`hub_mod_wiki::store` is the API for modules that mirror another system into the knowledge
base (see `modules/confluence`): `upsert_space`, `imported_versions`, `detached_sources`,
`upsert_imported_page`, `refresh_links`, `imported_pages`, `spaces_by_source`, `space_access`.
Imported pages are stored `locked`; `ImportOutcome` says whether a page was created, updated,
unchanged or had been detached.

## Tables

`wiki_space`, `wiki_page`, `wiki_revision`, `wiki_detached` (source ids of detached pages),
and the FTS5 table `wiki_fts(title, body)` with rowid = page id, holding text folded with
`hub_core::text::fold`.

## Web (`web/`)

The default export of `index.tsx` is `wikiModule`; texts are English in place with Polish in `pl.ts`, styles in
`styles.css` (classes prefixed `wiki-`).

| Address | Screen |
|---|---|
| `/wiki` | The spaces as cards, "New space", and search over all pages (`?q=`, `&space=KEY` limits it). `?project=KEY` opens the space bound to that project, or offers to create one |
| `/wiki/:space` | The space: page tree on the left (folding remembered per space, "+" adds a child), its first page on the right |
| `/wiki/:space/:page` | A page: breadcrumbs, text, table of contents (three headings or more), children, linked issues with their live status, backlinks, the lock banner of a Confluence mirror with "Detach to edit" |
| `/wiki/:space/:page/edit` | The editor, with a live preview beside it on wide screens. When somebody else saves meanwhile, the draft stays and the person compares, then saves theirs as the next version or takes the other one |
| `/wiki/:space/:page/history` | The versions; `?v=N` shows one, `?from=A&to=B` the changes between two; restore |
| `/wiki-go?title=&space=` | Where `[[Title]]` links land: opens the page of that title, or offers to create it |

Also: the entry "Knowledge base" in the main navigation and "Knowledge" in a project's; the
panel on an issue (linked pages, "Link a page", "Create a page"); dialogs for a new page
(with templates, in the language of the interface), moving a page, turning task lines into
issues, linking an issue, space settings.
