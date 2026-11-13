import { useState, type ReactNode } from 'react';

import { api } from '@/api.ts';
import type { AgentProposal } from '@/bindings/AgentProposal.ts';
import type { AgentProposalKind } from '@/bindings/AgentProposalKind.ts';
import type { AgentRun } from '@/bindings/AgentRun.ts';
import type { AgentRunChanges } from '@/bindings/AgentRunChanges.ts';
import type { AgentRunDetail } from '@/bindings/AgentRunDetail.ts';
import type { AgentRunEvent } from '@/bindings/AgentRunEvent.ts';
import type { AgentRunKind } from '@/bindings/AgentRunKind.ts';
import type { AgentRunState } from '@/bindings/AgentRunState.ts';
import type { AgentRunner } from '@/bindings/AgentRunner.ts';
import type { PersonalKeyState } from '@/bindings/PersonalKeyState.ts';
import type { AgentSettings } from '@/bindings/AgentSettings.ts';
import type { JsonValue } from '@/bindings/serde_json/JsonValue.ts';
import { Icon } from '@/components/icon.tsx';
import { IssueKey } from '@/components/issue-bits.tsx';
import { Markdown } from '@/components/markdown.tsx';
import { Avatar, Button, Dialog, ErrorNote, Field, Picker, Spinner, Tag, useBusy } from '@/components/ui.tsx';
import { currentLanguage, formatDateTime, t, timeAgo } from '@/i18n/index.ts';
import { translateServer } from '@/i18n/server.ts';
import { Link } from '@/router.tsx';
import { useRemote, useSession, type Remote } from '@/session.tsx';

export const BASE = '/api/m/agents';
const SERVER_KINDS: AgentRunKind[] = ['refine', 'breakdown', 'summarize', 'triage', 'custom'];

export function kindLabel(kind: AgentRunKind): string {
  const labels: Record<AgentRunKind, string> = {
    refine: t('Refine description'),
    breakdown: t('Break down into subtasks'),
    summarize: t('Summarise'),
    triage: t('Triage'),
    implement: t('Implement in code'),
    review: t('Review the pull request'),
    custom: t('Custom task'),
  };

  return labels[kind];
}

/** A short word that means different things in different places: the Polish is looked up under `word|context`. */
function inContext(word: string, context: string): string {
  const key = `${word}|${context}`;
  const found = t(key);

  return found === key ? word : found;
}

function stateLabel(state: AgentRunState): string {
  const labels: Record<AgentRunState, string> = {
    queued: inContext('queued', 'run'),
    claimed: inContext('taken', 'run'),
    running: inContext('working', 'run'),
    waiting: t('needs you'),
    succeeded: inContext('finished', 'run'),
    failed: inContext('failed', 'run'),
    cancelled: inContext('cancelled', 'run'),
  };

  return labels[state];
}

const STATE_TONE: Record<AgentRunState, 'neutral' | 'blue' | 'green' | 'amber' | 'red'> = {
  queued: 'neutral',
  claimed: 'blue',
  running: 'blue',
  waiting: 'amber',
  succeeded: 'green',
  failed: 'red',
  cancelled: 'neutral',
};

export function proposalLabel(kind: AgentProposalKind): string {
  const labels: Record<AgentProposalKind, string> = {
    comment: t('Post a comment'),
    transition: t('Change the status'),
    update_fields: t('Change the issue'),
    create_subtasks: t('Create subtasks'),
    create_issue: t('Create an issue'),
    log_work: t('Log work'),
    push: t('Push the branch'),
    pull_request: t('Open a pull request'),
  };

  return labels[kind];
}

/** Pushing and opening a pull request are decided where the code is, not here. */
function decidedByRunner(kind: AgentProposalKind): boolean {
  return kind === 'push' || kind === 'pull_request';
}

function fieldLabel(field: string): string {
  const labels: Record<string, string> = {
    summary: t('Summary'),
    priority: t('Priority'),
    labels: t('Labels'),
    points: t('Story points'),
    assignee: t('Assignee'),
    dueDate: t('Due date'),
    status: t('Status'),
  };

  return labels[field] ?? field;
}

const PROPOSAL_WORDS: Record<string, AgentProposalKind> = {
  'a comment': 'comment',
  'a change of status': 'transition',
  "changes to the issue's fields": 'update_fields',
  subtasks: 'create_subtasks',
  'a new issue': 'create_issue',
  'a work log': 'log_work',
  'pushing the branch': 'push',
  'opening a pull request': 'pull_request',
};

/**
 * The server writes its status lines in English. In another language the ones it is known
 * to write are put into that language; anything else (a runner's own words) stays as it is.
 */
