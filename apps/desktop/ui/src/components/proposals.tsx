import { useState } from 'react';

import { Button, Tag, useBusy } from '@web/components/ui.tsx';
import { Icon } from '@web/components/icon.tsx';
import { t } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import type { Proposal } from '../bindings/Proposal.ts';
import { proposalIcon, proposalLabel, proposalStateLabel, proposalStateTone } from '../words.ts';
import { Markdown } from './markdown.tsx';
import { PullRequestDialog, PushDialog } from './outgoing.tsx';

type Payload = { body?: string; status?: string; items?: { summary: string; description?: string }[]; branch?: string; note?: string; title?: string };

/** What a proposal would do, in the person's terms. */
function ProposalBody({ proposal }: { proposal: Proposal }) {
  const payload = (proposal.payload ?? {}) as Payload;

  switch (proposal.kind) {
    case 'comment':
      return <Markdown text={payload.body ?? ''} className="desk-proposal-text" />;
    case 'transition':
      return (
        <p>
          {t('Move {key} to', { key: proposal.issueKey })} <strong>{payload.status}</strong>
        </p>
      );
    case 'create_subtasks':
      return (
        <ul className="desk-subtasks">
          {(payload.items ?? []).map((item, index) => (
            <li key={index}>
              <strong>{item.summary}</strong>
              {item.description && <div className="muted">{item.description}</div>}
            </li>
          ))}
        </ul>
      );
    case 'push':
      return (
        <p>
          <code>{payload.branch}</code>
          {payload.note && <span className="muted"> · {payload.note}</span>}
        </p>
      );
    default:
      return (
        <div>
          <p>
            <strong>{payload.title}</strong>
          </p>
          {payload.body && <Markdown text={payload.body} className="desk-proposal-text" />}
        </div>
      );
  }
}

export function ProposalCard({ proposal, onDecided }: { proposal: Proposal; onDecided?: () => void }) {
  const [busy, run] = useBusy();
  const [dialog, setDialog] = useState<'push' | 'pull_request' | null>(null);
  const pending = proposal.state === 'pending';
  const outgoing = proposal.kind === 'push' || proposal.kind === 'pull_request';
  const result = proposal.result as { keys?: string[]; url?: string } | null;

  async function decide(approve: boolean): Promise<void> {
    await run(() => core.decideProposal(proposal.id, approve, null));
    onDecided?.();
  }

  return (
    <div className={pending ? 'desk-proposal desk-proposal-pending' : 'desk-proposal'}>
      <header>
        <Icon name={proposalIcon(proposal.kind)} size={14} />
        <strong>{proposalLabel(proposal.kind)}</strong>
        <Tag tone={proposalStateTone(proposal.state)}>{proposalStateLabel(proposal.state)}</Tag>
        {!outgoing && !proposal.remoteId && pending && <span className="muted">{t('applied from this app')}</span>}
      </header>
      <ProposalBody proposal={proposal} />
      {proposal.note && <p className="muted">{proposal.note}</p>}
      {result?.keys && <p className="muted">{result.keys.join(', ')}</p>}
      {pending && (
        <div className="desk-proposal-actions">
          {outgoing ? (
            <Button variant="primary" size="sm" icon="eye" onClick={() => setDialog(proposal.kind === 'push' ? 'push' : 'pull_request')}>
              {proposal.kind === 'push' ? t('Review and push…') : t('Review and open…')}
            </Button>
          ) : (
            <Button variant="primary" size="sm" icon="check" disabled={busy} onClick={() => decide(true)}>
              {t('Approve')}
            </Button>
          )}
          <Button size="sm" icon="x" disabled={busy} onClick={() => decide(false)}>
            {t('Reject')}
          </Button>
        </div>
      )}
      {dialog === 'push' && <PushDialog threadId={proposal.threadId} onClose={() => (setDialog(null), onDecided?.())} />}
      {dialog === 'pull_request' && <PullRequestDialog threadId={proposal.threadId} onClose={() => (setDialog(null), onDecided?.())} />}
    </div>
  );
}
