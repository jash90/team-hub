// One issue, with the basics done here instead of in a browser: comment, change the status,
// take it, log work, and start a thread on it.
import { useState } from 'react';

import type { E2eIssuePanel } from '@web/bindings/E2eIssuePanel.ts';
import type { GithubIssuePanel } from '@web/bindings/GithubIssuePanel.ts';
import type { Issue } from '@web/bindings/Issue.ts';
import type { Project } from '@web/bindings/Project.ts';
import type { WikiIssuePanel } from '@web/bindings/WikiIssuePanel.ts';
import { PriorityIcon, priorityName } from '@web/components/issue-bits/priority.tsx';
import { StatusPill } from '@web/components/issue-bits/status-pill.tsx';
import { TypeIcon } from '@web/components/issue-bits/type-icon.tsx';
import { Icon } from '@web/components/icon.tsx';
import { Avatar, Button, Dialog, ErrorNote, Field, Picker, Spinner, Tag, toast, useBusy } from '@web/components/ui.tsx';
import { formatDateTime, formatMinutes, t, timeAgo } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import type { IssueView as IssueData } from '../bindings/IssueView.ts';
import { Section, ThreadStateTag } from '../components/bits.tsx';
import { Markdown } from '../components/markdown.tsx';
import { openExternal } from '../bridge.ts';
import { useRemote } from '../live.ts';
import { useSession } from '../session.tsx';
import { DRIFT_TEXT } from '../shell/work-queue.tsx';
import { kindLabel } from '../words.ts';

function LogWorkDialog({ issueKey, onClose, onLogged }: { issueKey: string; onClose: () => void; onLogged: (view: IssueData) => void }) {
  const [spent, setSpent] = useState('');
  const [note, setNote] = useState('');
  const [busy, run] = useBusy();

  async function save(): Promise<void> {
    const view = await run(() => core.logWork(issueKey, spent, note), t('{spent} logged', { spent }));

    if (view) {
      onLogged(view);
      onClose();
    }
  }

  return (
    <Dialog
      title={t('Log work on {key}', { key: issueKey })}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" disabled={busy || !spent.trim()} onClick={save}>
            {t('Log work')}
          </Button>
        </>
      }
    >
      <div className="form">
        <Field label={t('Time spent')} hint={t('For example 1h 30m, 45m or 2d (a day is 8 hours)')}>
          <input className="input" value={spent} onChange={(event) => setSpent(event.target.value)} />
        </Field>
        <Field label={t('What was done')}>
          <input className="input" value={note} onChange={(event) => setNote(event.target.value)} />
        </Field>
      </div>
    </Dialog>
  );
}

function CodePanel({ panel }: { panel: GithubIssuePanel }) {
  const nothing = panel.branches.length + panel.prs.length + panel.commits.length === 0;

  return (
    <Section id="issue-code" title={t('Development')} count={panel.prs.length}>
      {panel.drift.length > 0 && (
        <div className="drift-box">
          {panel.drift.map((kind) => (
            <span key={kind} className="drift-note">
              {t(DRIFT_TEXT[kind])}
            </span>
          ))}
        </div>
      )}
      {nothing && (
        <div className="panel-empty muted">
          {t('No branch, commit or pull request yet.')} <code>{panel.suggestedBranch}</code>
        </div>
      )}
      {panel.prs.map((pull) => (
        <div key={pull.id} className="pr-row desk-row" onClick={() => void openExternal(pull.url)}>
          <span className={`pr-state pr-${pull.draft ? 'draft' : pull.state}`}>
            <Icon name={pull.state === 'merged' ? 'merge' : 'pullRequest'} size={14} /> {pull.repo} #{pull.number}
          </span>
          <span className="pr-title">{pull.title}</span>
          {pull.checks && <Tag tone={pull.checks === 'failing' ? 'red' : pull.checks === 'passing' ? 'green' : 'neutral'}>{t(pull.checks === 'failing' ? 'checks fail' : pull.checks === 'passing' ? 'checks pass' : 'checks running')}</Tag>}
          {pull.review === 'approved' && <Tag tone="green">{t('approved')}</Tag>}
          {pull.review === 'changes_requested' && <Tag tone="amber">{t('changes requested')}</Tag>}
        </div>
      ))}
      {panel.branches.map((branch) => (
        <div key={branch.id} className="pr-row">
          <span className="pr-state pr-branch">
            <Icon name="branch" size={14} /> {branch.repo}
          </span>
          <code className="pr-title">{branch.name}</code>
          <span className="muted">{timeAgo(branch.updatedAt)}</span>
        </div>
      ))}
      {panel.commits.slice(0, 8).map((commit) => (
        <div key={commit.id} className="pr-row">
          <span className="pr-state pr-branch">
            <Icon name="commit" size={14} /> <code>{commit.sha.slice(0, 7)}</code>
          </span>
          <span className="pr-title">{commit.message.split('\n')[0]}</span>
          <span className="muted">{commit.authorName ?? commit.authorLogin}</span>
        </div>
      ))}
    </Section>
  );
}