function statusText(text: string): string {
  if (currentLanguage() === 'en') {
    return text;
  }

  const what = (words: string) => (PROPOSAL_WORDS[words] ? proposalLabel(PROPOSAL_WORDS[words]) : words);
  const patterns: [RegExp, (found: RegExpMatchArray) => string][] = [
    [/^Taken by (.+)$/, (found) => t('Taken by {name}', { name: found[1] === 'Server' ? t('Server') : found[1] })],
    [/^Reading the issue$/, () => t('Reading the issue')],
    [/^Asking (.+) again \(step (\d+)\)$/, (found) => t('Asking {model} again (step {step})', { model: found[1], step: found[2] })],
    [/^Asking (.+)$/, (found) => t('Asking {model}', { model: found[1] })],
    [/^Proposed (.+)$/, (found) => t('Proposed: {what}', { what: what(found[1]) })],
    [/^Approved: (.+)$/, (found) => t('Approved: {what}', { what: what(found[1]) })],
    [/^Rejected: (.+)$/, (found) => t('Rejected: {what}', { what: what(found[1]) })],
    [/^Done in the runner's app: (.+)$/, (found) => t('Done in the desktop app: {what}', { what: what(found[1]) })],
    [/^Declined in the runner's app: (.+)$/, (found) => t('Declined in the desktop app: {what}', { what: what(found[1]) })],
    [/^Approved: (.+); the runner carries it out$/, (found) => t('Approved: {what}; the runner carries it out', { what: what(found[1]) })],
    [/^Carried out: (.+)$/, (found) => t('Carried out: {what}', { what: what(found[1]) })],
    [/^Could not carry out (.+?): (.+)$/, (found) => t('Could not carry out ({what}): {why}', { what: what(found[1]), why: translateServer(found[2]) })],
    [/^Could not apply (.+?): (.+)$/, (found) => t('Could not apply ({what}): {why}', { what: what(found[1]), why: translateServer(found[2]) })],
    [/^A proposal was refused: (.+)$/, (found) => t('A proposal was refused: {why}', { why: translateServer(found[1]) })],
    [/^Cancelled$/, () => t('Cancelled')],
    [/^Asked to stop$/, () => t('Asked to stop')],
    [/^Sent to (.+)$/, (found) => t('Sent to {name}', { name: found[1] })],
    [/^The runner stopped answering; the run is queued again$/, () => t('The runner stopped answering; the run is queued again')],
    [/^The server restarted; the run is queued again$/, () => t('The server restarted; the run is queued again')],
  ];

  for (const [pattern, put] of patterns) {
    const found = text.match(pattern);

    if (found) {
      return put(found);
    }
  }

  // Whatever else the server writes on a run (and nothing a runner wrote itself).
  return translateServer(text);
}

export function isFinished(state: AgentRunState): boolean {
  return state === 'succeeded' || state === 'failed' || state === 'cancelled';
}

/** A run that waits for a person: a question to answer, or proposals to decide. */
export function needsPerson(run: AgentRun): boolean {
  return run.state !== 'cancelled' && (run.pendingQuestion || run.pendingProposals > 0);
}

export function useAgentSettings(): Remote<AgentSettings> {
  return useRemote(() => api.get<AgentSettings>(`${BASE}/settings`), []);
}

function money(amount: number): string {
  return new Intl.NumberFormat(currentLanguage() === 'pl' ? 'pl-PL' : 'en-GB', { style: 'currency', currency: 'USD', currencyDisplay: 'narrowSymbol', maximumFractionDigits: amount < 1 ? 4 : 2 }).format(amount);
}

function count(value: number): string {
  return new Intl.NumberFormat(currentLanguage() === 'pl' ? 'pl-PL' : 'en-GB').format(value);
}

function asRecord(value: JsonValue): Record<string, JsonValue> {
  return typeof value === 'object' && value !== null && !Array.isArray(value) ? value : {};
}

function asText(value: JsonValue | undefined): string {
  return typeof value === 'string' ? value : '';
}

type SubtaskDraft = { summary: string; description: string };

function subtasksOf(payload: JsonValue): SubtaskDraft[] {
  const items = asRecord(payload).items;

  return Array.isArray(items) ? items.map((item) => ({ summary: asText(asRecord(item).summary), description: asText(asRecord(item).description) })) : [];
}

function StateTag({ run }: { run: AgentRun }) {
  return (
    <Tag tone={needsPerson(run) ? 'amber' : STATE_TONE[run.state]}>
      {!isFinished(run.state) && run.state !== 'queued' && run.state !== 'waiting' && <span className="agents-pulse" />}
      {needsPerson(run) && isFinished(run.state) ? t('needs you') : stateLabel(run.state)}
    </Tag>
  );
}

/** Why a queued run has not started, as far as the page can tell. */
function QueueNote({ run, settings }: { run: AgentRun; settings: AgentSettings | undefined }) {
  const forServer = SERVER_KINDS.includes(run.kind) && !run.repo;

  if (!settings) {
    return null;
  }

  if (run.onRunner) {
    return <SentToNote run={run} />;
  }

  if (!forServer || !settings.serverKinds.includes(run.kind)) {
    return (
      <p className="muted">
        {run.shared ? t('Waiting for a runner: any connected desktop app or coding agent that works on this project can take it.') : t('Waiting for one of your runners: the desktop app, or a coding agent connected with your token.')}
      </p>
    );
  }

  if (settings.serverStatus === 'no_key') {
    return (
      <p className="agents-hold">
        <Icon name="alert" size={14} />
        {t('It stays queued because the server has no model key yet. An administrator sets one in Settings → AI agents.')}
      </p>
    );
  }

  if (settings.serverStatus === 'cap_reached') {
    return (
      <p className="agents-hold">
        <Icon name="alert" size={14} />
        {t('It stays queued because this month’s spending cap for the server’s agent is used up.')}
      </p>
    );
  }

  return <p className="muted">{t('The server picks it up in a moment.')}</p>;
}

function DescriptionDiff({ lines }: { lines: { kind: string; text: string }[] }) {
  return (
    <div className="agents-diff" role="group" aria-label={t('Description, before and after')}>
      {lines.map((line, index) => (
        <div key={index} className={`agents-diff-line agents-diff-${line.kind}`}>
          <span className="agents-diff-sign" aria-hidden="true">
            {line.kind === 'added' ? '+' : line.kind === 'removed' ? '−' : ' '}
          </span>
          <span>{line.text || ' '}</span>
        </div>
      ))}
    </div>
  );
}

function SubtaskEditor({ items, onChange }: { items: SubtaskDraft[]; onChange: (items: SubtaskDraft[]) => void }) {
  const change = (index: number, patch: Partial<SubtaskDraft>) => onChange(items.map((item, position) => (position === index ? { ...item, ...patch } : item)));

  return (
    <div className="agents-subtasks">
      {items.map((item, index) => (
        <div key={index} className="agents-subtask">
          <span className="agents-subtask-number">{index + 1}</span>
          <div className="agents-subtask-fields">
            <input className="input" value={item.summary} aria-label={t('Subtask {number}', { number: index + 1 })} onChange={(event) => change(index, { summary: event.target.value })} />
            {item.description && (
              <textarea className="input" rows={2} value={item.description} aria-label={t('Description')} onChange={(event) => change(index, { description: event.target.value })} />
            )}
          </div>
          <Button size="sm" variant="ghost" icon="x" aria-label={t('Leave this one out')} title={t('Leave this one out')} onClick={() => onChange(items.filter((_, position) => position !== index))} />
        </div>
      ))}
      <div>
        <Button size="sm" variant="ghost" icon="plus" onClick={() => onChange([...items, { summary: '', description: '' }])}>
          {t('Add a subtask')}
        </Button>
      </div>
    </div>
  );
}

/** What a proposal would do, shown the way its result will look. */
function ProposalPreview({ proposal, drafts, onDrafts }: { proposal: AgentProposal; drafts: SubtaskDraft[] | null; onDrafts: ((items: SubtaskDraft[]) => void) | null }) {
  const payload = asRecord(proposal.payload);
  const preview = proposal.preview;

  if (proposal.kind === 'comment') {
    return <Markdown text={asText(payload.body)} className="agents-quote" />;
  }

  if (proposal.kind === 'log_work') {
    return (
      <p>
        <strong>{asText(payload.spent)}</strong>
        {asText(payload.note) && <span className="muted"> · {asText(payload.note)}</span>}
      </p>
    );
  }

  if (proposal.kind === 'push') {
    return (
      <p>
        <code>{asText(payload.branch)}</code>
        {asText(payload.commit) && (
          <span className="muted">
            {' @ '}
            <code>{asText(payload.commit).slice(0, 12)}</code>
          </span>
        )}
        {asText(payload.note) && <span className="muted"> · {asText(payload.note)}</span>}
      </p>
    );
  }

  if (proposal.kind === 'pull_request') {
    return (
      <>
        <p>
          <strong>{asText(payload.title)}</strong>
          {asText(payload.head) && (
            <span className="muted">
              {' · '}
              <code>{asText(payload.head)}</code>
              {asText(payload.base) && (
                <>
                  {' → '}
                  <code>{asText(payload.base)}</code>
                </>
              )}
            </span>
          )}
        </p>
        {asText(payload.body) && <Markdown text={asText(payload.body)} className="agents-quote" />}
      </>
    );
  }

  if (proposal.kind === 'create_subtasks') {
    return (
      <>
        {preview?.issueType && <p className="muted">{t('New issues of type {type} under this one:', { type: preview.issueType })}</p>}
        {drafts && onDrafts ? (
          <SubtaskEditor items={drafts} onChange={onDrafts} />
        ) : (
          <ol className="agents-list">
            {subtasksOf(proposal.payload).map((item, index) => (
              <li key={index}>
                {item.summary}
                {item.description && <span className="muted"> · {item.description}</span>}
              </li>
            ))}
          </ol>
        )}
      </>
    );
  }

  if (proposal.kind === 'create_issue') {
    return (
      <>
        <p>
          <Tag>{asText(payload.projectKey)}</Tag> {preview?.issueType && <Tag>{preview.issueType}</Tag>} <strong>{asText(payload.summary)}</strong>
          {asText(payload.parentKey) && <span className="muted"> · {t('under {key}', { key: asText(payload.parentKey) })}</span>}
        </p>
        {asText(payload.description) && <Markdown text={asText(payload.description)} className="agents-quote" />}
      </>
    );
  }

  return (
    <>
      {preview && preview.changes.length > 0 && (
        <table className="table table-plain agents-changes">
          <tbody>
            {preview.changes.map((change) => (
              <tr key={change.field}>
                <th scope="row">{fieldLabel(change.field)}</th>
                <td>
                  <span className="agents-before">{change.before ?? t('none')}</span>
                  <span className="agents-arrow" aria-label={t('becomes')}>
                    →
                  </span>
                  <strong>{change.after ?? t('none')}</strong>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {preview?.allowed === false && (
        <p className="agents-hold">
          <Icon name="alert" size={14} />
          {t('The project’s workflow does not allow this move, so approving it will fail.')}
        </p>
      )}
      {preview?.descriptionDiff && (
        <>
          <p className="muted">{t('Description: removed lines are struck out, new lines are marked +.')}</p>
          <DescriptionDiff lines={preview.descriptionDiff} />
        </>
      )}
      {!preview && <pre className="code-block">{JSON.stringify(proposal.payload, null, 2)}</pre>}
    </>
  );
}

function ProposalResult({ proposal }: { proposal: AgentProposal }) {
  const { userById } = useSession();
  const result = asRecord(proposal.result);
  const keys = [...(Array.isArray(result.keys) ? result.keys : []), result.key].filter((key): key is string => typeof key === 'string');
  const who = userById(proposal.decidedBy)?.name;
  const commit = typeof result.commit === 'string' ? result.commit : null;
  const url = typeof result.url === 'string' && /^https?:\/\//.test(result.url) ? result.url : null;

  return (
    <div className="agents-decision">
      <span className="muted">
        {proposal.state === 'approved' && (who ? t('Approved by {name}', { name: who }) : t('Approved'))}
        {proposal.state === 'rejected' && (who ? t('Rejected by {name}', { name: who }) : t('Rejected'))}
        {proposal.state === 'failed' && t('Could not be applied')}
        {proposal.decidedAt && ` · ${timeAgo(proposal.decidedAt)}`}
      </span>
      {proposal.note && <span className={proposal.state === 'failed' ? 'agents-failure' : undefined}>{translateServer(proposal.note)}</span>}
      {commit && (
        <span>
          {t('Pushed at')} <code>{commit.slice(0, 12)}</code>
        </span>
      )}
      {url && (
        <a href={url} target="_blank" rel="noreferrer noopener">
          {url}
        </a>
      )}
      {keys.length > 0 && (
        <span className="agents-keys">
          {t('Created:')}
          {keys.map((key) => (
            <IssueKey key={key} issue={{ key }} />
          ))}
        </span>
      )}
    </div>
  );
}

type ProposalCardProps = {
  proposal: AgentProposal;
  canDecide: boolean;
  /** False when nobody is at the run's runner: its pushes and pull requests are then decided here. */
  runnerInteractive: boolean;
  onDecided: (detail: AgentRunDetail) => void;
};

function ProposalCard({ proposal, canDecide, runnerInteractive, onDecided }: ProposalCardProps) {
  const ofRunner = decidedByRunner(proposal.kind);
  const elsewhere = ofRunner && runnerInteractive;
  // Approved here, and the runner has not said yet how it went.
  const underway = ofRunner && !runnerInteractive && proposal.state === 'approved' && proposal.result === null;
  const editable = proposal.state === 'pending' && canDecide && !elsewhere;
  const [drafts, setDrafts] = useState(() => subtasksOf(proposal.payload));
  const [note, setNote] = useState('');
  const [busy, run] = useBusy();
  const tone = proposal.state === 'approved' ? 'green' : proposal.state === 'failed' ? 'red' : proposal.state === 'pending' ? 'amber' : 'neutral';
  const stateText = { pending: t('to decide'), approved: inContext('approved', 'proposal'), rejected: inContext('rejected', 'proposal'), failed: inContext('failed', 'proposal') }[proposal.state];
  const usable = drafts.filter((item) => item.summary.trim());

  function decide(approve: boolean): void {
    const edited = proposal.kind === 'create_subtasks' && approve ? { items: usable.map((item) => (item.description.trim() ? item : { summary: item.summary })) } : undefined;

    void run(async () => onDecided(await api.post<AgentRunDetail>(`${BASE}/proposals/${proposal.id}/decide`, { approve, note, payload: edited })));
  }

  return (
    <article className={`agents-proposal agents-proposal-${proposal.state}`}>
      <header>
        <strong>{proposalLabel(proposal.kind)}</strong>
        <Tag tone={underway ? 'blue' : tone}>{underway ? t('being carried out') : stateText}</Tag>
      </header>
      <ProposalPreview proposal={proposal} drafts={editable && proposal.kind === 'create_subtasks' ? drafts : null} onDrafts={editable ? setDrafts : null} />
      {editable ? (
        <div className="agents-decide">
          <input className="input" placeholder={t('A note on your decision (optional)')} value={note} onChange={(event) => setNote(event.target.value)} />
          <Button variant="primary" icon="check" disabled={busy || (proposal.kind === 'create_subtasks' && usable.length === 0)} onClick={() => decide(true)}>
            {proposal.kind === 'push' ? t('Approve the push') : proposal.kind === 'pull_request' ? t('Approve and open') : t('Approve')}
          </Button>
          <Button disabled={busy} onClick={() => decide(false)}>
            {t('Reject')}
          </Button>
        </div>
      ) : proposal.state === 'pending' && elsewhere ? (
        <p className="muted">{t('Decided in the desktop app, on the computer that has the code, where the changes can be looked at first.')}</p>
      ) : proposal.state === 'pending' ? (
        <p className="muted">{t('Waiting for someone who may edit this project to decide.')}</p>
      ) : (
        <>
          <ProposalResult proposal={proposal} />
          {underway && (
            <p className="muted agents-underway">
              <Spinner /> {t('Approved. The runner does it on its next round and reports back here.')}
            </p>
          )}
        </>
      )}
      {editable && proposal.kind === 'push' && <p className="muted">{t('Look at the changes above first: approving sends exactly the commit shown there, and nothing later.')}</p>}
    </article>
  );
}

/** A unified diff, line by line: what was added and removed is said by the sign as well as by colour. */
function Patch({ text }: { text: string }) {
  return (
    <pre className="agents-patch">
      {text.split('\n').map((line, index) => {
        const kind = line.startsWith('+++') || line.startsWith('---') || line.startsWith('diff ') || line.startsWith('index ')
          ? 'file'
          : line.startsWith('@@')
            ? 'hunk'
            : line.startsWith('+')
              ? 'added'
              : line.startsWith('-')
                ? 'removed'
                : 'same';

        return (
          <span key={index} className={`agents-patch-${kind}`}>
            {line}
            {'\n'}
          </span>
        );
      })}
    </pre>
  );
}

/**
 * What the run's thread has committed, as its runner uploaded it: the commits, the files, and
 * the diff. This is what a person reads before approving a push from a runner nobody sits at.
 */
function RunChanges({ runId }: { runId: number }) {
  const changes = useRemote(() => api.get<AgentRunChanges>(`${BASE}/runs/${runId}/changes`), [runId], (event) => event.kind === 'agent.run' && event.payload.runId === runId);
  const data = changes.data;

  if (!data) {
    return changes.error ? <ErrorNote message={changes.error} /> : null;
  }

  const added = data.files.reduce((sum, file) => sum + file.additions, 0);
  const removed = data.files.reduce((sum, file) => sum + file.deletions, 0);
  const statusLabel: Record<string, string> = { added: t('added'), modified: t('modified'), deleted: t('deleted'), renamed: t('renamed') };

  return (
    <details className="agents-changes-block">
      <summary>
        <strong>{t('Changes')}</strong>
        <span className="muted">
          {' · '}
          {t('{commits} commits, {files} files', { commits: data.commits.length, files: data.files.length })}
          {' · '}
          <span className="agents-added">+{added}</span> <span className="agents-removed">−{removed}</span>
          {' · '}
          <code>{data.head.slice(0, 12)}</code>
        </span>
      </summary>
      <ol className="agents-list">
        {data.commits.map((commit) => (
          <li key={commit.sha}>
            <code>{commit.sha.slice(0, 9)}</code> {commit.subject}
            {commit.author && <span className="muted"> · {commit.author}</span>}
          </li>
        ))}
      </ol>
      <table className="table table-plain agents-files">
        <tbody>
          {data.files.map((file) => (
            <tr key={file.path}>
              <td>
                <code>{file.path}</code>
              </td>
              <td className="muted">{statusLabel[file.status] ?? file.status}</td>
              <td className="agents-file-counts">
                <span className="agents-added">+{file.additions}</span> <span className="agents-removed">−{file.deletions}</span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {data.patch && <Patch text={data.patch} />}
      {data.truncated && <p className="muted">{t('The diff was cut short by the runner; the rest is on the branch once it is pushed.')}</p>}
      <p className="muted">{t('uploaded {when}', { when: timeAgo(data.updatedAt) })}</p>
    </details>
  );
}

/** What the agent said and did, oldest first. Runs of tool lines fold into one. */
function Stream({ events, skip }: { events: AgentRunEvent[]; skip: string | null }) {
  const { userById } = useSession();
  const blocks: ReactNode[] = [];
  let tools: AgentRunEvent[] = [];

  function flush(): void {
    if (tools.length === 0) {
      return;
    }

    const lines = tools;
    tools = [];
    blocks.push(
      <details key={`tools-${lines[0].id}`} className="agents-tools">
        <summary>{lines.length === 1 ? statusText(lines[0].text) : t('{count} steps: {last}', { count: lines.length, last: statusText(lines[lines.length - 1].text) })}</summary>
        {lines.length > 1 && lines.map((line) => <div key={line.id}>{statusText(line.text)}</div>)}
        {lines.map((line) => line.data !== null && typeof line.data === 'object' && !('proposalId' in line.data) && <pre key={`data-${line.id}`}>{JSON.stringify(line.data, null, 2)}</pre>)}
      </details>,
    );
  }

  for (const event of events) {
    if (event.kind === 'tool') {
      tools.push(event);
      continue;
    }

    flush();

    if (event.kind === 'status') {
      blocks.push(
        <div key={event.id} className="agents-status" title={formatDateTime(event.at)}>
          {statusText(event.text)}
        </div>,
      );
    } else if (event.kind === 'error') {
      blocks.push(<ErrorNote key={event.id} message={translateServer(event.text)} />);
    } else if (event.kind === 'answer') {
      blocks.push(
        <div key={event.id} className="agents-answer">
          <Avatar user={userById(event.actorId)} size={20} />
          <Markdown text={event.text} />
        </div>,
      );
    } else if (event.kind === 'question') {
      blocks.push(
        <div key={event.id} className="agents-question">
          <Icon name="message" size={15} />
          <Markdown text={event.text} />
        </div>,
      );
    } else if (event.text !== skip) {
      blocks.push(<Markdown key={event.id} text={event.text} className="agents-message" />);
    }
  }

  flush();

  return <div className="agents-stream">{blocks}</div>;
}

function AnswerBox({ run, onAnswered }: { run: AgentRun; onAnswered: (detail: AgentRunDetail) => void }) {
  const [text, setText] = useState('');
  const [busy, work] = useBusy();

  return (
    <form
      className="agents-decide"
      onSubmit={(event) => {
        event.preventDefault();
        void work(async () => {
          onAnswered(await api.post<AgentRunDetail>(`${BASE}/runs/${run.id}/answer`, { text }));
          setText('');
        });
      }}
    >
      <input className="input" placeholder={t('Your answer')} value={text} onChange={(event) => setText(event.target.value)} aria-label={t('Your answer')} />
      <Button variant="primary" type="submit" disabled={busy || !text.trim()}>
        {t('Answer')}
      </Button>
    </form>
  );
}

type RunCardProps = {
  run: AgentRun;
  canDecide: boolean;
  settings: AgentSettings | undefined;
  /** Whether the card starts open; a run that needs someone always does. */
  open?: boolean;
  /** Shown before the kind, where runs of several issues are listed together. */
  showIssue?: boolean;
  onChange?: () => void;
};

/** One run: who works on it, what was said, what it asks and proposes, and how it ended. */
export function RunCard({ run: listed, canDecide, settings, open = false, showIssue = false, onChange }: RunCardProps) {
  const { userById } = useSession();
  const [expanded, setExpanded] = useState(open || needsPerson(listed) || !isFinished(listed.state));
  const detail = useRemote(
    () => (expanded ? api.get<AgentRunDetail>(`${BASE}/runs/${listed.id}`) : Promise.resolve(undefined)),
    [listed.id, expanded],
    (event) => event.kind === 'agent.run' && event.payload.runId === listed.id,
  );
  const run = detail.data?.run ?? listed;
  // What can be decided here; a push or a pull request waits in the desktop app.
  const pending = detail.data?.proposals.filter((proposal) => proposal.state === 'pending' && !decidedByRunner(proposal.kind)) ?? [];
  const [busy, work] = useBusy();

  function changed(next: AgentRunDetail): void {
    detail.set(next);
    onChange?.();
  }

  async function act(path: string, body: unknown): Promise<void> {
    await api.post(`${BASE}${path}`, body);
    await detail.reload();
    onChange?.();
  }

  // Whole subtask lists go through as proposed here; edit one by approving it on its own.
  async function approveAll(): Promise<void> {
    for (const proposal of pending) {
      await api.post(`${BASE}/proposals/${proposal.id}/decide`, { approve: true });
    }

    await detail.reload();
    onChange?.();
  }

  return (
    <section className={needsPerson(run) ? 'agents-run agents-run-waiting' : 'agents-run'}>
      <header className="agents-run-head">
        <button type="button" className="agents-run-toggle" aria-expanded={expanded} onClick={() => setExpanded(!expanded)}>
          <Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={14} />
          <StateTag run={run} />
          {showIssue && <span className="agents-run-issue">{run.issueKey}</span>}
          <strong>{kindLabel(run.kind)}</strong>
          {showIssue && <span className="agents-run-summary">{run.issueSummary}</span>}
        </button>
        <span className="agents-run-meta muted">
          {run.runnerName && <span title={run.model ?? undefined}>{run.runnerId === 'server' ? t('Server') : run.runnerName}</span>}
          <span title={formatDateTime(run.createdAt)}>{timeAgo(run.createdAt)}</span>
          <Avatar user={userById(run.requestedBy)} size={18} label={t('Asked by {name}', { name: userById(run.requestedBy)?.name ?? '?' })} />
        </span>
        {canDecide && !isFinished(run.state) && (
          <Button size="sm" variant="ghost" disabled={busy || run.cancelRequested} onClick={() => void work(() => act(`/runs/${run.id}/cancel`, {}))}>
            {run.cancelRequested ? t('Stopping…') : t('Cancel')}
          </Button>
        )}
        {canDecide && isFinished(run.state) && (
          <Button
            size="sm"
            variant="ghost"
            icon="refresh"
            disabled={busy}
            onClick={() => void work(() => act(`/issues/${run.issueKey}/runs`, { kind: run.kind, instructions: run.instructions, model: run.model, repo: run.repo, shared: run.shared }))}
          >
            {t('Retry')}
          </Button>
        )}
      </header>
      {expanded && (
        <div className="agents-run-body">
          {showIssue && (
            <p>
              <IssueKey issue={{ key: run.issueKey }} /> {run.issueSummary}
            </p>
          )}
          {run.instructions && <p className="agents-instructions">{run.instructions}</p>}
          {run.repo && (
            <p className="muted">
              {t('Repository')}: <code>{run.repo}</code>
            </p>
          )}
          {run.state === 'queued' && <QueueNote run={run} settings={settings} />}
          {detail.error && !detail.data && <ErrorNote message={detail.error} />}
          {!detail.data && !detail.error && <Spinner />}
          {detail.data && (
            <>
              <Stream events={detail.data.events} skip={isFinished(run.state) || run.state === 'waiting' ? run.summary : null} />
              {run.pendingQuestion && canDecide && <AnswerBox run={run} onAnswered={changed} />}
              {run.hasChanges && <RunChanges runId={run.id} />}
              {detail.data.proposals.length > 0 && (
                <div className="agents-proposals">
                  <div className="agents-proposals-head">
                    <h3>{t('Proposals')}</h3>
                    {canDecide && pending.length > 1 && (
                      <Button size="sm" icon="check" disabled={busy} onClick={() => void work(approveAll)}>
                        {t('Approve all')}
                      </Button>
                    )}
                  </div>
                  {detail.data.proposals.map((proposal) => (
                    <ProposalCard
                      key={`${proposal.id}-${proposal.state}-${proposal.result === null ? 'open' : 'done'}`}
                      proposal={proposal}
                      canDecide={canDecide}
                      runnerInteractive={run.runnerInteractive}
                      onDecided={changed}
                    />
                  ))}
                </div>
              )}
              {run.summary && (isFinished(run.state) || run.state === 'waiting') && (
                <div className="agents-result">
                  <h3>{t('Result')}</h3>
                  <Markdown text={run.summary} />
                </div>
              )}
              {(run.branch || run.pullRequestUrl || run.inputTokens !== null) && (
                <p className="agents-facts muted">
                  {run.branch && (
                    <span>
                      <Icon name="branch" size={13} /> <code>{run.branch}</code>
                    </span>
                  )}
                  {run.pullRequestUrl && (
                    <a href={run.pullRequestUrl} target="_blank" rel="noreferrer">
                      <Icon name="pullRequest" size={13} /> {t('Pull request')}
                    </a>
                  )}
                  {run.model && <span>{run.model}</span>}
                  {run.inputTokens !== null && (
                    <span>
                      {t('{input} tokens in, {output} out', { input: count(run.inputTokens), output: count(run.outputTokens ?? 0) })}
                      {run.costUsd !== null && ` · ${money(run.costUsd)}`}
                    </span>
                  )}
                </p>
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}

/** A queued run that was sent to one runner: for whom it waits, and whether that one is connected. */
function SentToNote({ run }: { run: AgentRun }) {
  const runners = useRemote(() => api.get<AgentRunner[]>(`${BASE}/runners`), [run.id], (event) => event.kind === 'agent.run');
  const target = runners.data?.find((runner) => runner.id === run.onRunner);
  const name = target?.name ?? run.onRunnerName ?? run.onRunner ?? '';

  if (runners.data && !target?.online) {
    return (
      <p className="agents-hold">
        <Icon name="alert" size={14} />
        {t('Waiting for {name}, which is not connected right now. The run starts when it is; nobody else takes it.', { name })}
      </p>
    );
  }

  return <p className="muted">{t('Waiting for {name} to take it.', { name })}</p>;
}

export type StartPreset = { kind: AgentRunKind; title: string; instructions?: string; hint: string };

/** Starting a run: what to tell the agent, and with which model or runner. */
export function StartDialog({ issueKey, projectKey, preset, repo, onClose, onStarted }: { issueKey: string; projectKey: string; preset: StartPreset; repo: string; onClose: () => void; onStarted: () => void }) {
  const settings = useAgentSettings();
  const inCode = preset.kind === 'implement' || preset.kind === 'review';
  const runners = useRemote(() => (inCode ? api.get<AgentRunner[]>(`${BASE}/runners`) : Promise.resolve([] as AgentRunner[])), [inCode], (event) => event.kind === 'agent.run');
  const { user, userById } = useSession();
  const [instructions, setInstructions] = useState(preset.instructions ?? '');
  const [model, setModel] = useState<string | null>(null);
  const [repository, setRepository] = useState(repo);
  const [shared, setShared] = useState(false);
  // A runner's id when the run is sent to that one; null leaves it to the queue.
  const [onRunner, setOnRunner] = useState<string | null>(null);
  const [busy, run] = useBusy();
  const fits = (runners.data ?? []).filter((runner) => runner.kinds.includes(preset.kind) && (runner.projects === null || runner.projects.some((key) => key.toUpperCase() === projectKey)));
  const able = fits.filter((runner) => runner.online);
  const usable = able.filter((runner) => !runner.pinnedOnly && (shared || runner.userId === user.id));
  // Who can be named: one's own (connected or not: the run waits), and services that are there.
  const nameable = fits.filter((runner) => runner.userId === user.id || (!runner.interactive && runner.online));
  const chosen = nameable.find((runner) => runner.id === onRunner) ?? null;
  // The team's runner works with the requester's own model key.
  const keys = useRemote(() => (inCode ? api.get<PersonalKeyState[]>('/api/me/keys') : Promise.resolve([] as PersonalKeyState[])), [inCode]);
  const hasModelKey = keys.data?.some((key) => key.name === 'anthropic_api_key' && key.set) ?? true;
  const needsKey = chosen !== null && chosen.shared && !hasModelKey;
  const needsInstructions = preset.kind === 'custom' && !instructions.trim();

  function start(): void {
    void run(async () => {
      await api.post(`${BASE}/issues/${issueKey}/runs`, {
        kind: preset.kind,
        instructions,
        model: inCode ? null : model,
        repo: inCode ? repository : null,
        shared: inCode && shared && !chosen,
        onRunner: inCode ? (chosen?.id ?? null) : null,
      });
      onStarted();
      onClose();
    }, t('Handed to the agent'));
  }

  return (
    <Dialog
      title={`${preset.title} · ${issueKey}`}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" icon="play" disabled={busy || needsInstructions || needsKey} onClick={start}>
            {t('Start')}
          </Button>
        </>
      }
    >
      <div className="form">
        <p className="muted">{preset.hint}</p>
        <Field label={preset.kind === 'custom' ? t('What should the agent do?') : t('Extra instructions (optional)')}>
          <textarea className="input" rows={4} value={instructions} autoFocus onChange={(event) => setInstructions(event.target.value)} />
        </Field>
        {inCode ? (
          <>
            <Field label={t('Repository')} hint={t('owner/name, as the runner knows it. Leave blank to let the runner choose.')}>
              <input className="input input-mono" value={repository} placeholder="owner/repository" onChange={(event) => setRepository(event.target.value)} />
            </Field>
            <div className="agents-runner-pick">
              <strong>{t('Where it is carried out')}</strong>
              {!runners.data && <Spinner />}
              <label className="agents-where">
                <input type="radio" name="agents-where" checked={!chosen} onChange={() => setOnRunner(null)} />
                <span>
                  <strong>{t('Any runner that fits')}</strong>
                  <span className="muted">{t('The first connected runner that works on this project takes it.')}</span>
                </span>
              </label>
              {nameable.map((runner) => (
                <label key={runner.id} className="agents-where">
                  <input type="radio" name="agents-where" checked={chosen?.id === runner.id} onChange={() => setOnRunner(runner.id)} />
                  <RunnerLine runner={runner} owner={userById(runner.userId)?.name} />
                </label>
              ))}
            </div>
            {!chosen && (
              <>
                <label className="check">
                  <input type="checkbox" checked={shared} onChange={(event) => setShared(event.target.checked)} />
                  <span>{t('Let anyone’s runner take it, not only mine')}</span>
                </label>
                {runners.data && usable.length === 0 && (
                  <p className="agents-hold">
                    <Icon name="alert" size={14} />
                    {able.some((runner) => !runner.pinnedOnly) ? t('Only other people’s runners are online. Tick the box above to let one of them take it.') : t('None right now. The run waits in the queue until one connects.')}{' '}
                    <Link href="/settings/agents">{t('How to connect a runner')}</Link>
                  </p>
                )}
              </>
            )}
            {needsKey && (
              <p className="agents-hold">
                <Icon name="alert" size={14} />
                {t('{name} is the team’s runner and works with your own model key, which you have not stored yet.', { name: chosen.name })}{' '}
                <Link href="/settings/keys">{t('Add it in Settings → My keys')}</Link>
              </p>
            )}
            {chosen && !chosen.online && (
              <p className="agents-hold">
                <Icon name="alert" size={14} />
                {t('{name} is not connected right now. The run waits for it.', { name: chosen.name })}
              </p>
            )}
          </>
        ) : (
          <>
            {settings.data && (
              <Field label={t('Model')}>
                <Picker
                  value={model ?? settings.data.defaultModel}
                  options={settings.data.models.map((option) => ({ value: option.id, label: option.name }))}
                  onChange={(value) => setModel(value)}
                />
              </Field>
            )}
            {settings.data && settings.data.serverStatus !== 'ready' && (
              <p className="agents-hold">
                <Icon name="alert" size={14} />
                {settings.data.serverStatus === 'no_key'
                  ? t('The server has no model key yet, so the run will wait in the queue. An administrator sets one in Settings → AI agents.')
                  : t('This month’s spending cap is used up, so the run will wait in the queue.')}
              </p>
            )}
          </>
        )}
      </div>
    </Dialog>
  );
}

export function RunnerLine({ runner, owner }: { runner: AgentRunner; owner: string | undefined }) {
  return (
    <div className="agents-runner">
      <span className={runner.online ? 'agents-dot agents-dot-on' : 'agents-dot'} title={runner.online ? t('online') : t('offline')} />
      <strong>{runner.name}</strong>
      <span className="agents-runner-kind">{runner.shared ? t('shared service') : runner.interactive ? t('computer') : t('service')}</span>
      <span className="muted">
        {owner} · {runner.kinds.map((kind) => t(kind)).join(', ')} · {runner.projects === null ? t('any project') : runner.projects.join(', ')}
        {runner.pinnedOnly && ` · ${t('only what is sent to it')}`}
        {!runner.online && ` · ${t('last seen {when}', { when: timeAgo(runner.lastSeenAt) })}`}
      </span>
    </div>
  );
}

/** Runs, kept current as agents work. */
export function useRuns(path: string, deps: unknown[], accepts: (issueId: number | null, projectId: number | null) => boolean): Remote<AgentRun[]> {
  return useRemote(() => api.get<AgentRun[]>(`${BASE}${path}`), deps, (event) => event.kind === 'agent.run' && accepts(event.issueId, event.projectId));
}
