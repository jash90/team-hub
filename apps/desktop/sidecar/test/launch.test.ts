// The launcher an installed app carries: it runs the helper from the agent runtime the app
// fetched, and answers a thread in words when it cannot. Checked against made-up runtimes in
// a temporary data folder, and once with the real helper beside the installed dependencies.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { cpSync, mkdirSync, mkdtempSync, rmSync, symlinkSync, utimesSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const LAUNCHER = fileURLToPath(new URL('../launch.mjs', import.meta.url));
const SIDECAR = fileURLToPath(new URL('..', import.meta.url));

/** A data folder with a space in its name, as macOS has ("Application Support"). */
const dataFolder = () => mkdtempSync(join(tmpdir(), 'team-hub launch-'));

/** A runtime folder as the app leaves it: marked whole or not, with a helper that says who it is. */
const runtime = (data: string, name: string, options: { whole?: boolean; says?: string; age?: number } = {}) => {
  const folder = join(data, 'agent-runtime', name);
  const helper = join(folder, 'helper-0123456789abcdef');

  mkdirSync(helper, { recursive: true });
  writeFileSync(join(folder, 'package.json'), '{"type":"module"}');
  writeFileSync(join(helper, 'main.ts'), `export const main = async (): Promise<void> => { console.log(JSON.stringify({ type: 'ready', from: '${options.says ?? name}' })); };\n`);

  if (options.whole ?? true) {
    writeFileSync(join(folder, '.complete'), '');
  }

  if (options.age !== undefined) {
    const then = new Date(Date.now() - options.age * 1000);

    utimesSync(folder, then, then);
  }

  return join(helper, 'main.ts');
};

/** Runs the launcher with nothing to read, and gives back the lines of the protocol it printed. */
const launch = (data: string, args: string[] = []) => {
  const run = spawnSync(process.execPath, [LAUNCHER, ...args], { input: '', encoding: 'utf8', env: { ...process.env, TEAM_HUB_DESKTOP_DIR: data } });
  const messages = run.stdout.split('\n').filter(Boolean).map((line) => JSON.parse(line) as { type: string; from?: string; event?: { kind: string; message: string } });

  return { status: run.status, messages };
};

const errorOf = (outcome: ReturnType<typeof launch>) => {
  assert.equal(outcome.messages.length, 1);
  assert.equal(outcome.messages[0].type, 'event');
  assert.equal(outcome.messages[0].event?.kind, 'error');
  assert.equal(outcome.status, 1);

  return outcome.messages[0].event?.message ?? '';
};

test('without a runtime a thread is told so, and how it gets one', () => {
  const data = dataFolder();
  const named = join(data, 'agent-runtime', 'aaaa', 'helper-bbbb', 'main.ts');

  for (const args of [[named], []]) {
    const message = errorOf(launch(data, args));

    assert.match(message, /agent runtime .* is not on this computer yet/);
    assert.match(message, /Download the agent runtime/);
  }

  rmSync(data, { recursive: true, force: true });
});

test('a download under way, cut off, or failed is told apart', () => {
  const data = dataFolder();
  const named = join(data, 'agent-runtime', 'aaaa', 'helper-bbbb', 'main.ts');
  const state = (text: object) => writeFileSync(join(data, 'agent-runtime', 'state.json'), JSON.stringify(text));

  mkdirSync(join(data, 'agent-runtime'), { recursive: true });

  state({ state: 'downloading', pid: process.pid, reason: '' });
  assert.match(errorOf(launch(data, [named])), /still being downloaded/);

  // Whoever was downloading is gone.
  state({ state: 'downloading', pid: 2 ** 22 + 12345, reason: '' });
  assert.match(errorOf(launch(data, [named])), /download was interrupted/);

  state({ state: 'failed', pid: 1, reason: 'the npm registry could not be reached.' });
  assert.match(errorOf(launch(data, [named])), /the download failed, because the npm registry could not be reached\./);

  rmSync(data, { recursive: true, force: true });
});

test('the helper named by the app is run, from a folder with a space in its name', () => {
  const data = dataFolder();
  const script = runtime(data, 'aaaa');
  const outcome = launch(data, [script]);

  assert.deepEqual(outcome.messages, [{ type: 'ready', from: 'aaaa' }]);
  assert.equal(outcome.status, 0);

  rmSync(data, { recursive: true, force: true });
});

test('left to find it, the launcher takes the newest whole runtime and no half one', () => {
  const data = dataFolder();

  runtime(data, 'older', { age: 3600 });
  runtime(data, 'newer', { age: 60 });
  runtime(data, 'half', { whole: false });
  runtime(data, 'cccc.partial-123');

  assert.deepEqual(launch(data).messages, [{ type: 'ready', from: 'newer' }]);

  // A half runtime is not run even when the app names it.
  assert.match(errorOf(launch(data, [join(data, 'agent-runtime', 'half', 'helper-0123456789abcdef', 'main.ts')])), /not on this computer yet/);

  rmSync(data, { recursive: true, force: true });
});

test('a helper that cannot be loaded fails the thread with the reason', () => {
  const data = dataFolder();
  const script = runtime(data, 'aaaa');

  writeFileSync(script, "import 'a-package-that-is-not-there';\n");
  assert.match(errorOf(launch(data, [script])), /helper could not be started from .*a-package-that-is-not-there/);

  rmSync(data, { recursive: true, force: true });
});

test('the real helper starts through the launcher, beside the installed dependencies', () => {
  const data = dataFolder();
  const folder = join(data, 'agent-runtime', 'real');

  mkdirSync(folder, { recursive: true });
  cpSync(join(SIDECAR, 'package.json'), join(folder, 'package.json'));
  cpSync(join(SIDECAR, 'src'), join(folder, 'helper-0'), { recursive: true });
  symlinkSync(join(SIDECAR, 'node_modules'), join(folder, 'node_modules'), 'junction');
  writeFileSync(join(folder, '.complete'), '');

  // The input is closed at once: the helper says it is ready and ends, as when a thread is stopped.
  const outcome = launch(data);

  assert.deepEqual(outcome.messages, [{ type: 'ready', protocol: 1 }]);
  assert.equal(outcome.status, 0);

  rmSync(data, { recursive: true, force: true });
});
