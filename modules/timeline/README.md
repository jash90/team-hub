# timeline

Timeline: say here what the module adds.

Crate `hub-mod-timeline`, `pub struct TimelineModule;`, id `timeline`, enabled by default.
No tables, no background work, no events (yet).

## Routes

All under `/api/m/timeline`, for signed-in callers who may see the project.

### `GET /projects/{key}/summary`

`TimelineSummary` (`web/src/bindings/TimelineSummary.ts`): the project and how many of its
issues are open.

## Screens

The default export of `web/index.tsx`: one entry in the project navigation and the page
behind it (`/p/:key/timeline`). Polish in `web/pl.ts`, styles in `web/styles.css`. The browser
fetches them as one chunk, and only while the module is switched on.
