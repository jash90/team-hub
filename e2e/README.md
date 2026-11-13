# End-to-end tests of team-hub

team-hub tested with the [e2e framework](https://github.com/e2e-inc/e2e) it has a module for:
a handful of tests drive a **demo instance** in a browser, and the finished run is posted back
into that instance, where the *Tests* page of the WEB project shows it.

| File | What it checks |
| --- | --- |
| `tests/auth.setup.e2e.ts` | signs in once; the other tests start from that session |
| `tests/sign-in.e2e.ts` | signing in, and a wrong password |
| `tests/board.e2e.ts` | the board shows the active sprint; a card opens its issue (`WEB-9`) |
| `tests/issues.e2e.ts` | creating an issue (and deleting it again); moving an issue's status (`WEB-16`) |
| `tests/search.e2e.ts` | a query finds the finished bugs (`WEB-11`); a broken query explains itself |
| `tests/knowledge-base.e2e.ts` | a knowledge base page opens; skipped when the instance has no `wiki` module |
| `tests/agent.example.e2e.ts` | an example with `agent.act` and `agent.assert`; skipped unless a model is configured |

The tests use locators and assertions only, so they need no model. A key in a title
(`WEB-9 …`) links the test to that demo issue.

## Run

```bash
# 1. the reporter (once), then this project
npm --prefix ../integrations/e2e-reporter install
npm install

# 2. a demo instance: start the server with a fresh data directory, then
curl -X POST http://localhost:8192/api/setup -H 'content-type: application/json' \
  -d '{"name":"Bartek Admin","email":"admin@example.com","password":"correct horse","demo":true}'

# 3. the suite
TEAM_HUB_URL=http://localhost:8192 npx e2e run
```

| Variable | Default | |
| --- | --- | --- |
| `TEAM_HUB_URL` | `http://localhost:8192` | the instance under test |
| `TEAM_HUB_TOKEN` | - | an API token (Settings → API tokens); with it the run is posted to the instance |
| `TEAM_HUB_PROJECT` | `WEB` | the project the run is posted to |
| `TEAM_HUB_REPORT_URL` | `TEAM_HUB_URL` | post the run to another instance than the one under test |
| `E2E_USER_ADMIN_USERNAME`, `E2E_USER_ADMIN_PASSWORD` | the demo account above | the account to sign in with |
| `CHROMIUM_EXECUTABLE` | - | use a Chromium that is already installed (see below) |
| `E2E_AGENT_MODEL` | - | a model id for the agent example, e.g. `openai/gpt-6-luna-fast` (`npm install -D ai` first) |

Reporters: `list` and `junit` (`.e2e/junit.xml`) by default; `npx e2e run --reporter json`
prints the JSON report instead, and `.e2e/report.json` is written either way.

### The browser

`npx e2e-web install chromium` downloads the Chromium the engine's Playwright expects. Where
a Chromium is already installed and nothing may be downloaded, point the suite at it:

```bash
CHROMIUM_EXECUTABLE=/opt/pw-browsers/chromium npx e2e run
```

`support/chromium.ts` then starts that browser headless and the engine attaches to it over
the DevTools protocol (`web({ connect })`), the option the engine has for a browser it does
not launch itself.

### Notes

- The tests expect the English interface; the config sets the browser's locale to `en-GB`.
- They change the instance a little and put it back: an issue is created and deleted, `WEB-16`
  moves to In Progress and back. Run them against a demo instance, not one people work in.
- `E2E_TELEMETRY_DISABLED=1` turns the framework's anonymous usage telemetry off.
