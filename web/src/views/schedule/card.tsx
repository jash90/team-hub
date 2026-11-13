import { useState, type ReactNode } from 'react';

import { api } from '../../api.ts';
import type { Project } from '../../bindings/Project.ts';
import type { ScheduledTask } from '../../bindings/ScheduledTask.ts';
import type { TaskKind } from '../../bindings/TaskKind.ts';
import { Icon } from '../../components/icon.tsx';
import { canEdit } from '../../components/issue-bits.tsx';
import { Button, ConfirmDialog, Menu, Tag, attempt, useBusy } from '../../components/ui.tsx';
import { formatDateTime, t, timeAgo } from '../../i18n/index.ts';
import { translateServer } from '../../i18n/server.ts';
import { Link } from '../../router.tsx';
import { useSession } from '../../session.tsx';
import { atMoment, fromMoment } from './times.ts';

/** What the task does, in one line: the values of its kind's form, in the form's order. */
function settingsLine(task: ScheduledTask, kind: TaskKind | undefined): { parts: ReactNode[]; text: string } {
  const parts: ReactNode[] = [];
  const words: string[] = [];
  const add = (text: string, node: ReactNode = text) => {
    parts.push(<span key={parts.length}>{node}</span>);
    words.push(text);
  };

  for (const field of kind?.fields ?? []) {
    const value = task.config[field.name];
    const label = translateServer(field.label);

    if (field.type === 'query' && typeof value === 'string' && value) {
      add(value, <code>{value}</code>);
    } else if (field.type === 'select' && typeof value === 'string' && value) {
      add(translateServer(field.options.find((option) => option.value === value)?.label ?? value));
    } else if (field.type === 'number' && typeof value === 'number') {
      add(`${label}: ${value}`);
    } else if (field.type === 'boolean' && value === true) {
      add(label);
    } else if (field.type === 'text' && typeof value === 'string' && value) {
      add(`${label}: ${value}`);
    }
  }

  return { parts, text: words.join(' · ') };
}

/** Why a task cannot run: its module is off for everyone, off in this project, or no longer there. */
function unavailableReason(task: ScheduledTask, project: Project, modules: { id: string; name: string; enabled: boolean }[]): string {
  const module = modules.find((candidate) => task.kind.startsWith(`${candidate.id}.`));

  if (!module) {
    return t('This kind of task no longer exists, so the task does not run.');
  }

  const name = translateServer(module.name);

  if (!module.enabled) {
    return t('The “{name}” module is switched off, so this task does not run.', { name });
  }

  if (project.disabledModules.includes(module.id)) {
    return t('The “{name}” module is switched off in this project, so this task does not run.', { name });
  }

  return t('This kind of task no longer exists, so the task does not run.');
}

/** The last result, and above it what went wrong since, if anything did. */
function Outcome({ task, now }: { task: ScheduledTask; now: Date }) {
  const failed = task.state === 'failed' && task.lastFinishedAt !== null;
  const reason = translateServer(task.lastError ?? '');

  return (
    <div className="task-outcome">
      {task.state === 'running' && (
        <p className="task-running">
          <span className="spinner" />
          <span>
            {t('Running…')} {task.progress && <span>{translateServer(task.progress)} </span>}
            <span className="muted">{t('started {when}', { when: timeAgo(task.lastStartedAt) })}</span>
          </span>
        </p>
      )}
      {failed && task.lastFinishedAt && (
        <p className={task.result ? 'note task-stale' : 'note note-error task-stale'} role="status">
          <Icon name="alert" />
          <span>
            {task.result
              ? t('Showing the result {from} — the attempt {at} failed: {reason}', { from: fromMoment(task.result.at, now), at: atMoment(task.lastFinishedAt, now), reason })
              : t('The attempt {at} failed: {reason}', { at: atMoment(task.lastFinishedAt, now), reason })}
          </span>
        </p>
      )}
      {task.result && (
        <p className="task-result">
          <Icon name={failed ? 'clock' : 'check'} className={failed ? 'task-result-old' : 'task-result-ok'} />
          <span className="task-result-text">
            {translateServer(task.result.summary)}{' '}
            {task.result.link && (
              <Link href={task.result.link} className="task-result-link">
                {t('Open')}
              </Link>
            )}
          </span>
          <span className="muted task-result-when" title={formatDateTime(task.result.at)}>
            {timeAgo(task.result.at)}
          </span>
        </p>
      )}
      {!task.result && task.state === 'idle' && <p className="muted">{t('Not run yet.')}</p>}
    </div>
  );
}

type TaskCardProps = {
  task: ScheduledTask;
  kind: TaskKind | undefined;
  project: Project;
  now: Date;
  onEdit: () => void;
  onChanged: () => Promise<void>;
};

/** One task of the schedule: what it is, how its last run went, and the buttons for it. */
export function TaskCard({ task, kind, project, now, onEdit, onChanged }: TaskCardProps) {
  const { boot, userById } = useSession();
  const [busy, run] = useBusy();
  const [deleting, setDeleting] = useState(false);
  const administrator = project.role === 'admin';
  const line = settingsLine(task, kind);
  const person = userById(task.runAs);

  return (
    <article className={task.kindAvailable ? 'panel task-card' : 'panel task-card task-card-off'} data-state={task.state}>
      <header className="task-head">
        <h3>{task.name}</h3>
        <label className="switch" title={task.enabled ? t('Runs with its group') : t('Left out when its group runs')}>
          <input
            type="checkbox"
            checked={task.enabled}
            disabled={!administrator}
            aria-label={t('Run “{name}” with its group', { name: task.name })}
            onChange={(event) => void attempt(() => api.patch(`/api/tasks/${task.id}`, { enabled: event.target.checked })).then(onChanged)}
          />
          <span />
        </label>
        {administrator && (
          <Menu
            label={t('Task actions')}
            items={[
              { label: t('Edit'), icon: 'edit', disabled: !task.kindAvailable, onSelect: onEdit },
              { label: t('Delete'), icon: 'trash', danger: true, onSelect: () => setDeleting(true) },
            ]}
          />
        )}
      </header>
      <p className="task-what" title={line.text}>
        <Tag>{kind ? translateServer(kind.name) : task.kind}</Tag>
        <span className="task-settings muted">{line.parts}</span>
      </p>
      {!task.kindAvailable && (
        <p className="note task-stale">
          <Icon name="info" />
          <span>{unavailableReason(task, project, boot.modules)}</span>
        </p>
      )}
      <Outcome task={task} now={now} />
      <footer className="task-foot">
        <span className="muted task-runs-as">{person ? t('runs as {name}', { name: person.name }) : ''}</span>
        {canEdit(project) && (
          <Button
            size="sm"
            icon="play"
            disabled={busy || task.state === 'running' || !task.kindAvailable}
            onClick={() => void run(() => api.post(`/api/tasks/${task.id}/run`)).then(onChanged)}
          >
            {t('Run now')}
          </Button>
        )}
      </footer>
      {deleting && (
        <ConfirmDialog
          title={t('Delete {name}?', { name: task.name })}
          body={t('The task leaves the schedule. What it started so far stays as it is.')}
          confirm={t('Delete')}
          danger
          onClose={() => setDeleting(false)}
          onConfirm={async () => {
            await api.del(`/api/tasks/${task.id}`);
            await onChanged();
          }}
        />
      )}
    </article>
  );
}
