// One thread: what the agent said and did, what it changed, and what it asks of the person.
import { useEffect, useRef, useState } from 'react';

import { Icon } from '@web/components/icon.tsx';
import { Button, ConfirmDialog, ErrorNote, Spinner, Tabs, Tag, useBusy } from '@web/components/ui.tsx';
import { countOf, formatDateTime, t, timeAgo } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import type { Thread } from '../bindings/Thread.ts';
import type { ThreadDetail } from '../bindings/ThreadDetail.ts';
import type { ThreadEvent } from '../bindings/ThreadEvent.ts';
import { IssueKeyLink, ThreadStateTag } from '../components/bits.tsx';
import { CommitList, DiffFiles } from '../components/diff.tsx';
import { Markdown } from '../components/markdown.tsx';
import { PullRequestDialog, PushDialog } from '../components/outgoing.tsx';
import { ProposalCard } from '../components/proposals.tsx';
import { openExternal } from '../bridge.ts';
import { useLive, useRemote, useStored } from '../live.ts';
import { useSession } from '../session.tsx';
import { EVENT_ICONS, kindLabel } from '../words.ts';

function EventLine({ event }: { event: ThreadEvent }) {
  const data = (event.data ?? {}) as { name?: string; detail?: string };

  if (event.kind === 'tool') {
    return (
      <div className="desk-event desk-event-tool" title={formatDateTime(event.at)}>
        <Icon name="settings" size={12} />
        <code>{data.name ?? event.text}</code>
        {data.detail && <span className="mono muted desk-event-detail">{data.detail}</span>}
      </div>
    );
  }

  if (event.kind === 'status') {
    return (
      <div className="desk-event desk-event-status muted" title={formatDateTime(event.at)}>
        {event.text}
      </div>
    );
  }

  return (
    <div className={`desk-event desk-event-${event.kind}`}>
      <div className="desk-event-who">
        <Icon name={EVENT_ICONS[event.kind]} size={14} />
        <strong>{event.kind === 'answer' ? t('You') : event.kind === 'error' ? t('Error') : event.kind === 'question' ? t('The agent asks') : t('Agent')}</strong>
        <span className="muted">{timeAgo(event.at)}</span>
      </div>
      {event.kind === 'error' ? <pre className="desk-error-text">{event.text}</pre> : <Markdown text={event.text} />}
    </div>
  );
}

function Conversation({ detail }: { detail: ThreadDetail }) {
  const end = useRef<HTMLDivElement>(null);
  const count = detail.events.length;

  useEffect(() => {
    end.current?.scrollIntoView({ block: 'nearest' });
  }, [count]);

  return (
    <div className="desk-events">
      {detail.events.map((event) => (
        <EventLine key={event.id} event={event} />
      ))}
      {detail.thread.state === 'running' && <Spinner label={t('working…')} />}
      <div ref={end} />
    </div>
  );
}

function Composer({ thread }: { thread: Thread }) {
  const [text, setText] = useState('');
  const [busy, run] = useBusy();
  const working = thread.state === 'running' || thread.state === 'queued';
  const placeholder = thread.pendingQuestion ? t('Answer the question…') : working ? t('Say something; the thread hears it when its turn ends…') : t('Tell the thread what to do next; it continues in the same session…');

  async function send(): Promise<void> {
    if (await run(() => core.sayToThread(thread.id, text))) {
      setText('');
    }
  }

  return (
    <div className="desk-composer">
      <textarea
        className="input"
        rows={2}
        placeholder={placeholder}
        value={text}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => (event.ctrlKey || event.metaKey) && event.key === 'Enter' && text.trim() && void send()}
      />
      <div className="row-end">
        <span className="muted">{t('Ctrl+Enter sends')}</span>
        <Button variant="primary" icon="message" disabled={busy || !text.trim()} onClick={send}>
          {thread.pendingQuestion ? t('Answer') : t('Send')}
        </Button>
      </div>
    </div>
  );
}

function Changes({ thread }: { thread: Thread }) {
  const diff = useRemote(() => core.threadDiff(thread.id), [thread.id, thread.state, thread.turns], (event) => event.type === 'thread_event' && event.thread_id === thread.id && event.event.kind === 'tool');

  if (!thread.worktree) {
    return <div className="panel-empty muted">{t('The thread has no working tree (not started yet, or removed).')}</div>;
  }

  if (!diff.data) {
    return diff.error ? <ErrorNote message={diff.error} /> : <Spinner />;
  }

  return (
    <div>
      <p className="desk-pad muted">
        {t('Against {base}', { base: diff.data.base })} · {countOf(diff.data.commits.length, 'commit', 'commits')}
        {diff.data.uncommitted > 0 && <> · {t('{count} files not committed', { count: diff.data.uncommitted })}</>}
      </p>
      <h3 className="desk-pad">{t('Commits')}</h3>
      <CommitList commits={diff.data.commits} />
      <h3 className="desk-pad">{t('Changes')}</h3>
      <DiffFiles files={diff.data.files} />
    </div>
  );
}

