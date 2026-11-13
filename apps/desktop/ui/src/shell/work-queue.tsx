// The right column: my work on the server. In progress, to do, and what the modules add:
// where status and code disagree, my pull requests, failing tests, runs that wait for me.
import { useMemo, useState } from 'react';

import type { AgentRun } from '@web/bindings/AgentRun.ts';
import type { DriftKind } from '@web/bindings/DriftKind.ts';
import type { E2eDashboard } from '@web/bindings/E2eDashboard.ts';
import type { GithubPr } from '@web/bindings/GithubPr.ts';
import type { IssueRow } from '@web/bindings/IssueRow.ts';
import { Button, ErrorNote, Spinner, Tag } from '@web/components/ui.tsx';
import { Icon } from '@web/components/icon.tsx';
import { t, timeAgo } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import type { DashboardView } from '../bindings/DashboardView.ts';
import type { Thread } from '../bindings/Thread.ts';
import { IssueKeyLink, IssueRowLine, Section, issuePath } from '../components/bits.tsx';
import { openExternal } from '../bridge.ts';
import { useRemote, useStored } from '../live.ts';
import { partsOf, useSession } from '../session.tsx';

/** The tracker's search for these words: its own query language, or plain text. */
export function searchPath(words: string): string {
  return `/search?q=${encodeURIComponent(words.trim())}`;
}

export const DRIFT_TEXT: Record<DriftKind, string> = {
  done_with_open_pr: 'Done, but a pull request is still open',
  merged_not_done: 'Merged, but the issue is not done',
  no_code_yet: 'In progress for days with no branch or commit',
  todo_with_code: 'Code exists, but the issue has not been started',
  checks_failing: 'Checks are failing',
  changes_requested: 'Changes were requested in review',
  stale_pr: 'The pull request has gone quiet',
  closed_unmerged: 'The pull request was closed without merging',
};

type GithubSection = { pulls: GithubPr[]; drift: { key: string; summary: string; kinds: DriftKind[] }[] };
type AgentsSection = { waiting: AgentRun[] };
type Filter = 'all' | 'no-thread' | 'drift' | 'high';

const FILTERS: { id: Filter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'no-thread', label: 'No thread yet' },
  { id: 'drift', label: 'Code disagrees' },
  { id: 'high', label: 'High priority' },
];

function StartButton({ issueKey }: { issueKey: string }) {
  const { go } = useSession();

  return (
    <Button
      size="sm"
      variant="ghost"
      icon="play"
      title={t('Start a thread')}
      aria-label={t('Start a thread')}
      onClick={(event) => {
        event.stopPropagation();
        go({ name: 'new-thread', key: issueKey });
      }}
    />
  );
}

function Issues({ rows, view, threads }: { rows: IssueRow[]; view: DashboardView; threads: Thread[] }) {
  const { go } = useSession();

  return (
    <>
      {rows.map((row) => {
        const thread = threads.find((candidate) => candidate.issueKey === row.key);

        return (
          <IssueRowLine
            key={row.id}
            issue={row}
            projects={view.projects}
            compact
            trailing={
              thread ? (
                <Button
                  size="sm"
                  variant="ghost"
                  icon="bot"
                  title={t('Open its thread')}
                  aria-label={t('Open its thread')}
                  onClick={(event) => {
                    event.stopPropagation();
                    go({ name: 'thread', id: thread.id });
                  }}
                />
              ) : (
                <StartButton issueKey={row.key} />
              )
            }
          />
        );
      })}
    </>
  );
}

