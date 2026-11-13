import { useEffect, useLayoutEffect, useMemo, useRef, useState, type RefObject } from 'react';

import { api } from '@/api.ts';
import type { WikiPageDetail } from '@/bindings/WikiPageDetail.ts';
import type { WikiSpace } from '@/bindings/WikiSpace.ts';
import type { WikiTreeNode } from '@/bindings/WikiTreeNode.ts';
import { Icon } from '@/components/icon.tsx';
import { IssueLine } from '@/components/issue-bits.tsx';
import { Button, ConfirmDialog, ErrorNote, Menu, Spinner, Tag, attempt, copyText, type MenuItem } from '@/components/ui.tsx';
import { countOf, formatDateTime, t, timeAgo } from '@/i18n/index.ts';
import { Link, navigate } from '@/router.tsx';
import { useRemote, useSession, type Remote } from '@/session.tsx';

import { LinkIssueDialog, MoveDialog, TasksDialog } from './dialogs.tsx';
import { Editor } from './editor.tsx';
import { HistoryView } from './history.tsx';
import { BASE, openTasks, pagePath, useProjects } from './lib.ts';
import { LockIcon, PageBody, useAuthorName } from './parts.tsx';

export type PageMode = 'view' | 'edit' | 'history';

type AreaProps = {
  space: WikiSpace;
  pageId: number;
  mode: PageMode;
  nodes: WikiTreeNode[];
  titles: Map<string, number>;
  spaceKeys: Set<string>;
  onAdd: (parent: WikiTreeNode | null) => void;
  reloadTree: () => void;
};

/** Loads the open page and shows it, its editor or its history. */
export function PageArea({ space, pageId, mode, nodes, titles, spaceKeys, onAdd, reloadTree }: AreaProps) {
  const page: Remote<WikiPageDetail> = useRemote(
    () => api.get<WikiPageDetail>(`${BASE}/pages/${pageId}`),
    [pageId],
    (event) =>
      (event.kind.startsWith('page.') && event.payload.spaceKey === space.key) ||
      (event.kind.startsWith('issue.') && (page.data?.issues.some((entry) => entry.issue.id === event.issueId) ?? false)),
  );
  const data = page.data;

  // An address can name a page of another space; the page knows where it lives.
  useEffect(() => {
    if (data && data.spaceKey !== space.key) {
      navigate(pagePath(data.spaceKey, data.id, mode === 'view' ? undefined : mode), { replace: true });
    }
  }, [data, space.key, mode]);

  if (page.error && !data) {
    return (
      <div className="panel wiki-article">
        <ErrorNote message={page.error} />
        <p>
          <Link href={`/wiki/${space.key}`}>{t('Back to the space')}</Link>
        </p>
      </div>
    );
  }

  if (!data || data.spaceKey !== space.key) {
    return <Spinner />;
  }

  if (mode === 'edit') {
    return <Editor page={data} titles={titles} spaceKeys={spaceKeys} reload={page.reload} onSaved={page.set} />;
  }

  if (mode === 'history') {
    return <HistoryView page={data} titles={titles} spaceKeys={spaceKeys} onRestored={page.set} />;
  }

  return <PageView page={data} nodes={nodes} titles={titles} spaceKeys={spaceKeys} onAdd={onAdd} onChanged={page.set} reloadTree={reloadTree} />;
}

type Heading = { id: number; level: number; text: string; element: HTMLElement };

/** The headings of the rendered body, for the table of contents. */
function useHeadings(body: string): [RefObject<HTMLDivElement | null>, Heading[]] {
  const container = useRef<HTMLDivElement>(null);
  const [headings, setHeadings] = useState<Heading[]>([]);

  useLayoutEffect(() => {
    const found = [...(container.current?.querySelectorAll<HTMLElement>('.markdown h1, .markdown h2, .markdown h3') ?? [])];

    setHeadings(found.map((element, id) => ({ id, level: Number(element.tagName.slice(1)), text: element.textContent ?? '', element })));
  }, [body]);

  return [container, headings];
}

type ViewProps = {
  page: WikiPageDetail;
  nodes: WikiTreeNode[];
  titles: Map<string, number>;
  spaceKeys: Set<string>;
  onAdd: (parent: WikiTreeNode | null) => void;
  onChanged: (page: WikiPageDetail) => void;
  reloadTree: () => void;
};

