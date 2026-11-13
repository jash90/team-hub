#!/usr/bin/env bash
# The runner end to end against a real team-hub server, with no model and no GitHub:
# the `hub` command line and the fake agent, a throwaway git repository whose pushes go to a
# local bare repository, and a server started on its own port with demo data.
#
#   apps/desktop/scripts/e2e-real-server.sh [work dir]
#
# Needs: the `team-hub`, `hub` and `hub-fake-agent` binaries built in $TARGET (default
# target/debug), web/dist built, curl, python3, git. Everything it writes is under the work dir.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
TARGET=${TARGET:-${CARGO_TARGET_DIR:-$ROOT/target}/debug}
WORK=${1:-$(mktemp -d)}
PORT=${TEAM_HUB_PORT:-8195}
B=http://127.0.0.1:$PORT
R=$B/api/m/agents
HUB=$TARGET/hub
G="git -c user.name=E2E -c user.email=e2e@example.invalid -c commit.gpgsign=false"

say() { printf '\n=== %s\n' "$*"; }
fail() { echo "FAILED: $*" >&2; exit 1; }
json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
expect() { [ "$2" = "$3" ] || fail "$1: expected '$3', got '$2'"; echo "ok  $1: $2"; }

rm -rf "$WORK/server-data" "$WORK/desktop-data" "$WORK/repo"
mkdir -p "$WORK/server-data" "$WORK/repo"

say "a real server on port $PORT with demo data"
(cd "$ROOT" && exec env TEAM_HUB_DATA_DIR="$WORK/server-data" TEAM_HUB_PORT="$PORT" "$TARGET/team-hub" >"$WORK/server.log" 2>&1) &
SERVER=$!
trap 'kill $SERVER 2>/dev/null || true' EXIT

for _ in $(seq 50); do curl -sf "$B/api/bootstrap" >/dev/null && break; sleep 0.2; done

curl -sf -c "$WORK/cookies" -X POST "$B/api/setup" -H 'content-type: application/json' \
  -d '{"name":"Ada Admin","email":"ada@example.com","password":"correct horse battery staple","demo":true}' >"$WORK/setup.json"
CSRF=$(json "d['csrf']" <"$WORK/setup.json")
TOKEN=$(curl -sf -b "$WORK/cookies" -H "x-csrf-token: $CSRF" -H 'content-type: application/json' -X POST "$B/api/me/tokens" -d '{"name":"desktop e2e"}' | json "d['token']")
A="Authorization: Bearer $TOKEN"
api() { curl -sf -H "$A" -H 'content-type: application/json' "$@"; }
run_field() { api "$R/runs/$1" | json "d['run']['$2']"; }

say "a throwaway repository: origin reads as acme/sklep-web, pushes go to a local bare repository"
(
  cd "$WORK/repo"
  $G init -q --bare --initial-branch=main origin.git
  $G clone -q origin.git sklep-web 2>/dev/null
  cd sklep-web
  mkdir src && echo "# sklep-web" >README.md && echo "export const pageSize = 20;" >src/orders.ts
  $G add -A && $G commit -qm "Initial commit" && $G push -q origin HEAD:main 2>/dev/null && $G remote set-head origin main
  $G remote set-url origin https://github.com/acme/sklep-web.git
  $G remote set-url --push origin "$WORK/repo/origin.git"
  echo "my own unfinished work" >>README.md
)
MAIN_BEFORE=$(git -C "$WORK/repo/sklep-web" rev-parse HEAD; git -C "$WORK/repo/sklep-web" status --porcelain)

export TEAM_HUB_DESKTOP_DIR=$WORK/desktop-data TEAM_HUB_TOKEN=$TOKEN TEAM_HUB_SIDECAR=$TARGET/hub-fake-agent

say "hub connect, hub repo add, hub status, hub issue"
"$HUB" connect "$B"
"$HUB" repo add WEB "$WORK/repo/sklep-web" --github acme/other && fail "a clone with another origin was mapped" || echo "ok  a clone whose origin is not the named repository is refused"
"$HUB" repo add WEB "$WORK/repo/sklep-web" --github acme/sklep-web
"$HUB" status
"$HUB" issue WEB-12 | head -4
expect "files under the app's data folder that hold the token" "$(grep -rl "$TOKEN" "$WORK/desktop-data" | wc -l | tr -d ' ')" 0

say "a person delegates WEB-12 on the web (implement)"
RUN=$(api -X POST "$R/issues/WEB-12/runs" -d '{"kind":"implement","instructions":"Keep the page size configurable. [fake:full]","repo":"acme/sklep-web"}' | json "d['id']")
expect "run $RUN" "$(run_field "$RUN" state)" queued

