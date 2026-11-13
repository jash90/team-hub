# team-hub-e2e-reporter

A reporter for the [e2e testing framework](https://github.com/e2e-inc/e2e) that posts each
finished run to team-hub. team-hub links every test to the issues it
verifies, so the issue, the board and the project's **Tests** page show whether the
behaviour an issue describes actually works.

## Install

```bash
npm install --save-dev team-hub-e2e-reporter
```

It needs `e2e` 0.22.1 or newer and the Node.js the framework needs (22.22.3+, or 24.8+).

## Configure

```ts
// e2e.config.ts
import type { E2EConfig } from 'e2e';
import { web } from '@e2e-dev/web';
import { teamHub } from 'team-hub-e2e-reporter';

export default {
  targets: [{ engine: web(), app: { url: process.env.APP_URL ?? 'http://localhost:3000' } }],
  reporters: [
    'list',
    'junit',
    teamHub({ url: 'https://hub.example.com', project: 'WEB', token: process.env.TEAM_HUB_TOKEN }),
  ],
} satisfies E2EConfig;
```

| Option | Default | What it is |
| --- | --- | --- |
| `url` | `TEAM_HUB_URL` | The address of your team-hub instance. |
| `project` | `TEAM_HUB_PROJECT` | The key of the project the tests belong to. |
| `token` | `TEAM_HUB_TOKEN` | An API token of somebody who may edit the project: team-hub → Settings → API tokens. |
| `name` | the CI workflow and run number | What the run is called in team-hub. |
| `branch`, `commit`, `pullRequest`, `ciUrl` | from the CI environment | Override what CI says about the run. |

- Without a token nothing is posted and the run is not failed for it, so the same config
  works on a laptop. The summary under the run says `team-hub  not posted: TEAM_HUB_TOKEN is not set`.
- The token is read when the run finishes, sent only in the `Authorization` header to the
  `url` you configured, and never printed: not in the summary, not in an error.
- A reporter cannot change a run's result. When posting fails, the summary says why
  (`not posted: HTTP 403: …`) and the exit code stays what the tests made it.
- Posting the same run twice stores it once: the report carries a run id.

After a run the terminal shows where it went:

```
      JUnit  .e2e/junit.xml
   team-hub  https://hub.example.com/p/WEB/tests?run=128 (tests verify 3 issues)
```

## Link tests to issues

A test verifies the issues named in its title, in the titles of the `describe` groups around
it, or in its tags:

```ts
test('WEB-12 a member upgrades to Pro', async ({ app, screen }) => { /* … */ });

describe('WEB-12 billing', { tags: ['@WEB-31'] }, () => { /* … */ });
```

A key must be written as the issue's key (`WEB-12`); in a tag any case works (`@web-12`).
Links can also be made by hand on the Tests page of the project; those stay through later runs.

## What is sent

The framework's report document (`report-1`, the same JSON as `.e2e/report.json`) with a
`meta` object beside it:

| `meta` | GitHub Actions | GitLab CI/CD | Others |
| --- | --- | --- | --- |
| `branch` | `GITHUB_HEAD_REF`, else `GITHUB_REF_NAME` | `CI_MERGE_REQUEST_SOURCE_BRANCH_NAME`, else `CI_COMMIT_REF_NAME` | Bitrise, Codemagic, CircleCI, Buildkite, Azure Pipelines and Jenkins variables |
| `commit` | the pull request's head, else `GITHUB_SHA` | `CI_COMMIT_SHA` | |
| `pullRequest` | from the event payload | `CI_MERGE_REQUEST_IID` | |
| `ciUrl` | the workflow run | the job | |
| `name` | `<workflow> #<run number>` | `<job> #<pipeline>` | |

Outside CI `meta` is empty and team-hub uses the commit and branch the report itself carries
(the framework reads them from git). Screenshots, videos and traces are not sent; keep
uploading `.e2e/` as a CI artifact for those.

## GitHub Actions

```yaml
name: e2e
on:
  push:
    branches: [main]
  pull_request:

jobs:
  e2e:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 24
      - run: npm ci
      - run: npx e2e-web install chromium --with-deps
      - name: End-to-end tests
        run: npx e2e run
        env:
          APP_URL: http://localhost:3000
          TEAM_HUB_TOKEN: ${{ secrets.TEAM_HUB_TOKEN }}
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: e2e-report
          path: .e2e/
```

Pull requests from forks get no secrets, so their runs are not posted.

## Without the reporter

The same endpoint takes the report from any job, as JSON or as the JUnit file of
`--reporter junit`:

```bash
curl -fsS -X POST "https://hub.example.com/api/m/e2e/projects/WEB/runs?branch=$GITHUB_REF_NAME&commit=$GITHUB_SHA" \
  -H "Authorization: Bearer $TEAM_HUB_TOKEN" \
  -H "Content-Type: application/json" \
  --data-binary @.e2e/report.json

curl -fsS -X POST "https://hub.example.com/api/m/e2e/projects/WEB/runs?runId=$GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT&branch=$GITHUB_REF_NAME&commit=$GITHUB_SHA" \
  -H "Authorization: Bearer $TEAM_HUB_TOKEN" \
  -H "Content-Type: application/xml" \
  --data-binary @.e2e/junit.xml
```

Query parameters: `name`, `branch`, `commit`, `pr`, `ciUrl`, and `runId` (a JUnit file has no
run id of its own; with one, a repeated post changes nothing).

## Develop

```bash
npm install     # also builds dist/
npm test        # node --test, no network
npm run typecheck
```
