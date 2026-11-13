import { useEffect, useState } from 'react';

import { api } from '@/api.ts';
import type { WikiSpace } from '@/bindings/WikiSpace.ts';
import type { WikiTreeNode } from '@/bindings/WikiTreeNode.ts';
import { Icon } from '@/components/icon.tsx';
import { Button, Empty, ErrorNote, Picker, Spinner, Tag } from '@/components/ui.tsx';
import { t } from '@/i18n/index.ts';
import { Link, navigate, useSearchParams } from '@/router.tsx';
import { useRemote } from '@/session.tsx';

import { NewPageDialog } from './dialogs.tsx';
import { BASE, fold, pagePath } from './lib.ts';

type Match = { spaceKey: string; spaceName: string; pageId: number; title: string };
type Lookup = { title: string; space: WikiSpace | null; spaces: WikiSpace[]; matches: Match[] };

/** Titles compare without case, diacritics, or the spacing around a colon. */
function loose(title: string): string {
  return fold(title).replace(/\s*:\s*/g, ':');
}

async function lookUp(title: string, spaceKey: string | null): Promise<Lookup> {
  const spaces = await api.get<WikiSpace[]>(`${BASE}/spaces`);
  const space = spaceKey ? (spaces.find((candidate) => candidate.key === spaceKey.toUpperCase()) ?? null) : null;
  // `[[Runbook: deploy]]` can arrive split at its colon; without a space of that name the title is whole.
  const wanted = spaceKey && !space ? `${spaceKey}: ${title}` : title;
  const searched = space ? [space] : spaces;
  const trees = await Promise.all(searched.map((candidate) => api.get<WikiTreeNode[]>(`${BASE}/spaces/${candidate.key}/tree`)));
  const matches: Match[] = [];

  searched.forEach((candidate, index) => {
    const found = trees[index].filter((node) => loose(node.title) === loose(wanted)).sort((left, right) => left.id - right.id)[0];

    if (found) {
      matches.push({ spaceKey: candidate.key, spaceName: candidate.name, pageId: found.id, title: found.title });
    }
  });

  return { title: wanted, space, spaces, matches };
}

/** Where `[[Title]]` links land: opens the page of that title, or offers to create it. */
export function GoView() {
  const params = useSearchParams();
  const title = (params.get('title') ?? '').trim();
  const spaceKey = params.get('space');
  const found = useRemote(() => lookUp(title, spaceKey), [title, spaceKey]);
  const [target, setTarget] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const only = found.data?.matches.length === 1 ? found.data.matches[0] : null;

  useEffect(() => {
    if (only) {
      navigate(pagePath(only.spaceKey, only.pageId), { replace: true });
    }
  }, [only]);

  if (found.error && !found.data) {
    return <ErrorNote message={found.error} />;
  }

  if (!found.data || only) {
    return <Spinner />;
  }

  const { matches, spaces, space } = found.data;
  const writable = spaces.filter((candidate) => candidate.canEdit);
  const chosen = target ?? (space?.canEdit ? space.key : (writable[0]?.key ?? null));

  if (matches.length > 1) {
    return (
      <div className="view view-narrow wiki">
        <header className="view-head">
          <div>
            <h1>{t('Several pages are titled “{title}”', { title: found.data.title })}</h1>
            <p className="muted">{t('The link does not say which space it means.')}</p>
          </div>
        </header>
        <section className="panel">
          {matches.map((match) => (
            <Link key={match.pageId} href={pagePath(match.spaceKey, match.pageId)} className="wiki-hit">
              <span className="wiki-hit-title">
                <Icon name="page" size={14} />
                <strong>{match.title}</strong>
                <Tag>{match.spaceKey}</Tag>
                <span className="muted">{match.spaceName}</span>
              </span>
            </Link>
          ))}
        </section>
      </div>
    );
  }

  return (
    <div className="view view-narrow wiki">
      <div className="panel">
        <Empty icon="page" title={found.data.title ? t('There is no page titled “{title}” yet', { title: found.data.title }) : t('The link names no page')}>
          {space ? t('It was looked for in the space {name}.', { name: space.name }) : t('It was looked for in every space you can see.')}
          {found.data.title && writable.length > 0 && chosen && (
            <span className="wiki-go-create">
              <Picker value={chosen} options={writable.map((candidate) => ({ value: candidate.key, label: `${candidate.name} (${candidate.key})` }))} onChange={setTarget} label={t('Space')} />
              <Button variant="primary" icon="plus" onClick={() => setCreating(true)}>
                {t('Create this page')}
              </Button>
            </span>
          )}
          <p>
            <Link href="/wiki">{t('All spaces')}</Link>
          </p>
        </Empty>
      </div>
      {creating && chosen && <NewPageDialog spaceKey={chosen} parent={null} title={found.data.title} onClose={() => setCreating(false)} />}
    </div>
  );
}
