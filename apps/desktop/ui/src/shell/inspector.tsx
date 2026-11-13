// The right column, beside every view: what waits for a decision, what is working now, and
// every thread, one list at a time. A decision is made on its card, without leaving the page.
import { useState } from 'react';

import { Button, Tag } from '@web/components/ui.tsx';
import { Icon } from '@web/components/icon.tsx';
import { t, timeAgo } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import type { Thread } from '../bindings/Thread.ts';
import { ThreadStateTag, ThreadTitle } from '../components/bits.tsx';
import { ProposalCard } from '../components/proposals.tsx';
import { useRemote } from '../live.ts';
import { useSession } from '../session.tsx';
import { needOf } from '../words.ts';

/** The threads of this computer and what they wait for, kept up to date. */
export function useThreadWork() {
  const concerns = (event: { type: string }) => event.type === 'thread_changed' || event.type === 'thread_event';
  const threads = useRemote(() => core.threads(), [], concerns);
  const pending = useRemote(() => core.pendingProposals(), [], concerns);
  const all = threads.data ?? [];
  const proposals = pending.data ?? [];
  const asking = all.filter((thread) => thread.pendingQuestion);

  return {
    all,
    proposals,
    asking,
    working: all.filter((thread) => thread.state === 'running' || thread.state === 'queued'),
    waiting: asking.length + proposals.length,
    error: threads.error,
  };
}

export function ThreadItem({ thread, detail }: { thread: Thread; detail?: boolean }) {
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

type List = 'decide' | 'working' | 'threads';

export function Inspector() {
  const { go, view } = useSession();
  const work = useThreadWork();
  const [chosen, setChosen] = useState<List | null>(null);
  const [filter, setFilter] = useState('');
  // The list with something to do opens first; when the last decision is made, the threads.
  const list: List = chosen === null || (chosen === 'decide' && work.waiting === 0) ? (work.waiting > 0 ? 'decide' : 'threads') : chosen;
  const byId = new Map(work.all.map((thread) => [thread.id, thread]));
  const words = filter.trim().toLowerCase();
  const shown = words ? work.all.filter((thread) => `${thread.issueKey} ${thread.issueSummary} ${thread.branch ?? ''} ${thread.kind}`.toLowerCase().includes(words)) : work.all;
  const lists: [List, string, number][] = [
    ['decide', t('To decide'), work.waiting],
    ['working', t('At work'), work.working.length],
    ['threads', t('Threads'), work.all.length],
  ];

  return (
    <aside className="desk-inspector" aria-label={t('Threads and decisions')}>
      <div className="desk-seg" role="tablist">
        {lists.map(([id, title, count]) => (
          <button key={id} type="button" role="tab" aria-selected={list === id} className={list === id ? 'desk-seg-item desk-seg-on' : 'desk-seg-item'} onClick={() => setChosen(id)}>
            {title} <span className={id === 'decide' && count > 0 ? 'desk-seg-count desk-seg-count-waiting' : 'desk-seg-count'}>{count}</span>
          </button>
        ))}
      </div>
      {list === 'decide' && (
        <>
          {work.waiting === 0 && <div className="panel panel-empty muted">{t('Nothing waits for you.')}</div>}
          {work.asking.length > 0 && (
            <section className="panel desk-decisions">
              {work.asking.map((thread) => (
                <ThreadItem key={thread.id} thread={thread} detail />
              ))}
            </section>
          )}
          {work.proposals.map((proposal) => {
            const thread = byId.get(proposal.threadId);

            return (
              <section key={proposal.id} className="panel desk-decisions">
                <button type="button" className="desk-item" onClick={() => go({ name: 'thread', id: proposal.threadId })}>
                  <span className="desk-item-head">
                    <strong>{proposal.issueKey}</strong>
                    {thread && <span className="desk-item-text">{thread.issueSummary}</span>}
                  </span>
                </button>
                {/* The tracker's page lies above this interface, and would cover a dialog. */}
                <ProposalCard proposal={proposal} onReview={view.name === 'web' ? () => go({ name: 'thread', id: proposal.threadId }) : undefined} />
              </section>
            );
          })}
        </>
      )}
      {list === 'working' && (
        <section className="panel">
          {work.working.length === 0 && <div className="panel-empty muted">{t('No thread is working.')}</div>}
          {work.working.map((thread) => (
            <ThreadItem key={thread.id} thread={thread} detail />
          ))}
        </section>
      )}
      {list === 'threads' && (
        <section className="panel">
          {work.all.length > 6 && (
            <div className="desk-filter">
              <input className="input" placeholder={t('Filter…')} value={filter} onChange={(event) => setFilter(event.target.value)} />
            </div>
          )}
          {work.all.length === 0 && <div className="panel-empty muted">{t('No threads yet. Start one from an issue.')}</div>}
          {shown.map((thread) => (
            <ThreadItem key={thread.id} thread={thread} />
          ))}
          {shown.length > 0 && <div className="desk-foot muted">{t('newest {when}', { when: timeAgo(shown[0].createdAt) })}</div>}
        </section>
      )}
      {work.error && <Tag tone="red">{work.error}</Tag>}
      <span className="desk-grow" />
      <Button variant="primary" icon="plus" onClick={() => go({ name: 'new-thread' })}>
        {t('New thread')}
      </Button>
    </aside>
  );
}
