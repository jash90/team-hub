#!/usr/bin/env node
// Can an installed app get its agent runtime and start the helper? Given the folder an
// installed (or unpacked) app keeps its resources in, this does what the app does on a
// computer that has never run it, in a data folder of its own that is removed afterwards:
//
//   1. the installer carries the helper's source, its package.json and lock file and the
//      launcher, and nothing of the Claude Agent SDK;
//   2. before anything is fetched, the launcher answers a thread with a sentence, not a crash;
//   3. the runtime is fetched: by the app itself (`--app <program>`, which is started with
//      --fetch-agent-runtime), or else with the same npm command the app runs, read from
//      src-tauri/src/runtime.rs;
//   4. the agent's program in what was fetched answers as itself;
//   5. the launcher starts the helper from the fetched runtime, and Node gets "ready".
//
//   node apps/desktop/scripts/check-packaged.mjs <resources> [--app <program>]
//
//   <resources>                                         --app
//     <unpacked .deb>/usr/lib/TeamHub                     <unpacked .deb>/usr/bin/team-hub-desktop
//     squashfs-root/usr/lib/team-hub                      squashfs-root/AppRun
//     TeamHub.app/Contents/Resources                     TeamHub.app/Contents/MacOS/team-hub-desktop
//     %LOCALAPPDATA%\TeamHub                              %LOCALAPPDATA%\TeamHub\team-hub-desktop.exe
//     apps/desktop                                        (the source tree: no program)
//
// It needs npm and the network (about 350 MB), and no account.
import { spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const { values, positionals } = parseArgs({ allowPositionals: true, options: { app: { type: 'string' } } });

if (positionals.length !== 1) {
  console.error('usage: check-packaged.mjs <resources folder> [--app <the installed program>]');
  process.exit(2);
}

const sidecar = join(resolve(positionals[0]), 'sidecar');
const launcher = join(sidecar, 'launch.mjs');
const data = mkdtempSync(join(tmpdir(), 'team-hub-check-'));
const root = join(data, 'agent-runtime');
// A home and a data folder of its own: nothing of the person running this is used or left.
const env = { ...process.env, TEAM_HUB_DESKTOP_DIR: data, HOME: process.env.HOME ?? data };
let failed = false;

const ok = (text) => console.log(`ok   ${text}`);
const fail = (text) => {
  failed = true;
  console.error(`::error::${text}`);
};
const finish = () => {
  rmSync(data, { recursive: true, force: true });
  process.exit(failed ? 1 : 0);
};

/** What the launcher prints with nothing to read: the lines of the helper's protocol. */
const launch = (...args) => {
  const run = spawnSync(process.execPath, [launcher, ...args], { input: '', encoding: 'utf8', timeout: 120_000, cwd: data, env });
  const messages = (run.stdout ?? '').split('\n').flatMap((line) => {
    try {
      return [JSON.parse(line)];
    } catch {
      return [];
    }
  });

  return { messages, said: `${run.stdout ?? ''}${run.stderr ?? ''}`.trim().split('\n').slice(-4).join(' | ') };
};

const folders = (folder) => (existsSync(folder) ? readdirSync(folder, { withFileTypes: true }).filter((entry) => entry.isDirectory()).map((entry) => entry.name) : []);

// 1. What the installer carries, and what it must not.
for (const file of ['launch.mjs', 'package.json', 'package-lock.json', join('src', 'main.ts'), join('src', 'hub-tools.ts')]) {
  if (existsSync(join(sidecar, file))) {
    ok(join(sidecar, file));
  } else {
    fail(`${join(sidecar, file)} is missing`);
  }
}

// A source tree has the helper's own tsconfig.json, which no installer carries, and may have
// its development dependencies installed.
if (existsSync(join(sidecar, 'tsconfig.json'))) {
  ok('a source tree, not a package: its node_modules is not looked at');
} else if (existsSync(join(sidecar, 'node_modules'))) {
  fail(`${join(sidecar, 'node_modules')} is in the package: an installer does not carry the Claude Agent SDK`);
} else {
  ok('the package carries no node_modules');
}

if (failed) {
  finish();
}

// 2. Nothing fetched yet: a thread is told so.
const before = launch().messages[0];

if (before?.type === 'event' && before.event?.kind === 'error' && /agent runtime/.test(before.event.message)) {
  ok(`without the runtime a thread is told: ${before.event.message.slice(0, 90)}…`);
} else {
  fail(`without the runtime the launcher said ${JSON.stringify(before)} instead of an error event about the agent runtime`);
}

// 3. Fetch it.
if (values.app !== undefined) {
  const program = resolve(values.app);

  if (!existsSync(program)) {
    fail(`${program} is not there; beside it: ${folders(dirname(program)).concat(existsSync(dirname(program)) ? readdirSync(dirname(program)) : []).join(', ')}`);
    finish();
  }

  console.log(`$ ${program} --fetch-agent-runtime`);

  const run = spawnSync(program, ['--fetch-agent-runtime'], { stdio: 'inherit', timeout: 20 * 60_000, env });

  if (run.status === 0) {
    ok('the app fetched its runtime');
  } else {
    const log = join(root, 'install.log');

    fail(`${program} --fetch-agent-runtime ended with ${run.error?.message ?? `exit ${run.status}`}${existsSync(log) ? `: ${readFileSync(log, 'utf8').trim().split('\n').slice(-6).join(' | ')}` : ''}`);
  }
} else {
  // The app's own command, from the app's source.
  const source = join(dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri', 'src', 'runtime.rs');
  const list = readFileSync(source, 'utf8').match(/NPM_ARGUMENTS: \[&str; \d+\] = \[([^\]]+)\]/)?.[1];
  const args = list?.split(',').map((arg) => arg.trim().replace(/^"|"$/g, '')).filter(Boolean);

  if (!args?.includes('ci')) {
    fail(`could not read the npm command from ${source}`);
    finish();
  }

  const partial = join(root, 'check.partial-0');

  mkdirSync(partial, { recursive: true });
  cpSync(join(sidecar, 'package.json'), join(partial, 'package.json'));
  cpSync(join(sidecar, 'package-lock.json'), join(partial, 'package-lock.json'));
  // The same command, without the line per download that the app shows as progress.
  const quiet = args.filter((arg) => arg !== '--loglevel=http');

  console.log(`$ npm ${quiet.join(' ')}`);

  // npm is `npm.cmd` on Windows, which only a shell starts.
  const run = spawnSync('npm', quiet, { cwd: partial, stdio: 'inherit', timeout: 20 * 60_000, env, shell: process.platform === 'win32' });

  if (run.status === 0) {
    writeFileSync(join(partial, '.complete'), '');
    renameSync(partial, join(root, 'check'));
    cpSync(join(sidecar, 'src'), join(root, 'check', 'helper-0'), { recursive: true });
    ok('npm fetched the runtime with the command the app runs');
  } else {
    fail(`npm ended with ${run.error?.message ?? `exit ${run.status}`}`);
  }
}

if (failed) {
  finish();
}

// What is there now: one whole runtime, nothing half.
const runtimes = folders(root);
const runtime = join(root, runtimes[0] ?? '');
const helper = folders(runtime).find((name) => name.startsWith('helper-'));

if (runtimes.length !== 1 || !existsSync(join(runtime, '.complete')) || helper === undefined) {
  fail(`${root} should hold one finished runtime with a helper beside it, and holds: ${runtimes.join(', ')} / ${folders(runtime).join(', ')}`);
  finish();
}

ok(`one runtime in ${root}: ${runtimes[0]}`);

// 4. The agent's program is the agent's program.
const scope = join(runtime, 'node_modules', '@anthropic-ai');
const mine = `claude-agent-sdk-${process.platform}-${process.arch}`;
const programs = folders(scope).filter((name) => name === mine || name === `${mine}-musl`);

if (programs.length === 0) {
  fail(`${scope} has no program for ${process.platform}-${process.arch}: ${folders(scope).join(', ')}`);
}

for (const name of programs) {
  const program = join(scope, name, process.platform === 'win32' ? 'claude.exe' : 'claude');
  const run = spawnSync(program, ['--version'], { encoding: 'utf8', timeout: 120_000, env });
  const printed = `${run.stdout ?? ''}${run.stderr ?? ''}`.trim();

  if (run.status === 0 && /\(Claude Code\)/.test(printed)) {
    ok(`${name} answers: ${printed}`);
  } else if (name.endsWith('-musl') && programs.length > 1) {
    console.log(`skip ${name} does not run here (${run.error?.message ?? `exit ${run.status}`}): the other one is this system's`);
  } else {
    fail(`${program} --version gave "${printed}" (${run.error?.message ?? `exit ${run.status}`}), not "… (Claude Code)"`);
  }
}

// 5. The helper starts: told where it is, as the app tells it, and finding it, as `hub` would.
for (const [how, args] of [
  ['told where the helper is', [join(runtime, helper, 'main.ts')]],
  ['finding the runtime itself', []],
]) {
  const { messages, said } = launch(...args);

  if (messages.some((message) => message.type === 'ready')) {
    ok(`Node ${process.version} starts the helper through the launcher, ${how}`);
  } else {
    fail(`the launcher, ${how}, did not get "ready" from the helper: ${said}`);
  }
}

finish();
