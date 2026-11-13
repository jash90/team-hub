import { useState, type FormEvent } from 'react';

import { api } from '@/api.ts';
import type { CheckState } from '@/bindings/CheckState.ts';
import type { DevSummary } from '@/bindings/DevSummary.ts';
import type { DriftKind } from '@/bindings/DriftKind.ts';
import type { GithubDelivery } from '@/bindings/GithubDelivery.ts';
import type { GithubIssuePanel } from '@/bindings/GithubIssuePanel.ts';
import type { GithubPr } from '@/bindings/GithubPr.ts';
import type { GithubProjectCode } from '@/bindings/GithubProjectCode.ts';
import type { GithubSettings } from '@/bindings/GithubSettings.ts';
import type { GithubSyncReport } from '@/bindings/GithubSyncReport.ts';
import type { GithubTrigger } from '@/bindings/GithubTrigger.ts';
import type { Issue } from '@/bindings/Issue.ts';
import type { IssueRow } from '@/bindings/IssueRow.ts';
import type { PrState } from '@/bindings/PrState.ts';
import type { Project } from '@/bindings/Project.ts';
import type { ReviewState } from '@/bindings/ReviewState.ts';
import { Icon, type IconName } from '@/components/icon.tsx';
import { IssueKey, IssueLine, canEdit } from '@/components/issue-bits.tsx';
import { Button, Dialog, Empty, ErrorNote, Field, Picker, Spinner, Tabs, Tag, attempt, copyText, toast, useBusy } from '@/components/ui.tsx';
import { countOf, formatDateTime, t, timeAgo } from '@/i18n/index.ts';
import type { WebModule } from '@/modules.ts';
import { useProject, useRemote, useSession, useStored } from '@/session.tsx';
import { SettingsCard } from '@/views/settings.tsx';

import { polish } from './pl.ts';

const DRIFT_TEXT: Record<DriftKind, string> = {
  done_with_open_pr: 'Done, but a pull request is still open',
  merged_not_done: 'Merged, but the issue is not done',
  no_code_yet: 'In progress for days with no branch or commit',
  todo_with_code: 'Code exists, but the issue has not been started',
  checks_failing: 'Checks are failing',
  changes_requested: 'Changes were requested in review',
  stale_pr: 'The pull request has gone quiet',
  closed_unmerged: 'The pull request was closed without merging',
};

const STATE_ICON: Record<PrState, IconName> = { open: 'pullRequest', merged: 'merge', closed: 'pullRequest' };

function checkMark(checks: CheckState | null): string {
  return checks === 'passing' ? '✓' : checks === 'failing' ? '✕' : checks === 'running' ? '●' : '';
}

function ChecksTag({ checks }: { checks: CheckState | null }) {
  if (!checks) {
    return null;
  }

  return <Tag tone={checks === 'passing' ? 'green' : checks === 'failing' ? 'red' : 'amber'}>{checks === 'passing' ? t('checks pass') : checks === 'failing' ? t('checks fail') : t('checks running')}</Tag>;
}

function ReviewTag({ review }: { review: ReviewState | null }) {
  if (!review || review === 'pending') {
    return null;
  }

  return <Tag tone={review === 'approved' ? 'green' : 'amber'}>{review === 'approved' ? t('approved') : t('changes requested')}</Tag>;
}

function DriftList({ kinds }: { kinds: DriftKind[] }) {
  return (
    <>
      {kinds.map((kind) => (
        <span key={kind} className="drift-note">
          <Icon name="alert" size={14} />
          {t(DRIFT_TEXT[kind])}
        </span>
      ))}
    </>
  );
}

