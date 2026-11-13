import { Fragment, useMemo, useState } from 'react';

import { api } from '../api.ts';
import type { Board } from '../bindings/Board.ts';
import type { IssueRow } from '../bindings/IssueRow.ts';
import type { Project } from '../bindings/Project.ts';
import type { Sprint } from '../bindings/Sprint.ts';
import { useIssueDrag, type DropSpot } from '../components/dnd.ts';
import { Icon } from '../components/icon.tsx';
import { DueDate, ModuleBadges, PriorityIcon, TypeIcon, canEdit, openIssue } from '../components/issue-bits.tsx';
import { Avatar, Button, Dialog, Empty, ErrorNote, Field, Picker, ProgressBar, Spinner, attempt, useBusy } from '../components/ui.tsx';
import { countOf, formatDate, t } from '../i18n/index.ts';
import { Link } from '../router.tsx';
import { changesIssues, useProject, useRemote, useSession, useStored } from '../session.tsx';
import type { CreateDefaults } from './create-issue.tsx';
import { CreateIssueDialog } from './lazy.ts';

type Grouping = 'none' | 'epic' | 'assignee';
type Lane = { id: string; title: string; assigneeId?: number | null; matches: (issue: IssueRow) => boolean };


export type IssueFilter = { text: string; assignees: number[]; unassigned: boolean };

export function matchesFilter(issue: IssueRow, filter: IssueFilter): boolean {
  const text = filter.text.trim().toLowerCase();

  if (text && !`${issue.key} ${issue.summary} ${issue.labels.join(' ')}`.toLowerCase().includes(text)) {
    return false;
  }

  if (filter.assignees.length === 0 && !filter.unassigned) {
    return true;
  }

  return issue.assigneeId === null ? filter.unassigned : filter.assignees.includes(issue.assigneeId);
}

/** Text search plus one avatar per person who has issues in view; clicking narrows to them. */
export function FilterBar({ issues, filter, onChange, children }: { issues: IssueRow[]; filter: IssueFilter; onChange: (filter: IssueFilter) => void; children?: React.ReactNode }) {
  const { userById, user } = useSession();
  const people = useMemo(() => Array.from(new Set(issues.map((issue) => issue.assigneeId).filter((id): id is number => id !== null))), [issues]);
  const hasUnassigned = issues.some((issue) => issue.assigneeId === null);
  const toggle = (id: number) => onChange({ ...filter, assignees: filter.assignees.includes(id) ? filter.assignees.filter((existing) => existing !== id) : [...filter.assignees, id] });

  return (
    <div className="filter-bar">
      <div className="filter-search">
        <Icon name="search" />
        <input className="input" placeholder={t('Filter…')} value={filter.text} onChange={(event) => onChange({ ...filter, text: event.target.value })} />
      </div>
      <div className="filter-people">
        {people.map((id) => (
          <button key={id} type="button" className={filter.assignees.includes(id) ? 'avatar-toggle is-on' : 'avatar-toggle'} onClick={() => toggle(id)} aria-pressed={filter.assignees.includes(id)}>
            <Avatar user={userById(id)} size={28} />
          </button>
        ))}
        {hasUnassigned && (
          <button type="button" className={filter.unassigned ? 'avatar-toggle is-on' : 'avatar-toggle'} onClick={() => onChange({ ...filter, unassigned: !filter.unassigned })} aria-pressed={filter.unassigned}>
            <Avatar user={undefined} size={28} />
          </button>
        )}
      </div>
      {people.includes(user.id) && (
        <Button size="sm" className={filter.assignees.length === 1 && filter.assignees[0] === user.id ? 'is-on' : ''} onClick={() => onChange({ ...filter, assignees: filter.assignees.length === 1 && filter.assignees[0] === user.id ? [] : [user.id], unassigned: false })}>
          {t('Only mine')}
        </Button>
      )}
      {(filter.text || filter.assignees.length > 0 || filter.unassigned) && (
        <button type="button" className="link-button" onClick={() => onChange({ text: '', assignees: [], unassigned: false })}>
          {t('Clear filters')}
        </button>
      )}
      <span className="filter-spacer" />
      {children}
    </div>
  );
}

