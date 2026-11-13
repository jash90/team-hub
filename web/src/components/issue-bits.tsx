import type { ReactNode } from 'react';

import type { IssueRow } from '../bindings/IssueRow.ts';
import type { IssueType } from '../bindings/IssueType.ts';
import type { Project } from '../bindings/Project.ts';
import type { Sprint } from '../bindings/Sprint.ts';
import type { Status } from '../bindings/Status.ts';
import type { StatusCategory } from '../bindings/StatusCategory.ts';
import type { User } from '../bindings/User.ts';
import { formatDate, formatShortDate, t } from '../i18n/index.ts';
import { useModules } from '../modules.ts';
import { Link, setParam } from '../router.tsx';
import { useSession } from '../session.tsx';
import { Icon } from './icon.tsx';
import { Avatar, Picker, type Option } from './ui.tsx';

export const PRIORITIES = [
  { value: 1, name: 'Highest' },
  { value: 2, name: 'High' },
  { value: 3, name: 'Medium' },
  { value: 4, name: 'Low' },
  { value: 5, name: 'Lowest' },
];

export function priorityName(value: number): string {
  return t(PRIORITIES.find((priority) => priority.value === value)?.name ?? 'Medium');
}

const TYPE_GLYPHS: Record<string, ReactNode> = {
  epic: <path d="M13 3 5 13h6l-1 8 8-10h-6z" fill="currentColor" stroke="none" />,
  story: <path d="M7 4h10v16l-5-4-5 4z" fill="currentColor" stroke="none" />,
  task: <path d="m6.5 12 4 4 7-8" />,
  bug: <circle cx="12" cy="12" r="5" fill="currentColor" stroke="none" />,
  subtask: (
    <>
      <rect x="4" y="4" width="8" height="8" rx="1.5" fill="currentColor" stroke="none" />
      <rect x="12" y="12" width="8" height="8" rx="1.5" />
    </>
  ),
};

export function TypeIcon({ type, size = 16 }: { type: IssueType | undefined; size?: number }) {
  const icon = type && TYPE_GLYPHS[type.icon] ? type.icon : 'task';

  return (
    <span className={`type-icon type-${icon}`} style={{ width: size, height: size }} title={type?.name}>
      <svg viewBox="0 0 24 24" width={size - 4} height={size - 4} fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
        {TYPE_GLYPHS[icon]}
      </svg>
    </span>
  );
}

const PRIORITY_PATHS: Record<number, string[]> = {
  1: ['m6 11 6-6 6 6', 'm6 18 6-6 6 6'],
  2: ['m6 15 6-6 6 6'],
  3: ['M6 9h12', 'M6 15h12'],
  4: ['m6 9 6 6 6-6'],
  5: ['m6 6 6 6 6-6', 'm6 13 6 6 6-6'],
};

export function PriorityIcon({ priority, size = 16 }: { priority: number; size?: number }) {
  return (
    <span className={`priority priority-${priority}`} title={`${t('Priority')}: ${priorityName(priority)}`}>
      <svg viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round">
        {(PRIORITY_PATHS[priority] ?? PRIORITY_PATHS[3]).map((path) => (
          <path key={path} d={path} />
        ))}
      </svg>
    </span>
  );
}

export function categoryLabel(category: StatusCategory): string {
  return category === 'todo' ? t('To do') : category === 'done' ? t('Done') : t('In progress');
}

export function StatusPill({ status }: { status: Status | undefined }) {
  return <span className={`status status-${status?.category ?? 'todo'}`}>{status?.name ?? '?'}</span>;
}

/** Opens the issue beside the current view. */
export function openIssue(key: string): void {
  setParam('issue', key);
}

export function IssueKey({ issue, done }: { issue: { key: string }; done?: boolean }) {
  return (
    <Link
      href={`/issue/${issue.key}`}
      className={done ? 'issue-key issue-key-done' : 'issue-key'}
      onClick={(event) => {
        if (event.button === 0 && !event.metaKey && !event.ctrlKey) {
          event.preventDefault();
          openIssue(issue.key);
        }
      }}
    >
      {issue.key}
    </Link>
  );
}

/** Marks from every enabled module (pull request state, drift, …). */
export function ModuleBadges({ issue }: { issue: IssueRow }) {
  const modules = useModules();

  return (
    <>
      {modules.map((module) => {
        const Badges = module.badges;

        return Badges ? <Badges key={module.id} issue={issue} /> : null;
      })}
    </>
  );
}

export function DueDate({ date, done, short }: { date: string | null; done?: boolean; short?: boolean }) {
  if (!date) {
    return null;
  }

  const overdue = !done && date < new Date().toISOString().slice(0, 10);

  return (
    <span className={overdue ? 'due due-late' : 'due'} title={t('Due date')}>
      <Icon name="calendar" size={12} />
      {short ? formatShortDate(date) : formatDate(date)}
    </span>
  );
}

type LineProps = {
  issue: IssueRow;
  project?: Project;
  /** Extra content at the end of the line. */
  trailing?: ReactNode;
  /** Leaves out labels and the epic, where the line has little room. */
  compact?: boolean;
};

