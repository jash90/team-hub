import { Fragment, useMemo, useState, type FormEvent } from 'react';

import { api } from '../api.ts';
import type { Backlog } from '../bindings/Backlog.ts';
import type { IssueRow } from '../bindings/IssueRow.ts';
import type { Project } from '../bindings/Project.ts';
import type { Sprint } from '../bindings/Sprint.ts';
import { useIssueDrag, type DropSpot } from '../components/dnd.ts';
import { Icon } from '../components/icon.tsx';
import { IssueLine, canEdit } from '../components/issue-bits.tsx';
import { Button, ConfirmDialog, Dialog, Empty, ErrorNote, Field, Menu, Spinner, Tag, attempt, toast, useBusy } from '../components/ui.tsx';
import { countOf, formatDate, t } from '../i18n/index.ts';
import { changesIssues, useProject, useRemote, useStored } from '../session.tsx';
import { CompleteSprintDialog, FilterBar, matchesFilter, type IssueFilter } from './board.tsx';


function points(issues: IssueRow[]): number {
  return issues.reduce((sum, issue) => sum + (issue.points ?? 0), 0);
}

function QuickAdd({ project, sprintId, onDone }: { project: Project; sprintId: number | null; onDone: () => void }) {
  const [summary, setSummary] = useState('');
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();

    if (!summary.trim()) {
      return;
    }

    void run(async () => {
      await api.post('/api/issues', { projectKey: project.key, summary, sprintId });
      setSummary('');
      onDone();
    });
  }

  return (
    <form className="quick-add" onSubmit={submit}>
      <Icon name="plus" size={14} />
      <input className="input input-inline" placeholder={t('What needs to be done? Enter adds it here')} value={summary} disabled={busy} onChange={(event) => setSummary(event.target.value)} />
    </form>
  );
}

