// A run that another runner carries out, followed from here like the web follows it: what
// its agent said and did, and its proposals. Nothing of it runs on this computer.
import type { AgentProposal } from '@web/bindings/AgentProposal.ts';
import { Icon } from '@web/components/icon.tsx';
import { Button, ErrorNote, Spinner, Tag, useBusy } from '@web/components/ui.tsx';
import { t, timeAgo } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import { IssueKeyLink } from '../components/bits.tsx';
import { openExternal } from '../bridge.ts';
import { useRemote } from '../live.ts';
import { useSession } from '../session.tsx';

const ENDED = ['succeeded', 'failed', 'cancelled'];

function proposalTitle(kind: string): string {
  const words: Record<string, string> = {
    comment: t('A comment on the issue'),
    transition: t('A status change'),
    update_fields: t('A change to the issue'),
    create_subtasks: t('Subtasks'),
    create_issue: t('A new issue'),
    log_work: t('Logged work'),
    push: t('Push the branch'),
    pull_request: t('Open a pull request'),
  };

  return words[kind] ?? kind;
}

function RemoteProposal({ proposal, decidable, onDecided }: { proposal: AgentProposal; decidable: boolean; onDecided: () => void }) {
  const [busy, run] = useBusy();
  const payload = proposal.payload as Record<string, unknown>;
  const decide = (approve: boolean) => void run(() => core.decideOnRun(proposal.id, approve, null)).then(onDecided);

  return (
    <div className="desk-proposal">
      <div className="issue-line">
        <strong>{proposalTitle(proposal.kind)}</strong>
        <Tag tone={proposal.state === 'approved' ? 'green' : proposal.state === 'pending' ? 'amber' : 'red'}>{t(proposal.state)}</Tag>
        <span className="desk-grow" />
        {proposal.state === 'pending' && decidable && (
          <>
            <Button size="sm" onClick={() => decide(false)} disabled={busy}>
              {t('Reject')}
            </Button>
            <Button size="sm" variant="primary" onClick={() => decide(true)} disabled={busy}>
              {t('Approve')}
            </Button>
          </>
        )}
      </div>
      <pre className="desk-payload">{typeof payload.body === 'string' ? payload.body : JSON.stringify(payload, null, 2)}</pre>
      {proposal.state === 'pending' && !decidable && <div className="muted">{t('Decided in the app of the computer that carries this run out, where the changes can be looked at.')}</div>}
      {proposal.note && <div className="muted">{proposal.note}</div>}
    </div>
  );
}

export function RunView({ runId }: { runId: number }) {
  const { serverUrl, status } = useSession();
  const detail = useRemote(() => core.runDetail(runId), [runId], (event) => event.type === 'data_changed' && event.scope === 'runs');
  const runners = useRemote(() => core.runners(), []);
  const [busy, run] = useBusy();

  if (!detail.data) {
    return detail.error ? <ErrorNote message={detail.error} /> : <Spinner />;
  }

  const { run: agentRun, events, proposals } = detail.data;
  const runner = (runners.data ?? []).find((candidate) => candidate.id === (agentRun.runnerId ?? agentRun.onRunner));
  const here = agentRun.runnerId === status.runner.id;
  // A push or a pull request is decided where the code is, unless nobody sits there.
  const decidable = (proposal: AgentProposal) => (proposal.kind !== 'push' && proposal.kind !== 'pull_request') || runner?.interactive === false;

  return (
    <div className="desk-view">
      <div className="desk-view-head">
        <Icon name="bot" size={18} />
        <h1>
          <IssueKeyLink issueKey={agentRun.issueKey} /> {agentRun.issueSummary}
        </h1>
      </div>
      <div className="issue-line">
        <Tag tone={agentRun.state === 'succeeded' ? 'green' : agentRun.state === 'failed' ? 'red' : 'blue'}>{t(agentRun.state)}</Tag>
        <Tag>{t(agentRun.kind)}</Tag>
        <Tag tone={here ? 'blue' : 'amber'}>{here ? t('This computer') : (agentRun.runnerName ?? agentRun.onRunnerName ?? t('waits for a runner'))}</Tag>
        {!here && <span className="muted">{t('Carried out elsewhere; this app only follows it.')}</span>}
        <span className="desk-grow" />
        {!ENDED.includes(agentRun.state) && (
          <Button size="sm" icon="x" disabled={busy || agentRun.cancelRequested} onClick={() => void run(() => core.cancelRun(agentRun.id)).then(detail.reload)}>
            {agentRun.cancelRequested ? t('Cancelling…') : t('Cancel')}
          </Button>
        )}
        <Button size="sm" variant="ghost" icon="external" onClick={() => void openExternal(serverUrl(`/issues/${agentRun.issueKey}`) ?? '')}>
          {t('Open on the web')}
        </Button>
      </div>
      {agentRun.summary && <div className="note">{agentRun.summary}</div>}
      {proposals.length > 0 && (
        <section className="panel settings-card">
          <header className="panel-head">
            <h2>{t('Proposals')}</h2>
          </header>
          {proposals.map((proposal) => (
            <RemoteProposal key={proposal.id} proposal={proposal} decidable={decidable(proposal)} onDecided={detail.reload} />
          ))}
        </section>
      )}
      <section className="panel settings-card">
        <header className="panel-head">
          <h2>{t('What the agent said and did')}</h2>
        </header>
        {events.length === 0 && <div className="panel-empty muted">{t('Nothing yet.')}</div>}
        {events.map((event) => (
          <div key={event.id} className={`desk-run-event is-${event.kind}`}>
            <span className="muted">{timeAgo(event.at)}</span>
            <span>{event.text}</span>
          </div>
        ))}
      </section>
    </div>
  );
}
