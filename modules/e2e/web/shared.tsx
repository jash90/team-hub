import type { ReactNode } from 'react';

import { api } from '@/api.ts';
import type { E2eHistoryPoint } from '@/bindings/E2eHistoryPoint.ts';
import type { E2eIssueRef } from '@/bindings/E2eIssueRef.ts';
import type { E2eIssueState } from '@/bindings/E2eIssueState.ts';
import type { E2eProjectTests } from '@/bindings/E2eProjectTests.ts';
import type { E2eRun } from '@/bindings/E2eRun.ts';
import type { E2eRunStatus } from '@/bindings/E2eRunStatus.ts';
import type { E2eStatus } from '@/bindings/E2eStatus.ts';
import type { Project } from '@/bindings/Project.ts';
import { Icon, type IconName } from '@/components/icon.tsx';
import { IssueKey } from '@/components/issue-bits.tsx';
import { Tag } from '@/components/ui.tsx';
import { formatDateTime, t, timeAgo } from '@/i18n/index.ts';
import { Link } from '@/router.tsx';
import { useRemote, type LiveEvent } from '@/session.tsx';

export const BASE = '/api/m/e2e';

export const STATUS_TEXT: Record<E2eStatus, string> = { passed: 'passed', failed: 'failed', flaky: 'flaky', skipped: 'skipped' };
export const STATUS_ICON: Record<E2eStatus, IconName | null> = { passed: 'check', failed: 'x', flaky: 'refresh', skipped: null };
export const RUN_TEXT: Record<E2eRunStatus, string> = { passed: 'passed', failed: 'failed', error: 'error', interrupted: 'interrupted', blocked: 'blocked' };
export const STATE_TEXT: Record<E2eIssueState, string> = { passing: 'Tests pass', failing: 'Tests fail', flaky: 'Tests are flaky', skipped: 'Tests have not run' };

export function statusText(status: E2eStatus | null): string {
  return status ? t(STATUS_TEXT[status]) : t('not run yet');
}

/** 320 ms, 4.2 s, 1 min 12 s. */
export function duration(ms: number | null): string {
  if (ms === null) {
    return '';
  }

  if (ms < 1000) {
    return `${ms} ms`;
  }

  if (ms < 60_000) {
    return `${(ms / 1000).toFixed(1)} s`;
  }

  return `${Math.floor(ms / 60_000)} min ${Math.round((ms % 60_000) / 1000)} s`;
}

export function shortCommit(commit: string | null): string {
  return commit ? commit.slice(0, 7) : '';
}

export function runPath(projectKey: string, runId: number): string {
  return `/p/${projectKey}/tests?run=${runId}`;
}

export function StatusMark({ status, label = true }: { status: E2eStatus | null; label?: boolean }) {
  const icon = status ? STATUS_ICON[status] : null;

  return (
    <span className={`e2e-status e2e-is-${status ?? 'none'}`} title={label ? undefined : statusText(status)}>
      <span className="e2e-status-dot">{icon ? <Icon name={icon} size={11} /> : <span className="e2e-dash" />}</span>
      {label && statusText(status)}
    </span>
  );
}

/** The last runs of a test as a strip of marks, oldest first, so a test that comes and goes shows. */
export function History({ points, projectKey }: { points: E2eHistoryPoint[]; projectKey: string }) {
  if (points.length === 0) {
    return null;
  }

  return (
    <span className="e2e-history" aria-label={t('Recent runs, oldest first')}>
      {points.map((point) => {
        const text = `${statusText(point.status)} · ${formatDateTime(point.at)}${point.branch ? ` · ${point.branch}` : ''}`;

        return <Link key={point.runId} href={runPath(projectKey, point.runId)} className={`e2e-tick e2e-is-${point.status}`} title={text} aria-label={text} onClick={(event) => event.stopPropagation()} />;
      })}
    </span>
  );
}