function PageView({ page, nodes, titles, spaceKeys, onAdd, onChanged, reloadTree }: ViewProps) {
  const { boot } = useSession();
  const authorName = useAuthorName();
  const [dialog, setDialog] = useState<'move' | 'tasks' | 'link' | 'delete' | 'detach' | null>(null);
  const [container, headings] = useHeadings(page.body);
  const projects = useProjects(page.issues.map((entry) => entry.issue.projectKey));
  const node = nodes.find((candidate) => candidate.id === page.id);
  const siblings = nodes.filter((candidate) => candidate.parentId === page.parentId);
  const place = siblings.findIndex((candidate) => candidate.id === page.id);
  const editable = page.space.canEdit;
  const writable = editable && !page.locked;
  const tasks = useMemo(
    () =>
      openTasks(
        page.body,
        boot.projects.map((project) => project.key),
      ),
    [page.body, boot.projects],
  );
  const top = Math.min(...headings.map((heading) => heading.level));

  async function shift(by: number): Promise<void> {
    await attempt(() => api.post(`${BASE}/pages/${page.id}/move`, { parentId: page.parentId, position: place + by }));
    reloadTree();
  }

  const actions: MenuItem[] = [
    { label: t('New page under this one'), icon: 'plus', onSelect: () => onAdd(node ?? null), disabled: !editable || !node },
    { label: t('Move…'), icon: 'folder', onSelect: () => setDialog('move'), disabled: !writable || !node },
    { label: t('Move up'), icon: 'chevronUp', onSelect: () => void shift(-1), disabled: !writable || place <= 0 },
    { label: t('Move down'), icon: 'chevronDown', onSelect: () => void shift(1), disabled: !writable || place < 0 || place >= siblings.length - 1 },
    { label: t('Link an issue'), icon: 'link', onSelect: () => setDialog('link'), disabled: !editable },
    { label: t('Create issues from tasks'), icon: 'check', onSelect: () => setDialog('tasks'), disabled: !writable || tasks.length === 0 },
    { label: t('Copy link'), icon: 'copy', onSelect: () => void copyText(`${window.location.origin}${pagePath(page.spaceKey, page.id)}`) },
    { label: t('Delete page'), icon: 'trash', danger: true, onSelect: () => setDialog('delete'), disabled: !editable },
  ];

  return (
    <article className="panel wiki-article">
      {page.breadcrumbs.length > 0 && (
        <nav className="crumbs wiki-crumbs" aria-label={t('Parent pages')}>
          {page.breadcrumbs.map((crumb) => (
            <span key={crumb.id} className="wiki-crumb">
              <Link href={pagePath(page.spaceKey, crumb.id)}>{crumb.title}</Link>
              <Icon name="chevronRight" size={12} />
            </span>
          ))}
        </nav>
      )}

      <header className="wiki-head">
        <h1 className="wiki-title">{page.title}</h1>
        <div className="wiki-head-actions">
          {writable && (
            <Button variant="primary" icon="edit" onClick={() => navigate(pagePath(page.spaceKey, page.id, 'edit'))}>
              {t('Edit')}
            </Button>
          )}
          <Button icon="history" onClick={() => navigate(pagePath(page.spaceKey, page.id, 'history'))}>
            {t('History')}
          </Button>
          <Menu label={t('Page actions')} items={actions} />
        </div>
      </header>

      <p className="muted wiki-meta">
        {t('Last edited by {name}', { name: authorName(page.updatedBy, page.locked ? 'Confluence' : null) })}
        {' · '}
        <span title={formatDateTime(page.updatedAt)}>{timeAgo(page.updatedAt)}</span>
        {' · '}
        <Link href={pagePath(page.spaceKey, page.id, 'history')}>{t('version {version}', { version: page.version })}</Link>
      </p>

      {page.locked && (
        <div className="wiki-lock" role="note">
          <Icon name="lock" />
          <span className="wiki-lock-text">{t('This page mirrors Confluence and cannot be edited here. Changes made in Confluence arrive with the next sync.')}</span>
          {page.sourceUrl && (
            <a href={page.sourceUrl} target="_blank" rel="noreferrer noopener" className="wiki-lock-source">
              {t('Open in Confluence')} <Icon name="external" size={13} />
            </a>
          )}
          {editable && <Button onClick={() => setDialog('detach')}>{t('Detach to edit')}</Button>}
        </div>
      )}

      <div className={headings.length >= 3 ? 'wiki-content wiki-content-toc' : 'wiki-content'}>
        <div className="wiki-body" ref={container}>
          <PageBody body={page.body} spaceKey={page.spaceKey} titles={titles} spaceKeys={spaceKeys} />
        </div>
        {headings.length >= 3 && (
          <nav className="wiki-toc" aria-label={t('On this page')}>
            <strong>{t('On this page')}</strong>
            {headings.map((heading) => (
              <button key={heading.id} type="button" className="wiki-toc-item" style={{ paddingLeft: (heading.level - top) * 12 }} onClick={() => heading.element.scrollIntoView({ behavior: 'smooth', block: 'start' })}>
                {heading.text}
              </button>
            ))}
          </nav>
        )}
      </div>

      {writable && tasks.length > 0 && (
        <div className="wiki-tasks-bar">
          <Icon name="check" />
          <span>{t('{count} open tasks on this page are not issues yet.', { count: tasks.length })}</span>
          <Button size="sm" onClick={() => setDialog('tasks')}>
            {t('Create issues')}
          </Button>
        </div>
      )}

      {page.children.length > 0 && (
        <section className="wiki-section">
          <div className="section-head">
            <h3>{t('Pages under this one')}</h3>
            <span className="tab-count">{page.children.length}</span>
          </div>
          <ul className="wiki-links">
            {page.children.map((child) => (
              <li key={child.id}>
                <Icon name="page" size={14} />
                <Link href={pagePath(page.spaceKey, child.id)}>{child.title}</Link>
                {child.locked && <LockIcon />}
                <span className="muted">{timeAgo(child.updatedAt)}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      <section className="wiki-section">
        <div className="section-head">
          <h3>{t('Issues')}</h3>
          <span className="tab-count">{page.issues.length}</span>
          <span className="filter-spacer" />
          {editable && (
            <Button size="sm" variant="ghost" icon="link" onClick={() => setDialog('link')}>
              {t('Link an issue')}
            </Button>
          )}
        </div>
        {page.issues.length === 0 && <p className="muted">{t('No issue is linked. Mention a key such as WEB-12 in the text, or link an issue here.')}</p>}
        <div className="wiki-issues">
          {page.issues.map(({ issue, relation }) => (
            <IssueLine
              key={issue.id}
              issue={issue}
              project={projects[issue.projectKey]}
              compact
              trailing={
                relation === 'documents' ? (
                  <>
                    <Tag tone="blue" title={t('Linked by hand; stays until removed')}>
                      {t('linked')}
                    </Tag>
                    {editable && (
                      <Button
                        size="sm"
                        variant="ghost"
                        icon="x"
                        aria-label={t('Remove the link to {key}', { key: issue.key })}
                        title={t('Remove the link to {key}', { key: issue.key })}
                        onClick={() => void attempt(() => api.del<WikiPageDetail>(`${BASE}/pages/${page.id}/issues/${issue.key}`)).then((updated) => updated && onChanged(updated))}
                      />
                    )}
                  </>
                ) : (
                  <Tag title={t('The text of the page mentions this issue')}>{t('mentioned')}</Tag>
                )
              }
            />
          ))}
        </div>
      </section>

      {page.backlinks.length > 0 && (
        <section className="wiki-section">
          <div className="section-head">
            <h3>{t('Linked from')}</h3>
            <span className="tab-count">{page.backlinks.length}</span>
          </div>
          <ul className="wiki-links">
            {page.backlinks.map((link) => (
              <li key={link.pageId}>
                <Icon name="page" size={14} />
                <Link href={pagePath(link.spaceKey, link.pageId)}>{link.title}</Link>
                {link.spaceKey !== page.spaceKey && <Tag>{link.spaceKey}</Tag>}
              </li>
            ))}
          </ul>
        </section>
      )}

      {dialog === 'move' && node && <MoveDialog page={node} nodes={nodes} onClose={() => setDialog(null)} onMoved={reloadTree} />}
      {dialog === 'tasks' && <TasksDialog page={page} onClose={() => setDialog(null)} onDone={onChanged} />}
      {dialog === 'link' && <LinkIssueDialog page={page} onClose={() => setDialog(null)} onDone={onChanged} />}
      {dialog === 'detach' && (
        <ConfirmDialog
          title={t('Detach “{title}” from Confluence?', { title: page.title })}
          body={t('The page becomes an ordinary page that can be edited here. From then on a sync no longer updates it, even when it changes in Confluence. This cannot be undone.')}
          confirm={t('Detach')}
          onClose={() => setDialog(null)}
          onConfirm={async () => {
            onChanged(await api.post<WikiPageDetail>(`${BASE}/pages/${page.id}/detach`, {}));
            reloadTree();
          }}
        />
      )}
      {dialog === 'delete' && (
        <ConfirmDialog
          title={t('Delete “{title}”?', { title: page.title })}
          body={
            page.children.length > 0
              ? t('The page is deleted together with everything under it ({pages} directly), with history and links. This cannot be undone.', { pages: countOf(page.children.length, 'page', 'pages') })
              : t('The page is deleted with its history and its links. This cannot be undone.')
          }
          confirm={page.children.length > 0 ? t('Delete with the pages under it') : t('Delete page')}
          danger
          onClose={() => setDialog(null)}
          onConfirm={async () => {
            await api.del(`${BASE}/pages/${page.id}?recursive=${page.children.length > 0}`);
            navigate(page.parentId !== null ? pagePath(page.spaceKey, page.parentId) : `/wiki/${page.spaceKey}`);
          }}
        />
      )}
    </article>
  );
}