/** One issue on one line: lists on the dashboard, in the backlog, under an epic. */
export function IssueLine({ issue, project, trailing, compact }: LineProps) {
  const { userById } = useSession();
  const status = project?.statuses.find((candidate) => candidate.id === issue.statusId);
  const type = project?.types.find((candidate) => candidate.id === issue.typeId);

  return (
    <div className="issue-line">
      {project && <TypeIcon type={type} />}
      <IssueKey issue={issue} done={status?.category === 'done'} />
      <span className="issue-line-summary" title={issue.summary}>
        {issue.summary}
      </span>
      <span className="issue-line-meta">
        {!compact && issue.parentKey && type?.level === 'standard' && <span className="epic-chip" title={issue.parentSummary ?? ''}>{issue.parentSummary}</span>}
        {!compact &&
          issue.labels.slice(0, 3).map((label) => (
            <span key={label} className="label">
              {label}
            </span>
          ))}
        <ModuleBadges issue={issue} />
        <DueDate date={issue.dueDate} done={status?.category === 'done'} short />
        {status && <StatusPill status={status} />}
        {issue.points !== null && <span className="points">{issue.points}</span>}
        <PriorityIcon priority={issue.priority} />
        <Avatar user={userById(issue.assigneeId)} size={22} />
        {trailing}
      </span>
    </div>
  );
}

// ---------- pickers for issue fields ----------

function userOption(user: User): Option<number | null> {
  return {
    value: user.id,
    label: user.name,
    keywords: user.email,
    render: (
      <span className="with-avatar">
        <Avatar user={user} size={20} />
        {user.name}
      </span>
    ),
  };
}

export function UserPicker({ value, onChange, variant, allowNone = true, disabled }: { value: number | null; onChange: (value: number | null) => void; variant?: 'field' | 'inline'; allowNone?: boolean; disabled?: boolean }) {
  const { boot } = useSession();
  const people = boot.users.filter((user) => user.active || user.id === value).map(userOption);
  const none: Option<number | null> = {
    value: null,
    label: t('Unassigned'),
    render: (
      <span className="with-avatar">
        <Avatar user={undefined} size={20} />
        {t('Unassigned')}
      </span>
    ),
  };

  return <Picker value={value} options={allowNone ? [none, ...people] : people} onChange={onChange} variant={variant} disabled={disabled} label={t('Assignee')} />;
}

export function StatusPicker({ project, value, onChange, disabled, allowed }: { project: Project; value: number; onChange: (value: number) => void; disabled?: boolean; allowed?: number[] }) {
  const options = project.statuses
    .filter((status) => !allowed || status.id === value || allowed.includes(status.id))
    .map((status) => ({ value: status.id, label: status.name, render: <StatusPill status={status} /> }));

  return <Picker value={value} options={options} onChange={onChange} variant="inline" disabled={disabled} label={t('Status')} />;
}

export function PriorityPicker({ value, onChange, variant, disabled }: { value: number; onChange: (value: number) => void; variant?: 'field' | 'inline'; disabled?: boolean }) {
  const options = PRIORITIES.map((priority) => ({
    value: priority.value,
    label: t(priority.name),
    render: (
      <span className="with-avatar">
        <PriorityIcon priority={priority.value} />
        {t(priority.name)}
      </span>
    ),
  }));

  return <Picker value={value} options={options} onChange={onChange} variant={variant} disabled={disabled} label={t('Priority')} />;
}

export function TypePicker({ project, value, onChange, variant, disabled, levels }: { project: Project; value: number; onChange: (value: number) => void; variant?: 'field' | 'inline'; disabled?: boolean; levels?: string[] }) {
  const options = project.types
    .filter((type) => !levels || levels.includes(type.level) || type.id === value)
    .map((type) => ({
      value: type.id,
      label: type.name,
      render: (
        <span className="with-avatar">
          <TypeIcon type={type} />
          {type.name}
        </span>
      ),
    }));

  return <Picker value={value} options={options} onChange={onChange} variant={variant} disabled={disabled} label={t('Type')} />;
}

export function sprintLabel(sprint: Sprint): string {
  return sprint.state === 'active' ? `${sprint.name} · ${t('active')}` : sprint.name;
}

export function SprintPicker({ project, value, onChange, variant, disabled }: { project: Project; value: number | null; onChange: (value: number | null) => void; variant?: 'field' | 'inline'; disabled?: boolean }) {
  const options: Option<number | null>[] = [
    { value: null, label: t('Backlog') },
    ...project.sprints.filter((sprint) => sprint.state !== 'closed' || sprint.id === value).map((sprint) => ({ value: sprint.id as number | null, label: sprintLabel(sprint) })),
  ];

  return <Picker value={value} options={options} onChange={onChange} variant={variant} disabled={disabled} label={t('Sprint')} />;
}

export function VersionPicker({ project, value, onChange, variant, disabled }: { project: Project; value: number | null; onChange: (value: number | null) => void; variant?: 'field' | 'inline'; disabled?: boolean }) {
  const options: Option<number | null>[] = [
    { value: null, label: t('None') },
    ...project.versions
      .filter((version) => (!version.archived && !version.released) || version.id === value)
      .map((version) => ({ value: version.id as number | null, label: version.released ? `${version.name} · ${t('released')}` : version.name })),
  ];

  return <Picker value={value} options={options} onChange={onChange} variant={variant} disabled={disabled} label={t('Fix version')} />;
}

export function canEdit(project: { role: string | null }): boolean {
  return project.role === 'admin' || project.role === 'member';
}
