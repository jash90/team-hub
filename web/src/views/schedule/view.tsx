import { useState } from 'react';

import { api } from '../../api.ts';
import type { ScheduleSettings } from '../../bindings/ScheduleSettings.ts';
import type { ScheduledTask } from '../../bindings/ScheduledTask.ts';
import type { TaskKind } from '../../bindings/TaskKind.ts';
import type { TaskSlot } from '../../bindings/TaskSlot.ts';
import { canEdit } from '../../components/issue-bits.tsx';
import { Button, Empty, ErrorNote, Spinner, toast, useBusy } from '../../components/ui.tsx';
import { countOf, t } from '../../i18n/index.ts';
import { useProject, useRemote, useSession } from '../../session.tsx';
import { TaskCard } from './card.tsx';
import { SettingsDialog } from './settings-dialog.tsx';
import { TaskDialog } from './task-dialog.tsx';
import { browserZone, nextRun, useNow } from './times.ts';

type Open = { kind: 'new' } | { kind: 'edit'; task: ScheduledTask } | { kind: 'settings' };

const SLOTS: TaskSlot[] = ['morning', 'afternoon', 'on_demand'];

function slotTitle(slot: TaskSlot): string {
  return slot === 'morning' ? t('Morning') : slot === 'afternoon' ? t('After work') : t('On demand');
}

type GroupProps = {
  slot: TaskSlot;
  tasks: ScheduledTask[];
  settings: ScheduleSettings | undefined;
  kinds: TaskKind[];
  now: Date;
  onEdit: (task: ScheduledTask) => void;
  onStarted: (started: ScheduledTask[]) => void;
  onChanged: () => Promise<void>;
};

/** One of the three groups: when it runs, the button that runs it now, and its tasks. */
function Group({ slot, tasks, settings, kinds, now, onEdit, onStarted, onChanged }: GroupProps) {
  const project = useProject();
  const [busy, run] = useBusy();
  const startable = tasks.filter((task) => task.enabled && task.kindAvailable && task.state !== 'running');
  const next = settings && slot !== 'on_demand' ? nextRun(settings, slot, now) : null;

  async function runGroup(): Promise<void> {
    const started = await api.post<ScheduledTask[]>(`/api/projects/${project.key}/tasks/run`, { slot });

    toast(started.length > 0 ? t('Started: {tasks}', { tasks: countOf(started.length, 'task', 'tasks') }) : t('Nothing to start'));
    onStarted(started);
  }

  return (
    <section className="schedule-group" data-slot={slot}>
      <header className="schedule-group-head">
        <h2>{slotTitle(slot)}</h2>
        {settings && slot !== 'on_demand' && <span className="schedule-time">{settings[slot]}</span>}
        <span className="muted schedule-next">{slot === 'on_demand' ? t('runs only when asked') : next}</span>
        <span className="filter-spacer" />
        {canEdit(project) && (
          <Button size="sm" icon="play" disabled={busy || startable.length === 0} onClick={() => void run(runGroup)}>
            {t('Run this group now')}
          </Button>
        )}
      </header>
      {tasks.length === 0 ? (
        <p className="muted schedule-group-empty">{t('Nothing in this group.')}</p>
      ) : (
        <div className="schedule-cards">
          {tasks.map((task) => (
            <TaskCard key={task.id} task={task} kind={kinds.find((kind) => kind.id === task.kind)} project={project} now={now} onEdit={() => onEdit(task)} onStarted={onStarted} onChanged={onChanged} />
          ))}
        </div>
      )}
    </section>
  );
}

/** A project's recurring work: two daily groups and one that runs on demand. */
export function ScheduleView() {
  const project = useProject();
  const { user } = useSession();
  const now = useNow();
  const [dialog, setDialog] = useState<Open | null>(null);
  // A task changes when it is edited or run; whether it can run changes with the modules.
  const tasks = useRemote(
    () => api.get<ScheduledTask[]>(`/api/projects/${project.key}/tasks`),
    [project.key],
    (event) => (event.projectId === project.id && (event.kind === 'task.updated' || event.kind === 'project.updated')) || event.kind === 'instance.modules',
  );
  const kinds = useRemote(() => api.get<TaskKind[]>('/api/task-kinds'), [], (event) => event.kind === 'instance.modules');
  const settings = useRemote(() => api.get<ScheduleSettings>('/api/schedule'), [], (event) => event.kind === 'schedule.settings');
  const administrator = project.role === 'admin';
  const available = (kinds.data ?? []).filter((kind) => !project.disabledModules.includes(kind.module));
  const zone = settings.data?.timezone;

  // The answer to "run" is shown at once; how the run ends arrives as `task.updated`.
  function showStarted(started: ScheduledTask[]): void {
    tasks.set((tasks.data ?? []).map((task) => started.find((candidate) => candidate.id === task.id) ?? task));
  }

  if (tasks.error && !tasks.data) {
    return <ErrorNote message={tasks.error} />;
  }

  if (!tasks.data) {
    return <Spinner />;
  }

  return (
    <div className="view schedule">
      <header className="view-head">
        <div>
          <h1>{t('Schedule')}</h1>
          <p className="muted">
            {t('Recurring work of this project: it runs every morning, every afternoon, or when somebody asks.')}
            {zone && zone !== browserZone() && <> {t('Times are in {zone}.', { zone })}</>}
          </p>
        </div>
        <div className="view-head-side">
          {user.role === 'admin' && settings.data && (
            <Button icon="settings" onClick={() => setDialog({ kind: 'settings' })}>
              {t('Schedule settings')}
            </Button>
          )}
          {administrator && (
            <Button variant="primary" icon="plus" onClick={() => setDialog({ kind: 'new' })}>
              {t('New task')}
            </Button>
          )}
        </div>
      </header>

      {tasks.data.length === 0 ? (
        <Empty icon="clock" title={t('Nothing is scheduled yet')}>
          <p className="schedule-empty-text">
            {t('A task is something a module does for this project again and again: an agent that triages yesterday’s new issues, say.')}{' '}
            {t('It runs each day with the morning or the afternoon group, or only when somebody presses its button.')}
          </p>
          {administrator ? (
            <Button variant="primary" icon="plus" onClick={() => setDialog({ kind: 'new' })}>
              {t('Add the first task')}
            </Button>
          ) : (
            <p>{t('A project administrator can add the first task.')}</p>
          )}
        </Empty>
      ) : (
        SLOTS.map((slot) => (
          <Group
            key={slot}
            slot={slot}
            tasks={tasks.data?.filter((task) => task.slot === slot) ?? []}
            settings={settings.data}
            kinds={kinds.data ?? []}
            now={now}
            onEdit={(task) => setDialog({ kind: 'edit', task })}
            onStarted={showStarted}
            onChanged={tasks.reload}
          />
        ))
      )}

      {dialog && dialog.kind !== 'settings' && (
        <TaskDialog project={project} kinds={available} settings={settings.data} task={dialog.kind === 'edit' ? dialog.task : null} onClose={() => setDialog(null)} onSaved={tasks.reload} />
      )}
      {dialog?.kind === 'settings' && settings.data && <SettingsDialog settings={settings.data} onClose={() => setDialog(null)} onSaved={settings.set} />}
    </div>
  );
}
