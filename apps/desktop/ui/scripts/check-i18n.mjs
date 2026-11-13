// Lists the phrases of the desktop interface that have no Polish yet, and the entries of its
// dictionary that nothing uses. `npm run i18n`; exits with 1 when something is missing.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const repository = join(root, '../../..');
const { pl, plPlurals } = await import(join(repository, 'web/src/i18n/pl.ts'));
const { polish: github } = await import(join(repository, 'modules/github/web/pl.ts'));
const { polish: agents } = await import(join(repository, 'modules/agents/web/pl.ts'));
const { polish: own } = await import(join(root, 'src/pl.ts'));

const files = (folder) =>
  readdirSync(folder).flatMap((name) => {
    const path = join(folder, name);

    if (statSync(path).isDirectory()) {
      return name === 'bindings' ? [] : files(path);
    }

    return /\.tsx?$/.test(name) && name !== 'pl.ts' ? [path] : [];
  });

const unescape = (text) => text.replace(/\\(['"\\])/g, '$1').replace(/\\n/g, '\n');
const used = new Set();
const plurals = new Set();
// t('…'), and phrases kept in tables for t() to read later: label, hint, and the *_TEXT maps.
const patterns = [/\bt\(\s*'((?:[^'\\]|\\.)*)'/g, /\bt\(\s*"((?:[^"\\]|\\.)*)"/g, /\b(?:label|hint): '((?:[^'\\]|\\.)*)'/g, /^\s+[a-z_]+: '((?:[^'\\]|\\.)*)',?$/gm];

for (const file of files(join(root, 'src'))) {
  const source = readFileSync(file, 'utf8');

  for (const pattern of patterns) {
    for (const match of source.matchAll(pattern)) {
      used.add(unescape(match[1]));
    }
  }

  for (const match of source.matchAll(/countOf\([^,]+,\s*'([^']+)'/g)) {
    plurals.add(match[1]);
  }
}

const known = (phrase) => phrase in pl || phrase in github || phrase in agents || phrase in own;
const missing = [...used].filter((phrase) => !known(phrase)).sort();
const missingPlurals = [...plurals].filter((word) => !(word in plPlurals));
const unused = Object.keys(own).filter((phrase) => !used.has(phrase));
const shadowed = Object.keys(own).filter((phrase) => phrase in pl);

console.log(`${used.size} phrases in the interface, ${Object.keys(own).length} in the desktop dictionary.`);
missing.forEach((phrase) => console.log(`  no Polish: ${JSON.stringify(phrase)}`));
missingPlurals.forEach((word) => console.log(`  no Polish plural forms (web/src/i18n/pl.ts): ${word}`));
unused.forEach((phrase) => console.log(`  not used: ${JSON.stringify(phrase)}`));
shadowed.forEach((phrase) => console.log(`  already in the web dictionary (which wins): ${JSON.stringify(phrase)}`));
process.exit(missing.length + missingPlurals.length > 0 ? 1 : 0);
