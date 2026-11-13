# confluence — Confluence import

Imports Confluence spaces (Cloud and Server/Data Center) into knowledge base spaces as a
read-only mirror, and brings them up to date on request. Crate `hub-mod-confluence`, module
id `confluence`, depends on `wiki`, enabled by default.

All routes are under `/api/m/confluence`. JSON is camelCase; the types named below are
generated into `web/src/bindings/` (`ConfluenceSettings.ts`, …). Everything except
`GET /imports` is for instance administrators (403 otherwise): the import works with one
instance-wide token.

## Routes

| Route | Request | Response |
|---|---|---|
| `GET /settings` | | `ConfluenceSettings` `{ baseUrl, email, tokenHint, configured }`. The token is never returned; `tokenHint` is its ending or `null` |
| `PUT /settings` | `ConfluenceSettingsUpdate` `{ baseUrl, email?, token?, clearToken? }` | `ConfluenceSettings`. A blank or absent `token` keeps the stored one; `clearToken: true` removes it. 400 for an address that is not `https://…` (plain `http://` only for `127.0.0.1` and `localhost`), and when the address moves to another server (scheme, host or port) without a token for it: a stored token is never sent somewhere new |
| `POST /test` | | `ConfluenceTestResult` `{ ok, message }`. Calls `GET {base}/rest/api/space?limit=1`. A refusal by Confluence is `ok: false` with the reason, not an HTTP error. 409 when nothing is configured |
| `GET /spaces` | | `ConfluenceRemoteSpace[]` `{ key, name }`, all spaces the token can see |
| `POST /import` | `ConfluenceImportRequest` `{ spaceKey, rootPageId?, targetSpaceKey?, exclude? }` | `ConfluenceImportReport` |
| `GET /imports` | | `ConfluenceImport[]` `{ wikiSpaceKey, wikiSpaceName, spaceKey, rootPageId, exclude, baseUrl, lastRunAt, lastReport }`, for every signed-in user, limited to the spaces they may see |
| `POST /imports/{wikiSpaceKey}/sync` | | `ConfluenceImportReport`; runs the stored import again. 404 when the space is not a Confluence import |

`ConfluenceImportReport`: `{ wikiSpaceKey, created, updated, unchanged, skipped, failed: { title, reason }[], gone: { pageId, title }[] }`.
`skipped` counts excluded pages (with their subtrees) and pages detached here; `gone` lists
mirrored pages Confluence no longer has (they are left alone).

Errors from Confluence come back as 502 `upstream` with a message that can be shown as is,
e.g. "Confluence rejected the token" for a 401.

## Settings

- `baseUrl`: for Cloud include `/wiki` (`https://example.atlassian.net/wiki`); for Server and
  Data Center the address with its context path.
- `email` set → `Authorization: Basic base64(email:token)` (Cloud API token);
  empty → `Authorization: Bearer token` (personal access token).
- Stored as settings `confluence:base_url`, `confluence:email` and the encrypted secret `confluence.token`.

## What an import does

1. Fetches `GET {base}/rest/api/content?spaceKey=K&type=page&limit=50&start=N&expand=body.storage,version,ancestors`
   page by page (following `_links.next`, or counting `start` up by `size` where the server
   gives no link) on a blocking thread. 429 and 5xx answers are tried three times with a
   pause in between (honouring `Retry-After`); a 401 is final at once.
2. Chooses the pages: with `rootPageId` only that page and what is under it; a page named in
   `exclude` by title (any case) or id is left out **with everything under it**.
3. Converts the storage format of new and changed pages to Markdown, in parallel.
4. Writes into the knowledge base space `targetSpaceKey` (default: the Confluence key) in
   one transaction, parents before children:
   - a new page is created `locked`, with `sourceId`, `sourceVersion`, `sourceUrl`;
   - a page whose Confluence version number changed gets a new revision (author "Confluence");
   - a page with the same version is not touched (only moved if Confluence moved it);
   - a detached page is never touched again; its children still hang under it;
   - a page that cannot be stored (no body sent) is listed in `failed`, the rest goes on.
5. Stores the configuration on the space (`wiki_space.source = 'confluence'`, `source_ref`)
   and the report in `confluence_import`, and emits `confluence.imported`.

The target space is created unbound (visible to every signed-in user) and named after the
Confluence space; bind it to a project with `PATCH /api/m/wiki/spaces/{key}`. An existing
space that is not a Confluence import is never written into (409).

## Conversion (`convert::storage_to_markdown`)

A pure function with its own small, forgiving parser. Headings, paragraphs, line breaks,
bold/italic/strikethrough/inline code, `pre`, links, nested lists, blockquotes, tables (GFM),
rules, images (remote ones as images, attachments as `*(image: name)*`), the `code` and
`noformat` macros (fenced, with the language), `info`/`note`/`warning`/`tip`/`panel` macros
and new-editor panels (blockquote with a bold label), `expand`, layouts, the Jira macro (the
issue key, so it links to an issue of the same key here), `ac:link` to a page (`[[Title]]`,
`[[SPACE:Title]]`, `[[Title|shown text]]`), `ac:task-list` (`- [ ]` / `- [x]`), entities.
Macro parameters are dropped; unknown tags are dropped and their text is kept. Text that
would read as Markdown (`*`, `_`, `[`, `<`, a leading `#` …) is escaped.

## Web (`web/`)

The default export of `index.tsx` is `confluenceModule`: the tab "Confluence" in the instance settings
(`/settings/confluence`, administrators only) with the connection (and "Test connection"),
the import form, and the imported spaces with their last report and "Sync now". An import or
sync runs inside one request; the screen says so and keeps its buttons disabled meanwhile.
Polish is in `pl.ts`, styles in `styles.css` (classes prefixed `confluence-`).
