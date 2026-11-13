#!/usr/bin/env node
// Is the helper that an installer carries able to start? Given the folder an installed (or
// unpacked) app keeps its resources in, checks that the helper's files are there, that the
// agent's native program is still that program, and that Node starts the helper from there.
//
//   node apps/desktop/scripts/check-packaged.mjs <resources> [--os linux --cpu x64]
//
//   <resources> is, for example:
//     /usr/lib/team-hub                                  a .deb or .rpm, installed or unpacked
//     squashfs-root/usr/lib/team-hub                     an AppImage after --appimage-extract
//     /Applications/team-hub.app/Contents/Resources      macOS
//     %LOCALAPPDATA%\team-hub                            Windows
//     apps/desktop                                       the source tree, after `npm run sidecar`
//
// It needs no account and no network. The release workflow runs it on what it has built,
// because packaging tools rewrite programs they find (`strip` turns the agent's program into
// a bare JavaScript runtime that still starts), and nobody sees that until a thread fails.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { parseArgs } from 'node:util';

const { values, positionals } = parseArgs({ allowPositionals: true, options: { os: { type: 'string' }, cpu: { type: 'string' } } });
const os = values.os ?? process.platform;
const cpu = values.cpu ?? process.arch;

if (positionals.length !== 1) {
  console.error('usage: check-packaged.mjs <resources folder> [--os darwin|linux|win32] [--cpu x64|arm64]');
  process.exit(2);
}

const sidecar = join(resolve(positionals[0]), 'sidecar');
const modules = join(sidecar, 'node_modules');
const program = join(modules, '@anthropic-ai', `claude-agent-sdk-${os}-${cpu}`, os === 'win32' ? 'claude.exe' : 'claude');
const script = join(sidecar, 'src', 'main.ts');
const problems = [];
const fail = (problem) => {
  problems.push(problem);
  console.error(`::error::${problem}`);
};

for (const file of [script, join(sidecar, 'src', 'hub-tools.ts'), join(sidecar, 'package.json'), join(modules, '@anthropic-ai', 'claude-agent-sdk', 'sdk.mjs'), join(modules, 'zod', 'package.json'), program]) {
  if (existsSync(file)) {
    console.log(`ok   ${file}`);
  } else {
    fail(`${file} is missing`);
  }
}

// An empty home: the program must not need, or leave, anything of the person running this.
const home = mkdtempSync(join(tmpdir(), 'team-hub-check-'));
const env = { ...process.env, HOME: home, USERPROFILE: home };

if (os !== process.platform || cpu !== process.arch) {
  console.log(`skip the program is for ${os}-${cpu} and this is ${process.platform}-${process.arch}: not run`);
} else if (existsSync(program)) {
  const run = spawnSync(program, ['--version'], { encoding: 'utf8', timeout: 120_000, env });
  const printed = `${run.stdout ?? ''}${run.stderr ?? ''}`.trim();

  if (run.status === 0 && /\(Claude Code\)/.test(printed)) {
    console.log(`ok   the agent's program answers: ${printed}`);
  } else {
    fail(`${program} --version gave "${printed}" (${run.error?.message ?? `exit ${run.status}`}), not "… (Claude Code)": the packaging changed the program`);
  }
}

if (existsSync(script)) {
  // The input closes at once, as it does when the app stops a thread: the helper says it is
  // ready and ends.
  const run = spawnSync(process.execPath, [script], { input: '', encoding: 'utf8', timeout: 120_000, cwd: home, env });
  const ready = (run.stdout ?? '').split('\n').some((line) => {
    try {
      return JSON.parse(line).type === 'ready';
    } catch {
      return false;
    }
  });

  if (ready) {
    console.log(`ok   Node ${process.version} starts the helper from ${sidecar}`);
  } else {
    fail(`Node ${process.version} did not get "ready" from ${script} (${run.error?.message ?? `exit ${run.status}`}): ${(run.stderr ?? '').trim().split('\n').slice(-5).join(' | ')}`);
  }
}

rmSync(home, { recursive: true, force: true });
process.exit(problems.length > 0 ? 1 : 0);