function Card({ issue, project, showEpic }: { issue: IssueRow; project: Project; showEpic: boolean }) {
  const { userById } = useSession();
  const type = project.types.find((candidate) => candidate.id === issue.typeId);
  const done = project.statuses.find((status) => status.id === issue.statusId)?.category === 'done';

  return (
    <button type="button" className="card" onClick={() => openIssue(issue.key)}>
      <span className="card-summary">{issue.summary}</span>
      {(showEpic && issue.parentSummary) || issue.labels.length > 0 ? (
        <span className="card-tags">
          {showEpic && issue.parentSummary && <span className="epic-chip">{issue.parentSummary}</span>}
          {issue.labels.slice(0, 2).map((label) => (
            <span key={label} className="label">
              {label}
            </span>
          ))}
        </span>
      ) : null}
      <span className="card-foot">
        <span className="card-foot-group">
          <TypeIcon type={type} />
          <span className={done ? 'issue-key issue-key-done' : 'issue-key'}>{issue.key}</span>
          <ModuleBadges issue={issue} />
        </span>
        <span className="card-foot-group card-foot-end">
          <DueDate date={issue.dueDate} done={done} short />
          {issue.subtasks.total > 0 && (
            <span className="subtasks" title={t('Subtasks')}>
              <Icon name="check" size={12} />
              {issue.subtasks.done}/{issue.subtasks.total}
            </span>
          )}
          {issue.commentCount > 0 && (
            <span className="subtasks" title={t('Comments')}>
              <Icon name="message" size={12} />
              {issue.commentCount}
            </span>
          )}
          {issue.points !== null && <span className="points">{issue.points}</span>}
          <PriorityIcon priority={issue.priority} />
          <Avatar user={userById(issue.assigneeId)} size={22} />
        </span>
      </span>
    </button>
  );
}

