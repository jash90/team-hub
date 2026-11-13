import { useState, type FormEvent, type ReactNode } from 'react';

import { api } from '@/api.ts';
import type { E2eBadge } from '@/bindings/E2eBadge.ts';
import type { E2eDashboard } from '@/bindings/E2eDashboard.ts';
import type { E2eHistoryPoint } from '@/bindings/E2eHistoryPoint.ts';
import type { E2eIssuePanel } from '@/bindings/E2eIssuePanel.ts';
import type { E2eIssueRef } from '@/bindings/E2eIssueRef.ts';
import type { E2eIssueState } from '@/bindings/E2eIssueState.ts';
import type { E2eIssueTest } from '@/bindings/E2eIssueTest.ts';
import type { E2eProjectTests } from '@/bindings/E2eProjectTests.ts';
import type { E2eRun } from '@/bindings/E2eRun.ts';
import type { E2eRunDetail } from '@/bindings/E2eRunDetail.ts';
import type { E2eRunResult } from '@/bindings/E2eRunResult.ts';
import type { E2eRunStatus } from '@/bindings/E2eRunStatus.ts';
import type { E2eStatus } from '@/bindings/E2eStatus.ts';
import type { E2eTest } from '@/bindings/E2eTest.ts';
import type { Issue } from '@/bindings/Issue.ts';
import type { IssueRow } from '@/bindings/IssueRow.ts';
import type { Project } from '@/bindings/Project.ts';
import { Icon, type IconName } from '@/components/icon.tsx';
import { IssueKey, openIssue } from '@/components/issue-bits.tsx';
import { Button, ConfirmDialog, Dialog, Empty, ErrorNote, Field, Menu, Spinner, Tabs, Tag, attempt, copyText, toast, useBusy, type MenuItem } from '@/components/ui.tsx';
import { formatDateTime, t, timeAgo } from '@/i18n/index.ts';
import type { WebModule } from '@/modules.ts';
import { Link, setParam, useSearchParams } from '@/router.tsx';
import { useProject, useRemote, useStored, type LiveEvent } from '@/session.tsx';
import { SettingsCard } from '@/views/settings.tsx';

import { polish } from './pl.ts';
import './styles.css';

const BASE = '/api/m/e2e';

const STATUS_TEXT: Record<E2eStatus, string> = { passed: 'passed', failed: 'failed', flaky: 'flaky', skipped: 'skipped' };
const STATUS_ICON: Record<E2eStatus, IconName | null> = { passed: 'check', failed: 'x', flaky: 'refresh', skipped: null };
const RUN_TEXT: Record<E2eRunStatus, string> = { passed: 'passed', failed: 'failed', error: 'error', interrupted: 'interrupted', blocked: 'blocked' };
const STATE_TEXT: Record<E2eIssueState, string> = { passing: 'Tests pass', failing: 'Tests fail', flaky: 'Tests are flaky', skipped: 'Tests have not run' };

function statusText(status: E2eStatus | null): string {
  return status ? t(STATUS_TEXT[status]) : t('not run yet');
}

