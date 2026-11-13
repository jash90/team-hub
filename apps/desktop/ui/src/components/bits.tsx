// Small pieces used all over: an issue's row, a thread's tag, a panel that folds.
import type { ReactNode } from 'react';

import type { IssueRow } from '@web/bindings/IssueRow.ts';
import type { Project } from '@web/bindings/Project.ts';
import { PriorityIcon } from '@web/components/issue-bits/priority.tsx';
import { StatusPill } from '@web/components/issue-bits/status-pill.tsx';
import { TypeIcon } from '@web/components/issue-bits/type-icon.tsx';
import { Icon } from '@web/components/icon.tsx';
import { Tag } from '@web/components/ui.tsx';
import { t } from '@web/i18n/index.ts';
import type { Thread } from '../bindings/Thread.ts';
import { useStored } from '../live.ts';
import { partsOf, useSession } from '../session.tsx';
import { kindLabel, stateLabel, stateTone } from '../words.ts';

export function IssueKeyLink({ issueKey, done }: { issueKey: string; done?: boolean }) {
  const { go } = useSession();

  return (
    <button type="button" className={done ? 'issue-key issue-key-done link-button' : 'issue-key link-button'} onClick={() => go({ name: 'issue', key: issueKey })}>
      {issueKey}
    </button>
  );
}

/** One issue in a list: type, key, summary, status; `trailing` holds what the list adds. */
export function IssueRowLine({ issue, projects, trailing, compact }: { issue: IssueRow; projects: Project[]; trailing?: ReactNode; compact?: boolean }) {
  const { go, view } = useSession();
  const { status, type } = partsOf(projects, issue);
  const selected = view.name === 'issue' && view.key === issue.key;

  return (
    <div className={selected ? 'issue-line desk-row desk-row-selected' : 'issue-line desk-row'} onClick={() => go({ name: 'issue', key: issue.key })}>
      <TypeIcon type={type} />
      <IssueKeyLink issueKey={issue.key} done={status?.category === 'done'} />
      <span className="issue-line-summary" title={issue.summary}>
        {issue.summary}
      </span>
      {!compact && <PriorityIcon priority={issue.priority} />}
      <StatusPill status={status} />
      {trailing}
    </div>
  );
}

export function ThreadStateTag({ thread }: { thread: Thread }) {
  return <Tag tone={stateTone(thread.state)}>{stateLabel(thread.state)}</Tag>;
}

export function ThreadTitle({ thread }: { thread: Thread }) {
  return (
    <span className="desk-thread-title">
      <strong>{thread.issueKey}</strong>
      <span className="muted">
        {' '}
        · {kindLabel(thread.kind)} #{thread.id}
      </span>
    </span>
  );
}

/** A panel whose body can be folded away; the choice is remembered. */
export function Section({ id, title, count, actions, children, startFolded }: { id: string; title: string; count?: number; actions?: ReactNode; children: ReactNode; startFolded?: boolean }) {
  const [folded, setFolded] = useStored(`fold:${id}`, startFolded ?? false);

  return (
    <section className="panel desk-section">
      <header className="panel-head">
        <button type="button" className="desk-fold" onClick={() => setFolded(!folded)} aria-expanded={!folded} aria-label={folded ? t('Unfold') : t('Fold')}>
          <Icon name={folded ? 'chevronRight' : 'chevronDown'} size={14} />
        </button>
        <h2>{title}</h2>
        {count !== undefined && <span className="tab-count">{count}</span>}
        <span className="desk-grow" />
        {actions}
      </header>
      {!folded && children}
    </section>
  );
}
