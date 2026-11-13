// Every thread of this computer, at the width of the window.
import { useState } from 'react';

import { Button } from '@web/components/ui.tsx';
import { t, timeAgo } from '@web/i18n/index.ts';
import { ThreadStateTag, ThreadTitle } from '../components/bits.tsx';
import { useSession } from '../session.tsx';
import { useThreadWork } from '../shell/inspector.tsx';

type Which = 'all' | 'working' | 'waiting';

export function ThreadsView() {
  const { go } = useSession();
  const work = useThreadWork();
  const [which, setWhich] = useState<Which>('all');
  const waiting = work.all.filter((thread) => thread.pendingQuestion || work.proposals.some((proposal) => proposal.threadId === thread.id));
  const rows = which === 'working' ? work.working : which === 'waiting' ? waiting : work.all;
  const choices: [Which, string, number][] = [
    ['all', t('All'), work.all.length],
    ['working', t('At work'), work.working.length],
    ['waiting', t('Waiting for you'), waiting.length],
  ];

  return (
    <div className="desk-view">
      <div className="desk-view-head">
        <h1>{t('Threads')}</h1>
        <span className="desk-grow" />
        <Button variant="primary" icon="plus" onClick={() => go({ name: 'new-thread' })}>
          {t('New thread')}
        </Button>
      </div>
      <p className="muted">{t('A thread is an agent working on one issue, in a working tree of its own on this computer.')}</p>
      <div className="desk-chips">
        {choices.map(([id, title, count]) => (
          <button key={id} type="button" className={which === id ? 'desk-chip is-on' : 'desk-chip'} aria-pressed={which === id} onClick={() => setWhich(id)}>
            {title} {count}
          </button>
        ))}
      </div>
      <section className="panel">
        {rows.length === 0 && <div className="panel-empty muted">{work.all.length === 0 ? t('No threads yet. Start one from an issue.') : t('No thread matches.')}</div>}
        {rows.map((thread) => (
          <button key={thread.id} type="button" className="desk-thread-row" onClick={() => go({ name: 'thread', id: thread.id })}>
            <ThreadTitle thread={thread} />
            <span className="desk-thread-row-text">{thread.issueSummary}</span>
            <span>
              <ThreadStateTag thread={thread} />
            </span>
            <code className="desk-thread-row-text muted">{thread.branch ?? ''}</code>
            <span className="muted desk-thread-row-when">{timeAgo(thread.createdAt)}</span>
          </button>
        ))}
      </section>
    </div>
  );
}
