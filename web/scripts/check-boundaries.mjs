// Fails when a file imports across a boundary:
//
//  - a module's screens (`modules/<id>/web/`) import only `react`, the application through
//    `@/…`, and their own files; never another module, never a package of their own;
//  - the application (`web/src/`) reaches into a module's folder only from `src/modules.ts`,
//    the one place where modules are listed;
//  - `src/modules.ts` lists every module that has screens, and fetches each with `import()`,
//    so that a module's screens stay a chunk of their own.
//
// Run by `npm run build`; on its own with `npm run check:boundaries`.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseSync } from 'vite';

const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const src = join(web, 'src');
const modules = resolve(web, '..', 'modules');
const registry = join(src, 'modules.ts');

function inside(path, folder) {
  return path === folder || path.startsWith(folder + sep);
}

function sources(folder) {
  const found = [];

  for (const name of readdirSync(folder)) {
    const path = join(folder, name);

    if (statSync(path).isDirectory()) {
      if (name !== 'node_modules') {
        found.push(...sources(path));
      }
    } else if (/\.(ts|tsx|mts|js|jsx|mjs)$/.test(name)) {
      found.push(path);
    }
  }

  return found;
}

/** Every specifier a file imports from, with the line it is on. */
function imports(path) {
  const text = readFileSync(path, 'utf8');
  const parsed = parseSync(path, text);

  if (parsed.errors.length > 0) {
    throw new Error(`${path}: ${parsed.errors[0].message}`);
  }

  const line = (offset) => text.slice(0, offset).split('\n').length;
  const found = [];

  for (const entry of parsed.module.staticImports) {
    found.push({ specifier: entry.moduleRequest.value, line: line(entry.start), fixed: true });
  }

  for (const statement of parsed.module.staticExports) {
    for (const entry of statement.entries) {
      if (entry.moduleRequest) {
        found.push({ specifier: entry.moduleRequest.value, line: line(statement.start), fixed: true });
      }
    }
  }

  for (const entry of parsed.module.dynamicImports) {
    const literal = /^(['"])(.*)\1$/s.exec(text.slice(entry.moduleRequest.start, entry.moduleRequest.end).trim());

    // What `import()` loads must be readable here, so it has to be a plain string.
    found.push({ specifier: literal ? literal[2] : null, line: line(entry.start) });
  }

  return found;
}

/** The file or folder a specifier points at; null for a package. */
function target(specifier, from) {
  if (specifier.startsWith('@/')) {
    return resolve(src, specifier.slice(2));
  }

  return specifier.startsWith('.') ? resolve(dirname(from), specifier) : null;
}

const problems = [];

function report(path, line, message) {
  problems.push(`${relative(resolve(web, '..'), path)}:${line}  ${message}`);
}

for (const id of readdirSync(modules)) {
  const own = join(modules, id, 'web');

  if (!statSync(join(modules, id)).isDirectory() || !statSync(own, { throwIfNoEntry: false })?.isDirectory()) {
    continue;
  }

  for (const path of sources(own)) {
    for (const { specifier, line } of imports(path)) {
      if (specifier === null) {
        report(path, line, 'import() of something that is not a plain string cannot be checked');
      } else if (specifier === 'react') {
        continue;
      } else if (specifier.startsWith('@/')) {
        if (!inside(target(specifier, path), src)) {
          report(path, line, `"${specifier}" leaves the application's folder`);
        }
      } else if (specifier.startsWith('.')) {
        const to = target(specifier, path);

        if (inside(to, modules) && !inside(to, join(modules, id))) {
          report(path, line, `"${specifier}" reaches into another module (${relative(modules, to).split(sep)[0]}); declare what you need through the host instead`);
        } else if (!inside(to, own)) {
          report(path, line, `"${specifier}" leaves the module's web folder; use "@/…" for the application`);
        }
      } else {
        report(path, line, `"${specifier}": a module's screens import only react, "@/…" and their own files`);
      }
    }
  }
}

for (const path of sources(src)) {
  if (path === registry) {
    continue;
  }

  for (const { specifier, line } of imports(path)) {
    const to = specifier === null ? null : target(specifier, path);

    if (to && inside(to, modules)) {
      report(path, line, `"${specifier}" names a module's folder; only src/modules.ts lists modules, everything else goes through useModules()`);
    }
  }
}

const listed = [];

for (const { specifier, line, fixed } of imports(registry)) {
  const to = specifier === null ? null : target(specifier, registry);

  if (to && inside(to, modules)) {
    listed.push(to);

    if (fixed) {
      report(registry, line, `"${specifier}" is imported with the shell; list the module as \`id: () => import('…')\` so its screens are fetched only when it is switched on`);
    }
  }
}

for (const id of readdirSync(modules)) {
  const screens = join(modules, id, 'web', 'index.tsx');

  if (statSync(screens, { throwIfNoEntry: false })?.isFile() && !listed.includes(screens)) {
    report(screens, 1, `the module has screens that src/modules.ts does not list; add \`${id}: () => import('../../modules/${id}/web/index.tsx'),\` there`);
  }
}

if (problems.length > 0) {
  console.error(`Module boundaries crossed (${problems.length}):\n`);
  console.error(problems.join('\n'));
  console.error('\nThe rules are in docs/modules.md, under "Boundaries".');
  process.exit(1);
}

console.log('module boundaries: ok');