say "hub runner --once: claims the run, makes the working tree, works until it needs a person"
"$HUB" runner --once
expect "run state" "$(run_field "$RUN" state)" waiting
expect "pending question" "$(run_field "$RUN" pendingQuestion)" True
expect "branch on the run" "$(run_field "$RUN" branch)" feature/WEB-12-historia-zamowien-w-profilu
expect "runner on the run" "$(run_field "$RUN" runnerName)" "$("$HUB" status --json | json "d['status']['runner']['name']")"
api "$R/runs/$RUN" | python3 -c "
import json,sys
for e in json.load(sys.stdin)['events']: print('   ', e['kind'].ljust(9), e['text'][:110].replace(chr(10), ' '))"
api "$R/runners" | python3 -c "
import json,sys
for r in json.load(sys.stdin): print('    runner', r['id'], '·', r['name'], r['kinds'], r['projects'], 'online' if r['online'] else 'offline')"

say "the person answers on the web; the runner hears it and resumes the same session"
api -X POST "$R/runs/$RUN/answer" -d '{"text":"Twenty per page."}' >/dev/null
"$HUB" runner --once
expect "run state (the push and the pull request wait for a click)" "$(run_field "$RUN" state)" waiting
expect "pending proposals on the run" "$(run_field "$RUN" pendingProposals)" 1
expect "comments on the issue before any decision" "$(api "$B/api/issues/WEB-12" | json "len(d['comments'])")" 0
expect "branches on origin before the click" "$(git -C "$WORK/repo/origin.git" for-each-ref --format='%(refname)' | tr '\n' ' ')" "refs/heads/main "
"$HUB" proposals

say "the comment is approved on the web; the app learns of it"
PROPOSAL=$(api "$R/runs/$RUN" | json "[p['id'] for p in d['proposals'] if p['kind'] == 'comment'][0]")
api -X POST "$R/proposals/$PROPOSAL/decide" -d '{"approve":true}' >/dev/null
"$HUB" runner --once >/dev/null
expect "comments on the issue after the decision" "$(api "$B/api/issues/WEB-12" | json "len(d['comments'])")" 1
expect "the comment proposal in the app" "$("$HUB" thread show 1 --json | json "[p['state'] for p in d['proposals'] if p['kind'] == 'comment'][0]")" approved

say "hub push 1: the preview, then the click"
"$HUB" push 1
expect "branches on origin after the preview alone" "$(git -C "$WORK/repo/origin.git" for-each-ref --format='%(refname)' | tr '\n' ' ')" "refs/heads/main "
"$HUB" push 1 --yes
expect "the commit on origin is the thread's" "$(git -C "$WORK/repo/origin.git" log -1 --format=%s feature/WEB-12-historia-zamowien-w-profilu)" "WEB-12: implement with the decided limit"

say "the pull request (noted by hand: there is no GitHub here to open one on)"
"$HUB" thread pr 1 --url https://github.com/acme/sklep-web/pull/1
expect "run state" "$(run_field "$RUN" state)" succeeded
expect "branch" "$(run_field "$RUN" branch)" feature/WEB-12-historia-zamowien-w-profilu
expect "pull request" "$(run_field "$RUN" pullRequestUrl)" https://github.com/acme/sklep-web/pull/1
expect "tokens" "$(run_field "$RUN" inputTokens)/$(run_field "$RUN" outputTokens)" 1200/340
expect "the main clone is as it was" "$(git -C "$WORK/repo/sklep-web" rev-parse HEAD; git -C "$WORK/repo/sklep-web" status --porcelain)" "$MAIN_BEFORE"

say "a thread started here shows on the server as a run too (hub thread run)"
"$HUB" thread run WEB-12 --kind analyse --instructions "[fake:analyse]" --branch feature/WEB-12-analysis
SECOND=$("$HUB" thread list --json | json "d[0]['runId']")
expect "its run" "$(run_field "$SECOND" state) $(run_field "$SECOND" kind) $(run_field "$SECOND" repo)" "succeeded custom acme/sklep-web"

say "a run cancelled on the web stops the thread"
THIRD=$(api -X POST "$R/issues/WEB-12/runs" -d '{"kind":"custom","instructions":"[fake:slow]","repo":"acme/sklep-web"}' | json "d['id']")
("$HUB" runner --once >"$WORK/runner.log" 2>&1) &
for _ in $(seq 50); do [ "$(run_field "$THIRD" state)" = running ] && break; sleep 0.2; done
api -X POST "$R/runs/$THIRD/cancel" >/dev/null
wait %2 2>/dev/null || wait
expect "the cancelled run" "$(run_field "$THIRD" state)" cancelled
expect "its thread" "$("$HUB" thread list --json | json "d[0]['state']")" stopped
expect "agent processes left" "$(pgrep -fc hub-fake-agent || true)" 0

say "hub runs, hub thread list, hub repo list"
"$HUB" runs
"$HUB" thread list
"$HUB" repo list

say "all of it held"
