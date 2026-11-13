#!/usr/bin/env node
// Starts the helper from the agent runtime the app fetched, or tells the thread in plain words
// why it cannot. This file is what an installer carries instead of `node_modules`: it needs
// nothing but Node.
//
//   node launch.mjs <agent-runtime>/<hash>/helper-<hash>/main.ts     what the app runs
//   node launch.mjs                                                  finds the runtime the app fetched;
//                                                                    for the `hub` command line:
//                                                                    TEAM_HUB_SIDECAR="node …/sidecar/launch.mjs"
//
// The helper's source sits beside the fetched `node_modules` (see src-tauri/src/runtime.rs),
// because that is where Node looks for what the source imports. Whatever goes wrong before
// the helper runs is said as an `error` event of the helper's protocol, so the thread fails
// with that sentence and not with a stack trace.
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const FOLDER = 'agent-runtime';

const fail = (message) => {
  process.stdout.write(`${JSON.stringify({ type: 'event', event: { kind: 'error', message } })}\n`, () => process.exit(1));
};

/** Node runs TypeScript itself from 22.18, 23.6 and 24.0 on. */
const runsTypeScript = () => {
  const [major, minor] = process.versions.node.split('.').map(Number);

  return major > 23 || (major === 23 && minor >= 6) || (major === 22 && minor >= 18);
};

/** The app's data folder, as `hub_desktop_core::config::Paths::from_env` finds it. */
const dataFolder = () => {
  if (process.env.TEAM_HUB_DESKTOP_DIR) {
    return process.env.TEAM_HUB_DESKTOP_DIR;
  }

  const base =
    process.platform === 'win32'
      ? (process.env.APPDATA ?? join(homedir(), 'AppData', 'Roaming'))
      : process.platform === 'darwin'
        ? join(homedir(), 'Library', 'Application Support')
        : process.env.XDG_DATA_HOME || join(homedir(), '.local', 'share');

  return join(base, 'team-hub-desktop');
};

const folders = (folder) => {
  try {
    return readdirSync(folder, { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => join(folder, entry.name));
  } catch {
    return [];
  }
};

const newestFirst = (paths) => paths.map((path) => [statSync(path).mtimeMs, path]).sort((one, other) => other[0] - one[0]).map(([, path]) => path);

/** Without being told: the newest whole runtime under the data folder, and the helper in it. */
const find = (root) => {
  const whole = newestFirst(folders(root).filter((folder) => !folder.includes('.partial-') && existsSync(join(folder, '.complete'))));

  for (const runtime of whole) {
    const helper = newestFirst(folders(runtime).filter((folder) => /helper-[0-9a-f]+$/.test(folder))).find((folder) => existsSync(join(folder, 'main.ts')));

    if (helper) {
      return join(helper, 'main.ts');
    }
  }

  return undefined;
};

/** What the app wrote down about a download that is under way or went wrong. */
const lastWord = (root) => {
  try {
    return JSON.parse(readFileSync(join(root, 'state.json'), 'utf8'));
  } catch {
    return {};
  }
};

/** Whether the app that wrote "downloading" is still there to finish it. */
const isRunning = (pid) => {
  try {
    process.kill(pid, 0);

    return true;
  } catch (error) {
    // Not allowed to signal it: it exists.
    return error.code === 'EPERM';
  }
};

const asked = process.argv[2];
const root = asked ? dirname(dirname(dirname(asked))) : join(dataFolder(), FOLDER);
const script = asked ?? find(root);
const ready = script !== undefined && existsSync(script) && existsSync(join(dirname(dirname(script)), '.complete'));

if (!runsTypeScript()) {
  fail(`Node.js ${process.versions.node} is too old to run the agent's helper: it needs 22.18 or newer. Install it from https://nodejs.org, or put the full path of a newer node in Settings → Node command, and start team-hub again.`);
} else if (!ready) {
  const state = lastWord(root);
  const how = 'team-hub fetches it when it starts and from its tray menu ("Download the agent runtime"); it needs npm and the network once.';

  if (state.state === 'downloading' && isRunning(state.pid)) {
    fail('The agent runtime is still being downloaded (about 350 MB, once). Start the thread again when team-hub says it is ready.');
  } else if (state.state === 'downloading') {
    fail(`The agent runtime is not on this computer: its download was interrupted. ${how}`);
  } else if (state.state === 'failed') {
    fail(`The agent runtime is not on this computer: the download failed, because ${state.reason} ${how}`);
  } else {
    fail(`The agent runtime (the Claude Agent SDK and its program) is not on this computer yet. ${how} Looked in ${root}.`);
  }
} else {
  try {
    const helper = await import(pathToFileURL(script).href);

    await helper.main();
  } catch (error) {
    fail(`The agent's helper could not be started from ${script}: ${error instanceof Error ? error.message : String(error)}`);
  }
}
