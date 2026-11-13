import type { Project } from '../../bindings/Project.ts';
import type { Sprint } from '../../bindings/Sprint.ts';
import type { User } from '../../bindings/User.ts';
import { t } from '../../i18n/index.ts';
import { useSession } from '../../session.tsx';
import { Avatar } from '../ui/avatar.tsx';
import { Picker, type Option } from '../ui/picker.tsx';
import { PRIORITIES, PriorityIcon } from './priority.tsx';
import { StatusPill } from './status-pill.tsx';
import { TypeIcon } from './type-icon.tsx';

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
