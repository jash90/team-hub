# team-hub desktop

A developer's cockpit on top of team-hub. The issues assigned to you become **agent threads**:
an AI agent works on an issue in a local clone of the repository, in a git working tree and
branch of its own, and whatever should leave your computer (a comment, a status change, a
push, a pull request) waits for your click.

The same app is a **runner** for team-hub: work delegated to an AI agent in the web interface
("Implement in code", "Review the pull request", a custom task) is claimed by the app on a
computer that has the repository, carried out as a thread, and reported back to the run the
web shows. See [`docs/agent-runs.md`](../../docs/agent-runs.md) and
[`modules/agents/README.md`](../../modules/agents/README.md).

```
core/        hub-desktop-core: everything that is not a window. No Tauri; shares hub-core with the server.
             Also the `hub` command line, the fake agent and the development server.
src-tauri/   The window: one command that forwards to the core, the tray, notifications, window state.
ui/          The interface: React, TypeScript, Vite. Polish and English, light and dark.
sidecar/     Node helper that runs one agent turn through the Claude Agent SDK, JSON lines on stdin/stdout.
scripts/     The end-to-end check against a real server; the icon generator.
```

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
For installers (`.dmg`, `.msi`, `.deb`/AppImage): `(cd apps/desktop && npm install && npm run build)`.

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
```

Environment: `TEAM_HUB_DESKTOP_DIR` moves the app's data; `TEAM_HUB_TOKEN` gives the token for
one process on a machine without a keychain (it is then stored nowhere); `TEAM_HUB_SIDECAR`
replaces the helper process (e.g. with `target/debug/hub-fake-agent`).

## Connecting to a server

In team-hub: **Settings → API tokens → Create token**. In the app: the server's address and
that token. The app acts as you, with your permissions.

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

## The safety model, in plain words

- **One working tree and branch per thread.** The main clone's checkout, index and branch are
  never touched: the app only reads it (`git status`, `rev-parse`) and adds working trees.
- **The shell is in the operating system's sandbox**: no network, writes only inside the
  working tree plus the four places in the clone's `.git` a commit needs (objects, the tree's
  own folder, the branch's ref and reflog). Hooks, config and the tree's pointers back to the
  clone stay closed. Where the system has no sandbox (Windows; Linux without bubblewrap) the
  thread gets **no shell at all**, unless you switch that on in Settings.
- **File tools are tied to the working tree**: `Edit` and `Write` are allowed for that folder
  only; review and analysis threads do not get them. Credential folders (`~/.ssh`, `~/.aws`,
  `~/.config/gh`, …) and the app's own state cannot be read.
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
  installed Claude Agent SDK, and its side of the protocol.
- The interface in a browser, with the real core behind it:

  ```bash
  cargo run -p hub-desktop-core --features dev-server --bin hub-dev-server   # the core on 127.0.0.1:4455
  (cd apps/desktop/ui && npm run dev)                                        # http://localhost:1430
  ```

  `ui/src/bridge.ts` is the only file that knows whether it talks to Tauri or to that server;
  both end in `hub_desktop_core::commands::invoke`.
- `(cd ui && npm run i18n)` lists phrases without Polish.

TypeScript types under `ui/src/bindings` are generated from the Rust ones by `cargo test`;
types of the server come straight from `web/src/bindings`. Never edit either by hand.

## What is not done

- The real helper has not been run against a model on the machine it was written on (no
  Claude account there). It type-checks against `@anthropic-ai/claude-agent-sdk` and its
  protocol side is tested; the permission settings (`dontAsk`, the sandbox, the allow list)
  follow the SDK's types and agent-hub's working configuration, and need a first real run.
- Packaging is configured (`tauri.conf.json` bundles the sidecar's sources and its
  `node_modules` as resources) but no installer was built or signed; Node has to be installed
  on the user's machine.
- A pull request is opened with the `gh` command when it is installed; otherwise the app opens
  GitHub's compare page and you paste the address back. The app has no GitHub token of its own.
- One server and one account. No review remarks in line, no evidence or device tooling, no
  supervisor threads: those parts of agent-hub were left out on purpose.
- Core messages (errors, status lines of threads) are English only; the interface is
  translated.
- Agent proposals are limited to a comment, a status change, subtasks, a push and a pull
  request. The run contract's `update_fields`, `create_issue` and `log_work` are not offered
  to agents, and an edited payload on approval is not sent.
