// The left column: what waits for a decision, what is working now, and every thread.
import { useMemo, useState } from 'react';

import { Button, Tag } from '@web/components/ui.tsx';
import { Icon } from '@web/components/icon.tsx';
import { t, timeAgo } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import type { Proposal } from '../bindings/Proposal.ts';
import type { Thread } from '../bindings/Thread.ts';
import { Section, ThreadStateTag, ThreadTitle } from '../components/bits.tsx';
import { useRemote } from '../live.ts';
import { useSession } from '../session.tsx';
import { needOf, proposalIcon, proposalLabel } from '../words.ts';

function ThreadItem({ thread, detail }: { thread: Thread; detail?: boolean }) {
  const { go, view } = useSession();
  const selected = view.name === 'thread' && view.id === thread.id;
  const need = needOf(thread);

  return (
    <button type="button" className={selected ? 'desk-item desk-item-selected' : 'desk-item'} onClick={() => go({ name: 'thread', id: thread.id })}>
      <span className="desk-item-head">
        <ThreadTitle thread={thread} />
        <span className="desk-grow" />
        <ThreadStateTag thread={thread} />
      </span>
      <span className="desk-item-text">{thread.issueSummary}</span>
      {detail && thread.lastText && <span className="desk-item-said">{thread.lastText}</span>}
      {need && (
        <span className="desk-item-need">
          <Icon name="alert" size={12} /> {need}
        </span>
      )}
    </button>
  );
}

/** Proposals of one kind from one thread read as one line: "WEB-12 · Post a comment". */
function DecisionItem({ thread, proposals }: { thread: Thread | undefined; proposals: Proposal[] }) {
  const { go } = useSession();
  const first = proposals[0];

  return (
    <button type="button" className="desk-item" onClick={() => go({ name: 'thread', id: first.threadId })}>
      <span className="desk-item-head">
        <Icon name={proposalIcon(first.kind)} size={14} />
        <strong>{first.issueKey}</strong>
        <span>{proposalLabel(first.kind)}</span>
        <span className="desk-grow" />
        {proposals.length > 1 && <span className="tab-count">{proposals.length}</span>}
      </span>
      {thread && <span className="desk-item-text">{thread.issueSummary}</span>}
    </button>
  );
}

export function ThreadsColumn() {
  const { go } = useSession();
  const concerns = (event: { type: string }) => event.type === 'thread_changed' || event.type === 'thread_event';
  const threads = useRemote(() => core.threads(), [], concerns);
  const pending = useRemote(() => core.pendingProposals(), [], concerns);
  const [filter, setFilter] = useState('');
  const all = threads.data ?? [];
  const byId = useMemo(() => new Map(all.map((thread) => [thread.id, thread])), [all]);
  const asking = all.filter((thread) => thread.pendingQuestion);
  const working = all.filter((thread) => thread.state === 'running' || thread.state === 'queued');
  const groups = useMemo(() => {
    const grouped = new Map<string, Proposal[]>();

    for (const proposal of pending.data ?? []) {
      const key = `${proposal.threadId}:${proposal.kind}`;

      grouped.set(key, [...(grouped.get(key) ?? []), proposal]);
    }

    return [...grouped.values()];
  }, [pending.data]);
  const words = filter.trim().toLowerCase();
  const shown = words ? all.filter((thread) => `${thread.issueKey} ${thread.issueSummary} ${thread.branch ?? ''} ${thread.kind}`.toLowerCase().includes(words)) : all;

  return (
    <aside className="desk-column desk-left" aria-label={t('Threads and decisions')}>
      <Section id="decide" title={t('To decide')} count={groups.length + asking.length}>
        {groups.length + asking.length === 0 && <div className="panel-empty muted">{t('Nothing waits for you.')}</div>}
        {asking.map((thread) => (
          <ThreadItem key={`ask-${thread.id}`} thread={thread} detail />
        ))}
        {groups.map((proposals) => (
          <DecisionItem key={proposals[0].id} thread={byId.get(proposals[0].threadId)} proposals={proposals} />
        ))}
      </Section>
      <Section id="working" title={t('Working now')} count={working.length}>
        {working.length === 0 && <div className="panel-empty muted">{t('No thread is working.')}</div>}
        {working.map((thread) => (
          <ThreadItem key={thread.id} thread={thread} detail />
        ))}
      </Section>
      <Section
        id="threads"
        title={t('Threads')}
        count={all.length}
        actions={
          <Button size="sm" variant="primary" icon="plus" onClick={() => go({ name: 'new-thread' })}>
            {t('New')}
          </Button>
        }
      >
        {all.length > 6 && (
          <div className="desk-filter">
            <input className="input" placeholder={t('Filter…')} value={filter} onChange={(event) => setFilter(event.target.value)} />
          </div>
        )}
        {all.length === 0 && <div className="panel-empty muted">{t('No threads yet. Start one from an issue.')}</div>}
        {shown.map((thread) => (
          <ThreadItem key={thread.id} thread={thread} />
        ))}
        {shown.length > 0 && <div className="desk-foot muted">{t('newest {when}', { when: timeAgo(shown[0].createdAt) })}</div>}
      </Section>
      {threads.error && <Tag tone="red">{threads.error}</Tag>}
    </aside>
  );
}