export function BoardView() {
  const project = useProject();
  const { userById } = useSession();
  const editable = canEdit(project);
  const board = useRemote(
    () => api.get<Board>(`/api/projects/${project.key}/board`),
    [project.key],
    (event) => event.projectId === project.id && changesIssues(event),
  );
  const [filter, setFilter] = useState<IssueFilter>({ text: '', assignees: [], unassigned: false });
  const [grouping, setGrouping] = useStored<Grouping>(`board-group:${project.key}`, 'none');
  const [creating, setCreating] = useState<CreateDefaults | null>(null);
  const [completing, setCompleting] = useState(false);

  const drag = useIssueDrag((key, spot) => void move(key, spot));

  async function move(key: string, spot: DropSpot): Promise<void> {
    const current = board.data;

    if (!current) {
      return;
    }

    const [statusText, laneId] = spot.zone.split('|');
    const statusId = Number(statusText);
    const moved = current.columns.flatMap((column) => column.issues).find((issue) => issue.key === key);

    if (!moved) {
      return;
    }

    const lane = lanes.find((candidate) => candidate.id === laneId);
    const assigneeChange = grouping === 'assignee' && lane && lane.assigneeId !== undefined && lane.assigneeId !== moved.assigneeId ? lane.assigneeId : undefined;
    const updated: IssueRow = { ...moved, statusId, assigneeId: assigneeChange === undefined ? moved.assigneeId : assigneeChange };

    // Show the card where it was dropped right away; the server's answer follows.
    board.set({
      ...current,
      columns: current.columns.map((column) => {
        const without = column.issues.filter((issue) => issue.key !== key);

        if (column.status.id !== statusId) {
          return { ...column, issues: without };
        }

        const index = spot.beforeKey ? without.findIndex((issue) => issue.key === spot.beforeKey) : -1;
        const at = index === -1 ? without.length : index;

        return { ...column, issues: [...without.slice(0, at), updated, ...without.slice(at)] };
      }),
    });

    await attempt(async () => {
      if (assigneeChange !== undefined) {
        await api.patch(`/api/issues/${key}`, { assigneeId: assigneeChange });
      }

      await api.post(`/api/issues/${key}/move`, { statusId: statusId === moved.statusId ? undefined : statusId, afterKey: spot.afterKey, beforeKey: spot.beforeKey });
    });
    await board.reload();
  }

  const all = useMemo(() => board.data?.columns.flatMap((column) => column.issues) ?? [], [board.data]);
  const lanes = useMemo<Lane[]>(() => {
    if (grouping === 'epic') {
      const epics = new Map<string, string>();

      for (const issue of all) {
        const [key, title] = epicOf(issue, all);

        if (key) {
          epics.set(key, title);
        }
      }

      return [
        ...Array.from(epics, ([key, title]) => ({ id: key, title: `${key} ${title}`, matches: (issue: IssueRow) => epicOf(issue, all)[0] === key })),
        { id: 'none', title: t('No epic'), matches: (issue: IssueRow) => epicOf(issue, all)[0] === null },
      ];
    }

    if (grouping === 'assignee') {
      const ids = Array.from(new Set(all.map((issue) => issue.assigneeId)));

      return ids
        .sort((left, right) => (left === null ? 1 : right === null ? -1 : (userById(left)?.name ?? '').localeCompare(userById(right)?.name ?? '')))
        .map((id) => ({ id: String(id), title: id === null ? t('Unassigned') : (userById(id)?.name ?? '?'), assigneeId: id, matches: (issue: IssueRow) => issue.assigneeId === id }));
    }

    return [{ id: 'all', title: '', matches: () => true }];
  }, [grouping, all, userById]);

  if (board.error && !board.data) {
    return <ErrorNote message={board.error} />;
  }

  if (!board.data) {
    return <Spinner />;
  }

  const data = board.data;
  const sprint = data.sprint;
  const total = all.filter((issue) => project.types.find((type) => type.id === issue.typeId)?.level !== 'subtask');
  const doneIds = new Set(project.statuses.filter((status) => status.category === 'done').map((status) => status.id));
  const doneCount = total.filter((issue) => doneIds.has(issue.statusId)).length;
  const firstColumn = data.columns[0]?.status.id;

  return (
    <div className="view view-board">
      <header className="view-head">
        <div>
          <h1>{sprint ? sprint.name : data.mode === 'kanban' ? t('Board') : t('No active sprint')}</h1>
          {sprint && (
            <p className="muted">
              {formatDate(sprint.startDate)} – {formatDate(sprint.endDate)}
              {sprint.goal && ` · ${sprint.goal}`}
            </p>
          )}
        </div>
        {(sprint || data.mode === 'kanban') && (
          <div className="view-head-side">
            <span className="board-progress">
              <ProgressBar done={doneCount} total={total.length} />
              <span className="muted">
                {doneCount}/{total.length}
              </span>
            </span>
            {sprint && editable && <Button onClick={() => setCompleting(true)}>{t('Complete sprint')}</Button>}
          </div>
        )}
      </header>

      {data.mode === 'scrum' && !sprint ? (
        <Empty icon="play" title={t('No sprint is running')}>
          {t('Plan a sprint in the backlog and start it; its issues show up here.')} <Link href={`/p/${project.key}/backlog`}>{t('Open the backlog')}</Link>
        </Empty>
      ) : (
        <>
          <FilterBar issues={all} filter={filter} onChange={setFilter}>
            <label className="inline-field">
              <span className="muted">{t('Group by')}</span>
              <Picker
                value={grouping}
                options={[
                  { value: 'none' as Grouping, label: t('Nothing') },
                  { value: 'epic' as Grouping, label: t('Epic') },
                  { value: 'assignee' as Grouping, label: t('Assignee') },
                ]}
                onChange={setGrouping}
                align="end"
              />
            </label>
          </FilterBar>

          <div className="board" style={{ gridTemplateColumns: `repeat(${data.columns.length}, minmax(240px, 1fr))` }}>
            {data.columns.map((column) => {
              const over = column.status.wipLimit !== null && column.issues.length > column.status.wipLimit;

              return (
                <div key={column.status.id} className={`column-head column-${column.status.category}`}>
                  <span className="column-name">{column.status.name}</span>
                  <span className={over ? 'column-count column-over' : 'column-count'} title={over ? t('Over the work-in-progress limit') : undefined}>
                    {column.issues.length}
                    {column.status.wipLimit !== null && ` / ${column.status.wipLimit}`}
                  </span>
                </div>
              );
            })}

            {lanes.map((lane) => {
              const laneIssues = all.filter((issue) => lane.matches(issue) && matchesFilter(issue, filter));

              if (grouping !== 'none' && laneIssues.length === 0) {
                return null;
              }

              return (
                <Fragment key={lane.id}>
                  {grouping !== 'none' && (
                    <div className="lane-head" style={{ gridColumn: `1 / span ${data.columns.length}` }}>
                      {grouping === 'assignee' && <Avatar user={userById(lane.assigneeId)} size={22} />}
                      <strong>{lane.title}</strong>
                      <span className="muted">{countOf(laneIssues.length, 'issue', 'issues')}</span>
                    </div>
                  )}
                  {data.columns.map((column) => {
                    const zone = `${column.status.id}|${lane.id}`;
                    const cards = column.issues.filter((issue) => lane.matches(issue) && matchesFilter(issue, filter));

                    return (
                      <div key={zone} className={drag.spot?.zone === zone ? 'column column-target' : 'column'} {...(editable ? drag.zone(zone) : {})}>
                        {cards.map((issue) => (
                          <Fragment key={issue.key}>
                            {drag.lineBefore(zone, issue.key) && <div className="drop-line" />}
                            <div className={drag.dragging === issue.key ? 'card-slot card-dragging' : 'card-slot'} {...drag.item(issue.key, editable)}>
                              <Card issue={issue} project={project} showEpic={grouping !== 'epic'} />
                            </div>
                          </Fragment>
                        ))}
                        {drag.lineBefore(zone, null) && <div className="drop-line" />}
                        {editable && column.status.id === firstColumn && (
                          <button type="button" className="column-add" onClick={() => setCreating({ sprintId: sprint?.id ?? null, statusId: column.status.id })}>
                            <Icon name="plus" size={14} /> {t('Create issue')}
                          </button>
                        )}
                      </div>
                    );
                  })}
                </Fragment>
              );
            })}
          </div>
        </>
      )}

      {creating && <CreateIssueDialog projectKey={project.key} defaults={creating} onClose={() => setCreating(null)} />}
      {completing && sprint && <CompleteSprintDialog sprint={sprint} project={project} open={total.length - doneCount} done={doneCount} onClose={() => setCompleting(false)} onDone={() => void board.reload()} />}
    </div>
  );
}