/** Where a result comes from: branch, commit, pull request. */
export function Origin({ branch, commit, prNumber }: { branch: string | null; commit: string | null; prNumber?: number | null }) {
  if (!branch && !commit) {
    return null;
  }

  return (
    <span className="e2e-origin">
      {branch && (
        <span className="e2e-branch" title={branch}>
          <Icon name="branch" size={12} />
          <span>{branch}</span>
        </span>
      )}
      {commit && (
        <span className="mono" title={commit}>
          {shortCommit(commit)}
        </span>
      )}
      {prNumber && <span>#{prNumber}</span>}
    </span>
  );
}

export function IssueChips({ issues, failing, onRemove }: { issues: E2eIssueRef[]; failing?: boolean; onRemove?: (issue: E2eIssueRef) => void }) {
  return (
    <span className="e2e-issues">
      {issues.map((issue) => (
        <span key={issue.key} className="e2e-issue" title={`${issue.summary} · ${issue.status}${issue.manual ? ` · ${t('linked by hand')}` : ''}`}>
          <IssueKey issue={issue} done={issue.done} />
          {failing && issue.done && <Tag tone="red">{issue.status}</Tag>}
          {onRemove && (
            <button type="button" className="e2e-unlink" aria-label={t('Unlink {key}', { key: issue.key })} title={t('Unlink {key}', { key: issue.key })} onClick={() => onRemove(issue)}>
              <Icon name="x" size={11} />
            </button>
          )}
        </span>
      ))}
    </span>
  );
}

// ---------- a run, summed up ----------

/** What only a job's page or a session cares about: a line of output, a file that arrived. */
const QUIET = ['e2e.job', 'e2e.job.log', 'e2e.artifact'];

/** A change to the project's runs, tests or their issues; jobs have their own list. */
export function isOurs(project: Project) {
  return (event: LiveEvent) =>
    event.projectId === project.id && ((event.kind.startsWith('e2e.') && !QUIET.includes(event.kind)) || event.kind === 'issue.transitioned' || event.kind === 'issue.deleted');
}

export function jobPath(projectKey: string, jobId: number): string {
  return `/p/${projectKey}/tests/jobs/${jobId}`;
}

export function sessionPath(projectKey: string, resultId: number): string {
  return `/p/${projectKey}/tests/results/${resultId}`;
}

export function artifactUrl(artifactId: number): string {
  return `${BASE}/artifacts/${artifactId}`;
}

export function useTests(project: Project) {
  return useRemote(() => api.get<E2eProjectTests>(`${BASE}/projects/${project.key}`), [project.key], isOurs(project));
}

export function RunStatusTag({ status }: { status: E2eRunStatus }) {
  return <Tag tone={status === 'passed' ? 'green' : status === 'failed' || status === 'error' ? 'red' : 'amber'}>{t(RUN_TEXT[status])}</Tag>;
}

/** The share of each result as one bar. */
export function ResultBar({ run }: { run: E2eRun }) {
  const parts: [E2eStatus, number][] = [
    ['passed', run.passed],
    ['flaky', run.flaky],
    ['failed', run.failed],
    ['skipped', run.skipped],
  ];

  if (run.total === 0) {
    return null;
  }

  return (
    <div className="e2e-bar" role="img" aria-label={parts.map(([status, count]) => `${count} ${t(STATUS_TEXT[status])}`).join(', ')}>
      {parts
        .filter(([, count]) => count > 0)
        .map(([status, count]) => (
          <span key={status} className={`e2e-is-${status}`} style={{ flexGrow: count }} title={`${count} ${t(STATUS_TEXT[status])}`} />
        ))}
    </div>
  );
}

export function Stat({ label, value, tone }: { label: string; value: ReactNode; tone?: E2eStatus }) {
  return (
    <div className={tone ? `e2e-stat e2e-is-${tone}` : 'e2e-stat'}>
      <strong>{value}</strong>
      <span>{label}</span>
    </div>
  );
}

export function RunLine({ run }: { run: E2eRun }) {
  return (
    <span className="e2e-run-line muted">
      <Origin branch={run.branch} commit={run.commit} prNumber={run.prNumber} />
      <span title={formatDateTime(run.startedAt)}>{timeAgo(run.startedAt)}</span>
      <span>{duration(run.durationMs)}</span>
      {run.runner && <span>{run.runner}</span>}
      {run.ciUrl && (
        <a href={run.ciUrl} target="_blank" rel="noreferrer">
          CI <Icon name="external" size={12} />
        </a>
      )}
    </span>
  );
}

export function RunSummary({ run, heading, action }: { run: E2eRun; heading: string; action?: ReactNode }) {
  return (
    <section className="panel e2e-summary">
      <header className="e2e-summary-head">
        <div className="e2e-summary-title">
          <span className="muted e2e-eyebrow">{heading}</span>
          <h2>
            {run.name ?? t('Run {id}', { id: run.id })} <RunStatusTag status={run.status} />
          </h2>
          <RunLine run={run} />
        </div>
        {action}
      </header>
      {run.error && <p className="e2e-message e2e-is-failed">{run.error}</p>}
      <ResultBar run={run} />
      <div className="e2e-stats">
        <Stat label={t('passed')} value={run.passed} tone="passed" />
        <Stat label={t('failed')} value={run.failed} tone="failed" />
        <Stat label={t('flaky')} value={run.flaky} tone="flaky" />
        <Stat label={t('skipped')} value={run.skipped} tone="skipped" />
        {run.agentSteps > 0 && <Stat label={t('agent steps')} value={run.agentSteps} />}
        {run.modelCalls > 0 && <Stat label={t('model calls')} value={run.modelCalls} />}
        {run.costUsd !== null && <Stat label={t('model cost')} value={`$${run.costUsd.toFixed(run.costUsd < 0.1 ? 4 : 2)}`} />}
      </div>
    </section>
  );
}

export function Counts({ run }: { run: E2eRun }) {
  const parts: [E2eStatus, number][] = [
    ['passed', run.passed],
    ['failed', run.failed],
    ['flaky', run.flaky],
    ['skipped', run.skipped],
  ];

  return (
    <span className="e2e-counts">
      {parts
        .filter(([, count]) => count > 0)
        .map(([status, count]) => (
          <span key={status} className={`e2e-count e2e-is-${status}`} title={`${count} ${t(STATUS_TEXT[status])}`}>
            <StatusMark status={status} label={false} />
            {count}
          </span>
        ))}
    </span>
  );
}
