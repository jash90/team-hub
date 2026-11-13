import { useEffect, useState } from 'react';

import { api } from '@/api.ts';
import type { DiffLine } from '@/bindings/DiffLine.ts';
import type { Project } from '@/bindings/Project.ts';
import type { WikiSpace } from '@/bindings/WikiSpace.ts';
import type { WikiTreeNode } from '@/bindings/WikiTreeNode.ts';
import { useRemote, type LiveEvent, type Remote } from '@/session.tsx';

export const BASE = '/api/m/wiki';

/** Lower case, no diacritics, single spaces: how titles are compared, here and on the server. */
export function fold(text: string): string {
  return text
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/ł/g, 'l')
    .replace(/Ł/g, 'L')
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .join(' ');
}

export function pagePath(spaceKey: string, pageId: number, mode?: 'edit' | 'history'): string {
  return `/wiki/${spaceKey}/${pageId}${mode ? `/${mode}` : ''}`;
}

/** Where a `[[link]]` goes when the page it names still has to be looked up. */
export function goPath(title: string, spaceKey?: string | null): string {
  const encode = (text: string) => encodeURIComponent(text).replace(/\(/g, '%28').replace(/\)/g, '%29');

  return `/wiki-go?title=${encode(title)}${spaceKey ? `&space=${encode(spaceKey)}` : ''}`;
}

/** A change to a page of the given space, as announced by the server. */
export function isPageEvent(event: LiveEvent, spaceKey: string): boolean {
  return event.kind.startsWith('page.') && event.payload.spaceKey === spaceKey;
}

// The project navigation is built without waiting for anything, so the spaces bound to
// projects are remembered from the last time the list was loaded.
const boundSpaces = new Map<string, string>();

export function spaceOfProject(projectKey: string): string | undefined {
  return boundSpaces.get(projectKey);
}

export function useSpaces(): Remote<WikiSpace[]> {
  return useRemote(
    async () => {
      const spaces = await api.get<WikiSpace[]>(`${BASE}/spaces`);
      boundSpaces.clear();

      for (const space of spaces) {
        if (space.projectKey) {
          boundSpaces.set(space.projectKey, space.key);
        }
      }

      return spaces;
    },
    [],
    (event) => event.kind === 'page.created' || event.kind === 'page.deleted' || event.kind === 'confluence.imported',
  );
}

const SPACE_KEY = /^[A-Z][A-Z0-9]{1,9}$/;
const PROTECTED = /(```[\s\S]*?```|`[^`\n]*`|\[[^\]]*\]\([^)]*\))/g;

/**
 * Turns `[[Title]]`, `[[SPACE:Title]]` and `[[Title|shown text]]` into Markdown links, the way
 * the server reads them: a page of this space unless the prefix is the key of another space.
 * Pages of this space that exist are linked directly; the rest goes through the lookup.
 */
export function linkWikiLinks(body: string, spaceKey: string, titles: Map<string, number>, spaceKeys: Set<string>): string {
  return body
    .split(PROTECTED)
    .map((part, index) => {
      if (index % 2 === 1) {
        return part;
      }

      return part.replace(/\[\[([^[\]\n]+?)\]\]/g, (whole, inner: string) => {
        const [target, ...shown] = inner.split('|');
        const named = target.trim();

        if (!named) {
          return whole;
        }

        const colon = named.indexOf(':');
        const prefix = colon > 0 ? named.slice(0, colon).trim() : '';
        const rest = colon > 0 ? named.slice(colon + 1).trim() : '';
        const elsewhere = SPACE_KEY.test(prefix) && rest !== '' && spaceKeys.has(prefix);
        const title = elsewhere ? rest : named;
        const space = elsewhere ? prefix : spaceKey;
        const known = space === spaceKey ? titles.get(fold(title)) : undefined;
        const label = (shown.join('|').trim() || title).replace(/([\\*_`<[\]])/g, '\\$1');

        return `[${label}](${known !== undefined ? pagePath(spaceKey, known) : goPath(title, space)})`;
      });
    })
    .join('');
}