function Header({ thread }: { thread: Thread }) {
  const { go } = useSession();
  const [busy, run] = useBusy();
  const [dialog, setDialog] = useState<'push' | 'pull_request' | 'remove' | null>(null);
  const working = thread.state === 'running' || thread.state === 'queued';

  return (
    <header className="desk-thread-head">
      <div className="crumbs">
        <Icon name="bot" size={16} />
        <IssueKeyLink issueKey={thread.issueKey} />
        <span className="muted">
          {kindLabel(thread.kind)} #{thread.id}
        </span>
        <ThreadStateTag thread={thread} />
        {thread.origin === 'delegated' && <Tag tone="purple">{t('delegated on the web')}</Tag>}
        {thread.runId === null && <Tag title={t('The server has no run for this thread, so the web does not show it.')}>{t('local only')}</Tag>}
      </div>
      <h1 className="issue-title">{thread.issueSummary}</h1>
      <div className="desk-issue-facts muted">
        {thread.branch && (
          <span>
            <Icon name="branch" size={13} /> <code>{thread.branch}</code>
          </span>
        )}
        <span>{thread.model}</span>
        {thread.turns > 0 && (
          <span>
            {t('{count} turns', { count: thread.turns })} · {thread.inputTokens + thread.outputTokens} tok · ${thread.costUsd.toFixed(2)}
          </span>
        )}
        {thread.pullRequestUrl && (
          <a href={thread.pullRequestUrl} onClick={(event) => (event.preventDefault(), void openExternal(thread.pullRequestUrl ?? ''))}>
            <Icon name="pullRequest" size={13} /> {t('pull request')}
          </a>
        )}
      </div>
      <div className="desk-issue-actions">
        {working ? (
          <Button variant="danger" icon="x" disabled={busy} onClick={() => run(() => core.stopThread(thread.id))}>
            {t('Stop')}
          </Button>
        ) : (
          <Button icon="play" disabled={busy} onClick={() => run(() => core.sayToThread(thread.id, null))}>
            {t('Resume')}
          </Button>
        )}
        <Button icon="upload" disabled={!thread.branch} onClick={() => setDialog('push')}>
          {t('Push…')}
        </Button>
        <Button icon="pullRequest" disabled={!thread.branch} onClick={() => setDialog('pull_request')}>
          {t('Pull request…')}
        </Button>
        {thread.worktree && !working && (
          <Button variant="ghost" icon="trash" onClick={() => setDialog('remove')}>
            {t('Remove working tree')}
          </Button>
        )}
        {!thread.worktree && !working && (
          <Button variant="ghost" icon="trash" disabled={busy} onClick={async () => (await run(() => core.forgetThread(thread.id), t('Removed'))) !== undefined && go({ name: 'home' })}>
            {t('Forget this thread')}
          </Button>
        )}
      </div>
      {dialog === 'push' && <PushDialog threadId={thread.id} onClose={() => setDialog(null)} />}
      {dialog === 'pull_request' && <PullRequestDialog threadId={thread.id} onClose={() => setDialog(null)} />}
      {dialog === 'remove' && (
        <ConfirmDialog
          title={t('Remove the working tree?')}
          body={t('The folder {path} is removed. The branch and its commits stay. Git refuses while the tree holds uncommitted changes.', { path: thread.worktree ?? '' })}
          confirm={t('Remove')}
          danger
          onConfirm={async () => {
            await run(() => core.removeWorktree(thread.id));
          }}
          onClose={() => setDialog(null)}
        />
      )}
    </header>
  );
}

export function ThreadView({ threadId }: { threadId: number }) {
  const loaded = useRemote(
    () => core.thread(threadId),
    [threadId],
    (event) => (event.type === 'thread_changed' || event.type === 'thread_event') && event.thread_id === threadId,
  );
  const [tab, setTab] = useStored<string>('thread-tab', 'conversation');
  const detail = loaded.data;

  // What the agent says appears at once; the reload that follows brings the rest.
  useLive((event) => {
    if (event.type === 'thread_event' && event.thread_id === threadId && detail && !detail.events.some((known) => known.id === event.event.id)) {
      loaded.set({ ...detail, events: [...detail.events, event.event] });
    }
  });

  if (!detail) {
    return loaded.error ? <ErrorNote message={loaded.error} /> : <Spinner />;
  }

  const pending = detail.proposals.filter((proposal) => proposal.state === 'pending');
  const decided = detail.proposals.filter((proposal) => proposal.state !== 'pending');

  return (
    <div className="desk-view desk-thread">
      <Header thread={detail.thread} />
      {pending.length > 0 && (
        <section className="panel desk-decisions">
          <header className="panel-head">
            <h2>{t('To decide')}</h2>
            <span className="tab-count">{pending.length}</span>
            <span className="muted">{t('Nothing here has happened yet. Each one takes effect only when you approve it.')}</span>
          </header>
          {pending.map((proposal) => (
            <ProposalCard key={proposal.id} proposal={proposal} onDecided={loaded.reload} />
          ))}
        </section>
      )}
      <section className="panel">
        <Tabs
          tabs={[
            { id: 'conversation', label: t('Conversation'), count: detail.events.filter((event) => event.kind === 'message').length },
            { id: 'changes', label: t('Changes') },
            { id: 'decided', label: t('Decided'), count: decided.length },
          ]}
          active={tab}
          onChange={setTab}
        />
        {tab === 'conversation' && (
          <>
            <Conversation detail={detail} />
            <Composer thread={detail.thread} />
          </>
        )}
        {tab === 'changes' && <Changes thread={detail.thread} />}
        {tab === 'decided' && (decided.length === 0 ? <div className="panel-empty muted">{t('Nothing decided yet.')}</div> : decided.map((proposal) => <ProposalCard key={proposal.id} proposal={proposal} />))}
      </section>
    </div>
  );
}