/** 320 ms, 4.2 s, 1 min 12 s. */
function duration(ms: number | null): string {
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

function shortCommit(commit: string | null): string {
  return commit ? commit.slice(0, 7) : '';
}

function runPath(projectKey: string, runId: number): string {
  return `/p/${projectKey}/tests?run=${runId}`;
}

function StatusMark({ status, label = true }: { status: E2eStatus | null; label?: boolean }) {
  const icon = status ? STATUS_ICON[status] : null;

  return (
    <span className={`e2e-status e2e-is-${status ?? 'none'}`} title={label ? undefined : statusText(status)}>
      <span className="e2e-status-dot">{icon ? <Icon name={icon} size={11} /> : <span className="e2e-dash" />}</span>
      {label && statusText(status)}
    </span>
  );
}

/** The last runs of a test as a strip of marks, oldest first, so a test that comes and goes shows. */
function History({ points, projectKey }: { points: E2eHistoryPoint[]; projectKey: string }) {
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
function Origin({ branch, commit, prNumber }: { branch: string | null; commit: string | null; prNumber?: number | null }) {
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

function IssueChips({ issues, failing, onRemove }: { issues: E2eIssueRef[]; failing?: boolean; onRemove?: (issue: E2eIssueRef) => void }) {
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

// ---------- marks on cards and rows ----------

function Badges({ issue }: { issue: IssueRow }) {
  const badge = issue.ext.e2e as E2eBadge | undefined;

  if (!badge) {
    return null;
  }

  const icon: IconName = badge.state === 'passing' ? 'check' : badge.state === 'failing' ? 'x' : badge.state === 'flaky' ? 'refresh' : 'clock';
  const parts = [
    badge.failing > 0 ? t('{count} failing', { count: badge.failing }) : '',
    badge.flaky > 0 ? t('{count} flaky', { count: badge.flaky }) : '',
    badge.passing > 0 ? t('{count} passing', { count: badge.passing }) : '',
  ].filter(Boolean);

  return (
    <span className={`e2e-chip e2e-chip-${badge.state}`} title={`${t(STATE_TEXT[badge.state])}${parts.length > 0 ? `: ${parts.join(', ')}` : ''}`}>
      <Icon name={icon} size={11} />
      {badge.state === 'failing' ? `${badge.failing}/${badge.total}` : badge.total}
    </span>
  );
}

// ---------- the panel on an issue ----------

function StateTag({ state }: { state: E2eIssueState }) {
  return <Tag tone={state === 'passing' ? 'green' : state === 'failing' ? 'red' : state === 'flaky' ? 'amber' : 'neutral'}>{t(STATE_TEXT[state])}</Tag>;
}

function IssueTestRow({ test }: { test: E2eIssueTest }) {
  const title = test.runId ? <Link href={runPath(test.projectKey, test.runId)}>{test.title}</Link> : test.title;

  return (
    <div className="e2e-issue-test">
      <StatusMark status={test.status} label={false} />
      <div className="e2e-issue-test-main">
        <span className="e2e-title">{title}</span>
        <span className="muted e2e-sub">
          <span className="mono">
            {test.file}
            {test.line ? `:${test.line}` : ''}
          </span>
          {test.manual && <span>· {t('linked by hand')}</span>}
        </span>
        {test.status !== 'passed' && test.message && <span className={`e2e-oneline e2e-is-${test.status ?? 'none'}`}>{test.message.split('\n')[0]}</span>}
      </div>
      <div className="e2e-issue-test-side">
        <History points={test.history} projectKey={test.projectKey} />
        <span className="muted e2e-sub">
          <Origin branch={test.branch} commit={test.commit} />
          {test.at && <span title={formatDateTime(test.at)}>{timeAgo(test.at)}</span>}
          {test.ciUrl && (
            <a href={test.ciUrl} target="_blank" rel="noreferrer" title={t('Open the CI run')} aria-label={t('Open the CI run')}>
              <Icon name="external" size={12} />
            </a>
          )}
        </span>
      </div>
    </div>
  );
}

function IssuePanel({ issue }: { issue: Issue; reload: () => void }) {
  const panel = issue.panels.e2e as E2eIssuePanel | undefined;

  if (!panel) {
    return null;
  }

  return (
    <section className="issue-section">
      <div className="section-head">
        <h3>{t('Tests')}</h3>
        {panel.state && <StateTag state={panel.state} />}
        <span className="filter-spacer" />
        <Link href={`/p/${issue.projectKey}/tests`} className="e2e-more">
          {t('All tests')}
        </Link>
      </div>
      {panel.tests.length === 0 ? (
        <p className="muted">{t('No test verifies this issue yet. Put {key} in a test’s title or tags and its results show up here.', { key: issue.key })}</p>
      ) : (
        <div className="e2e-issue-tests">
          {panel.tests.map((test) => (
            <IssueTestRow key={test.testId} test={test} />
          ))}
        </div>
      )}
    </section>
  );
}

// ---------- the project page ----------

function isOurs(project: Project) {
  return (event: LiveEvent) => event.projectId === project.id && (event.kind.startsWith('e2e.') || event.kind === 'issue.transitioned' || event.kind === 'issue.deleted');
}

function useTests(project: Project) {
  return useRemote(() => api.get<E2eProjectTests>(`${BASE}/projects/${project.key}`), [project.key], isOurs(project));
}

function RunStatusTag({ status }: { status: E2eRunStatus }) {
  return <Tag tone={status === 'passed' ? 'green' : status === 'failed' || status === 'error' ? 'red' : 'amber'}>{t(RUN_TEXT[status])}</Tag>;
}

/** The share of each result as one bar. */
function ResultBar({ run }: { run: E2eRun }) {
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

function Stat({ label, value, tone }: { label: string; value: ReactNode; tone?: E2eStatus }) {
  return (
    <div className={tone ? `e2e-stat e2e-is-${tone}` : 'e2e-stat'}>
      <strong>{value}</strong>
      <span>{label}</span>
    </div>
  );
}

function RunLine({ run }: { run: E2eRun }) {
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

function RunSummary({ run, heading, action }: { run: E2eRun; heading: string; action?: ReactNode }) {
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

/** What "Create bug" and the dialogs need to know about a failure, from a test or from a run's result. */
type Failure = {
  testId: number;
  title: string;
  file: string;
  line: number | null;
  message: string | null;
  runId: number | null;
  branch: string | null;
  commit: string | null;
  issues: E2eIssueRef[];
};

function bugDescription(project: Project, failure: Failure): string {
  const where = `${failure.file}${failure.line ? `:${failure.line}` : ''}`;
  const lines = [t('The end-to-end test **{title}** fails.', { title: failure.title }), '', `- ${t('File')}: \`${where}\``];

  if (failure.runId) {
    const origin = [failure.branch ? `${t('branch')} \`${failure.branch}\`` : '', failure.commit ? `commit \`${shortCommit(failure.commit)}\`` : ''].filter(Boolean).join(', ');

    lines.push(`- ${t('Run')}: [${t('Run {id}', { id: failure.runId })}](${runPath(project.key, failure.runId)})${origin ? ` (${origin})` : ''}`);
  }

  if (failure.issues.length > 0) {
    lines.push(`- ${t('Verifies')}: ${failure.issues.map((issue) => issue.key).join(', ')}`);
  }

  if (failure.message) {
    lines.push('', '```', failure.message.replace(/```/g, "'''"), '```');
  }

  return lines.join('\n');
}

function BugDialog({ project, failure, bugTypeId, onClose, onDone }: { project: Project; failure: Failure; bugTypeId: number | null; onClose: () => void; onDone: () => void }) {
  const [summary, setSummary] = useState(() => t('Failing test: {title}', { title: failure.title.split(' > ').pop() ?? failure.title }).slice(0, 200));
  const [description, setDescription] = useState(() => bugDescription(project, failure));
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      const created = await api.post<IssueRow>('/api/issues', { projectKey: project.key, summary, description, ...(bugTypeId ? { typeId: bugTypeId } : {}) });
      await api.put(`${BASE}/tests/${failure.testId}/issues/${created.key}`, {});
      toast(t('Created {key} and linked the test to it', { key: created.key }));
      onDone();
      onClose();
      openIssue(created.key);
    });
  }

  return (
    <Dialog
      title={bugTypeId ? t('Create bug') : t('Create issue')}
      wide
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="e2e-bug-form" disabled={busy || !summary.trim()}>
            {bugTypeId ? t('Create bug') : t('Create issue')}
          </Button>
        </>
      }
    >
      <form id="e2e-bug-form" className="form" onSubmit={submit}>
        <Field label={t('Summary')}>
          <input className="input" required maxLength={250} value={summary} onChange={(event) => setSummary(event.target.value)} />
        </Field>
        <Field label={t('Description')} hint={t('The test is linked to the new issue, so its next results show up there.')}>
          <textarea className="input input-mono e2e-textarea" rows={12} value={description} onChange={(event) => setDescription(event.target.value)} />
        </Field>
      </form>
    </Dialog>
  );
}

function LinkDialog({ project, failure, onClose, onDone }: { project: Project; failure: Failure; onClose: () => void; onDone: () => void }) {
  const [key, setKey] = useState('');
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      await api.put(`${BASE}/tests/${failure.testId}/issues/${encodeURIComponent(key.trim())}`, {});
      onDone();
      onClose();
    }, t('Test linked'));
  }

  return (
    <Dialog
      title={t('Link test to an issue')}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="e2e-link-form" disabled={busy || !key.trim()}>
            {t('Link')}
          </Button>
        </>
      }
    >
      <form id="e2e-link-form" className="form" onSubmit={submit}>
        <p className="e2e-title">{failure.title}</p>
        <Field label={t('Issue key')} hint={t('A link made here stays through later runs. Tests are also linked on their own when a title or tag names an issue, like {key}-12.', { key: project.key })}>
          <input className="input input-mono" required placeholder={`${project.key}-12`} value={key} onChange={(event) => setKey(event.target.value)} />
        </Field>
      </form>
    </Dialog>
  );
}

type Acting = { kind: 'bug' | 'link' | 'forget'; failure: Failure };

function failureOfTest(test: E2eTest): Failure {
  return { testId: test.id, title: test.title, file: test.file, line: test.line, message: test.message, runId: test.lastRunId, branch: test.branch, commit: test.commit, issues: test.issues };
}

function failureOfResult(result: E2eRunResult, run: E2eRun): Failure {
  return { testId: result.testId, title: result.title, file: result.file, line: result.line, message: result.message, runId: run.id, branch: run.branch, commit: run.commit, issues: result.issues };
}

function testMenu(failure: Failure, failed: boolean, bugTypeId: number | null, act: (acting: Acting) => void): MenuItem[] {
  return [
    ...(failed ? [{ label: bugTypeId ? t('Create bug') : t('Create issue'), icon: 'plus' as IconName, onSelect: () => act({ kind: 'bug', failure }) }] : []),
    { label: t('Link to an issue…'), icon: 'link', onSelect: () => act({ kind: 'link', failure }) },
    { label: t('Forget this test'), icon: 'trash', danger: true, onSelect: () => act({ kind: 'forget', failure }) },
  ];
}

function Actions({ project, acting, bugTypeId, onClose, onDone }: { project: Project; acting: Acting | null; bugTypeId: number | null; onClose: () => void; onDone: () => void }) {
  if (!acting) {
    return null;
  }

  if (acting.kind === 'bug') {
    return <BugDialog project={project} failure={acting.failure} bugTypeId={bugTypeId} onClose={onClose} onDone={onDone} />;
  }

  if (acting.kind === 'link') {
    return <LinkDialog project={project} failure={acting.failure} onClose={onClose} onDone={onDone} />;
  }

  return (
    <ConfirmDialog
      title={t('Forget this test?')}
      body={t('“{title}” and its results are removed, with its links to issues. If a later run still has the test, it comes back.', { title: acting.failure.title })}
      confirm={t('Forget test')}
      danger
      onClose={onClose}
      onConfirm={async () => {
        await attempt(() => api.del(`${BASE}/tests/${acting.failure.testId}`));
        onDone();
        onClose();
      }}
    />
  );
}

function Message({ text, status }: { text: string; status: E2eStatus | null }) {
  return <pre className={`e2e-message e2e-is-${status ?? 'none'}`}>{text}</pre>;
}

function TestRow({ test, data, project, act, reload }: { test: E2eTest; data: E2eProjectTests; project: Project; act: (acting: Acting) => void; reload: () => void }) {
  const [open, setOpen] = useState(false);
  const failed = test.status === 'failed' || test.status === 'flaky';
  const failure = failureOfTest(test);
  const hasMore = Boolean(test.message);

  async function unlink(issue: E2eIssueRef): Promise<void> {
    await attempt(() => api.del(`${BASE}/tests/${test.id}/issues/${issue.key}`), t('Unlinked {key}', { key: issue.key }));
    reload();
  }

  return (
    <div className={`e2e-test e2e-is-${test.status ?? 'none'}`}>
      <div className="e2e-test-status">
        <StatusMark status={test.status} />
        {test.unstable && test.status !== 'flaky' && <Tag tone="amber" title={t('Passed and failed by turns in its recent runs')}>{t('unstable')}</Tag>}
      </div>
      <div className="e2e-test-main">
        {hasMore ? (
          <button type="button" className="e2e-title e2e-toggle" aria-expanded={open} onClick={() => setOpen(!open)}>
            <Icon name={open ? 'chevronDown' : 'chevronRight'} size={13} />
            <span>{test.title}</span>
          </button>
        ) : (
          <span className="e2e-title">{test.title}</span>
        )}
        <span className="muted e2e-sub">
          <span className="mono">
            {test.file}
            {test.line ? `:${test.line}` : ''}
          </span>
          {test.tags.map((tag) => (
            <span key={tag} className="e2e-tag">
              {tag}
            </span>
          ))}
        </span>
        {failed && test.message && !open && <span className={`e2e-oneline e2e-is-${test.status ?? 'none'}`}>{test.message.split('\n')[0]}</span>}
        {open && test.message && <Message text={test.message} status={test.status} />}
      </div>
      <div className="e2e-test-issues">
        {test.issues.length === 0 ? <Tag tone="amber">{t('no issue')}</Tag> : <IssueChips issues={test.issues} failing={test.status === 'failed'} onRemove={data.canEdit ? (issue) => void unlink(issue) : undefined} />}
      </div>
      <div className="e2e-test-history">
        <History points={test.history} projectKey={project.key} />
      </div>
      <div className="e2e-test-last muted e2e-sub">
        {test.lastAt && test.lastRunId ? (
          <>
            <Link href={runPath(project.key, test.lastRunId)} title={formatDateTime(test.lastAt)}>
              {timeAgo(test.lastAt)}
            </Link>
            <Origin branch={test.branch} commit={null} />
            {test.status !== 'skipped' && (
              <span>
                {duration(test.durationMs)}
                {test.attempts !== null && test.attempts > 1 ? ` · ${t('{count} attempts', { count: test.attempts })}` : ''}
              </span>
            )}
          </>
        ) : (
          t('not run yet')
        )}
      </div>
      <div className="e2e-test-actions">{data.canEdit && <Menu label={t('Actions')} items={testMenu(failure, failed, data.bugTypeId, act)} />}</div>
    </div>
  );
}

type Filter = 'all' | 'failing' | 'flaky' | 'unlinked';

function matches(test: E2eTest, filter: Filter, words: string[]): boolean {
  const wanted = filter === 'all' || (filter === 'failing' && test.status === 'failed') || (filter === 'flaky' && test.unstable) || (filter === 'unlinked' && test.issues.length === 0);

  if (!wanted) {
    return false;
  }

  const text = `${test.title} ${test.file} ${test.tags.join(' ')} ${test.issues.map((issue) => issue.key).join(' ')}`.toLowerCase();

  return words.every((word) => text.includes(word));
}

function TestList({ data, project, act, reload }: { data: E2eProjectTests; project: Project; act: (acting: Acting) => void; reload: () => void }) {
  const [filter, setFilter] = useStored<Filter>(`e2e-filter:${project.key}`, 'all');
  const [search, setSearch] = useState('');
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  const shown = data.tests.filter((test) => matches(test, filter, words));

  return (
    <section className="panel">
      <div className="e2e-toolbar">
        <Tabs
          tabs={[
            { id: 'all', label: t('All'), count: data.counts.total },
            { id: 'failing', label: t('Failing'), count: data.counts.failing },
            { id: 'flaky', label: t('Flaky'), count: data.counts.flaky },
            { id: 'unlinked', label: t('Without an issue'), count: data.counts.unlinked },
          ]}
          active={filter}
          onChange={(id) => setFilter(id as Filter)}
        />
        <div className="filter-search e2e-search">
          <Icon name="search" />
          <input className="input" type="search" placeholder={t('Filter tests')} aria-label={t('Filter tests')} value={search} onChange={(event) => setSearch(event.target.value)} />
        </div>
      </div>
      {shown.length === 0 ? (
        <p className="muted panel-empty">{filter === 'failing' && words.length === 0 ? t('No test is failing.') : t('No test matches.')}</p>
      ) : (
        <div className="e2e-tests">
          <div className="e2e-test e2e-test-head" aria-hidden="true">
            <span>{t('Result')}</span>
            <span>{t('Test')}</span>
            <span>{t('Verifies')}</span>
            <span>{t('Recent runs')}</span>
            <span>{t('Last run')}</span>
            <span />
          </div>
          {shown.map((test) => (
            <TestRow key={test.id} test={test} data={data} project={project} act={act} reload={reload} />
          ))}
        </div>
      )}
    </section>
  );
}

function Counts({ run }: { run: E2eRun }) {
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

function RunList({ data }: { data: E2eProjectTests }) {
  return (
    <section className="panel">
      <div className="e2e-runs">
        {data.runs.map((run) => (
          <Link key={run.id} href={runPath(data.projectKey, run.id)} className="e2e-run">
            <span className="e2e-run-name">
              <RunStatusTag status={run.status} />
              <strong>{run.name ?? t('Run {id}', { id: run.id })}</strong>
            </span>
            <Counts run={run} />
            <RunLine run={run} />
          </Link>
        ))}
      </div>
    </section>
  );
}

function ResultCard({ result, run, data, act }: { result: E2eRunResult; run: E2eRun; data: E2eProjectTests | undefined; act: (acting: Acting) => void }) {
  const failure = failureOfResult(result, run);

  return (
    <article className={`e2e-result e2e-is-${result.status}`}>
      <header className="e2e-result-head">
        <StatusMark status={result.status} />
        <span className="e2e-title">{result.title}</span>
        {result.variant && <Tag>{result.variant}</Tag>}
        <span className="filter-spacer" />
        {data?.canEdit && result.status === 'failed' && (
          <Button size="sm" icon="plus" onClick={() => act({ kind: 'bug', failure })}>
            {data.bugTypeId ? t('Create bug') : t('Create issue')}
          </Button>
        )}
      </header>
      <span className="muted e2e-sub">
        <span className="mono">
          {result.file}
          {result.line ? `:${result.line}` : ''}
        </span>
        <span>{duration(result.durationMs)}</span>
        {result.attempts > 1 && <span>{t('{count} attempts', { count: result.attempts })}</span>}
        {result.errorCode && <span className="mono">{result.errorCode}</span>}
        {result.agentSteps > 0 && <span>{t('{count} agent steps', { count: result.agentSteps })}</span>}
        {result.costUsd !== null && <span>${result.costUsd.toFixed(4)}</span>}
      </span>
      {result.message && <Message text={result.message} status={result.status} />}
      {result.issues.length > 0 && (
        <span className="e2e-sub">
          <span className="muted">{t('Verifies')}</span>
          <IssueChips issues={result.issues} failing={result.status === 'failed'} />
        </span>
      )}
    </article>
  );
}

function RunView({ runId, project, data, act }: { runId: number; project: Project; data: E2eProjectTests | undefined; act: (acting: Acting) => void }) {
  const detail = useRemote(() => api.get<E2eRunDetail>(`${BASE}/runs/${runId}`), [runId], isOurs(project));
  const [removing, setRemoving] = useState(false);
  const back = (
    <Button icon="chevronUp" onClick={() => setParam('run', null)}>
      {t('All runs')}
    </Button>
  );

  if (!detail.data) {
    return detail.error ? (
      <>
        <ErrorNote message={detail.error} />
        <div>{back}</div>
      </>
    ) : (
      <Spinner />
    );
  }

  const { run, results } = detail.data;
  const troubled = results.filter((result) => result.status === 'failed' || result.status === 'flaky');
  const quiet = results.filter((result) => result.status === 'passed' || result.status === 'skipped');

  return (
    <>
      <RunSummary
        run={run}
        heading={t('Run')}
        action={
          <span className="e2e-summary-actions">
            {back}
            {data?.canAdmin && <Button variant="ghost" icon="trash" aria-label={t('Remove this run')} title={t('Remove this run')} onClick={() => setRemoving(true)} />}
          </span>
        }
      />
      <section className="panel">
        <header className="panel-head">
          <h2>{t('Failures')}</h2>
          <span className="tab-count">{troubled.length}</span>
        </header>
        {troubled.length === 0 ? (
          <p className="muted panel-empty">{t('Nothing failed in this run.')}</p>
        ) : (
          <div className="e2e-results">
            {troubled.map((result) => (
              <ResultCard key={result.id} result={result} run={run} data={data} act={act} />
            ))}
          </div>
        )}
      </section>
      {quiet.length > 0 && (
        <section className="panel">
          <header className="panel-head">
            <h2>{t('Other tests')}</h2>
            <span className="tab-count">{quiet.length}</span>
          </header>
          <div className="e2e-quiet">
            {quiet.map((result) => (
              <div key={result.id} className="e2e-quiet-row">
                <StatusMark status={result.status} label={false} />
                <span className="e2e-title" title={result.file}>
                  {result.title}
                  {result.variant && <span className="muted"> · {result.variant}</span>}
                  {result.status === 'skipped' && result.message && <span className="muted"> — {result.message.split('\n')[0]}</span>}
                </span>
                <IssueChips issues={result.issues} />
                <span className="muted e2e-quiet-time">{result.status === 'skipped' ? t('skipped') : duration(result.durationMs)}</span>
              </div>
            ))}
          </div>
        </section>
      )}
      {removing && (
        <ConfirmDialog
          title={t('Remove this run?')}
          body={t('Its results are removed. Tests fall back to what the run before said.')}
          confirm={t('Remove run')}
          danger
          onClose={() => setRemoving(false)}
          onConfirm={async () => {
            await attempt(() => api.del(`${BASE}/runs/${run.id}`));
            setRemoving(false);
            setParam('run', null);
          }}
        />
      )}
    </>
  );
}

function TestsView() {
  const project = useProject();
  const tests = useTests(project);
  const params = useSearchParams();
  const runId = Number(params.get('run') ?? '') || null;
  const [tab, setTab] = useStored(`e2e-tab:${project.key}`, 'tests');
  const [acting, setActing] = useState<Acting | null>(null);
  const data = tests.data;
  const reload = () => void tests.reload();

  return (
    <div className="view e2e-view">
      <header className="view-head">
        <div>
          <h1>{t('Tests')}</h1>
          <p className="muted">{t('End-to-end test runs of this project, and whether the behaviour each issue describes actually works.')}</p>
        </div>
      </header>

      {tests.error && !data && <ErrorNote message={tests.error} />}
      {!data && !tests.error && <Spinner />}

      {runId !== null ? (
        <RunView runId={runId} project={project} data={data} act={setActing} />
      ) : (
        data &&
        (data.latestRun === null ? (
          <Empty icon="check" title={t('No test run has arrived yet')}>
            {t('Add the team-hub reporter to your e2e config, or post the report from CI.')}{' '}
            <Link href={`/p/${project.key}/settings/e2e`}>{t('How to send results')}</Link>
          </Empty>
        ) : (
          <>
            <RunSummary
              run={data.latestRun}
              heading={t('Latest run')}
              action={
                <Link href={runPath(project.key, data.latestRun.id)} className="e2e-more">
                  {t('Open run')}
                </Link>
              }
            />
            <Tabs
              tabs={[
                { id: 'tests', label: t('Tests'), count: data.counts.total },
                { id: 'runs', label: t('Runs'), count: data.runs.length },
              ]}
              active={tab}
              onChange={setTab}
            />
            {tab === 'runs' ? <RunList data={data} /> : <TestList data={data} project={project} act={setActing} reload={reload} />}
          </>
        ))
      )}

      <Actions project={project} acting={acting} bugTypeId={data?.bugTypeId ?? null} onClose={() => setActing(null)} onDone={reload} />
    </div>
  );
}

// ---------- project settings: how to send results ----------

function Snippet({ title, code }: { title: string; code: string }) {
  return (
    <div className="e2e-snippet">
      <div className="e2e-snippet-head">
        <strong>{title}</strong>
        <Button size="sm" variant="ghost" icon="copy" onClick={() => void copyText(code)}>
          {t('Copy')}
        </Button>
      </div>
      <pre className="e2e-code">{code}</pre>
    </div>
  );
}

function ProjectSettings({ project }: { project: Project }) {
  const tests = useTests(project);

  if (!tests.data) {
    return tests.error ? <ErrorNote message={tests.error} /> : <Spinner />;
  }

  const { baseUrl, ingestUrl, latestRun } = tests.data;
  const config = `import type { E2EConfig } from 'e2e';
import { web } from '@e2e-dev/web';
import { teamHub } from 'team-hub-e2e-reporter';

export default {
  targets: [{ engine: web(), app: { url: process.env.APP_URL ?? 'http://localhost:3000' } }],
  reporters: [
    'list',
    'junit',
    teamHub({ url: '${baseUrl}', project: '${project.key}', token: process.env.TEAM_HUB_TOKEN }),
  ],
} satisfies E2EConfig;`;
  const actions = `- name: End-to-end tests
  run: npx e2e run
  env:
    TEAM_HUB_TOKEN: \${{ secrets.TEAM_HUB_TOKEN }}`;
  const curlJson = `curl -fsS -X POST "${ingestUrl}?branch=$GITHUB_REF_NAME&commit=$GITHUB_SHA" \\
  -H "Authorization: Bearer $TEAM_HUB_TOKEN" \\
  -H "Content-Type: application/json" \\
  --data-binary @.e2e/report.json`;
  const curlJunit = `curl -fsS -X POST "${ingestUrl}?runId=$GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT&branch=$GITHUB_REF_NAME&commit=$GITHUB_SHA" \\
  -H "Authorization: Bearer $TEAM_HUB_TOKEN" \\
  -H "Content-Type: application/xml" \\
  --data-binary @.e2e/junit.xml`;
  const example = `test('${project.key}-12 a member upgrades to Pro', async ({ app, screen }) => { … });

describe('${project.key}-12 billing', { tags: ['@${project.key}-31'] }, () => { … });`;

  return (
    <div className="settings-body">
      <SettingsCard
        title={t('Send test results')}
        lead={t('Runs of the e2e testing framework arrive here as reports. Each needs an API token of somebody who may edit this project: make one in Settings → API tokens and keep it in your CI secrets as TEAM_HUB_TOKEN.')}
      >
        <Field label={t('Address for reports')}>
          <span className="copy-row">
            <code className="code-block">{ingestUrl}</code>
            <Button icon="copy" aria-label={t('Copy')} onClick={() => void copyText(ingestUrl)} />
          </span>
        </Field>
        <p className="muted settings-lead">
          {latestRun ? t('Last run received {when}.', { when: timeAgo(latestRun.createdAt) }) : t('No run has been received yet.')} <Link href="/settings/tokens">{t('API tokens')}</Link>
        </p>
        <Snippet title={t('With the reporter: e2e.config.ts')} code={config} />
        <p className="muted settings-lead">{t('Install it beside the framework with npm install --save-dev team-hub-e2e-reporter. It posts the finished run with its branch, commit and pull request, which it reads from the CI environment.')}</p>
        <Snippet title={t('GitHub Actions step')} code={actions} />
        <Snippet title={t('Without the reporter: post the JSON report')} code={curlJson} />
        <Snippet title={t('Or the JUnit file')} code={curlJunit} />
        <p className="muted settings-lead">{t('Optional query parameters: name, branch, commit, pr, ciUrl, and runId, which makes a repeated post of the same run harmless. The JSON report carries its own run id.')}</p>
      </SettingsCard>
      <SettingsCard
        title={t('Linking tests to issues')}
        lead={t('A test verifies the issues its title, its describe titles, its tags or its annotations name. A link can also be made by hand on the Tests page; it stays through later runs.')}
      >
        <pre className="e2e-code">{example}</pre>
        <p className="muted settings-lead">
          {t('In queries:')} <code>tests = failing</code>, <code>category = done AND tests in (failing, flaky)</code>, <code>tests is empty</code>.
        </p>
      </SettingsCard>
    </div>
  );
}

// ---------- "My work" ----------

function DashboardPart({ data }: { data: unknown }) {
  const { issues } = data as E2eDashboard;

  if (issues.length === 0) {
    return null;
  }

  return (
    <section className="panel">
      <header className="panel-head">
        <h2>{t('My issues with failing tests')}</h2>
        <span className="tab-count">{issues.length}</span>
      </header>
      {issues.map((issue) => (
        <div key={issue.key} className="e2e-dash-row">
          <div className="issue-line">
            <IssueKey issue={issue} done={issue.done} />
            <span className="issue-line-summary" title={issue.summary}>
              {issue.summary}
            </span>
            <span className="issue-line-meta">
              <Tag tone={issue.done ? 'red' : 'neutral'} title={issue.done ? t('Done, but a test that verifies it fails') : undefined}>
                {issue.status}
              </Tag>
            </span>
          </div>
          <div className="e2e-dash-tests">
            {issue.tests.map((test) => (
              <IssueTestRow key={test.testId} test={test} />
            ))}
          </div>
        </div>
      ))}
    </section>
  );
}

export const e2eModule: WebModule = {
  id: 'e2e',
  projectNav: (projectKey) => [{ id: 'tests', label: () => t('Tests'), icon: 'check', path: `/p/${projectKey}/tests` }],
  routes: [{ pattern: '/p/:key/tests', render: () => <TestsView /> }],
  badges: Badges,
  issuePanel: IssuePanel,
  dashboard: DashboardPart,
  projectSettings: [{ id: 'e2e', label: () => t('Tests'), render: (project) => <ProjectSettings project={project} /> }],
  polish,
};