/** Marks on cards and rows: each pull request with its state, and a warning when status and code disagree. */
function Badges({ issue }: { issue: IssueRow }) {
  const dev = issue.ext.github as DevSummary | undefined;

  if (!dev) {
    return null;
  }

  return (
    <>
      {dev.prs.slice(0, 2).map((pr) => (
        <a
          key={`${pr.repo}#${pr.number}`}
          className={`pr-chip pr-${pr.draft && pr.state === 'open' ? 'draft' : pr.state} ${pr.checks === 'failing' ? 'pr-failing' : ''}`}
          href={pr.url}
          target="_blank"
          rel="noreferrer"
          title={`${pr.repo} #${pr.number}: ${pr.title}`}
          onClick={(event) => event.stopPropagation()}
        >
          <Icon name={STATE_ICON[pr.state]} size={12} />
          {pr.number}
          {pr.state === 'open' && <span className="pr-check">{checkMark(pr.checks)}</span>}
        </a>
      ))}
      {dev.prs.length === 0 && dev.branches > 0 && (
        <span className="pr-chip pr-branch" title={t('Has a branch')}>
          <Icon name="branch" size={12} />
        </span>
      )}
      {dev.drift.length > 0 && (
        <span className="drift-mark" title={dev.drift.map((kind) => t(DRIFT_TEXT[kind])).join('\n')}>
          <Icon name="alert" size={13} />
        </span>
      )}
    </>
  );
}

function PullRow({ pr, showIssues }: { pr: GithubPr; showIssues?: boolean }) {
  return (
    <div className="pr-row">
      <span className={`pr-state pr-${pr.draft && pr.state === 'open' ? 'draft' : pr.state}`}>
        <Icon name={STATE_ICON[pr.state]} size={14} />
        {pr.draft && pr.state === 'open' ? t('draft') : t(pr.state)}
      </span>
      <a href={pr.url} target="_blank" rel="noreferrer" className="pr-title">
        {pr.title}
      </a>
      <span className="muted pr-ref">
        {pr.repo}#{pr.number}
        {pr.author && ` · ${pr.author}`}
      </span>
      <span className="pr-tags">
        {showIssues && pr.issueKeys.length === 0 && <Tag tone="amber">{t('no issue')}</Tag>}
        {showIssues && pr.issueKeys.map((key) => <IssueKey key={key} issue={{ key }} />)}
        {pr.state === 'open' && <ReviewTag review={pr.review} />}
        {pr.state === 'open' && <ChecksTag checks={pr.checks} />}
        <span className="muted">{timeAgo(pr.mergedAt ?? pr.updatedAt)}</span>
      </span>
    </div>
  );
}

function IssuePanel({ issue, reload }: { issue: Issue; reload: () => void }) {
  const panel = issue.panels.github as GithubIssuePanel | undefined;
  const [creating, setCreating] = useState(false);

  if (!panel) {
    return null;
  }

  const nothing = panel.prs.length === 0 && panel.branches.length === 0 && panel.commits.length === 0;
  const command = `git checkout -b ${panel.suggestedBranch}`;

  return (
    <section className="issue-section">
      <div className="section-head">
        <h3>{t('Development')}</h3>
        <span className="filter-spacer" />
        {panel.canCreateBranch ? (
          <Button size="sm" variant="ghost" icon="branch" onClick={() => setCreating(true)}>
            {t('Create branch')}
          </Button>
        ) : (
          <Button size="sm" variant="ghost" icon="copy" onClick={() => void copyText(command)} title={command}>
            {t('Copy branch command')}
          </Button>
        )}
      </div>
      {panel.drift.length > 0 && (
        <div className="drift-box">
          <DriftList kinds={panel.drift} />
        </div>
      )}
      {nothing && (
        <p className="muted">
          {t('Nothing linked yet. Put {key} in a branch name, commit message or pull request title and it shows up here.', { key: issue.key })}
        </p>
      )}
      {panel.prs.map((pr) => (
        <PullRow key={pr.id} pr={pr} />
      ))}
      {panel.branches.map((branch) => (
        <div key={branch.id} className="pr-row">
          <span className="pr-state pr-branch">
            <Icon name="branch" size={14} />
            {t('branch')}
          </span>
          <a href={branch.url} target="_blank" rel="noreferrer" className="pr-title mono">
            {branch.name}
          </a>
          <span className="muted pr-ref">{branch.repo}</span>
          <span className="pr-tags">
            <Button size="sm" variant="ghost" icon="copy" aria-label={t('Copy checkout command')} title={`git checkout ${branch.name}`} onClick={() => void copyText(`git fetch && git checkout ${branch.name}`)} />
          </span>
        </div>
      ))}
      {panel.commits.length > 0 && (
        <details className="commits">
          <summary>
            {countOf(panel.commits.length, 'commit', 'commits')}
          </summary>
          {panel.commits.map((commit) => (
            <div key={commit.id} className="pr-row">
              <a href={commit.url} target="_blank" rel="noreferrer" className="mono">
                {commit.sha.slice(0, 7)}
              </a>
              <span className="pr-title">{commit.message.split('\n')[0]}</span>
              <span className="pr-tags muted">
                {commit.authorLogin ?? commit.authorName} · {timeAgo(commit.committedAt)}
              </span>
            </div>
          ))}
        </details>
      )}
      {creating && <BranchDialog issue={issue} panel={panel} onClose={() => setCreating(false)} onDone={reload} />}
    </section>
  );
}

