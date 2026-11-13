import { useState } from 'react';

import { api } from '@/api.ts';
import type { E2eBadge } from '@/bindings/E2eBadge.ts';
import type { E2eDashboard } from '@/bindings/E2eDashboard.ts';
import type { E2eIssuePanel } from '@/bindings/E2eIssuePanel.ts';
import type { E2eIssueRef } from '@/bindings/E2eIssueRef.ts';
import type { E2eIssueState } from '@/bindings/E2eIssueState.ts';
import type { E2eIssueTest } from '@/bindings/E2eIssueTest.ts';
import type { E2eProjectTests } from '@/bindings/E2eProjectTests.ts';
import type { E2eTest } from '@/bindings/E2eTest.ts';
import type { Issue } from '@/bindings/Issue.ts';
import type { IssueRow } from '@/bindings/IssueRow.ts';
import type { Project } from '@/bindings/Project.ts';
import { Icon, type IconName } from '@/components/icon.tsx';
import { IssueKey } from '@/components/issue-bits.tsx';
import { Button, Empty, ErrorNote, Field, Menu, SettingsCard, Spinner, Tabs, Tag, attempt, copyText } from '@/components/ui.tsx';
import { formatDateTime, t, timeAgo } from '@/i18n/index.ts';
import { lazyView } from '@/lazy.tsx';
import type { WebModule } from '@/modules.ts';
import { Link, useSearchParams } from '@/router.tsx';
import { useProject, useStored } from '@/session.tsx';

import { Actions, Message, failureOfTest, testMenu, type Acting } from './actions.tsx';
import { JobView, JobsPanel, RunTestsButton, useJobs, useTestSettings } from './jobs.tsx';
import { polish } from './pl.ts';
import { RunView } from './run.tsx';
import { TestSettings } from './settings.tsx';
import { BASE, Counts, History, IssueChips, Origin, RunLine, RunStatusTag, RunSummary, STATE_TEXT, StatusMark, duration, runPath, useTests } from './shared.tsx';
import './styles.css';

/** The session of one result: the player, the filmstrip and the steps are fetched when one is opened. */
const SessionView = lazyView(() => import('./session.tsx').then((module) => module.SessionView));

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

  const text = `${t(STATE_TEXT[badge.state])}${parts.length > 0 ? `: ${parts.join(', ')}` : ''}`;

  return (
    <span className={`e2e-chip e2e-chip-${badge.state}`} role="img" title={text} aria-label={text}>
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

function TestsView() {
  const project = useProject();
  const tests = useTests(project);
  const jobs = useJobs(project);
  const settings = useTestSettings(project);
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
        <div className="view-head-side">
          <RunTestsButton project={project} settings={settings.data} />
        </div>
      </header>

      {tests.error && !data && <ErrorNote message={tests.error} />}
      {!data && !tests.error && <Spinner />}

      {runId === null && jobs.data && <JobsPanel project={project} jobs={jobs.data} />}

      {runId !== null ? (
        <RunView runId={runId} project={project} data={data} act={setActing} />
      ) : (
        data &&
        (data.latestRun === null ? (
          <Empty icon="check" title={t('No test run has arrived yet')}>
            {t('Start a job with “Run tests”, add the team-hub reporter to your e2e config, or post the report from CI.')}{' '}
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
      <TestSettings project={project} />
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

const e2eModule: WebModule = {
  id: 'e2e',
  projectNav: (projectKey) => [{ id: 'tests', label: () => t('Tests'), icon: 'check', path: `/p/${projectKey}/tests` }],
  routes: [
    { pattern: '/p/:key/tests', render: () => <TestsView /> },
    { pattern: '/p/:key/tests/jobs/:id', render: (params) => <JobView id={Number(params.id)} /> },
    { pattern: '/p/:key/tests/results/:id', render: (params) => <SessionView id={Number(params.id)} /> },
  ],
  badges: Badges,
  issuePanel: IssuePanel,
  dashboard: DashboardPart,
  projectSettings: [{ id: 'e2e', label: () => t('Tests'), render: (project) => <ProjectSettings project={project} /> }],
  polish,
};

export default e2eModule;