import { useMemo, type ReactNode } from 'react';

import { api } from '../api.ts';
import type { ActivityItem } from '../bindings/ActivityItem.ts';
import type { Dashboard } from '../bindings/Dashboard.ts';
import type { IssueRow } from '../bindings/IssueRow.ts';
import type { Project } from '../bindings/Project.ts';
import { IssueKey, IssueLine } from '../components/issue-bits.tsx';
import { Avatar, Empty, ErrorNote, ProgressBar, Spinner } from '../components/ui.tsx';
import { countOf, formatDate, t, timeAgo } from '../i18n/index.ts';
import { useModules } from '../modules.ts';
import { Link } from '../router.tsx';
import { changesIssues, useRemote, useSession } from '../session.tsx';

/** The configurations of the projects whose issues are on screen. */
function useProjectsOf(issues: IssueRow[]): Map<number, Project> {
  const { boot } = useSession();
  const keys = useMemo(
    () =>
      Array.from(new Set(issues.map((issue) => issue.projectId)))
        .map((id) => boot.projects.find((project) => project.id === id)?.key)
        .filter((key): key is string => Boolean(key))
        .sort(),
    [issues, boot.projects],
  );
  const loaded = useRemote(() => Promise.all(keys.map((key) => api.get<Project>(`/api/projects/${key}`))), [keys.join(',')]);

  return useMemo(() => new Map((loaded.data ?? []).map((project) => [project.id, project])), [loaded.data]);
}

function Section({ title, count, children }: { title: string; count?: number; children: ReactNode }) {
  return (
    <section className="panel">
      <header className="panel-head">
        <h2>{title}</h2>
        {count !== undefined && <span className="tab-count">{count}</span>}
      </header>
      {children}
    </section>
  );
}

const EVENT_TEXT: Record<string, string> = {
  'issue.created': 'created',
  'issue.updated': 'updated',
  'issue.transitioned': 'moved',
  'issue.deleted': 'deleted',
  'comment.created': 'commented on',
  'worklog.added': 'logged work on',
  'sprint.started': 'started sprint',
  'sprint.completed': 'completed sprint',
  'sprint.created': 'created sprint',
  'version.released': 'released version',
  'version.created': 'created version',
  'project.created': 'created project',
  'github.pr': 'pull request',
  'page.created': 'created page',
  'page.updated': 'edited page',
};

/** What people did to a scheduled task; its runs are announced the same way and are not news. */
const TASK_TEXT: Record<string, string> = {
  created: 'added to the schedule:',
  changed: 'changed in the schedule:',
  deleted: 'removed from the schedule:',
};

function eventText(item: ActivityItem): string | undefined {
  return item.kind === 'task.updated' ? TASK_TEXT[String((item.payload as Record<string, unknown> | null)?.change ?? '')] : EVENT_TEXT[item.kind];
}

export function ActivityFeed({ items }: { items: ActivityItem[] }) {
  const { userById } = useSession();

  return (
    <div className="feed">
      {items
        .filter((item) => eventText(item))
        .map((item) => {
          const person = userById(item.actorId);
          const payload = item.payload as Record<string, string | number | null>;

          return (
            <div key={item.id} className="feed-row">
              <Avatar user={person} size={22} label={person?.name ?? item.actorLabel ?? undefined} />
              <span className="feed-text">
                <strong>{person?.name ?? item.actorLabel ?? t('Someone')}</strong> {t(eventText(item) ?? '')}{' '}
                {item.kind === 'github.pr' && (
                  <>
                    <a href={String(payload.url ?? '#')} target="_blank" rel="noreferrer">
                      #{payload.number}
                    </a>{' '}
                    {t(String(payload.action ?? ''))}{' '}
                  </>
                )}
                {item.issueKey ? (
                  <>
                    <IssueKey issue={{ key: item.issueKey }} /> <span className="muted">{item.issueSummary}</span>
                  </>
                ) : (
                  <span>{String(payload.name ?? payload.title ?? payload.key ?? '')}</span>
                )}
                {item.kind === 'issue.transitioned' && payload.to && <span className="muted"> → {payload.to}</span>}
              </span>
              <span className="muted feed-time">{timeAgo(item.at)}</span>
            </div>
          );
        })}
    </div>
  );
}

