# team-hub desktop

team-hub in a window of its own, with what a browser cannot do beside it. The window shows
the **tracker**, which is your server's own web interface: the same board, backlog, issues
and knowledge base a browser shows, always the server's version of them. Beside it is a panel
of **agent threads**: an AI agent works on an issue in a local clone of the repository, in a
git working tree and branch of its own, and whatever should leave your computer (a comment,
a status change, a push, a pull request) waits for your click.

The same app is a **runner** for team-hub: work delegated to an AI agent in the web interface
("Implement in code", "Review the pull request", a custom task) is claimed by the app on a
computer that has the repository, carried out as a thread, and reported back to the run the
web shows. See [`docs/agent-runs.md`](../../docs/agent-runs.md) and
[`modules/agents/README.md`](../../modules/agents/README.md).

```
core/        hub-desktop-core: everything that is not a window. No Tauri; shares hub-core with the server.
             Also the `hub` command line, the fake agent and the development server.
src-tauri/   The window: one command that forwards to the core, the tray, notifications, window state,
             and the two webviews the window holds (`src/stage.rs`).
ui/          The app's own interface: threads, decisions, repositories, settings. React, TypeScript,
             Vite. Polish and English, light and dark. The tracker is not here: it is the server's.
sidecar/     Node helper that runs one agent turn through the Claude Agent SDK, JSON lines on stdin/stdout;
             launch.mjs starts it from the agent runtime an installed app fetched.
scripts/     The end-to-end check against a real server; the icon generator; the release's version check
             and its check of what was packaged.
```

## Download

