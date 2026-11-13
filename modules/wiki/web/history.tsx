import { useState } from 'react';

import { api } from '@/api.ts';
import type { WikiDiff } from '@/bindings/WikiDiff.ts';
import type { WikiPageDetail } from '@/bindings/WikiPageDetail.ts';
import type { WikiRevision } from '@/bindings/WikiRevision.ts';
import type { WikiRevisionInfo } from '@/bindings/WikiRevisionInfo.ts';
import { Icon } from '@/components/icon.tsx';
import { Button, ConfirmDialog, ErrorNote, Picker, Spinner, Tag, toast } from '@/components/ui.tsx';
import { formatDateTime, t, timeAgo } from '@/i18n/index.ts';
import { Link, navigate, useSearchParams } from '@/router.tsx';
import { useRemote } from '@/session.tsx';

import { BASE, pagePath } from './lib.ts';
import { DiffView, PageBody, revisionAuthor, useAuthorName } from './parts.tsx';

type Props = {
  page: WikiPageDetail;
  titles: Map<string, number>;
  spaceKeys: Set<string>;
  onRestored: (page: WikiPageDetail) => void;
};

function versionOf(text: string | null): number | null {
  const version = text === null ? Number.NaN : Number(text);

  return Number.isInteger(version) && version > 0 ? version : null;
}

/** The versions of a page: the list, one version, or what changed between two. */
export function HistoryView({ page, titles, spaceKeys, onRestored }: Props) {
  const params = useSearchParams();
  const revisions = useRemote(() => api.get<WikiRevisionInfo[]>(`${BASE}/pages/${page.id}/revisions`), [page.id, page.version]);
  const [restoring, setRestoring] = useState<number | null>(null);
  const history = pagePath(page.spaceKey, page.id, 'history');
  const viewing = versionOf(params.get('v'));
  const from = versionOf(params.get('from'));
  const to = versionOf(params.get('to'));
  const canRestore = page.space.canEdit && !page.locked;
  const ask = (version: number) => (canRestore && version !== page.version ? () => setRestoring(version) : undefined);

  return (
    <div className="panel wiki-article">
      <header className="wiki-head">
        <h1 className="wiki-title">{t('History of “{title}”', { title: page.title })}</h1>
        <div className="wiki-head-actions">
          {(viewing !== null || (from !== null && to !== null)) && (
            <Button icon="history" onClick={() => navigate(history)}>
              {t('All versions')}
            </Button>
          )}
          <Button onClick={() => navigate(pagePath(page.spaceKey, page.id))}>{t('Back to the page')}</Button>
        </div>
      </header>

      {viewing !== null ? (
        <RevisionView page={page} version={viewing} titles={titles} spaceKeys={spaceKeys} onRestore={ask(viewing)} />
      ) : from !== null && to !== null ? (
        <DiffPane page={page} from={from} to={to} />
      ) : !revisions.data ? (
        revisions.error ? (
          <ErrorNote message={revisions.error} />
        ) : (
          <Spinner />
        )
      ) : (
        <RevisionList page={page} revisions={revisions.data} history={history} onRestore={ask} />
      )}

      {restoring !== null && (
        <ConfirmDialog
          title={t('Restore version {version}?', { version: restoring })}
          body={t('The title and text of version {version} become the newest version of the page. Nothing is lost: the current version stays in the history.', { version: restoring })}
          confirm={t('Restore')}
          onClose={() => setRestoring(null)}
          onConfirm={async () => {
            onRestored(await api.post<WikiPageDetail>(`${BASE}/pages/${page.id}/restore`, { version: restoring }));
            toast(t('Version {version} restored', { version: restoring }));
            navigate(pagePath(page.spaceKey, page.id));
          }}
        />
      )}
    </div>
  );
}

type ListProps = { page: WikiPageDetail; revisions: WikiRevisionInfo[]; history: string; onRestore: (version: number) => (() => void) | undefined };