function BranchDialog({ issue, panel, onClose, onDone }: { issue: Issue; panel: GithubIssuePanel; onClose: () => void; onDone: () => void }) {
  const [repoId, setRepoId] = useState(panel.repos[0]?.id ?? 0);
  const [name, setName] = useState(panel.suggestedBranch);
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      await api.post(`/api/m/github/issues/${issue.key}/branch`, { repoId, name });
      onDone();
      onClose();
    }, t('Branch created'));
  }

  return (
    <Dialog
      title={t('Create branch for {key}', { key: issue.key })}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="branch-form" disabled={busy || !name.trim()}>
            {t('Create branch')}
          </Button>
        </>
      }
    >
      <form id="branch-form" className="form" onSubmit={submit}>
        <Field label={t('Repository')}>
          <Picker value={repoId} options={panel.repos.map((repo) => ({ value: repo.id, label: repo.fullName }))} onChange={setRepoId} />
        </Field>
        <Field label={t('Branch name')} hint={t('Created on GitHub from the tip of the default branch.')}>
          <input className="input input-mono" required value={name} onChange={(event) => setName(event.target.value)} />
        </Field>
      </form>
    </Dialog>
  );
}

function useCode(project: Project) {
  return useRemote(
    () => api.get<GithubProjectCode>(`/api/m/github/projects/${project.key}`),
    [project.key],
    (event) => event.projectId === project.id && (event.kind.startsWith('github.') || event.kind === 'issue.transitioned'),
  );
}

function CodeView() {
  const project = useProject();
  const code = useCode(project);
  const [tab, setTab] = useStored(`code-tab:${project.key}`, 'open');
  const [syncing, setSyncing] = useState<number | null>(null);

  if (code.error && !code.data) {
    return <ErrorNote message={code.error} />;
  }

  if (!code.data) {
    return <Spinner />;
  }

  const data = code.data;
  const open = data.pulls.filter((pr) => pr.state === 'open');
  const unlinked = data.pulls.filter((pr) => pr.issueKeys.length === 0 && pr.state === 'open');
  const shown = tab === 'open' ? open : tab === 'unlinked' ? unlinked : data.pulls;

  async function sync(repoId: number): Promise<void> {
    setSyncing(repoId);
    const report = await attempt(() => api.post<GithubSyncReport>(`/api/m/github/repos/${repoId}/sync`, {}));
    setSyncing(null);

    if (report) {
      toast(t('Synced: {pulls} pull requests, {branches} branches, {commits} new commits', { pulls: report.pulls, branches: report.branches, commits: report.commits }));
    }

    await code.reload();
  }

  return (
    <div className="view">
      <header className="view-head">
        <div>
          <h1>{t('Code')}</h1>
          <p className="muted">{t('Pull requests of this project, and where issue statuses and code disagree.')}</p>
        </div>
      </header>

      {data.repos.length === 0 ? (
        <Empty icon="branch" title={t('No repository connected')}>
          {t('Connect one in Project settings → GitHub; branches, commits and pull requests that mention an issue key then appear on the issue.')}
        </Empty>
      ) : (
        <>
          <div className="repo-cards">
            {data.repos.map((repo) => (
              <div key={repo.id} className="panel repo-card">
                <span className="repo-name">
                  <Icon name="branch" />
                  <a href={`https://github.com/${repo.fullName}`} target="_blank" rel="noreferrer">
                    {repo.fullName}
                  </a>
                </span>
                <span className="muted">
                  {repo.openPrs} {t('open pull requests')} · {repo.branches} {t('branches')}
                </span>
                <span className="muted">
                  {repo.lastEventAt ? t('last event {when}', { when: timeAgo(repo.lastEventAt) }) : t('no webhook events yet')}
                  {repo.lastSyncedAt && ` · ${t('synced {when}', { when: timeAgo(repo.lastSyncedAt) })}`}
                </span>
                {repo.syncError && <span className="drift-note">{repo.syncError}</span>}
                {data.tokenConfigured && canEdit(project) && (
                  <Button size="sm" icon="refresh" disabled={syncing !== null} onClick={() => void sync(repo.id)}>
                    {syncing === repo.id ? t('Syncing…') : t('Sync now')}
                  </Button>
                )}
              </div>
            ))}
          </div>

          <section className="panel">
            <header className="panel-head">
              <h2>{t('Status and code disagree')}</h2>
              <span className="tab-count">{data.drift.length}</span>
            </header>
            {data.drift.length === 0 ? (
              <p className="muted panel-empty">{t('Every issue agrees with its code.')}</p>
            ) : (
              data.drift.map((item) => (
                <div key={item.issue.id} className="drift-row">
                  <IssueLine issue={item.issue} project={project} />
                  <div className="drift-kinds">
                    <DriftList kinds={item.kinds} />
                  </div>
                </div>
              ))
            )}
          </section>

          <section className="panel">
            <header className="panel-head">
              <h2>{t('Pull requests')}</h2>
            </header>
            <Tabs
              tabs={[
                { id: 'open', label: t('Open'), count: open.length },
                { id: 'unlinked', label: t('Without an issue'), count: unlinked.length },
                { id: 'all', label: t('Last 30 days'), count: data.pulls.length },
              ]}
              active={tab}
              onChange={setTab}
            />
            {shown.length === 0 ? <p className="muted panel-empty">{t('Nothing here.')}</p> : shown.map((pr) => <PullRow key={pr.id} pr={pr} showIssues />)}
          </section>
        </>
      )}
    </div>
  );
}