export function DashboardView() {
  const { user, boot } = useSession();
  const modules = useModules();
  const dashboard = useRemote(
    () => api.get<Dashboard>('/api/dashboard'),
    [],
    changesIssues,
  );
  const activity = useRemote(
    () => api.get<ActivityItem[]>('/api/activity?limit=40'),
    [],
    () => true,
  );
  const data = dashboard.data;
  const issues = useMemo(() => (data ? [...data.assigned, ...data.reported, ...data.watching, ...data.recentlyDone] : []), [data]);
  const projects = useProjectsOf(issues);

  if (dashboard.error && !data) {
    return <ErrorNote message={dashboard.error} />;
  }

  if (!data) {
    return <Spinner />;
  }

  const category = (issue: IssueRow) => projects.get(issue.projectId)?.statuses.find((status) => status.id === issue.statusId)?.category;
  const inProgress = data.assigned.filter((issue) => category(issue) === 'in_progress');
  const todo = data.assigned.filter((issue) => category(issue) !== 'in_progress');
  const lines = (list: IssueRow[]) => list.map((issue) => <IssueLine key={issue.id} issue={issue} project={projects.get(issue.projectId)} />);

  return (
    <div className="view">
      <header className="view-head">
        <div>
          <h1>{t('Hello, {name}', { name: user.name.split(' ')[0] })}</h1>
          <p className="muted">
            {countOf(data.assigned.length, 'open issue', 'open issues')} {t('assigned to you')}
          </p>
        </div>
      </header>

      {boot.projects.length === 0 && (
        <Empty icon="folder" title={t('No projects yet')}>
          <Link href="/projects">{t('Create the first project')}</Link>
        </Empty>
      )}

      {data.activeSprints.length > 0 && (
        <div className="sprint-cards">
          {data.activeSprints.map((entry) => (
            <Link key={entry.sprint.id} href={`/p/${entry.projectKey}/board`} className="sprint-card">
              <span className="sprint-card-head">
                <span className="project-badge">{entry.projectKey}</span>
                <strong>{entry.sprint.name}</strong>
              </span>
              <ProgressBar done={entry.done} total={entry.total} />
              <span className="muted">
                {entry.done}/{entry.total} {t('issues')} · {entry.pointsDone}/{entry.points} {t('pts')} · {t('ends {when}', { when: formatDate(entry.sprint.endDate) })}
              </span>
            </Link>
          ))}
        </div>
      )}

      <div className="dashboard-grid">
        <div className="dashboard-main">
          <Section title={t('In progress')} count={inProgress.length}>
            {inProgress.length === 0 ? <p className="muted panel-empty">{t('Nothing in progress. Pick something from below.')}</p> : lines(inProgress)}
          </Section>
          <Section title={t('To do')} count={todo.length}>
            {todo.length === 0 ? <p className="muted panel-empty">{t('Nothing waiting for you.')}</p> : lines(todo)}
          </Section>
          {modules.map((module) => {
            const Part = module.dashboard;

            return Part && data.ext[module.id] !== undefined ? <Part key={module.id} data={data.ext[module.id]} /> : null;
          })}
          {data.reported.length > 0 && (
            <Section title={t('Reported by me')} count={data.reported.length}>
              {lines(data.reported)}
            </Section>
          )}
          {data.watching.length > 0 && (
            <Section title={t('Watching')} count={data.watching.length}>
              {lines(data.watching)}
            </Section>
          )}
          {data.recentlyDone.length > 0 && (
            <Section title={t('Recently done')} count={data.recentlyDone.length}>
              {lines(data.recentlyDone)}
            </Section>
          )}
        </div>
        <div className="dashboard-side">
          <Section title={t('Activity')}>{activity.data ? activity.data.length > 0 ? <ActivityFeed items={activity.data} /> : <p className="muted panel-empty">{t('Nothing has happened yet.')}</p> : <Spinner />}</Section>
        </div>
      </div>
    </div>
  );
}