function RevisionList({ page, revisions, history, onRestore }: ListProps) {
  const authorName = useAuthorName();
  const [older, setOlder] = useState(Math.max(1, page.version - 1));
  const [newer, setNewer] = useState(page.version);
  const options = revisions.map((revision) => ({ value: revision.version, label: t('version {version}', { version: revision.version }) }));

  return (
    <>
      {revisions.length > 1 && (
        <div className="wiki-compare">
          <span>{t('Compare')}</span>
          <Picker value={older} options={options} onChange={setOlder} label={t('Older version')} />
          <span>{t('with')}</span>
          <Picker value={newer} options={options} onChange={setNewer} label={t('Newer version')} />
          <Button icon="eye" disabled={older === newer} onClick={() => navigate(`${history}?from=${Math.min(older, newer)}&to=${Math.max(older, newer)}`)}>
            {t('Show changes')}
          </Button>
        </div>
      )}
      <div className="table-wrap">
        <table className="table wiki-revisions">
          <thead>
            <tr>
              <th>{t('Version')}</th>
              <th>{t('Saved')}</th>
              <th>{t('By')}</th>
              <th>{t('What changed')}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {revisions.map((revision) => {
              const restore = onRestore(revision.version);

              return (
                <tr key={revision.version}>
                  <td className="cell-narrow">
                    <Link href={`${history}?v=${revision.version}`}>{revision.version}</Link> {revision.version === page.version && <Tag tone="green">{t('current')}</Tag>}
                  </td>
                  <td className="cell-date" title={timeAgo(revision.createdAt)}>
                    {formatDateTime(revision.createdAt)}
                  </td>
                  <td>{revisionAuthor(authorName, revision)}</td>
                  <td>
                    {revision.message || <span className="muted">{revision.version === 1 ? t('Page created') : '—'}</span>}
                    {revision.title !== page.title && <span className="muted"> · {t('titled “{title}”', { title: revision.title })}</span>}
                  </td>
                  <td className="cell-actions">
                    <Button size="sm" variant="ghost" onClick={() => navigate(`${history}?v=${revision.version}`)}>
                      {t('View')}
                    </Button>
                    {revision.version > 1 && (
                      <Button size="sm" variant="ghost" onClick={() => navigate(`${history}?from=${revision.version - 1}&to=${revision.version}`)}>
                        {t('Changes')}
                      </Button>
                    )}
                    {restore && (
                      <Button size="sm" variant="ghost" onClick={restore}>
                        {t('Restore')}
                      </Button>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </>
  );
}

type RevisionProps = { page: WikiPageDetail; version: number; titles: Map<string, number>; spaceKeys: Set<string>; onRestore: (() => void) | undefined };

function RevisionView({ page, version, titles, spaceKeys, onRestore }: RevisionProps) {
  const authorName = useAuthorName();
  const revision = useRemote(() => api.get<WikiRevision>(`${BASE}/pages/${page.id}/revisions/${version}`), [page.id, version]);

  if (!revision.data) {
    return revision.error ? <ErrorNote message={revision.error} /> : <Spinner />;
  }

  const data = revision.data;

  return (
    <>
      <div className="wiki-version-note" role="note">
        <Icon name="history" />
        <span>
          {version === page.version ? t('This is the current version, {version}.', { version }) : t('You are looking at version {version}; the current one is {current}.', { version, current: page.version })}{' '}
          {t('Saved by {name} on {when}.', { name: revisionAuthor(authorName, data), when: formatDateTime(data.createdAt) })}
          {data.message && ` “${data.message}”`}
        </span>
        <span className="filter-spacer" />
        {version > 1 && (
          <Button size="sm" onClick={() => navigate(`${pagePath(page.spaceKey, page.id, 'history')}?from=${version - 1}&to=${version}`)}>
            {t('Changes in this version')}
          </Button>
        )}
        {onRestore && (
          <Button size="sm" variant="primary" onClick={onRestore}>
            {t('Restore this version')}
          </Button>
        )}
      </div>
      <h2 className="wiki-version-title">{data.title}</h2>
      <div className="wiki-body">
        <PageBody body={data.body} spaceKey={page.spaceKey} titles={titles} spaceKeys={spaceKeys} />
      </div>
    </>
  );
}

function DiffPane({ page, from, to }: { page: WikiPageDetail; from: number; to: number }) {
  const diff = useRemote(() => api.get<WikiDiff>(`${BASE}/pages/${page.id}/diff?from=${from}&to=${to}`), [page.id, from, to]);
  const history = pagePath(page.spaceKey, page.id, 'history');

  if (!diff.data) {
    return diff.error ? <ErrorNote message={diff.error} /> : <Spinner />;
  }

  return (
    <>
      <p className="wiki-diff-head">
        {t('Changes from')} <Link href={`${history}?v=${from}`}>{t('version {version}', { version: from })}</Link> {t('to')}{' '}
        <Link href={`${history}?v=${to}`}>{t('version {version}', { version: to })}</Link>
      </p>
      {diff.data.fromTitle !== diff.data.toTitle && (
        <p>
          {t('Title')}: <del>{diff.data.fromTitle}</del> → <ins>{diff.data.toTitle}</ins>
        </p>
      )}
      <DiffView lines={diff.data.lines} />
    </>
  );
}
