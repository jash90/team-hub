import { useMemo, useState } from 'react';

import type { DiffLine } from '@/bindings/DiffLine.ts';
import type { WikiRevisionInfo } from '@/bindings/WikiRevisionInfo.ts';
import { Icon } from '@/components/icon.tsx';
import { Markdown } from '@/components/markdown.tsx';
import { t } from '@/i18n/index.ts';
import { useSession } from '@/session.tsx';

import { linkWikiLinks, splitMarks } from './lib.ts';

const CONTEXT = 3;

type Segment = { kind: 'lines'; lines: DiffLine[] } | { kind: 'gap'; id: number; lines: DiffLine[] };

/** Splits a diff into what changed with a few lines around it, and the long unchanged stretches between. */
function segments(lines: DiffLine[]): Segment[] {
  const keep = lines.map(() => false);

  lines.forEach((line, index) => {
    if (line.kind !== 'same') {
      for (let near = Math.max(0, index - CONTEXT); near <= Math.min(lines.length - 1, index + CONTEXT); near += 1) {
        keep[near] = true;
      }
    }
  });

  const found: Segment[] = [];

  lines.forEach((line, index) => {
    const last = found[found.length - 1];
    const kind = keep[index] ? 'lines' : 'gap';

    if (last && last.kind === kind) {
      last.lines.push(line);
    } else {
      found.push(kind === 'lines' ? { kind, lines: [line] } : { kind, id: index, lines: [line] });
    }
  });

  // A gap of a line or two is shorter shown than folded.
  return found.map((segment) => (segment.kind === 'gap' && segment.lines.length <= 2 ? { kind: 'lines', lines: segment.lines } : segment));
}

const MARKS = { same: ' ', added: '+', removed: '−' } as const;

/** A line diff. What was added or removed is said by the sign in the gutter as well as by colour. */
export function DiffView({ lines }: { lines: DiffLine[] }) {
  const [open, setOpen] = useState<number[]>([]);
  const parts = useMemo(() => segments(lines), [lines]);
  const added = lines.filter((line) => line.kind === 'added').length;
  const removed = lines.filter((line) => line.kind === 'removed').length;

  if (added === 0 && removed === 0) {
    return <p className="muted wiki-diff-none">{t('The text is the same in both.')}</p>;
  }

  return (
    <div className="wiki-diff">
      <div className="wiki-diff-summary muted">
        <span className="wiki-diff-count-added">{t('+{count} added', { count: added })}</span>
        <span className="wiki-diff-count-removed">{t('−{count} removed', { count: removed })}</span>
      </div>
      <div className="wiki-diff-lines" role="table" aria-label={t('Differences')}>
        {parts.map((part, index) =>
          part.kind === 'gap' && !open.includes(part.id) ? (
            <button key={`gap-${part.id}`} type="button" className="wiki-diff-gap" onClick={() => setOpen([...open, part.id])}>
              {t('{count} unchanged lines', { count: part.lines.length })}
            </button>
          ) : (
            part.lines.map((line, offset) => (
              <div key={`${index}-${offset}`} className={`wiki-diff-row wiki-diff-${line.kind}`} role="row">
                <span className="wiki-diff-mark" role="cell" aria-label={line.kind === 'added' ? t('added') : line.kind === 'removed' ? t('removed') : undefined}>
                  {MARKS[line.kind]}
                </span>
                <span className="wiki-diff-text" role="cell">
                  {line.text || ' '}
                </span>
              </div>
            ))
          ),
        )}
      </div>
    </div>
  );
}

/** A page body: Markdown with `[[links]]` resolved against the pages of its space. */
export function PageBody({ body, spaceKey, titles, spaceKeys }: { body: string; spaceKey: string; titles: Map<string, number>; spaceKeys: Set<string> }) {
  const source = useMemo(() => linkWikiLinks(body, spaceKey, titles, spaceKeys), [body, spaceKey, titles, spaceKeys]);

  if (!body.trim()) {
    return <p className="muted">{t('This page is empty.')}</p>;
  }

  return <Markdown text={source} />;
}

/** Who saved a version: a person, or the system it came from. */
export function useAuthorName(): (authorId: number | null, authorLabel?: string | null) => string {
  const { userById } = useSession();

  return (authorId, authorLabel) => userById(authorId)?.name ?? authorLabel ?? t('Someone');
}

export function revisionAuthor(name: (authorId: number | null, authorLabel?: string | null) => string, revision: WikiRevisionInfo): string {
  return name(revision.authorId, revision.authorLabel);
}

/** A search excerpt with the matched words marked; the text is never treated as HTML. */
export function Snippet({ text }: { text: string }) {
  return (
    <span className="wiki-snippet">
      {splitMarks(text).map((piece, index) => (piece.marked ? <mark key={index}>{piece.text}</mark> : <span key={index}>{piece.text}</span>))}
    </span>
  );
}

export function LockIcon() {
  return (
    <span className="wiki-lock-icon" title={t('Mirrored from Confluence')}>
      <Icon name="lock" size={12} />
    </span>
  );
}