Installers for macOS, Windows and Linux, and the `hub` command line alone, are on the
repository's [Releases page](https://github.com/jash90/team-hub/releases) (the releases named
"team-hub desktop"). What you need besides the installer, how to open an unsigned build, and
what is known not to work: [RELEASE_NOTES.md](RELEASE_NOTES.md), which is also the text of the
release. The installers are built by GitHub Actions and were not tried by a person; see
[Releasing](#releasing). An installed app needs Node.js 22.18 or newer **and npm**: it
downloads its [agent runtime](#the-agent-runtime-of-an-installed-app) when it first starts.

## Building and running

Requirements everywhere: Rust (stable), Node 22.18 or newer (it runs the sidecar's TypeScript
directly), git, and a team-hub server to connect to. For real agents: Claude Code signed in
(or `ANTHROPIC_API_KEY`), which the Claude Agent SDK uses.

```bash
(cd apps/desktop/sidecar && npm install)
(cd apps/desktop/ui && npm install && npm run build)
(cd apps/desktop/src-tauri && cargo build --release)      # the binary: src-tauri/target/release/team-hub-desktop
```

For development, with reloading: `(cd apps/desktop && npm install && npm run dev)`.
For installers on your own machine (`.dmg`; a Windows `-setup.exe`; `.deb`, `.rpm`, AppImage):

```bash
cd apps/desktop
npm ci && npm ci --prefix ui
npm run build          # → src-tauri/target/release/bundle/
```

Nothing has to be installed in `sidecar` for that: an installer carries the helper's source
and no `node_modules`. Released installers are built by GitHub Actions: see
[Releasing](#releasing).

A run from the source tree (`npm run dev`, or the binary built above) uses
`sidecar/node_modules` when it is there and downloads nothing.

| System | Besides Rust and Node |
|---|---|
| macOS | Xcode command line tools. The shell sandbox (`sandbox-exec`) is part of the system. |
| Linux | `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev`. For the shell sandbox: `bubblewrap` and `socat`. For the keychain: a Secret Service (GNOME Keyring, KWallet). |
| Windows | WebView2 (part of Windows 11) and the MSVC build tools. There is no shell sandbox on Windows; threads work without a shell unless you allow one in Settings. |

`src-tauri` is deliberately not a member of the repository's Cargo workspace (it needs the
system's WebKit libraries); build it from its own folder. The core is a member, so
`cargo test --workspace` covers it on any machine.

### The command line

```bash
cargo build -p hub-desktop-core
target/debug/hub connect https://hub.example.com     # asks for the token, keeps it in the keychain
target/debug/hub status                              # connection, my work, threads
target/debug/hub issue WEB-12
target/debug/hub repo add WEB ~/code/sklep-web --github acme/sklep-web
target/debug/hub thread run WEB-12 --kind implement --instructions "Keep it small."
target/debug/hub thread list | show 1 | diff 1 | say 1 "…" | pr 1 --url … | remove 1
target/debug/hub proposals ; hub decide 3 --approve ; hub push 1 ; hub push 1 --yes
target/debug/hub runs --mine
target/debug/hub runner            # claim and carry out delegated runs until Ctrl-C
target/debug/hub runners           # every runner I may see: this computer, others', services
target/debug/hub tests list WEB    # the project's test jobs
target/debug/hub tests run WEB --here [--ref main] [--filter "tests/a.e2e.ts"]   # run the suite on this computer and follow it
target/debug/hub tests run WEB --on <runner id>                                  # …or send it to another runner; without either, to any that fits
```

`hub-runner` (same crate) is the core without a person at it: see
[`apps/runner`](../runner/README.md) and [`docs/cloud-runner.md`](../../docs/cloud-runner.md).

Environment: `TEAM_HUB_DESKTOP_DIR` moves the app's data; `TEAM_HUB_TOKEN` gives the token for
one process on a machine without a keychain (it is then stored nowhere); `TEAM_HUB_SIDECAR`
replaces the helper process (e.g. with `target/debug/hub-fake-agent`).

## The agent runtime of an installed app

The Claude Agent SDK and its `claude` program are Anthropic's, and an installer does not pass
them on. An installed app fetches them on the person's computer, once:

- **When.** At start, when they are not there yet, and when a thread is started and they
  are missing; again only after an update of the app whose `sidecar/package-lock.json`
  changed. The tray menu has "Download the agent runtime"
  for a try by hand, and so has a terminal: `team-hub-desktop --fetch-agent-runtime`.
- **How and from where.** With the person's own npm (the one beside the Node of Settings →
  Node command, else `npm` on the `PATH`):
  `npm ci --omit=dev --no-audit --no-fund --ignore-scripts --no-bin-links --loglevel=http`,
  in a folder that holds copies of the installer's `package.json` and `package-lock.json`.
  So exactly the versions of the lock file, checked against its hashes by npm, from the
  registry npm is set up with (registry.npmjs.org unless the person changed it); no install
  script runs (no package of the helper has one). 106 packages; nearly all of the weight is
  `@anthropic-ai/claude-agent-sdk-<system>`.
- **How big.** Measured: a download of about 165 MB on macOS and Windows and about 330 MB
  on Linux (npm fetches the program for glibc and the one for musl, and the app removes the
  one it cannot use); 370 to 410 MB on disk afterwards.
- **Where.** `agent-runtime/` in the app's data folder:
  `~/.local/share/team-hub-desktop/` on Linux, `~/Library/Application Support/team-hub-desktop/`
  on macOS, `%APPDATA%\team-hub-desktop\` on Windows, or `TEAM_HUB_DESKTOP_DIR`. Inside, one
  folder named after a hash of `package.json` and `package-lock.json`, with `node_modules`
  and, beside it, a copy of this app's helper source (`helper-<hash>/`): Node finds what the
  source imports only in the folders above the source, so the helper runs from there and not
  from the app's own, read-only folder.
- **Whole or not at all.** npm works in `<hash>.partial-<pid>`, which gets the real name when
  npm ended well and the SDK and its program are there. An install that was cut off leaves
  nothing that looks finished and is removed at the next try, and so are the runtimes of
  earlier versions after a successful one.
- **What the person sees.** A dialog when the download starts; npm's last line in the
  window's title and the tray's tooltip while it runs; a notification when the runtime is
  ready. When it fails, a dialog with the reason (no npm, the registry not reached or
  refusing, the disk full), npm's last lines, the command, where the full log is
  (`agent-runtime/install.log`), and a **Retry** button. Starting a thread meanwhile is
  answered with that state, not with a helper error, and the runner does not claim delegated
  runs before the runtime is settled. With the runtime there, nothing needs the network.
- **Removing it.** Quit the app and delete `agent-runtime/`; the next start fetches it again.
  npm keeps its own copy of what it downloaded in its cache (`npm cache clean --force`).
- **Not involved.** `TEAM_HUB_SIDECAR` replaces the helper as before, and a run from a source
  tree with `sidecar/node_modules` uses that. The `hub` command line does not fetch anything;
  it can use what the app fetched: `TEAM_HUB_SIDECAR="node <the app's resources>/sidecar/launch.mjs"`.

The code is `src-tauri/src/runtime.rs` (no Tauri in it; `rustc --edition 2021 --test
src-tauri/src/runtime.rs` runs its tests), the dialogs are in `src-tauri/src/main.rs`, and
`sidecar/launch.mjs` is what the core is given as the helper: it runs the helper from the
runtime, or answers the thread with the reason why it cannot.

## The window: the tracker and the panel

The window holds two webviews.

- **The app's own interface** (`ui/`), loaded from the files that came with the app: the tabs
  at the top, the threads and decisions on the right, and every view that belongs to this
  computer (a thread and its diff, a push to look at, repositories, tests on this computer,
  settings). It is the only one that can call the core.
- **The tracker**: your server's web interface, loaded from the server, laid over the rest
  of the window. You sign in to it once, as in a browser, and the sign-in is remembered.
  Everything the web interface does is there, and a new version of the server is a new
  version of the tracker, with nothing to install.

What joins the two:

- An issue's key anywhere in the panel opens that issue in the tracker; so does the search
  field at the top.
- **New** in the panel, while the tracker shows an issue, starts a thread for that issue.
- Where the tracker asks which runner should carry out an agent run or a test job, this
  computer is offered first and named "this computer". That is the ordinary `onRunner` of
  [`docs/runners.md`](../../docs/runners.md): the run is pinned to the app's runner, the app
  claims it, and it appears as a thread in the panel.
- A run's card in the tracker has **Open the thread in the app** when the run belongs to
  this computer's runner; the thread's own header leads back to the issue.
- Back, forward and reload for the tracker are beside the tabs at the top; the button next
  to them hides the column of threads when the tracker should have the whole window.
- The language and the theme are chosen in the app's Settings, and the tracker follows.

The tracker **cannot call the app**. Tauri refuses a command that comes from a page which is
not the app's own unless a capability names that page's address, and none does
(`src-tauri/capabilities/default.json` names the local webview only); each command also
checks which webview is calling. The one thing the app tells a page of its server is which
runner the app is, and the one thing such a page can ask of the app is to show the thread
of a run: a link to `/desktop/thread/<number>` on the server itself, which the app does not
follow and answers by switching the view. It starts nothing and changes nothing. So a mistake in the web interface, or a server somebody else got into,
reaches what the server could already reach (it can queue work for this computer's runner,
inside the sandbox below) and no further: pushes, pull requests, repository folders and the
settings of the sandbox are decided in the files that came with the app. A link that leads
away from the server opens in your browser.

On Linux Tauri stacks a window's webviews one under another and ignores positions, so
`stage.rs` lays the two out itself with a GTK overlay; on macOS and Windows it uses Tauri's
own placement (the `unstable` feature of the `tauri` crate, which is what Tauri calls more
than one webview in a window). On macOS that placement counts from the top of the window's
frame, title bar included, so the page sends its own height along and the title bar is
added to where the tracker goes.

## Connecting to a server

In the app: the server's address and the e-mail address and password of your account, the
same as in the browser. One sign-in connects both halves of the window:

- The core signs in as a browser would, makes itself an API token in that session (named
  after this computer, listed in team-hub under **Settings → Profile and tokens**), and from
  there on acts as you, with your permissions.
- The window hands the same session to the tracker it shows, so its pages are signed in too.
  That session lasts as long as any browser's (30 days); after that the tracker shows its
  own sign-in form, and the app itself stays connected by its token.
- The password goes from the form to the core and from the core to your server. It is kept
  nowhere: not in a file, not in the keychain, not in a log.
- **Disconnecting** (Settings) removes the app's token on the server too, when the server
  can be reached, and forgets the tracker's sign-in.

**Other ways to connect**, on the same screen:

- **Sign in at the tracker**: the app shows a code, you sign in on the server's own page
  beside it and confirm the code there. The app never sees the password; for accounts that
  will not type one into an app. [`docs/desktop-pairing.md`](../../docs/desktop-pairing.md)
  says how it works and what it does and does not protect against.
- **An API token**, pasted: for a computer where nobody can sign in, and what `hub connect`
  and `TEAM_HUB_TOKEN` do. Such a token is yours; disconnecting does not remove it on the
  server.

- The token is kept in the operating system's keychain (Keychain, Credential Manager, Secret
  Service) and nowhere else: not in the settings file, not in the database, not in a log, not
  in anything the window receives, and not in the environment of the agent's helper process.
- **Offline**: the last dashboard, issues, projects and notifications are kept in the app's
  SQLite file, **sealed** (ChaCha20-Poly1305, one record at a time) with a key that also lives
  in the keychain. Encrypted rather than plain, because it is a copy of the team's tracker on a
  laptop that can be lost, and it should not be easier to read than the token that could fetch
  it again. Without a usable keychain the key lasts only as long as the process, so nothing
  readable is left on disk. The app's own records (threads, what agents said, proposals,
  repository mappings) are plain SQLite under the same folder: they describe working trees
  that are themselves plain files, and agent threads cannot read that folder.
- Then map repositories: **Repositories → Choose the folder…** next to a project's GitHub
  repository. The app checks that the clone's `origin` is that repository.

## How a thread works

1. You start a thread from an issue (implement, review, analyse, custom; extra instructions;
   model; repository), or the app claims a run delegated on the web. Either way there is a run
   on the server, so the web shows every thread.
2. The core creates a **working tree** under the app's data folder with the branch
   `hub_core::keys::suggest_branch_name` gives (`feature/WEB-12-…`, `fix/…`), from the remote's
   default branch as the clone last fetched it. An existing branch is continued; a second
   thread on the same branch takes over the tree of one at rest and is refused while one works.
3. The core starts the helper (one process per working thread) and passes it the prompt, the
   rules and the thread's permissions. What the agent says and does streams into the thread
   and is mirrored to the run. At most two threads work at once by default; the rest wait.
4. The agent reads the tracker through the server's MCP endpoint, proposes instead of doing,
   and asks when it needs a decision. A question ends its turn; the answer (from the app or
   from the web) resumes the same session.
5. You look at the diff and the commits, decide the proposals, push from a preview, open the
   pull request. The run ends with the branch and the pull request's address.

## A runner for both kinds of work

The app is one of the team's **runners** ([`docs/runners.md`](../../docs/runners.md)): a
machine with the repositories that carries out agent runs and test suites, with this
computer's processor, memory and disk. The same list and the same work as on the web:

- **Runners** in the tracker's sidebar is its page of runners: every runner you may see,
  with this computer marked, other people's computers, services such as the cloud runner;
  what each takes and what it is carrying out now.
- **Where** on a new thread: *This computer* (the default: the agent works here, with your
  own model sign-in), another runner that takes that kind of work for the project, or any
  that fits. Work sent elsewhere starts nothing here; the app follows the run (what the
  agent said, its proposals) and can decide what the web can decide: changes to the issue,
  and the push or pull request of a runner nobody sits at.
- **My work**, the tracker's start page, lists your runs wherever they are carried out, each
  with its runner's name.
- **Tests** in the tracker's sidebar is the project's page of test jobs: it starts the suite
  on this computer (offered first while this computer takes test jobs), on a runner by name,
  or on any that fits, follows a job live and cancels it. The app had views of its own for these two until 0.3.0; the
  tracker's pages do the same and more, so they are gone.

### Test suites on this computer

The app uses the same executor as the cloud runner (`core/src/jobs.rs`). By default it takes
only the jobs **sent to this computer by name** (`pinnedOnly`): running a suite is running a
repository's code on your machine, so it happens when you asked for it. Settings → *Also
take the team's queued test jobs* turns that off; *Run test suites on this computer* turns
jobs off altogether.

A job uses the clone mapped to the project when there is one (fetched with your own git
sign-in; your checkout, index and branches are not touched: the job gets a folder of its
own with a copy of the one commit), and otherwise clones the repository by itself under the
app's data folder, again with whatever git on this computer is signed in with. The app has
no GitHub token of its own.

What is and is not kept apart:

- The job's processes get an environment made from nothing: `PATH`, a home inside the job's
  folder, the browsers' folder, `CI=1`, and the variables and secrets the project's test
  settings list. Not your shell's environment, not the app's token.
- They run in their own process group, ended as a whole when the job ends, is cancelled or
  runs past its time. The job's folder is removed afterwards; browsers are kept.
- On Linux with bubblewrap (the same condition as for an agent's shell) they run inside it:
  the disk is read-only, they write only in the job's folder and the browsers' folder, and
  the app's state and the usual folders of keys (`~/.ssh`, `~/.aws`, `~/.config/gh`, …) are
  empty to them. The network stays on: a suite has to reach the application it tests.
- They run **as you**. There is no other user to switch to on a desktop, so where there is
  no such sandbox (macOS and Windows today, Linux without bubblewrap) a suite can read and
  write what you can. That is why the default is "only what I sent here".

## The safety model, in plain words

- **One working tree and branch per thread.** The main clone's checkout, index and branch are
  never touched: the app only reads it (`git status`, `rev-parse`) and adds working trees.
- **The shell is in the operating system's sandbox**: no network, writes only inside the
  working tree plus what a commit needs in the clone's `.git`: the object store, the tree's
  own folder, and the folders of its branch's ref and reflog. Everything else there is closed
  by name: every other branch (a thread cannot move `main` under your checkout),
  remote-tracking refs (a push is judged against them), tags, `HEAD`, the index, hooks,
  config, the packs, other threads' trees. Where the system has no sandbox (Windows; Linux
  without bubblewrap) the thread gets **no shell at all**, unless you switch that on in
  Settings; it then commits through the app (`commit_changes`), which runs no hook.
- **File tools are tied to the working tree**: `Edit` and `Write` are allowed for that folder
  only; review and analysis threads do not get them. Credential folders (`~/.ssh`, `~/.aws`,
  `~/.config/gh`, …) and the app's own state cannot be read. Reading other files of the
  computer is otherwise allowed in the desktop app (a person is beside it); the headless
  runner ties reading to the working tree too. A file named in the prompt as `@path` is
  never attached by the agent's SDK on anyone's word: the helper breaks such mentions, in
  both.
- **No sub-agents** (`Agent`, `Task`), no web tools. A deny list (`git push`, `gh`, `curl`,
  `ssh`, `git config`, …) wins over every allow rule, and the helper runs with
  `permissionMode: 'dontAsk'`: what is not on the allow list is refused, because nobody is
  there to ask.
- **The helper loads no settings from disk** (`settingSources: []`, `strictMcpConfig`): no
  hooks, plugins or MCP servers of yours or of the repository. The repository's `CLAUDE.md` is
  read by the app (a link is not followed) and passed on as text.
- **The agent never holds the token.** Its team-hub tools are relayed to the core, which
  calls the server's MCP endpoint itself and only for tools the server marks read-only. The
  helper's environment is stripped of `TEAM_HUB_TOKEN` and the usual tokens of code hosts and
  clouds.
- **Text from the tracker is data.** Descriptions and comments reach the agent inside a
  fence they cannot close, with the instruction not to take orders from them.
- **Nothing leaves without a click.** A comment, a status change and subtasks are proposals
  of the run, decided in the app or on the web and applied by the server as you. A push shows
  exactly the commits and the diff, pushes exactly the commit you looked at (a commit added
  since stays), is never forced, and is refused until you confirm by name when the diff
  touches CI or build files (`.github/`, `package.json`, `Dockerfile`, `build.rs`, …), looks
  like it holds a secret, adds a binary, a link or an executable. Every pushed commit is read
  on its own, so a secret added and removed again is still found.
- **Stopping leaves nothing behind.** The helper leads its own process group; stop asks it,
  then kills the group. A helper whose parent is gone ends itself. Quitting the app from the
  tray stops working threads.

## Checking it without a model or a window

- `cargo test -p hub-desktop-core`: unit tests, working trees and pushes on real temporary
  repositories, and the whole core against a fake team-hub (axum on `127.0.0.1:0`) with the
  **fake agent** (`hub-fake-agent`), which speaks the helper's JSON lines and can implement,
  ask, propose, fail, crash and hang on request (`[fake:question]`, `[fake:crash]`, … in the
  instructions).
- `apps/desktop/scripts/e2e-real-server.sh`: the `hub` command line against a real team-hub
  server it starts itself, with a throwaway repository and the fake agent: delegate, claim,
  working tree, question and answer, proposals, push, pull request, cancel.
- `(cd sidecar && npm run check && npm test)`: the real helper type-checked against the
  installed Claude Agent SDK, its options, and its side of the protocol; and the launcher of
  an installed app against made-up agent runtimes.
- The interface in a browser, with the real core behind it:

  ```bash
  cargo run -p hub-desktop-core --features dev-server --bin hub-dev-server   # the core on 127.0.0.1:4455
  (cd apps/desktop/ui && npm run dev)                                        # http://localhost:1430
  ```

  `ui/src/bridge.ts` is the only file that knows whether it talks to Tauri or to that server;
  both end in `hub_desktop_core::commands::invoke`. A browser cannot show the tracker inside
  the page, so there the place for it holds a link that opens the server in a tab.
- `(cd ui && npm run i18n)` lists phrases without Polish.

TypeScript types under `ui/src/bindings` are generated from the Rust ones by `cargo test`;
types of the server come straight from `web/src/bindings`. Never edit either by hand.

## What was run against a real model, and what was not

On the Linux machine this was written on (bubblewrap sandbox, Claude Agent SDK 0.3.323,
`claude-haiku-5-5`), real threads were run through the real helper with a list of things to
try, and the outcome read back:

| Tried by the agent | Outcome |
|---|---|
| `get_issue` through the relayed MCP tool; `propose_comment`, `propose_push` | worked; proposals recorded, nothing applied |
| `Write` inside the working tree | worked |
| `Write` outside it | refused ("running in don't ask mode") |
| shell: write a file outside the tree | `Read-only file system` |
| shell: fetch `https://example.com` with Python | blocked ("host is not on the allow list") |
| shell: `curl`, `git config` | refused by the deny list |
| shell: `git add -A`, `git commit` on its branch | worked |
| shell: move `main`, `origin/main`, a made-up `origin/<branch>`, create a tag, write a hook | `Read-only file system` |
| read the app's state folder (file tool and shell) | refused / not found |
| `Agent` / `Task` | the tools are not there |
| answer and follow-ups in the same session | remembered |

Two things only the real run showed, both fixed: the Linux sandbox puts placeholder dotfiles
into the tree that broke `git add -A` (the thread's git now ignores them), and it opens the
whole `.git` of the clone by itself, so "only its branch" has to be enforced by closing
everything else by name (before that fix the agent could move `main`).

The window itself (the Tauri binary, WebKitGTK) was built and run under Xvfb: it starts,
shows real data, navigates on a click, and shows a run delegated on the server arriving live.

Not verified anywhere: macOS (seatbelt sandbox, Keychain) and Windows (Credential Manager,
no sandbox); the tray icon, system notifications and the folder dialog (no desktop session
here); a long real implementation run; how the SDK reports cost for a resumed session (the
app takes the larger of "replace" and "add", see `Store::record_result`).

Added with the runner work, and checked: the real helper and Agent SDK against a scripted
stand-in for the model (`sidecar/test/confinement.test.ts`), which asks for files outside
the working tree and is refused each of them; the Runners and Tests views in a browser
against a real server with the real core behind them; this repository's own e2e suite run
on this computer by the app's executor inside bubblewrap, with its report and trace pages
arriving on that server.

What the first release added to the window's start was not run in a window either: taking
the `PATH` of the person's shell, the dialog about a missing or old Node, and fetching the
agent runtime with its dialogs, the progress in the title and the tray's menu item
(`src-tauri/src/environment.rs`, `runtime.rs`, `main.rs`). It compiles with the app, and the
parts without Tauri are tested alone (`rustc --edition 2021 --test` on the first two files,
then run the program it makes). Beyond those tests, the fetching code of `runtime.rs` was run
here as it is, with the real npm and registry, from a copy laid out like an installed app:
it left one whole runtime, the launcher started the helper from it (`ready`), the agent's
program answered, a second run needed neither npm nor the network, and nothing was written
into the copy of the installed folder.

## Releasing

A release is built on GitHub's machines by
[`.github/workflows/desktop-release.yml`](../../.github/workflows/desktop-release.yml) when a
tag `desktop-vX.Y.Z` is pushed, and waits as a **draft** until a person publishes it.

1. **One version everywhere.** Set it in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`
   and the three `package.json` files, and let the lock files follow:

   ```bash
   cd apps/desktop
   for folder in . ui sidecar; do npm version 0.2.0 --no-git-tag-version --prefix "$folder"; done
   # … edit "version" in src-tauri/tauri.conf.json and src-tauri/Cargo.toml, then:
   (cd src-tauri && cargo update --package team-hub-desktop)
   node scripts/check-version.mjs 0.2.0        # says which file still differs
   ```

2. **Write the notes.** [RELEASE_NOTES.md](RELEASE_NOTES.md) becomes the body of the release;
   its heading has to name the version, and its file names and limits have to be true for it.
3. Merge to `main`. For a rehearsal that publishes nothing: Actions → **Desktop release** →
   **Run workflow**, with the version; the installers are then artifacts of that run.
4. **Tag and push the tag**:

   ```bash
   git tag desktop-v0.2.0 && git push origin desktop-v0.2.0
   ```

5. The workflow checks that the tag is the version in the files, builds, and creates the draft
   release "team-hub desktop v0.2.0" with the installers, the `hub` archives and `SHA256SUMS`.
   Download them, try them, then **publish** the draft on the Releases page. Running the
   workflow again for the same tag (after deleting and pushing the tag again, or "Re-run
   jobs") replaces the files of the draft; a published release is never changed by it.

What the workflow builds, each on the system it is for:

| Job | Runner | Result |
|---|---|---|
| App · macos-arm64, macos-x64 | `macos-latest`, one build per architecture | `.dmg` |
| App · windows-x64 | `windows-latest` | NSIS `-setup.exe`, installed per user |
| App · linux-x64 | `ubuntu-22.04` | `.deb`, `.rpm`, `.AppImage` |
| hub · the same four | the same | `hub-…tar.gz`, `hub-…zip` |
| Draft the release | `ubuntu-latest`, the only job that may write | `SHA256SUMS`, the draft |

**What an installer carries.** The window, the interface, and under its resources the helper
without its dependencies: `sidecar/launch.mjs`, `sidecar/src`, `sidecar/package.json` and
`sidecar/package-lock.json`, 96 KB together. Not the Claude Agent SDK and not Node.js: the app
[fetches the first](#the-agent-runtime-of-an-installed-app) and runs the second (`node`, or
Settings → Node command) from the `PATH` of the person's shell, and says so in a dialog at
start when there is none or it is older than 22.18. The helper's share of an installer was
385 MB (190 MB packed) while the SDK was inside; no installer was built yet to weigh as a
whole. There is no `.msi`: add `"msi"` to `bundle.targets` in `tauri.conf.json` to get one.
There is no updater.

**The workflow tries what it built, as far as a build machine can.** After packaging,
`scripts/check-packaged.mjs` does what a first start does, on the unpacked `.deb`, the
extracted AppImage and the macOS `.app`: the package carries the helper and no
`node_modules`; before anything is fetched the launcher answers with a sentence; the
packaged program itself fetches the runtime (`--fetch-agent-runtime`, into a data folder of
its own); the agent's program answers as itself; the launcher starts the helper from there
and Node gets `ready`. For the Intel Mac build, which the Apple silicon build machine cannot
start, the same npm command is run instead of the program. On Windows the installer is run
silently and the same check made on what it installed; that step reports and does not stop
the release. None of this opens the window or runs an agent.

**Signing is optional and off.** Without secrets the macOS app gets an ad-hoc signature and
the Windows installer none; people have to allow the app by hand, as the notes describe.
Once a system is signed, take its "not signed" paragraph out of the notes.

- *macOS.* Add the repository secrets `APPLE_CERTIFICATE` (a Developer ID Application
  certificate as base64 of the `.p12`), `APPLE_CERTIFICATE_PASSWORD` and
  `APPLE_SIGNING_IDENTITY` to sign; add `APPLE_ID`, `APPLE_PASSWORD` (an app-specific
  password) and `APPLE_TEAM_ID` as well to notarize. The workflow passes them to Tauri only
  when they are set. This path was never run.
- *Windows.* Nothing is wired. It needs a code-signing certificate and, in `tauri.conf.json`,
  either `bundle.windows.certificateThumbprint` with `digestAlgorithm` and `timestampUrl` (the
  certificate imported into the runner's store from secrets in a step before the build), or
  `bundle.windows.signCommand` for a signing service such as Azure Trusted Signing (with its
  credentials as secrets). Certificates issued today live on hardware or in a service, so the
  second is the usual way.

**What the repository needs.** GitHub Actions enabled, with third-party actions allowed
(`tauri-apps/tauri-action`, `Swatinem/rust-cache`; every action is pinned to a commit). No
secret. The workflow asks for its own permissions, read everywhere and
`contents: write` for the one job that drafts the release, so the repository's default
workflow permissions can stay read-only.

**Licence.** team-hub is under the MIT licence; the bundle's metadata says so
(`bundle.license`). Nothing of Anthropic's is in an installer.

**Never run.** The workflow was written and checked (actionlint, shellcheck, the inputs of
each action, the Tauri configuration against the schema) on a machine that cannot run GitHub
Actions. Its first run is its first test.

## What is not done

- **The window with the tracker in it has run on Linux and on macOS, not on Windows.** On
  Linux on a virtual screen; on macOS 27 (Apple silicon) in a development build, driven
  through the accessibility interface against a local server with demo data: the two
  webviews side by side, resizing, hiding the panel, the app's own views in place of the
  tracker and the tracker back on the page it showed. On macOS that found the tracker lying
  a title bar too high, which is fixed (`title_bar` in `stage.rs`). On Windows the same
  code places the tracker with Tauri's own calls, which compile and have not been run by
  anybody.
- **Signing in, the thread link on a run's card, and the tracker's place while the server
  is away have run on Linux and on macOS, not on Windows.** On both: signing in with e-mail
  and password, with the tracker signed in by the same sign-in and still signed in after a
  restart; a wrong password; signing in at the tracker with a code; **Open the thread in
  the app** and the way back; the tracker leaving and returning with the server; and
  disconnecting removing the token on the server. On macOS **Try again** was pressed and
  brought the tracker back; the session reaches the tracker through WKWebView's cookie
  store. On Linux only: a keychain that refuses the token. Not seen on macOS: a link that
  leads away from the server opening in the browser (a press made through the
  accessibility interface does not count as a person's click for a link that asks for a
  new window), and the installed, signed app rather than a development build. On Windows
  nobody has seen any of it.
- What the core says in its own words (for example "Working tree ready…") is English in the
  Polish interface too.

- No installer was installed and tried by a person, on any system, and none is signed with a
  developer certificate. The release workflow has not run yet. Node and npm have to be
  installed on the user's machine.
- The download of the agent runtime shows its progress in the window's title and in native
  dialogs, not in the interface, and its texts are English only.
- The `hub` command line, downloaded alone, finds no helper and fetches none: it looks where
  the source tree was when it was built. Agent threads from it need `TEAM_HUB_SIDECAR`
  pointing at the app's launcher, after the app fetched the runtime (see the release notes).
- On Windows the core starts git and the helper without hiding their console, so windows may
  flash; nobody has looked.
- A pull request is opened with the `gh` command when it is installed (it is then yours). Without
  it, the server opens it with its own GitHub token when the repository is connected to the
  issue's project there, so the pull request is authored by whoever that token belongs to.
  Failing both, the app opens GitHub's compare page and you paste the address back. The app has
  no GitHub token of its own.
- One server and one account. No review remarks in line, no evidence or device tooling, no
  supervisor threads: those parts of agent-hub were left out on purpose.
- Core messages (errors, status lines of threads) are English only; the interface is
  translated.
- A run or a job carried out elsewhere is followed, not replayed: its diff is on the web.
  A finished job shows its state and opens its page in the browser; the totals are there.
- Test jobs on macOS and Windows run unconfined (as you, with a clean environment): there
  is no sandbox wrapper for them yet.
- Agent proposals are limited to a comment, a status change, subtasks, a push and a pull
  request. The run contract's `update_fields`, `create_issue` and `log_work` are not offered
  to agents, and an edited payload on approval is not sent.
- A thread whose agent commits with a signing setup that needs the network or an agent socket
  will fail to sign inside the sandbox.