const TRIGGER_TEXT: Record<GithubTrigger, string> = {
  branch_created: 'A branch is created',
  pr_opened: 'A pull request is opened (or leaves draft)',
  changes_requested: 'A review requests changes',
  pr_approved: 'A review approves',
  pr_merged: 'The last open pull request is merged',
};

function ProjectSettings({ project }: { project: Project }) {
  const code = useCode(project);
  const [name, setName] = useState('');
  const [busy, run] = useBusy();
  const admin = project.role === 'admin';

  if (!code.data) {
    return code.error ? <ErrorNote message={code.error} /> : <Spinner />;
  }

  const data = code.data;
  const base = `/api/m/github/projects/${project.key}`;

  return (
    <fieldset className="settings-body" disabled={!admin}>
      <SettingsCard title={t('Repositories')} lead={t('Branches, commits and pull requests of these repositories are linked to issues by key: {key}-12 in a branch name, a commit message or a pull request title.', { key: project.key })}>
        <table className="table table-plain">
          <tbody>
            {data.repos.map((repo) => (
              <tr key={repo.id}>
                <td>
                  <span className="with-avatar">
                    <Icon name="branch" />
                    <strong>{repo.fullName}</strong>
                  </span>
                </td>
                <td className="muted">
                  {repo.lastEventAt ? t('last event {when}', { when: timeAgo(repo.lastEventAt) }) : t('no webhook events yet')}
                  {repo.syncError && <span className="drift-note"> {repo.syncError}</span>}
                </td>
                <td className="cell-actions">
                  <Button size="sm" variant="ghost" icon="trash" aria-label={t('Disconnect')} onClick={() => void attempt(() => api.del(`/api/m/github/repos/${repo.id}`)).then(() => code.reload())} />
                </td>
              </tr>
            ))}
            {data.repos.length === 0 && (
              <tr>
                <td className="muted">{t('No repository connected yet.')}</td>
              </tr>
            )}
          </tbody>
        </table>
        <form
          className="inline-form"
          onSubmit={(event) => {
            event.preventDefault();
            void run(async () => {
              code.set(await api.post<GithubProjectCode>(`${base}/repos`, { fullName: name }));
              setName('');
            }, t('Repository connected'));
          }}
        >
          <input className="input input-mono" placeholder="owner/repository" value={name} onChange={(event) => setName(event.target.value)} />
          <Button type="submit" icon="plus" disabled={busy || !name.includes('/')}>
            {t('Connect')}
          </Button>
        </form>
        <p className="muted settings-lead">
          {data.webhookConfigured ? t('Webhook address for the repository settings on GitHub:') : t('No webhook secret is set yet (an administrator sets it in Settings → GitHub). Webhook address:')} <code>{data.webhookUrl}</code>
        </p>
      </SettingsCard>

      <SettingsCard title={t('Workflow rules')} lead={t('What happens to an issue when something happens to its code. Finished work is never reopened, and a merge finishes an issue only when none of its pull requests is still open.')}>
        <table className="table table-plain">
          <tbody>
            {data.rules.map((rule) => (
              <tr key={rule.trigger}>
                <td>{t(TRIGGER_TEXT[rule.trigger])}</td>
                <td>
                  <Picker
                    value={rule.statusId}
                    options={[{ value: null as number | null, label: t('Do nothing') }, ...project.statuses.map((status) => ({ value: status.id as number | null, label: `→ ${status.name}` }))]}
                    onChange={(statusId) =>
                      void attempt(() =>
                        api.put<GithubProjectCode>(
                          `${base}/rules`,
                          data.rules.map((existing) => (existing.trigger === rule.trigger ? { ...existing, statusId } : existing)),
                        ),
                      ).then((updated) => updated && code.set(updated))
                    }
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <p className="muted settings-lead">
          {t('Commit messages can act too:')} <code>{project.key}-12 #comment fixed the redirect #time 1h 30m #in-review</code>. {t('A pull request that says “Fixes {key}-12” finishes the issue when merged, even without a rule.', { key: project.key })}
        </p>
      </SettingsCard>
    </fieldset>
  );
}

function AdminSettings() {
  const settings = useRemote(() => api.get<GithubSettings>('/api/m/github/settings'), []);
  const deliveries = useRemote(() => api.get<GithubDelivery[]>('/api/m/github/deliveries'), []);
  const [token, setToken] = useState('');
  const [apiUrl, setApiUrl] = useState('');
  const [secret, setSecret] = useState<string | null>(null);
  const [busy, run] = useBusy();

  if (!settings.data) {
    return settings.error ? <ErrorNote message={settings.error} /> : <Spinner />;
  }

  const data = settings.data;

  return (
    <>
      <SettingsCard title={t('Webhook')} lead={t('GitHub tells team-hub about pushes, pull requests, reviews and checks as they happen. Add a webhook to each repository (or to the organisation) with this address, content type application/json, the secret below, and the events: pushes, branch creation and deletion, pull requests, pull request reviews, check suites, statuses.')}>
        <div className="form">
          <Field label={t('Payload URL')}>
            <span className="copy-row">
              <code className="code-block">{data.webhookUrl}</code>
              <Button icon="copy" aria-label={t('Copy')} onClick={() => void copyText(data.webhookUrl)} />
            </span>
          </Field>
          <Field label={t('Secret')} hint={t('Stored encrypted. It is shown once, when generated.')}>
            <span className="copy-row">
              <code className="code-block">{data.webhookSecretHint ?? t('not set')}</code>
              <Button
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    const made = await api.post<{ secret: string }>('/api/m/github/settings/webhook-secret', {});
                    setSecret(made.secret);
                    await settings.reload();
                  })
                }
              >
                {data.webhookSecretHint ? t('Generate a new one') : t('Generate')}
              </Button>
            </span>
          </Field>
        </div>
      </SettingsCard>

      <SettingsCard title={t('API access')} lead={t('Optional. With a token team-hub can catch up on what webhooks missed and create branches from issues. A fine-grained token with read access to pull requests, contents, checks and commit statuses is enough; creating branches needs write access to contents.')}>
        <form
          className="form"
          onSubmit={(event) => {
            event.preventDefault();
            void run(async () => {
              settings.set(await api.put<GithubSettings>('/api/m/github/settings', { token, apiUrl }));
              setToken('');
              setApiUrl('');
            }, t('Saved'));
          }}
        >
          <Field label={t('Token')} hint={data.tokenHint ? t('A token ending in {hint} is stored. Leave blank to keep it.', { hint: data.tokenHint }) : t('No token is stored.')}>
            <input className="input input-mono" type="password" autoComplete="off" value={token} onChange={(event) => setToken(event.target.value)} placeholder="github_pat_…" />
          </Field>
          <Field label={t('API address')} hint={t('Only for GitHub Enterprise Server. Currently {url}', { url: data.apiUrl })}>
            <input className="input input-mono" value={apiUrl} onChange={(event) => setApiUrl(event.target.value)} placeholder="https://github.example.com/api/v3" />
          </Field>
          <div className="inline-form">
            <Button variant="primary" type="submit" disabled={busy || (!token.trim() && !apiUrl.trim())}>
              {t('Save')}
            </Button>
            {data.tokenHint && (
              <Button variant="danger" disabled={busy} onClick={() => void run(async () => settings.set(await api.put<GithubSettings>('/api/m/github/settings', { clearToken: true })), t('Token removed'))}>
                {t('Remove token')}
              </Button>
            )}
          </div>
        </form>
      </SettingsCard>

      <SettingsCard
        title={t('Recent deliveries')}
        action={
          <Button size="sm" icon="refresh" onClick={() => void deliveries.reload()}>
            {t('Refresh')}
          </Button>
        }
      >
        {deliveries.data?.length === 0 && <p className="muted">{t('Nothing has arrived yet.')}</p>}
        <table className="table table-plain">
          <tbody>
            {deliveries.data?.map((delivery) => (
              <tr key={delivery.id}>
                <td className="muted cell-date">{formatDateTime(delivery.receivedAt)}</td>
                <td>
                  <code>{delivery.event}</code>
                </td>
                <td className="muted">{delivery.repo}</td>
                <td>
                  {delivery.outcome}
                  {delivery.detail && <span className="muted"> · {delivery.detail}</span>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </SettingsCard>

      {secret && (
        <Dialog
          title={t('Webhook secret')}
          onClose={() => setSecret(null)}
          footer={
            <>
              <Button icon="copy" onClick={() => void copyText(secret)}>
                {t('Copy')}
              </Button>
              <Button variant="primary" onClick={() => setSecret(null)}>
                {t('Done')}
              </Button>
            </>
          }
        >
          <div className="form">
            <p>{t('Paste it into the “Secret” field of the webhook on GitHub. It is shown only this once.')}</p>
            <code className="code-block">{secret}</code>
          </div>
        </Dialog>
      )}
    </>
  );
}

type DashboardData = { pulls: GithubPr[]; drift: { key: string; summary: string; kinds: DriftKind[] }[] };

function DashboardPart({ data }: { data: unknown }) {
  const { pulls, drift } = data as DashboardData;
  const { user } = useSession();

  return (
    <>
      {drift.length > 0 && (
        <section className="panel">
          <header className="panel-head">
            <h2>{t('Status and code disagree')}</h2>
            <span className="tab-count">{drift.length}</span>
          </header>
          {drift.map((item) => (
            <div key={item.key} className="drift-row">
              <div className="issue-line">
                <IssueKey issue={item} />
                <span className="issue-line-summary">{item.summary}</span>
              </div>
              <div className="drift-kinds">
                <DriftList kinds={item.kinds} />
              </div>
            </div>
          ))}
        </section>
      )}
      {pulls.length > 0 && (
        <section className="panel" data-user={user.id}>
          <header className="panel-head">
            <h2>{t('My pull requests')}</h2>
            <span className="tab-count">{pulls.length}</span>
          </header>
          {pulls.map((pr) => (
            <PullRow key={pr.id} pr={pr} showIssues />
          ))}
        </section>
      )}
    </>
  );
}

export const githubModule: WebModule = {
  id: 'github',
  projectNav: (projectKey) => [{ id: 'code', label: () => t('Code'), icon: 'branch', path: `/p/${projectKey}/code` }],
  routes: [{ pattern: '/p/:key/code', render: () => <CodeView /> }],
  badges: Badges,
  issuePanel: IssuePanel,
  dashboard: DashboardPart,
  projectSettings: [{ id: 'github', label: () => 'GitHub', render: (project) => <ProjectSettings project={project} /> }],
  settings: [{ id: 'github', label: () => 'GitHub', adminOnly: true, render: () => <AdminSettings /> }],
  polish,
};
