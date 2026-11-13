import { useEffect, useState, type FormEvent } from 'react';

import { api, query } from '@/api.ts';
import type { Issue } from '@/bindings/Issue.ts';
import type { WikiIssuePanel } from '@/bindings/WikiIssuePanel.ts';
import type { WikiPageDetail } from '@/bindings/WikiPageDetail.ts';
import type { WikiSearchHit } from '@/bindings/WikiSearchHit.ts';
import type { WikiSpace } from '@/bindings/WikiSpace.ts';
import type { WikiTemplate } from '@/bindings/WikiTemplate.ts';
import { Icon } from '@/components/icon.tsx';
import { Button, Dialog, Field, Picker, Spinner, Tag, attempt, useBusy } from '@/components/ui.tsx';
import { t, timeAgo } from '@/i18n/index.ts';
import { Link, navigate } from '@/router.tsx';
import { useRemote, useSession } from '@/session.tsx';

import { BASE, pagePath, useSpaces } from './lib.ts';
import { Snippet } from './parts.tsx';
import { templateBody } from './templates.ts';

/** The pages that mention or document an issue, on the issue's page. */
export function IssuePanel({ issue, reload }: { issue: Issue; reload: () => void }) {
  const panel = issue.panels.wiki as WikiIssuePanel | undefined;
  const [dialog, setDialog] = useState<'link' | 'create' | null>(null);

  if (!panel) {
    return null;
  }

  return (
    <section className="issue-section">
      <div className="section-head">
        <h3>{t('Knowledge base')}</h3>
        <span className="filter-spacer" />
        <Button size="sm" variant="ghost" icon="link" onClick={() => setDialog('link')}>
          {t('Link a page')}
        </Button>
        <Button size="sm" variant="ghost" icon="plus" onClick={() => setDialog('create')}>
          {t('Create a page')}
        </Button>
      </div>
      {panel.pages.length === 0 && <p className="muted">{t('No page talks about this issue yet. A page that mentions {key} shows up here by itself.', { key: issue.key })}</p>}
      {panel.pages.map((page) => (
        <div key={page.pageId} className="wiki-panel-row">
          <Icon name="page" size={14} />
          <Link href={pagePath(page.spaceKey, page.pageId)} className="wiki-panel-title">
            {page.title}
          </Link>
          <span className="wiki-panel-meta">
            <Tag>{page.spaceKey}</Tag>
            {page.relation === 'documents' ? <Tag tone="blue">{t('linked')}</Tag> : <Tag title={t('The text of the page mentions this issue')}>{t('mentions')}</Tag>}
            <span className="muted">{timeAgo(page.updatedAt)}</span>
            {page.relation === 'documents' && (
              <Button
                size="sm"
                variant="ghost"
                icon="x"
                aria-label={t('Remove the link to “{title}”', { title: page.title })}
                title={t('Remove the link to “{title}”', { title: page.title })}
                onClick={() => void attempt(() => api.del(`${BASE}/pages/${page.pageId}/issues/${issue.key}`)).then(reload)}
              />
            )}
          </span>
        </div>
      ))}
      {dialog === 'link' && <LinkPageDialog issue={issue} panel={panel} onClose={() => setDialog(null)} onDone={reload} />}
      {dialog === 'create' && <CreatePageDialog issue={issue} onClose={() => setDialog(null)} />}
    </section>
  );
}

