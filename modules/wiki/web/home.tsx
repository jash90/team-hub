import { useEffect, useState } from 'react';

import { api, query } from '@/api.ts';
import type { WikiSearchHit } from '@/bindings/WikiSearchHit.ts';
import { Icon } from '@/components/icon.tsx';
import { Button, Empty, ErrorNote, Spinner, Tag } from '@/components/ui.tsx';
import { countOf, t } from '@/i18n/index.ts';
import { Link, navigate, setParam, useSearchParams } from '@/router.tsx';
import { useRemote, useSession } from '@/session.tsx';

import { NewSpaceDialog } from './dialogs.tsx';
import { BASE, pagePath, useSpaces } from './lib.ts';
import { Snippet } from './parts.tsx';

/** The knowledge base: search over all pages, and the spaces. */
export function HomeView() {
  const { boot } = useSession();
  const params = useSearchParams();
  const spaces = useSpaces();
  const wanted = params.get('q') ?? '';
  const inSpace = params.get('space');
  const projectKey = params.get('project')?.toUpperCase() ?? null;
  const project = boot.projects.find((candidate) => candidate.key === projectKey);
  const [text, setText] = useState(wanted);
  const [creating, setCreating] = useState<{ forProject: { key: string; name: string } | null } | null>(null);
  const hits = useRemote(() => (wanted.trim() ? api.get<WikiSearchHit[]>(`${BASE}/search${query({ q: wanted, space: inSpace })}`) : Promise.resolve(null)), [wanted, inSpace]);
  const bound = projectKey ? spaces.data?.find((space) => space.projectKey === projectKey) : undefined;

  // "Knowledge" in a project's navigation lands here; a project with a space goes straight to it.
  useEffect(() => {
    if (bound) {
      navigate(`/wiki/${bound.key}`, { replace: true });
    }
  }, [bound]);

  useEffect(() => setText(wanted), [wanted]);

  useEffect(() => {
    if (text.trim() === wanted.trim()) {
      return undefined;
    }

    const timer = window.setTimeout(() => setParam('q', text.trim() || null, { replace: true }), 250);

    return () => window.clearTimeout(timer);
  }, [text, wanted]);

  if (spaces.error && !spaces.data) {
    return <ErrorNote message={spaces.error} />;
  }

  if (!spaces.data || bound) {
    return <Spinner />;
  }

  return (
    <div className="view wiki">
      <header className="view-head">
        <div>
          <h1>{t('Knowledge base')}</h1>
          <p className="muted">{t('Pages your team writes and keeps: decisions, notes, runbooks. They link to the issues they talk about.')}</p>
        </div>
        <Button variant="primary" icon="plus" onClick={() => setCreating({ forProject: null })}>
          {t('New space')}
        </Button>
      </header>

      {project && (
        <div className="panel wiki-offer">
          <Icon name="book" size={20} />
          <span>{t('The project {name} has no knowledge base space yet.', { name: project.name })}</span>
          <Button variant="primary" onClick={() => setCreating({ forProject: { key: project.key, name: project.name } })}>
            {t('Create a space for {key}', { key: project.key })}
          </Button>
        </div>
      )}

      <div className="wiki-search" role="search">
        <div className="filter-search wiki-search-box">
          <Icon name="search" />
          <input className="input" type="search" placeholder={t('Search all pages')} aria-label={t('Search all pages')} value={text} onChange={(event) => setText(event.target.value)} />
        </div>
        {inSpace && (
          <span className="tag tag-blue wiki-search-scope">
            {t('in the space {key}', { key: inSpace.toUpperCase() })}
            <button type="button" className="link-button" aria-label={t('Search all spaces')} title={t('Search all spaces')} onClick={() => setParam('space', null, { replace: true })}>
              <Icon name="x" size={12} />
            </button>
          </span>
        )}
        <span className="muted">{t('Every word must match; case and Polish letters do not matter.')}</span>
      </div>

      {wanted.trim() && (
        <section className="panel">
          <header className="panel-head">
            <h2>{t('Pages matching “{text}”', { text: wanted.trim() })}</h2>
            {hits.data && <span className="tab-count">{hits.data.length}</span>}
          </header>
          {hits.error && <ErrorNote message={hits.error} />}
          {!hits.data && !hits.error && <Spinner />}
          {hits.data?.length === 0 && <p className="muted panel-empty">{t('No page matches.')}</p>}
          {hits.data?.map((hit) => (
            <Link key={hit.pageId} href={pagePath(hit.spaceKey, hit.pageId)} className="wiki-hit">
              <span className="wiki-hit-title">
                <Icon name="page" size={14} />
                <strong>{hit.title}</strong>
                <Tag>{hit.spaceKey}</Tag>
              </span>
              <Snippet text={hit.snippet} />
            </Link>
          ))}
        </section>
      )}

      {spaces.data.length === 0 ? (
        <Empty icon="book" title={t('No spaces yet')}>
          {t('A space holds the pages of one project or one subject.')}
        </Empty>
      ) : (
        <div className="project-grid">
          {spaces.data.map((space) => (
            <Link key={space.id} href={`/wiki/${space.key}`} className="project-card">
              <span className="project-card-head">
                <span className="project-badge">{space.key}</span>
                <strong>{space.name}</strong>
              </span>
              <span className="muted project-card-text">{space.description}</span>
              <span className="project-card-foot">
                {space.projectKey ? <Tag title={t('The project this space belongs to')}>{t('project {key}', { key: space.projectKey })}</Tag> : <Tag>{t('for everybody')}</Tag>}
                {space.source === 'confluence' && <Tag tone="blue">Confluence</Tag>}
                <span className="filter-spacer" />
                <span className="muted">{countOf(space.pageCount, 'page', 'pages')}</span>
              </span>
            </Link>
          ))}
        </div>
      )}

      {creating && <NewSpaceDialog forProject={creating.forProject} onClose={() => setCreating(null)} />}
    </div>
  );
}