export function BacklogView() {
  const project = useProject();
  const editable = canEdit(project);
  const backlog = useRemote(
    () => api.get<Backlog>(`/api/projects/${project.key}/backlog`),
    [project.key],
    (event) => event.projectId === project.id && changesIssues(event),
  );
  const [filter, setFilter] = useState<IssueFilter>({ text: '', assignees: [], unassigned: false });
  const [folded, setFolded] = useStored<number[]>(`backlog-folded:${project.key}`, []);
  const [dialog, setDialog] = useState<{ kind: 'start' | 'edit' | 'complete' | 'delete'; sprint: Sprint } | null>(null);
  const drag = useIssueDrag((key, spot) => void move(key, spot));
  const all = useMemo(() => [...(backlog.data?.sprints.flatMap((entry) => entry.issues) ?? []), ...(backlog.data?.backlog ?? [])], [backlog.data]);

  async function move(key: string, spot: DropSpot): Promise<void> {
    const current = backlog.data;
    const moved = all.find((issue) => issue.key === key);

    if (!current || !moved) {
      return;
    }

    const sprintId = spot.zone === 'backlog' ? null : Number(spot.zone);
    const place = (issues: IssueRow[], here: boolean) => {
      const without = issues.filter((issue) => issue.key !== key);

      if (!here) {
        return without;
      }

      const index = spot.beforeKey ? without.findIndex((issue) => issue.key === spot.beforeKey) : -1;
      const at = index === -1 ? without.length : index;

      return [...without.slice(0, at), { ...moved, sprintId }, ...without.slice(at)];
    };

    backlog.set({
      sprints: current.sprints.map((entry) => ({ ...entry, issues: place(entry.issues, entry.sprint.id === sprintId) })),
      backlog: place(current.backlog, sprintId === null),
    });

    await attempt(() => api.post(`/api/issues/${key}/move`, { sprintId: sprintId === moved.sprintId ? undefined : sprintId, afterKey: spot.afterKey, beforeKey: spot.beforeKey }));
    await backlog.reload();
  }

  if (backlog.error && !backlog.data) {
    return <ErrorNote message={backlog.error} />;
  }

  if (!backlog.data) {
    return <Spinner />;
  }

  const data = backlog.data;
  const kanban = project.boardMode === 'kanban';
  const activeExists = data.sprints.some((entry) => entry.sprint.state === 'active');

  function rows(zone: string, issues: IssueRow[]) {
    const visible = issues.filter((issue) => matchesFilter(issue, filter));

    return (
      <div className={drag.spot?.zone === zone ? 'backlog-rows backlog-target' : 'backlog-rows'} {...(editable ? drag.zone(zone) : {})}>
        {visible.map((issue) => (
          <Fragment key={issue.key}>
            {drag.lineBefore(zone, issue.key) && <div className="drop-line" />}
            <div className={drag.dragging === issue.key ? 'backlog-row card-dragging' : 'backlog-row'} {...drag.item(issue.key, editable)}>
              <IssueLine issue={issue} project={project} />
            </div>
          </Fragment>
        ))}
        {drag.lineBefore(zone, null) && <div className="drop-line" />}
        {visible.length === 0 && <div className="backlog-empty muted">{issues.length === 0 ? t('Drag issues here, or add one below.') : t('Nothing matches the filter.')}</div>}
      </div>
    );
  }

  return (
    <div className="view">
      <header className="view-head">
        <div>
          <h1>{t('Backlog')}</h1>
          <p className="muted">{kanban ? t('Issues waiting in statuses that are not on the board. Move one to a board status to start it.') : t('Drag issues to order them and to plan sprints.')}</p>
        </div>
        {!kanban && editable && (
          <Button icon="plus" onClick={() => void attempt(() => api.post(`/api/projects/${project.key}/sprints`, {}), t('Sprint created')).then(() => backlog.reload())}>
            {t('Create sprint')}
          </Button>
        )}
      </header>

      <FilterBar issues={all} filter={filter} onChange={setFilter} />

      {data.sprints.map(({ sprint, issues }) => {
        const isFolded = folded.includes(sprint.id);

        return (
          <section key={sprint.id} className="backlog-section">
            <header className="backlog-head">
              <button type="button" className="fold" aria-expanded={!isFolded} onClick={() => setFolded(isFolded ? folded.filter((id) => id !== sprint.id) : [...folded, sprint.id])}>
                <Icon name={isFolded ? 'chevronRight' : 'chevronDown'} />
                <strong>{sprint.name}</strong>
              </button>
              {sprint.state === 'active' && <Tag tone="blue">{t('active')}</Tag>}
              {sprint.startDate && (
                <span className="muted">
                  {formatDate(sprint.startDate)} – {formatDate(sprint.endDate)}
                </span>
              )}
              <span className="muted">
                {countOf(issues.length, 'issue', 'issues')} · {points(issues)} {t('pts')}
              </span>
              <span className="filter-spacer" />
              {editable && sprint.state === 'future' && (
                <Button size="sm" variant="primary" disabled={activeExists || issues.length === 0} title={activeExists ? t('Another sprint is still running') : undefined} onClick={() => setDialog({ kind: 'start', sprint })}>
                  {t('Start sprint')}
                </Button>
              )}
              {editable && sprint.state === 'active' && (
                <Button size="sm" onClick={() => setDialog({ kind: 'complete', sprint })}>
                  {t('Complete sprint')}
                </Button>
              )}
              {editable && (
                <Menu
                  label={t('Sprint actions')}
                  items={[
                    { label: t('Edit sprint'), icon: 'edit', onSelect: () => setDialog({ kind: 'edit', sprint }) },
                    { label: t('Delete sprint'), icon: 'trash', danger: true, onSelect: () => setDialog({ kind: 'delete', sprint }) },
                  ]}
                />
              )}
            </header>
            {sprint.goal && !isFolded && <p className="backlog-goal muted">{sprint.goal}</p>}
            {!isFolded && rows(String(sprint.id), issues)}
            {!isFolded && editable && <QuickAdd project={project} sprintId={sprint.id} onDone={() => void backlog.reload()} />}
          </section>
        );
      })}

      <section className="backlog-section">
        <header className="backlog-head">
          <strong>{t('Backlog')}</strong>
          <span className="muted">
            {countOf(data.backlog.length, 'issue', 'issues')} · {points(data.backlog)} {t('pts')}
          </span>
        </header>
        {data.backlog.length === 0 && data.sprints.length === 0 ? <Empty icon="list" title={t('The backlog is empty')} /> : rows('backlog', data.backlog)}
        {editable && <QuickAdd project={project} sprintId={null} onDone={() => void backlog.reload()} />}
      </section>

      {dialog && (dialog.kind === 'start' || dialog.kind === 'edit') && <SprintDialog sprint={dialog.sprint} start={dialog.kind === 'start'} onClose={() => setDialog(null)} onDone={() => void backlog.reload()} />}
      {dialog?.kind === 'complete' && (
        <CompleteSprintDialog
          sprint={dialog.sprint}
          project={project}
          done={doneCount(project, data.sprints.find((entry) => entry.sprint.id === dialog.sprint.id)?.issues ?? [])}
          open={(data.sprints.find((entry) => entry.sprint.id === dialog.sprint.id)?.issues.length ?? 0) - doneCount(project, data.sprints.find((entry) => entry.sprint.id === dialog.sprint.id)?.issues ?? [])}
          onClose={() => setDialog(null)}
          onDone={() => void backlog.reload()}
        />
      )}
      {dialog?.kind === 'delete' && (
        <ConfirmDialog
          title={t('Delete {name}?', { name: dialog.sprint.name })}
          body={t('Its issues go back to the backlog.')}
          confirm={t('Delete sprint')}
          danger
          onClose={() => setDialog(null)}
          onConfirm={async () => {
            await api.del(`/api/sprints/${dialog.sprint.id}`);
            await backlog.reload();
          }}
        />
      )}
    </div>
  );
}