function PagesPanel({ panel }: { panel: WikiIssuePanel }) {
  const { serverUrl } = useSession();

  if (panel.pages.length === 0) {
    return null;
  }

  return (
    <Section id="issue-pages" title={t('Linked pages')} count={panel.pages.length}>
      {panel.pages.map((page) => (
        <div key={page.pageId} className="pr-row desk-row" onClick={() => void openExternal(serverUrl(`/wiki/${page.spaceKey}/${page.pageId}`) ?? '')}>
          <Icon name="page" size={14} />
          <span className="pr-title">{page.title}</span>
          <span className="muted">
            {page.spaceKey} · {timeAgo(page.updatedAt)}
          </span>
        </div>
      ))}
    </Section>
  );
}

function TestsPanel({ panel }: { panel: E2eIssuePanel }) {
  if (panel.tests.length === 0) {
    return null;
  }

  const tone = { passed: 'green', failed: 'red', flaky: 'amber', skipped: 'neutral' } as const;

  return (
    <Section id="issue-tests" title={t('Tests')} count={panel.tests.length}>
      {panel.tests.map((test) => (
        <div key={test.testId} className="pr-row" title={test.message ?? undefined}>
          <Tag tone={tone[(test.status ?? 'skipped') as keyof typeof tone] ?? 'neutral'}>{test.status ?? t('not run')}</Tag>
          <span className="pr-title">{test.title}</span>
          <span className="muted mono">{test.file}</span>
        </div>
      ))}
    </Section>
  );
}

function Comments({ issue, onChanged }: { issue: Issue; onChanged: (view: IssueData) => void }) {
  const { userById } = useSession();
  const [body, setBody] = useState('');
  const [busy, run] = useBusy();

  async function send(): Promise<void> {
    const view = await run(() => core.comment(issue.key, body));

    if (view) {
      setBody('');
      onChanged(view);
    }
  }

  return (
    <Section id="issue-comments" title={t('Comments')} count={issue.comments.length}>
      <div className="desk-pad">
        <div className="comments">
          {issue.comments.map((comment) => {
            const author = userById(comment.authorId);

            return (
              <article key={comment.id} className="comment">
                <header>
                  <Avatar user={author} label={author?.name ?? comment.authorLabel ?? undefined} />
                  <strong>{author?.name ?? comment.authorLabel ?? t('Someone')}</strong>
                  <span className="muted" title={formatDateTime(comment.createdAt)}>
                    {timeAgo(comment.createdAt)}
                  </span>
                </header>
                <Markdown text={comment.body} />
              </article>
            );
          })}
        </div>
        <div className="desk-composer">
          <textarea
            className="input"
            rows={3}
            placeholder={t('Add a comment… @name mentions someone, WEB-12 links an issue')}
            value={body}
            onChange={(event) => setBody(event.target.value)}
            onKeyDown={(event) => (event.ctrlKey || event.metaKey) && event.key === 'Enter' && body.trim() && void send()}
          />
          <div className="row-end">
            <span className="muted">{t('Ctrl+Enter sends')}</span>
            <Button variant="primary" disabled={busy || !body.trim()} onClick={send}>
              {t('Comment')}
            </Button>
          </div>
        </div>
      </div>
    </Section>
  );
}

