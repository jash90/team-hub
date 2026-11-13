/**
 * Holds a thread's file tools to the folders its allow list names.
 *
 * The allow list is the whole of a thread's permissions, and the Agent SDK applies it. Two
 * things the SDK does not do are done here, for threads nobody sits beside (a headless
 * runner scopes `Read` to the working tree; the desktop app leaves it bare):
 *
 * - `Glob` and `Grep` take a folder of their own. They follow the folders `Read` is scoped
 *   to, checked before the tool runs, with links resolved.
 * - A prompt can name a file as `@path`, and the SDK then reads it into the conversation
 *   without any tool call or permission check. Text from the tracker is part of the prompt,
 *   so every such mention is broken before the prompt is handed over.
 */
import { lstatSync, realpathSync } from 'node:fs';
import { basename, dirname, isAbsolute, join, resolve, sep } from 'node:path';

import type { Options } from '@anthropic-ai/claude-agent-sdk';

const READERS = new Set(['Read', 'Glob', 'Grep']);
const WRITERS = new Set(['Edit', 'Write', 'NotebookEdit']);
const GLOB_CHARACTERS = /[*?[\]{}!]/;
const ZERO_WIDTH_SPACE = '​';

type Input = Record<string, unknown>;

/** `Read(//data/tree/**)` names `/data/tree`; a rule in any other form names no folder. */
const folderOf = (rule: string, tool: string) => {
  const match = /^(\w+)\(\/(\/.*)\/\*\*\)$/.exec(rule);

  return match && match[1] === tool ? match[2] : undefined;
};

const foldersOf = (allowed: string[], tool: string) => allowed.flatMap((rule) => folderOf(rule, tool) ?? []);

/** Whether the allow list ties reading to folders: no bare `Read` on it. */
export const readsAreScoped = (allowed: string[]) => !allowed.includes('Read');

/**
 * Where a path really is, links resolved. A file that does not exist yet is placed by its
 * nearest folder that does; a link that leads nowhere has no place at all, because writing
 * through it would create its target.
 */
const realPlace = (path: string): string | undefined => {
  const missing: string[] = [];
  let existing = path;

  for (;;) {
    try {
      return join(realpathSync(existing), ...missing);
    } catch {
      if (lstatSync(existing, { throwIfNoEntry: false })?.isSymbolicLink()) {
        return undefined;
      }

      const parent = dirname(existing);

      if (parent === existing) {
        return undefined;
      }

      missing.unshift(basename(existing));
      existing = parent;
    }
  }
};

const isInside = (folder: string, path: string) => path === folder || path.startsWith(folder.endsWith(sep) ? folder : `${folder}${sep}`);

/** The folder a glob pattern starts from: its segments before the first wildcard. */
const fixedPart = (pattern: string) => {
  const segments = pattern.split('/');
  const wild = segments.findIndex((segment) => GLOB_CHARACTERS.test(segment));

  return segments.slice(0, wild === -1 ? Math.max(segments.length - 1, 0) : wild).join('/') || (pattern.startsWith('/') ? '/' : '.');
};

/** The paths a file tool would touch, or none when it names them in a form that cannot be judged. */
const pathsOf = (tool: string, input: Input): string[] | undefined => {
  const text = (name: string) => (typeof input[name] === 'string' && input[name] !== '' ? (input[name] as string) : undefined);

  switch (tool) {
    case 'Read':
    case 'Edit':
    case 'Write': {
      const file = text('file_path');

      return file ? [file] : undefined;
    }
    case 'NotebookEdit': {
      const notebook = text('notebook_path');

      return notebook ? [notebook] : undefined;
    }
    case 'Grep':
      return [text('path') ?? '.'];
    case 'Glob': {
      const pattern = text('pattern') ?? '';

      if (pattern.split('/').includes('..')) {
        return undefined;
      }

      const base = text('path') ?? '.';

      return [base, isAbsolute(pattern) ? fixedPart(pattern) : join(base, fixedPart(pattern))];
    }
    default:
      return [];
  }
};

/**
 * Why a call of a file tool is refused, or nothing when it stays inside the folders the
 * allow list names for it. Tools that touch no files are not judged here.
 */
export const refusal = (tool: string, input: unknown, cwd: string, allowed: string[]): string | undefined => {
  if (!READERS.has(tool) && !WRITERS.has(tool)) {
    return undefined;
  }

  // Edit rules cover every tool that changes files, Read rules every tool that reads them.
  const rules = WRITERS.has(tool) ? [...foldersOf(allowed, 'Edit'), ...foldersOf(allowed, 'Write')] : foldersOf(allowed, 'Read');
  const folders = rules.flatMap((folder) => realPlace(folder) ?? []);
  const paths = pathsOf(tool, (input ?? {}) as Input);

  if (!paths) {
    return `${tool} was called without a path that can be checked against this thread's working tree.`;
  }

  for (const path of paths) {
    const place = realPlace(resolve(cwd, path));

    if (!place || !folders.some((folder) => isInside(folder, place))) {
      return `${path} is outside this thread's working tree. ${tool} works only inside it; nobody can approve anything else here.`;
    }
  }

  return undefined;
};

/** The hook that applies `refusal` before every tool call; none when reading is not scoped. */
export const confinement = (cwd: string, allowed: string[]): Options['hooks'] | undefined => {
  if (!readsAreScoped(allowed)) {
    return undefined;
  }

  return {
    PreToolUse: [
      {
        // A check that fails must not let the call through.
        onFailure: 'block',
        hooks: [
          async (input) => {
            const reason = input.hook_event_name === 'PreToolUse' ? refusal(input.tool_name, input.tool_input, cwd, allowed) : undefined;

            return reason ? { hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'deny', permissionDecisionReason: reason } } : {};
          },
        ],
      },
    ],
  };
};

/**
 * The prompt with every `@path` broken, so the SDK attaches no file on its word. The text
 * reads the same; an invisible space follows each `@` that touches a word.
 */
export const withoutMentions = (prompt: string) => prompt.replace(/@(?=\S)/g, `@${ZERO_WIDTH_SPACE}`);
