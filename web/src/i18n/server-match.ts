/**
 * The matching behind `translateServer`: an exact text first, then the patterns. It imports
 * nothing, so that the build's check (`scripts/check-server-messages.mjs`) can run the very
 * code the interface runs.
 */

type Pattern = { key: string; shape: RegExp; names: string[]; polish: string; anchored: boolean; weight: number };

export type ServerTranslator = {
  /** The Polish for `text`; `text` itself when the dictionary does not know it. */
  translate(text: string): string;
  /** The dictionary key that translates `text`; null when none does. */
  source(text: string): string | null;
};

/** A text may hold a text that holds a text ("WEB-1: the agent proposes a comment"); not much deeper. */
const MAX_DEPTH = 4;

/** What `{key}` in a pattern matches: an issue key such as WEB-12. Every other name matches anything. */
export const ISSUE_KEY = '[A-Za-z][A-Za-z0-9_]*-\\d+';

function compile(key: string, polish: string): Pattern {
  // Odd pieces are the names of the placeholders, even ones the words around them.
  const pieces = key.split(/\{(\w+)\}/);
  const words = pieces.filter((_, index) => index % 2 === 0);
  const names = pieces.filter((_, index) => index % 2 === 1);
  const shape = pieces.map((piece, index) => (index % 2 === 0 ? piece.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') : piece === 'key' ? `(${ISSUE_KEY})` : '(.+?)'));

  return {
    key,
    shape: new RegExp(`^${shape.join('')}$`, 's'),
    names,
    polish,
    anchored: words[0] !== '' || names[0] === 'key',
    weight: words.join('').length,
  };
}

/**
 * `exact` maps a whole English text to its Polish. `patterns` maps a text with variable parts,
 * written `{name}`, to Polish that puts them back: `{name}` as it came, `{t:name}` translated
 * in turn (for a part that is itself one of the server's phrases).
 */
export function serverTranslator(exact: Record<string, string>, patterns: Record<string, string>): ServerTranslator {
  // A pattern that says how the text begins (with words, or with an issue key) is tried before
  // one that begins with anything at all; among equals, the one with the most words of its own.
  // So "WEB-1: the agent asks: who commented on WEB-2: …" is the agent asking, not a comment.
  const compiled = Object.entries(patterns)
    .map(([key, polish]) => compile(key, polish))
    .sort((a, b) => Number(b.anchored) - Number(a.anchored) || b.weight - a.weight || a.names.length - b.names.length);

  function find(text: string): { pattern: Pattern; parts: Record<string, string> } | null {
    for (const pattern of compiled) {
      const match = pattern.shape.exec(text);

      if (match) {
        return { pattern, parts: Object.fromEntries(pattern.names.map((name, index) => [name, match[index + 1]])) };
      }
    }

    return null;
  }

  function translate(text: string, depth: number): string {
    if (Object.hasOwn(exact, text)) {
      return exact[text];
    }

    const found = depth < MAX_DEPTH ? find(text) : null;

    if (!found) {
      return text;
    }

    return found.pattern.polish.replace(/\{(t:)?(\w+)\}/g, (whole, nested: string | undefined, name: string) => {
      if (!Object.hasOwn(found.parts, name)) {
        return whole;
      }

      return nested ? translate(found.parts[name], depth + 1) : found.parts[name];
    });
  }

  return {
    translate: (text) => translate(text, 0),
    source: (text) => (Object.hasOwn(exact, text) ? text : (find(text)?.pattern.key ?? null)),
  };
}