export function WorkQueue() {
  const { go, openPage } = useSession();
  const dashboard = useRemote(() => core.dashboard(), [], (event) => event.type === 'data_changed' || event.type === 'connection');
  const threads = useRemote(() => core.threads(), [], (event) => event.type === 'thread_changed');
  const { status } = useSession();
  // My runs wherever they are carried out: on this computer, on somebody's, on a service.
  const runs = useRemote(() => core.runs(true), [], (event) => event.type === 'thread_changed' || (event.type === 'data_changed' && event.scope === 'runs'));
  const live = (runs.data ?? []).filter((run) => !['succeeded', 'failed', 'cancelled'].includes(run.state));
  const [words, setWords] = useState('');
  const [filter, setFilter] = useStored<Filter>('queue-filter', 'all');
  const view = dashboard.data;
  const github = view?.dashboard.ext.github as GithubSection | undefined;
  const agents = view?.dashboard.ext.agents as AgentsSection | undefined;
  const tests = view?.dashboard.ext.e2e as E2eDashboard | undefined;
  const drifting = useMemo(() => new Set((github?.drift ?? []).map((item) => item.key)), [github]);
  const withThread = useMemo(() => new Set((threads.data ?? []).map((thread) => thread.issueKey)), [threads.data]);
  const matches = (row: IssueRow) => {
    const text = `${row.key} ${row.summary} ${row.labels.join(' ')}`.toLowerCase();
    const passes = filter === 'all' || (filter === 'no-thread' && !withThread.has(row.key)) || (filter === 'drift' && drifting.has(row.key)) || (filter === 'high' && row.priority <= 2);

    return passes && text.includes(words.trim().toLowerCase());
  };
  const assigned = (view?.dashboard.assigned ?? []).filter(matches);
  const inCategory = (category: string) => assigned.filter((row) => view && partsOf(view.projects, row).status?.category === category);
  const inProgress = inCategory('in_progress');
  const todo = inCategory('todo');

  return (
    <aside className="desk-column desk-right" aria-label={t('My work')}>
      <div className="desk-queue-head">
        <h2>{t('My work')}</h2>
        {view?.offline && <Tag tone="amber" title={t('The server cannot be reached; this is the copy from {when}.', { when: timeAgo(view.fetchedAt) })}>{t('offline copy')}</Tag>}
        <span className="desk-grow" />
        <Button size="sm" variant="ghost" icon="refresh" onClick={dashboard.reload} aria-label={t('Refresh')} title={t('Refresh')} />
      </div>
      <div className="filter-search desk-queue-search">
        <Icon name="search" size={14} />
        <input
          className="input"
          placeholder={t('Filter my work, Enter searches everything')}
          value={words}
          onChange={(event) => setWords(event.target.value)}
          onKeyDown={(event) => event.key === 'Enter' && words.trim() && openPage(searchPath(words))}
        />
      </div>
      <div className="desk-chips" role="group" aria-label={t('Quick filters')}>
        {FILTERS.map((item) => (
          <button key={item.id} type="button" className={filter === item.id ? 'desk-chip is-on' : 'desk-chip'} aria-pressed={filter === item.id} onClick={() => setFilter(item.id)}>
            {t(item.label)}
          </button>
        ))}
      </div>
      {dashboard.loading && !view && <Spinner />}
      {dashboard.error && <ErrorNote message={dashboard.error} />}
      {view && (
        <>
          {agents && agents.waiting.length > 0 && (
            <Section id="q-runs" title={t('Agent runs waiting for me')} count={agents.waiting.length}>
              {agents.waiting.map((run) => (
                <div
                  key={run.id}
                  className="issue-line desk-row"
                  onClick={() => (run.runnerId && run.runnerId !== status.runner.id ? go({ name: 'run', id: run.id }) : openPage(issuePath(run.issueKey)))}
                >
                  <Icon name="bot" size={14} />
                  <IssueKeyLink issueKey={run.issueKey} />
                  <span className="issue-line-summary">{run.issueSummary}</span>
                  {run.runnerId !== status.runner.id && run.runnerName && <Tag title={t('Carried out elsewhere; this app only follows it.')}>{run.runnerName}</Tag>}
                  <Tag tone="amber">{run.pendingQuestion ? t('asks a question') : t('{count} to decide', { count: run.pendingProposals })}</Tag>
                </div>
              ))}
            </Section>
          )}
          {live.length > 0 && (
            <Section id="q-live-runs" title={t('Agent runs at work')} count={live.length}>
              {live.map((run) => {
                const thread = (threads.data ?? []).find((candidate) => candidate.runId === run.id);
                const here = run.runnerId === status.runner.id || (!run.runnerId && run.onRunner === status.runner.id);

                return (
                  <div key={run.id} className="issue-line desk-row" onClick={() => go(thread ? { name: 'thread', id: thread.id } : { name: 'run', id: run.id })}>
                    <Icon name="bot" size={14} />
                    <IssueKeyLink issueKey={run.issueKey} />
                    <span className="issue-line-summary">{run.issueSummary}</span>
                    <Tag tone={here ? 'blue' : 'amber'} title={here ? t('Carried out on this computer') : t('Carried out elsewhere; this app only follows it.')}>
                      {here ? t('This computer') : (run.runnerName ?? run.onRunnerName ?? t('waits for a runner'))}
                    </Tag>
                  </div>
                );
              })}
            </Section>
          )}
          <Section id="q-progress" title={t('In progress')} count={inProgress.length}>
            {inProgress.length === 0 && <div className="panel-empty muted">{t('Nothing in progress. Pick something from below.')}</div>}
            <Issues rows={inProgress} view={view} threads={threads.data ?? []} />
          </Section>
          <Section id="q-todo" title={t('To do')} count={todo.length}>
            {todo.length === 0 && <div className="panel-empty muted">{t('Nothing waiting for you.')}</div>}
            <Issues rows={todo} view={view} threads={threads.data ?? []} />
          </Section>
          {github && github.drift.length > 0 && (
            <Section id="q-drift" title={t('Status and code disagree')} count={github.drift.length}>
              {github.drift.map((item) => (
                <div key={item.key} className="drift-row desk-row" onClick={() => openPage(issuePath(item.key))}>
                  <div className="issue-line">
                    <IssueKeyLink issueKey={item.key} />
                    <span className="issue-line-summary">{item.summary}</span>
                  </div>
                  <div className="drift-kinds">
                    {item.kinds.map((kind) => (
                      <span key={kind} className="drift-note">
                        {t(DRIFT_TEXT[kind])}
                      </span>
                    ))}
                  </div>
                </div>
              ))}
            </Section>
          )}
          {github && github.pulls.length > 0 && (
            <Section id="q-pulls" title={t('Pull requests')} count={github.pulls.length}>
              {github.pulls.map((pull) => (
                <div key={pull.id} className="pr-row desk-row" onClick={() => void openExternal(pull.url)} title={pull.url}>
                  <span className={`pr-state pr-${pull.draft ? 'draft' : pull.state}`}>
                    <Icon name="pullRequest" size={14} /> #{pull.number}
                  </span>
                  <span className="pr-title">{pull.title}</span>
                  {pull.checks === 'failing' && <Tag tone="red">{t('checks fail')}</Tag>}
                  {pull.review === 'changes_requested' && <Tag tone="amber">{t('changes requested')}</Tag>}
                  {pull.review === 'approved' && <Tag tone="green">{t('approved')}</Tag>}
                </div>
              ))}
            </Section>
          )}
          {tests && tests.issues.length > 0 && (
            <Section id="q-tests" title={t('My issues with failing tests')} count={tests.issues.length}>
              {tests.issues.map((issue) => (
                <div key={issue.key} className="issue-line desk-row" onClick={() => openPage(issuePath(issue.key))}>
                  <IssueKeyLink issueKey={issue.key} />
                  <span className="issue-line-summary">{issue.summary}</span>
                  <Tag tone="red">{issue.tests.length}</Tag>
                </div>
              ))}
            </Section>
          )}
          <div className="desk-foot muted">{t('as of {when}', { when: timeAgo(view.fetchedAt) })}</div>
        </>
      )}
    </aside>
  );
}