/** The epic an issue belongs to: its parent, or for a subtask its parent's parent. */
function epicOf(issue: IssueRow, all: IssueRow[]): [string | null, string] {
  const parent = issue.parentKey ? all.find((candidate) => candidate.key === issue.parentKey) : undefined;

  if (parent) {
    return [parent.parentKey, parent.parentSummary ?? ''];
  }

  return [issue.parentKey, issue.parentSummary ?? ''];
}

export function CompleteSprintDialog({ sprint, project, open, done, onClose, onDone }: { sprint: Sprint; project: Project; open: number; done: number; onClose: () => void; onDone: () => void }) {
  const next = project.sprints.filter((candidate) => candidate.state === 'future');
  const [moveTo, setMoveTo] = useState<number | null>(next[0]?.id ?? null);
  const [busy, run] = useBusy();

  return (
    <Dialog
      title={t('Complete {name}', { name: sprint.name })}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button
            variant="primary"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await api.post(`/api/sprints/${sprint.id}/complete`, { moveTo });
                onDone();
                onClose();
              }, t('Sprint completed'))
            }
          >
            {t('Complete sprint')}
          </Button>
        </>
      }
    >
      <div className="form">
        <p>
          <strong>{countOf(done, 'issue', 'issues')}</strong> {t('finished')}, <strong>{countOf(open, 'issue', 'issues')}</strong> {t('not finished')}.
        </p>
        {open > 0 && (
          <Field label={t('Move unfinished issues to')}>
            <Picker value={moveTo} options={[...next.map((candidate) => ({ value: candidate.id as number | null, label: candidate.name })), { value: null, label: t('Backlog') }]} onChange={setMoveTo} />
          </Field>
        )}
      </div>
    </Dialog>
  );
}
