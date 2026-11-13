#!/usr/bin/env node
// Makes `sidecar/node_modules` what an installer should carry: the helper's production
// dependencies and the one native `claude` program of the system the installer is for.
//
//   node apps/desktop/scripts/prepare-sidecar.mjs                         for this computer
//   node apps/desktop/scripts/prepare-sidecar.mjs --os darwin --cpu x64   for another one
//
// `tauri.conf.json` bundles the whole folder, so run this before `tauri build`. Afterwards
// the folder has no development dependencies: `npm install` in `sidecar` brings them back.
//
// The Claude Agent SDK ships its command line as one optional package per system, about
// 350 MB each, and looks for the one named after the system it runs on. npm installs the
// ones for the computer it runs on; `--os` and `--cpu` make it install another computer's
// (an Intel Mac's on an Apple silicon builder). On Linux npm cannot tell glibc from musl here
// and installs both; every Linux package we build needs glibc, so the musl one is removed.
import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync, rmSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const { values } = parseArgs({ options: { os: { type: 'string' }, cpu: { type: 'string' } } });
const os = values.os ?? process.platform;
const cpu = values.cpu ?? process.arch;
const sidecar = join(dirname(fileURLToPath(import.meta.url)), '..', 'sidecar');
const scope = join(sidecar, 'node_modules', '@anthropic-ai');
const native = 'claude-agent-sdk-';

if (!/^(darwin|linux|win32)$/.test(os) || !/^(x64|arm64)$/.test(cpu)) {
  console.error(`No installer is built for --os ${os} --cpu ${cpu}.`);
  process.exit(1);
}

// No links in `.bin`: nothing runs them, and a bundle should hold files only.
const install = spawnSync('npm', ['ci', '--omit=dev', '--no-bin-links', '--no-audit', '--no-fund', `--os=${os}`, `--cpu=${cpu}`], {
  cwd: sidecar,
  stdio: 'inherit',
  // npm is `npm.cmd` on Windows, which only a shell starts.
  shell: process.platform === 'win32',
});

if (install.status !== 0) {
  console.error('npm ci failed in apps/desktop/sidecar.');
  process.exit(install.status ?? 1);
}

const wanted = `${native}${os}-${cpu}`;

for (const name of readdirSync(scope)) {
  if (name.startsWith(native) && name !== wanted) {
    console.log(`Removing ${name}: not the program for ${os}-${cpu}.`);
    rmSync(join(scope, name), { recursive: true, force: true });
  }
}

const program = join(scope, wanted, os === 'win32' ? 'claude.exe' : 'claude');

if (!existsSync(program) || statSync(program).size === 0) {
  console.error(`::error::${program} is missing: the installer would not be able to run an agent. Is ${wanted} still an optional dependency of the SDK?`);
  process.exit(1);
}

const weigh = (folder) => {
  let bytes = 0;
  let files = 0;

  for (const entry of readdirSync(folder, { withFileTypes: true })) {
    const path = join(folder, entry.name);

    if (entry.isDirectory()) {
      const inside = weigh(path);

      bytes += inside.bytes;
      files += inside.files;
    } else {
      bytes += statSync(path).size;
      files += 1;
    }
  }

  return { bytes, files };
};

const megabytes = (bytes) => `${(bytes / 1024 / 1024).toFixed(0)} MB`;
const all = weigh(join(sidecar, 'node_modules'));

console.log(`sidecar/node_modules for ${os}-${cpu}: ${all.files} files, ${megabytes(all.bytes)}, of which ${megabytes(statSync(program).size)} is ${wanted}.`);
