#!/usr/bin/env node
// One version for a release of the desktop app, in every file that states it.
//
//   node apps/desktop/scripts/check-version.mjs              are the files consistent?
//   node apps/desktop/scripts/check-version.mjs 0.1.0        … and is that the version?
//   node apps/desktop/scripts/check-version.mjs desktop-v0.1.0     (a tag's name is fine too)
//
// Prints the version on the last line and exits 0, or says what differs and exits 1.
// The release workflow runs it before anything is built.
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const desktop = join(dirname(fileURLToPath(import.meta.url)), '..');
const read = (file) => readFileSync(join(desktop, file), 'utf8');
const json = (file) => JSON.parse(read(file));

/** `version = "…"` of the `[package]` table of a Cargo manifest. */
const cargoPackageVersion = (file) => {
  const table = read(file).split(/^\[/m).find((part) => part.startsWith('package]'));

  return table?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
};

/** The version a Cargo lock file records for one package. */
const lockedVersion = (file, name) => {
  const entry = read(file).split('[[package]]').find((part) => new RegExp(`^\\s*name = "${name}"$`, 'm').test(part));

  return entry?.match(/^version = "([^"]+)"/m)?.[1];
};

const stated = [
  ['src-tauri/tauri.conf.json', json('src-tauri/tauri.conf.json').version],
  ['src-tauri/Cargo.toml', cargoPackageVersion('src-tauri/Cargo.toml')],
  ['src-tauri/Cargo.lock (team-hub-desktop)', lockedVersion('src-tauri/Cargo.lock', 'team-hub-desktop')],
  ['package.json', json('package.json').version],
  ['package-lock.json', json('package-lock.json').packages[''].version],
  ['ui/package.json', json('ui/package.json').version],
  ['ui/package-lock.json', json('ui/package-lock.json').packages[''].version],
  ['sidecar/package.json', json('sidecar/package.json').version],
  ['sidecar/package-lock.json', json('sidecar/package-lock.json').packages[''].version],
];

const asked = process.argv[2]?.replace(/^desktop-v/, '').replace(/^v/, '');
const version = asked || stated[0][1];
const problems = [];

if (!/^\d+\.\d+\.\d+$/.test(version ?? '')) {
  problems.push(`"${version}" is not a version of the form 1.2.3 (a tag is desktop-v1.2.3)`);
}

for (const [file, found] of stated) {
  const mark = found === version ? 'ok ' : 'NOT';

  console.log(`${mark}  ${String(found).padEnd(10)} apps/desktop/${file}`);

  if (found !== version) {
    problems.push(`apps/desktop/${file} says ${found}, not ${version}`);
  }
}

// The notes become the body of the release, so they have to be about this version.
const heading = read('RELEASE_NOTES.md').split('\n').find((line) => line.startsWith('# ')) ?? '';

if (!heading.split(/\s+/).includes(version)) {
  problems.push(`apps/desktop/RELEASE_NOTES.md is headed "${heading}", which does not name ${version}: write the notes of this release`);
}

if (problems.length > 0) {
  for (const problem of problems) {
    console.error(`::error::${problem}`);
  }

  process.exit(1);
}

console.log(version);