export function titleIndex(nodes: WikiTreeNode[]): Map<string, number> {
  const titles = new Map<string, number>();

  // Of two pages with one title the older one is the link target, as on the server.
  for (const node of [...nodes].sort((left, right) => right.id - left.id)) {
    titles.set(fold(node.title), node.id);
  }

  return titles;
}

export type TaskLine = { index: number; text: string };

/** The open task lines of a body (`- [ ] text`) that are not issues yet, with their line numbers. */
export function openTasks(body: string, projectKeys: string[]): TaskLine[] {
  const linked = projectKeys.length > 0 ? new RegExp(`^(${projectKeys.join('|')})-\\d+(?![A-Za-z0-9])`) : null;
  const tasks: TaskLine[] = [];

  body.split('\n').forEach((line, index) => {
    const found = /^\s*(?:[-*+]|\d+[.)])\s+\[ \]\s+(\S.*?)\s*$/.exec(line);

    if (found && !linked?.test(found[1])) {
      tasks.push({ index, text: found[1] });
    }
  });

  return tasks;
}

const MAX_CELLS = 2_000_000;

/** A line diff of two texts, in the shape the server uses for versions. */
export function diffLines(before: string, after: string): DiffLine[] {
  const old = before.split('\n');
  const now = after.split('\n');
  const rows: DiffLine[] = [];

  if (old.length * now.length > MAX_CELLS) {
    return [...old.map((text) => ({ kind: 'removed' as const, text })), ...now.map((text) => ({ kind: 'added' as const, text }))];
  }

  const width = now.length + 1;
  const table = new Uint32Array((old.length + 1) * width);

  for (let row = old.length - 1; row >= 0; row -= 1) {
    for (let column = now.length - 1; column >= 0; column -= 1) {
      table[row * width + column] =
        old[row] === now[column] ? table[(row + 1) * width + column + 1] + 1 : Math.max(table[(row + 1) * width + column], table[row * width + column + 1]);
    }
  }

  let row = 0;
  let column = 0;

  while (row < old.length && column < now.length) {
    if (old[row] === now[column]) {
      rows.push({ kind: 'same', text: old[row] });
      row += 1;
      column += 1;
    } else if (table[(row + 1) * width + column] >= table[row * width + column + 1]) {
      rows.push({ kind: 'removed', text: old[row] });
      row += 1;
    } else {
      rows.push({ kind: 'added', text: now[column] });
      column += 1;
    }
  }

  for (; row < old.length; row += 1) {
    rows.push({ kind: 'removed', text: old[row] });
  }

  for (; column < now.length; column += 1) {
    rows.push({ kind: 'added', text: now[column] });
  }

  return rows;
}

const projects = new Map<string, Promise<Project>>();

function loadProject(key: string): Promise<Project> {
  let loading = projects.get(key);

  if (!loading) {
    loading = api.get<Project>(`/api/projects/${encodeURIComponent(key)}`);
    loading.catch(() => projects.delete(key));
    projects.set(key, loading);
  }

  return loading;
}

/** The configuration of the given projects (statuses, types), for showing their issues. */
export function useProjects(keys: string[]): Record<string, Project> {
  const [loaded, setLoaded] = useState<Record<string, Project>>({});
  const wanted = [...new Set(keys)].sort().join(',');

  useEffect(() => {
    let current = true;

    for (const key of wanted ? wanted.split(',') : []) {
      loadProject(key)
        .then((project) => current && setLoaded((before) => (before[key] === project ? before : { ...before, [key]: project })))
        .catch(() => undefined);
    }

    return () => {
      current = false;
    };
  }, [wanted]);

  return loaded;
}

/** Text in which the server marked matches with `<mark>`…`</mark>`, as alternating plain and marked pieces. */
export function splitMarks(snippet: string): { text: string; marked: boolean }[] {
  const pieces: { text: string; marked: boolean }[] = [];

  snippet.split('<mark>').forEach((part, index) => {
    if (index === 0) {
      pieces.push({ text: part, marked: false });

      return;
    }

    const end = part.indexOf('</mark>');

    if (end < 0) {
      pieces.push({ text: part, marked: true });
    } else {
      pieces.push({ text: part.slice(0, end), marked: true }, { text: part.slice(end + '</mark>'.length), marked: false });
    }
  });

  return pieces.filter((piece) => piece.text !== '');
}