function doneCount(project: Project, issues: IssueRow[]): number {
  const done = new Set(project.statuses.filter((status) => status.category === 'done').map((status) => status.id));

  return issues.filter((issue) => done.has(issue.statusId)).length;
}

function isoDay(offset: number): string {
  return new Date(Date.now() + offset * 24 * 3600 * 1000).toISOString().slice(0, 10);
}

function SprintDialog({ sprint, start, onClose, onDone }: { sprint: Sprint; start: boolean; onClose: () => void; onDone: () => void }) {
  const [form, setForm] = useState({
    name: sprint.name,
    goal: sprint.goal,
    startDate: sprint.startDate ?? (start ? isoDay(0) : ''),
    endDate: sprint.endDate ?? (start ? isoDay(14) : ''),
  });
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      await api.patch(`/api/sprints/${sprint.id}`, form);

      if (start) {
        await api.post(`/api/sprints/${sprint.id}/start`, { startDate: form.startDate, endDate: form.endDate });
        toast(t('Sprint started'));
      }

      onDone();
      onClose();
    });
  }

  return (
    <Dialog
      title={start ? t('Start {name}', { name: sprint.name }) : t('Edit sprint')}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="sprint-form" disabled={busy}>
            {start ? t('Start sprint') : t('Save')}
          </Button>
        </>
      }
    >
      <form id="sprint-form" className="form" onSubmit={submit}>
        <Field label={t('Name')}>
          <input className="input" required value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} />
        </Field>
        <Field label={t('Sprint goal')}>
          <textarea className="input" rows={2} value={form.goal} onChange={(event) => setForm({ ...form, goal: event.target.value })} />
        </Field>
        <div className="form-row">
          <Field label={t('Starts')}>
            <input className="input" type="date" required={start} value={form.startDate} onChange={(event) => setForm({ ...form, startDate: event.target.value })} />
          </Field>
          <Field label={t('Ends')}>
            <input className="input" type="date" required={start} value={form.endDate} onChange={(event) => setForm({ ...form, endDate: event.target.value })} />
          </Field>
        </div>
      </form>
    </Dialog>
  );
}