function LinkPageDialog({ issue, panel, onClose, onDone }: { issue: Issue; panel: WikiIssuePanel; onClose: () => void; onDone: () => void }) {
  const [text, setText] = useState('');
  const [hits, setHits] = useState<WikiSearchHit[] | null>(null);
  const [busy, run] = useBusy();
  const linked = new Set(panel.pages.filter((page) => page.relation === 'documents').map((page) => page.pageId));

  useEffect(() => {
    if (!text.trim()) {
      setHits(null);

      return undefined;
    }

    const timer = window.setTimeout(() => {
      api
        .get<WikiSearchHit[]>(`${BASE}/search${query({ q: text })}`)
        .then(setHits)
        .catch(() => setHits([]));
    }, 200);

    return () => window.clearTimeout(timer);
  }, [text]);

  function link(pageId: number): void {
    void run(async () => {
      await api.put(`${BASE}/pages/${pageId}/issues/${issue.key}`, {});
      onDone();
      onClose();
    }, t('Page linked'));
  }

  return (
    <Dialog title={t('Link a page to {key}', { key: issue.key })} onClose={onClose}>
      <div className="form">
        <Field label={t('Page')} hint={t('Type words from the title or the text of the page.')}>
          <input className="input" autoFocus value={text} onChange={(event) => setText(event.target.value)} />
        </Field>
        <div className="wiki-results">
          {hits?.length === 0 && <p className="muted">{t('No page matches.')}</p>}
          {hits?.map((hit) => (
            <button key={hit.pageId} type="button" className="menu-item wiki-result wiki-result-page" disabled={busy || linked.has(hit.pageId)} onClick={() => link(hit.pageId)}>
              <span className="wiki-hit-title">
                <Icon name="page" size={14} />
                <strong>{hit.title}</strong>
                <Tag>{hit.spaceKey}</Tag>
                {linked.has(hit.pageId) && <span className="muted">{t('already linked')}</span>}
              </span>
              <Snippet text={hit.snippet} />
            </button>
          ))}
        </div>
      </div>
    </Dialog>
  );
}

const NEW_SPACE = '+';

/** Starts a page about the issue in the knowledge base of its project, linked to the issue. */
function CreatePageDialog({ issue, onClose }: { issue: Issue; onClose: () => void }) {
  const { boot } = useSession();
  const spaces = useSpaces();
  const templates = useRemote(() => api.get<WikiTemplate[]>(`${BASE}/templates`), []);
  const project = boot.projects.find((candidate) => candidate.key === issue.projectKey);
  const [title, setTitle] = useState(issue.summary);
  const [templateId, setTemplateId] = useState('technical-spec');
  const [chosen, setChosen] = useState<string | null>(null);
  const [busy, run] = useBusy();

  if (!spaces.data || !templates.data) {
    return (
      <Dialog title={t('Create a page for {key}', { key: issue.key })} onClose={onClose}>
        <Spinner />
      </Dialog>
    );
  }

  const writable = spaces.data.filter((space) => space.canEdit);
  const own = writable.find((space) => space.projectKey === issue.projectKey);
  const spaceKey = chosen ?? own?.key ?? NEW_SPACE;
  const template = templates.data.find((candidate) => candidate.id === templateId);

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      const space = spaceKey === NEW_SPACE ? await api.post<WikiSpace>(`${BASE}/spaces`, { key: issue.projectKey, name: project?.name ?? issue.projectKey, projectKey: issue.projectKey }) : null;
      const body = `${t('Describes {key}: {summary}', { key: issue.key, summary: issue.summary })}\n\n${template ? templateBody(template) : ''}`;
      const page = await api.post<WikiPageDetail>(`${BASE}/spaces/${space?.key ?? spaceKey}/pages`, { title, body });

      await api.put(`${BASE}/pages/${page.id}/issues/${issue.key}`, {});
      onClose();
      navigate(pagePath(page.spaceKey, page.id, 'edit'));
    });
  }

  return (
    <Dialog
      title={t('Create a page for {key}', { key: issue.key })}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="wiki-issue-page" disabled={busy || !title.trim()}>
            {t('Create page')}
          </Button>
        </>
      }
    >
      <form id="wiki-issue-page" className="form" onSubmit={submit}>
        <Field label={t('Title')}>
          <input className="input" autoFocus required maxLength={255} value={title} onChange={(event) => setTitle(event.target.value)} />
        </Field>
        <Field label={t('Space')} hint={spaceKey === NEW_SPACE ? t('The project has no space yet; one with the key {key} is created for it.', { key: issue.projectKey }) : undefined}>
          <Picker
            value={spaceKey}
            options={[
              ...writable.map((space) => ({ value: space.key, label: `${space.name} (${space.key})` })),
              ...(own ? [] : [{ value: NEW_SPACE, label: t('A new space for the project {key}', { key: issue.projectKey }) }]),
            ]}
            onChange={setChosen}
          />
        </Field>
        <Field label={t('Start from')} hint={template ? t(template.description) : undefined}>
          <Picker value={templateId} options={templates.data.map((candidate) => ({ value: candidate.id, label: t(candidate.name) }))} onChange={setTemplateId} />
        </Field>
        <p className="muted">{t('The page is linked to {key} and opens for editing.', { key: issue.key })}</p>
      </form>
    </Dialog>
  );
}