function Header({ issue, project, onChanged }: { issue: Issue; project: Project | null; onChanged: (view: IssueData) => void }) {
  const { go, me, userById, serverUrl, status: connection } = useSession();
  const [busy, run] = useBusy();
  const [logging, setLogging] = useState(false);
  const status = project?.statuses.find((candidate) => candidate.id === issue.statusId);
  const type = project?.types.find((candidate) => candidate.id === issue.typeId);
  const assignee = userById(issue.assigneeId);
  const canEdit = Boolean(project?.role && project.role !== 'viewer') && connection.online;
  const webAddress = serverUrl(`/issues/${issue.key}`);

  async function change(work: () => Promise<IssueData>, done?: string): Promise<void> {
    const view = await run(work, done);

    if (view) {
      onChanged(view);
    }
  }

  return (
    <header className="desk-issue-head">
      <div className="crumbs">
        <TypeIcon type={type} />
        <span className="issue-key">{issue.key}</span>
        {issue.parentKey && (
          <button type="button" className="link-button muted" onClick={() => go({ name: 'issue', key: issue.parentKey ?? '' })}>
            ↑ {issue.parentKey}
          </button>
        )}
        <span className="desk-grow" />
        {webAddress && <Button size="sm" variant="ghost" icon="external" onClick={() => void openExternal(webAddress)} title={t('Open in the browser')} aria-label={t('Open in the browser')} />}
      </div>
      <h1 className="issue-title">{issue.summary}</h1>
      <div className="desk-issue-facts">
        {project && canEdit ? (
          <Picker
            variant="inline"
            label={t('Status')}
            value={issue.statusId}
            options={project.statuses.map((option) => ({ value: option.id, label: option.name, render: <StatusPill status={option} /> }))}
            display={<StatusPill status={status} />}
            onChange={(statusId) => statusId !== issue.statusId && void change(() => core.transition(issue.key, statusId))}
            disabled={busy}
          />
        ) : (
          <StatusPill status={status} />
        )}
        <span className="with-avatar" title={t('Priority')}>
          <PriorityIcon priority={issue.priority} /> {priorityName(issue.priority)}
        </span>
        <span className="with-avatar" title={t('Assignee')}>
          <Avatar user={assignee} size={20} /> {assignee?.name ?? t('Unassigned')}
        </span>
        {issue.spentMinutes > 0 && (
          <span className="muted">
            <Icon name="clock" size={13} /> {formatMinutes(issue.spentMinutes)}
          </span>
        )}
        {issue.labels.map((label) => (
          <span key={label} className="label">
            {label}
          </span>
        ))}
      </div>
      <div className="desk-issue-actions">
        <Button variant="primary" icon="play" onClick={() => go({ name: 'new-thread', key: issue.key })}>
          {t('Start a thread')}
        </Button>
        {canEdit && issue.assigneeId !== me?.id && (
          <Button icon="user" disabled={busy} onClick={() => change(() => core.assignToMe(issue.key), t('Assigned to you'))}>
            {t('Assign to me')}
          </Button>
        )}
        {canEdit && (
          <Button icon="clock" onClick={() => setLogging(true)}>
            {t('Log work')}
          </Button>
        )}
      </div>
      {logging && <LogWorkDialog issueKey={issue.key} onClose={() => setLogging(false)} onLogged={onChanged} />}
    </header>
  );
}

function IssueThreads({ issueKey }: { issueKey: string }) {
  const { go } = useSession();
  const threads = useRemote(() => core.threads(), [issueKey], (event) => event.type === 'thread_changed');
  const mine = (threads.data ?? []).filter((thread) => thread.issueKey === issueKey);

  if (mine.length === 0) {
    return null;
  }

  return (
    <Section id="issue-threads" title={t('Threads')} count={mine.length}>
      {mine.map((thread) => (
        <div key={thread.id} className="pr-row desk-row" onClick={() => go({ name: 'thread', id: thread.id })}>
          <Icon name="bot" size={14} />
          <span className="pr-title">
            {kindLabel(thread.kind)} #{thread.id} {thread.branch && <code>{thread.branch}</code>}
          </span>
          <ThreadStateTag thread={thread} />
        </div>
      ))}
    </Section>
  );
}

export function IssueView({ issueKey }: { issueKey: string }) {
  const loaded = useRemote(
    () => core.issue(issueKey),
    [issueKey],
    (event) => (event.type === 'data_changed' && (!event.issue_key || event.issue_key === issueKey)) || event.type === 'connection',
  );
  const view = loaded.data;

  if (!view) {
    return loaded.error ? <ErrorNote message={loaded.error} /> : <Spinner />;
  }

  const { issue, project } = view;
  const github = issue.panels.github as GithubIssuePanel | undefined;
  const wiki = issue.panels.wiki as WikiIssuePanel | undefined;
  const tests = issue.panels.e2e as E2eIssuePanel | undefined;
  const changed = (next: IssueData) => {
    loaded.set(next);
    toast(t('Saved'));
  };

  return (
    <div className="desk-view">
      {view.offline && <div className="note">{t('The server cannot be reached; this is the copy from {when}. Changes are not possible until it is back.', { when: timeAgo(view.fetchedAt) })}</div>}
      {loaded.error && <ErrorNote message={loaded.error} />}
      <Header issue={issue} project={project} onChanged={changed} />
      <Section id="issue-description" title={t('Description')}>
        <div className="desk-pad">{issue.description.trim() ? <Markdown text={issue.description} /> : <span className="muted">{t('No description yet.')}</span>}</div>
      </Section>
      <IssueThreads issueKey={issue.key} />
      {github && <CodePanel panel={github} />}
      {wiki && <PagesPanel panel={wiki} />}
      {tests && <TestsPanel panel={tests} />}
      <Comments issue={issue} onChanged={loaded.set} />
    </div>
  );
}
