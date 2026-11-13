# team-hub desktop 0.1.0

The first release of the desktop app for [team-hub](https://github.com/jash90/team-hub).

**Read this first.** The app was written and run on Linux only. The installers below were
built by GitHub's machines; none of them was installed and tried by a person before this
release, and the macOS and Windows builds have never been run at all. They are not signed
with a developer certificate. Treat 0.1.0 as a preview and
[report](https://github.com/jash90/team-hub/issues) what does not work.

## What it is

A developer's cockpit on top of a team-hub server.

- The issues assigned to you become **agent threads**: an AI agent works on an issue in a
  local clone of the repository, in a git working tree and branch of its own.
- **Nothing leaves your computer without a click.** A comment, a status change, subtasks, a
  push and a pull request are proposals you approve; a push shows exactly the commits and the
  diff it would send.
- The app is also a **runner**: work delegated to an agent in the team-hub web interface
  ("Implement in code", "Review the pull request", a custom task) is claimed by the app on a
  computer that has the repository, and reported back to the run the web shows.
- It is a runner for **test jobs** too: a project's e2e suite started from team-hub, or
  from the app's Tests view, runs on this computer, with its processor and memory, and the
  report, recordings and screenshots go back to the project. By default the app runs only
  the suites you send to it by name; a setting opens it to the team's queue, another turns
  test jobs off.
- **Runners** lists every runner of the server (this computer, other people's computers
  that are connected, the cloud runners the team shares), and both an agent thread and a
  test run can be handed to another runner instead of this computer; the app then follows
  the work like the web does.
- The agent's shell runs in the operating system's sandbox (no network, writes only inside
  its working tree); where there is no sandbox the thread gets no shell unless you allow one.
- Your team-hub token lives in the system's keychain; the agent never holds it.

How it works and the safety model in full:
[apps/desktop/README.md](https://github.com/jash90/team-hub/blob/main/apps/desktop/README.md).

## What you need

| | |
|---|---|
| A team-hub server | The app is a client: it needs the address of a server and an API token (in team-hub: Settings → API tokens). |
| Node.js 22.18 or newer, with npm | Not included. Node runs the agent's helper, and npm (it comes with Node) fetches the agent runtime the first time the app starts; see below. The app says so at start when it does not find them. If Node comes from a version manager and is not found, put its full path in the app's Settings → Node command. |
| The network, once, and about 400 MB of disk | For that first download. |
| git | And a local clone of each repository agents should work in. |
| Claude Code signed in, or an Anthropic API key | The helper uses the Claude Agent SDK, with the sign-in of Claude Code on your computer or `ANTHROPIC_API_KEY` from the app's environment. |
| Optional | GitHub's `gh` to open pull requests as yourself. On Linux `bubblewrap` and `socat` for the shell sandbox, and a Secret Service (GNOME Keyring, KWallet) for the keychain. |

## The agent runtime is downloaded at the first start

The installers do not contain the Claude Agent SDK and its `claude` program, which are
Anthropic's (© Anthropic PBC; their use is subject to Anthropic's
[legal agreements](https://code.claude.com/docs/en/legal-and-compliance)). The app fetches
them on your computer the first time it starts, and again only when an update of the app
needs other versions:

- **What and from where:** the 106 npm packages the helper depends on, in exactly the
  versions of the lock file inside the installer, with your own npm
  (`npm ci --omit=dev --no-audit --no-fund --ignore-scripts --no-bin-links`), from the
  registry your npm is set up with (registry.npmjs.org unless you changed it). No install
  script is run.
- **How big:** a download of about 165 MB on macOS and Windows and about 330 MB on Linux;
  370 to 410 MB on disk.
- **Where it is kept:** in `agent-runtime` in the app's data folder:
  `~/.local/share/team-hub-desktop/` on Linux,
  `~/Library/Application Support/team-hub-desktop/` on macOS,
  `%APPDATA%\team-hub-desktop\` on Windows.
- **What you see:** a dialog when the download starts, its progress in the window's title,
  a notification when the runtime is ready. If it fails, a dialog says why (no npm, no
  network, the registry refusing) and has a Retry button; the tray menu has "Download the
  agent runtime". From a terminal `team-hub-desktop --fetch-agent-runtime` does the same and
  shows what npm prints (on macOS the program is
  `/Applications/team-hub.app/Contents/MacOS/team-hub-desktop`; on Windows nothing is
  printed, and npm's output is in `agent-runtime\install.log`). Until the runtime is there,
  starting a thread tells you so, and the rest of the app works. Afterwards no network is
  needed for it.
- **Removing it:** quit the app and delete the `agent-runtime` folder. The next start
  downloads it again.

## Download and install

| System | File |
|---|---|
| macOS 13 or newer, Apple silicon | `team-hub-desktop-0.1.0-macos-arm64.dmg` |
| macOS 13 or newer, Intel | `team-hub-desktop-0.1.0-macos-x64.dmg` |
| Windows 10 and 11, 64-bit Intel/AMD | `team-hub-desktop-0.1.0-windows-x64-setup.exe` |
| Debian 12 or newer, Ubuntu 22.04 or newer | `team-hub-desktop-0.1.0-linux-x64.deb` |
| Fedora (other RPM systems are untried) | `team-hub-desktop-0.1.0-linux-x64.rpm` |
| Other Linux (glibc 2.35 or newer) | `team-hub-desktop-0.1.0-linux-x64.AppImage` |
| The `hub` command line alone | `hub-0.1.0-macos-arm64.tar.gz`, `hub-0.1.0-macos-x64.tar.gz`, `hub-0.1.0-linux-x64.tar.gz`, `hub-0.1.0-windows-x64.zip` |
| Checksums of all of the above | `SHA256SUMS` |

Check a download: `sha256sum --ignore-missing -c SHA256SUMS` (macOS:
`shasum -a 256 --ignore-missing -c SHA256SUMS`).

### macOS

Take the file for your Mac's processor (Apple menu → About This Mac), open the `.dmg` and
drag **team-hub** to Applications.

The app is not signed with an Apple Developer ID and not notarized, so macOS refuses to open
it the first time ("Apple could not verify…", or "is damaged and can't be opened"). Either:

- open it once, then go to System Settings → Privacy & Security, scroll to the message about
  team-hub and choose **Open Anyway**; or
- remove the download mark in a terminal:
  `xattr -dr com.apple.quarantine /Applications/team-hub.app`

Only do this for a file whose checksum you have checked.

### Windows

Run the `-setup.exe`. It is not signed, so SmartScreen shows "Windows protected your PC":
choose **More info**, then **Run anyway**. It installs for your user, without administrator
rights, and installs Microsoft's WebView2 runtime if the computer does not have it (that
needs the internet).

### Linux

```bash
sudo apt install ./team-hub-desktop-0.1.0-linux-x64.deb      # Debian, Ubuntu
sudo dnf install ./team-hub-desktop-0.1.0-linux-x64.rpm      # Fedora
chmod +x team-hub-desktop-0.1.0-linux-x64.AppImage && ./team-hub-desktop-0.1.0-linux-x64.AppImage
```

The packages need WebKitGTK 4.1 (`libwebkit2gtk-4.1-0`), which the `.deb` and `.rpm` ask
for. Ubuntu's own `nodejs` package is older than 22.18; install Node from
[nodejs.org](https://nodejs.org) or NodeSource, which bring npm with them.

### The `hub` command line

One program, no installer: unpack it and put it on your `PATH` (on macOS, after
`xattr -d com.apple.quarantine hub`). `hub connect https://hub.example.com`, `hub status`,
`hub issue WEB-12`, `hub proposals`, `hub push 1` and the rest work as they are. To run agent
threads (`hub thread run`, `hub runner`) it needs the helper and the agent runtime. It does
not fetch the runtime itself: install the app, let it download the runtime once, and point
`TEAM_HUB_SIDECAR` at the app's launcher, which finds what the app fetched:
`TEAM_HUB_SIDECAR="node /usr/lib/team-hub/sidecar/launch.mjs"` on Linux or
`TEAM_HUB_SIDECAR="node /Applications/team-hub.app/Contents/Resources/sidecar/launch.mjs"` on macOS.

## Licence

team-hub and this app are under the MIT licence. Nothing of Anthropic's is in the installers;
what the app downloads at its first start is described above.

## First steps

1. Start the app, enter the server's address and your API token.
2. **Repositories → Choose the folder…** next to a project's GitHub repository, and pick your
   local clone.
3. Open an issue assigned to you and start a thread, or leave the app running (closing the
   window keeps it in the tray) and delegate work to an agent on the web.

## Known limits

- **Only Linux was ever run.** Not verified anywhere: macOS (the seatbelt sandbox, Keychain)
  and Windows (Credential Manager; there is no shell sandbox on Windows, so threads work
  without a shell unless you allow one in Settings); the tray icon, system notifications and
  the folder dialog; a long real implementation run. On Windows, console windows may appear
  while the app runs git or an agent.
- **The installers are untested and unsigned**, as said above. The AppImage is the least
  certain of the three Linux packages.
- **Node.js and npm are not bundled.** The app looks for `node` and `npm` on the `PATH` of
  your shell; a Node it cannot find or that is older than 22.18 is reported when the app
  starts, and so is a download of the agent runtime that failed.
- **The download of the agent runtime** was tried on Linux only. Its progress is shown in
  the window's title and in system dialogs, in English, not in the interface.
- **An API key in the environment** reaches the app only when the app inherits that
  environment, for example when it is started from a terminal. Being signed in to Claude Code
  needs nothing of the kind.
- Windows on ARM and Linux on ARM have no build.
- There is no automatic update: install a newer release over this one.
- One server and one account. No review remarks in line.
- A pull request is opened with `gh` when it is installed (it is then yours). Without it the
  server opens it with its own GitHub token when the repository is connected to the project
  there; failing both, the app opens GitHub's compare page and you paste the address back.
- Agent proposals are limited to a comment, a status change, subtasks, a push and a pull
  request.
- Messages of the core (errors, status lines of threads) are English only; the interface is in
  English and Polish.
- A thread whose agent commits with a signing setup that needs the network or an agent socket
  will fail to sign inside the sandbox.
