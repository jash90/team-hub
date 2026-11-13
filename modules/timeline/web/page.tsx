// The timeline of a project: its epics down the left, time across, a bar per epic.

import { useLayoutEffect, useMemo, useRef, useState } from 'react';

import { api, errorMessage } from '@/api.ts';
import type { Project } from '@/bindings/Project.ts';
import type { TimelineChild } from '@/bindings/TimelineChild.ts';
import type { TimelineEpic } from '@/bindings/TimelineEpic.ts';
import type { TimelineProject } from '@/bindings/TimelineProject.ts';
import type { TimelineStatus } from '@/bindings/TimelineStatus.ts';
import { Icon } from '@/components/icon.tsx';
import { IssueKey, openIssue } from '@/components/issue-bits.tsx';
import { Button, Empty, ErrorNote, Menu, ProgressBar, Spinner, toast } from '@/components/ui.tsx';
import { t } from '@/i18n/index.ts';
import { useProject, useRemote, useStored, type LiveEvent } from '@/session.tsx';
import { CreateIssueDialog } from '@/views/lazy.ts';

import { Axis, Backdrop, Bar } from './chart.tsx';
import { daysOf, geometryFor, header, today, ZOOMS, type Geometry, type Zoom } from './dates.ts';
import { DatesDialog, type DatesTarget } from './dialog.tsx';

const NO_ESTIMATE = { start: false, due: false };

/** Changes that alter what the page shows: the module's own, and the tracker's to issues, sprints and versions. */
function isOurs(project: Project) {
  const kinds = ['timeline.updated', 'issue.created', 'issue.updated', 'issue.transitioned', 'issue.ranked', 'issue.deleted'];

  return (event: LiveEvent) => event.projectId === project.id && (kinds.includes(event.kind) || event.kind.startsWith('sprint.') || event.kind.startsWith('version.'));
}

function zoomLabel(zoom: Zoom): string {
  return zoom === 'weeks' ? t('Weeks') : zoom === 'months' ? t('Months') : t('Quarters');
}

/** The data with the dates of one issue replaced, as they will be once the server has them. */
function withDates(data: TimelineProject, key: string, start: string | null, due: string | null): TimelineProject {
  return {
    ...data,
    epics: data.epics.map((epic) =>
      epic.key === key ? { ...epic, start, due, derived: NO_ESTIMATE } : { ...epic, children: epic.children.map((child) => (child.key === key ? { ...child, start, due } : child)) },
    ),
  };
}

function Status({ status }: { status: TimelineStatus }) {
  return <span className={`status status-${status.category}`}>{status.name}</span>;
}

/** Key and summary; either opens the issue beside the timeline, as lists do everywhere. */
function Title({ issue }: { issue: { key: string; summary: string; status: TimelineStatus } }) {
  return (
    <span className="timeline-title">
      <IssueKey issue={issue} done={issue.status.category === 'done'} />
      <button type="button" className="timeline-summary" title={issue.summary} onClick={() => openIssue(issue.key)}>
        {issue.summary}
      </button>
    </span>
  );
}

type RowProps = {
  geometry: Geometry;
  editable: boolean;
  /** Stores new dates for an issue. */
  onDates: (key: string, start: string | null, due: string | null) => void;
  /** Opens the dialog with the dates of an issue. */
  onEdit: (target: DatesTarget) => void;
};

function RowMenu({ target, editable, onEdit, onDates }: { target: DatesTarget } & Pick<RowProps, 'editable' | 'onEdit' | 'onDates'>) {
  const own = (target.start !== null && !target.derived?.start) || (target.due !== null && !target.derived?.due);

  if (!editable) {
    return null;
  }

  return (
    <Menu
      label={t('Actions for {key}', { key: target.key })}
      items={[
        { label: t('Set dates…'), icon: 'calendar', onSelect: () => onEdit(target) },
        { label: t('Clear dates'), icon: 'x', disabled: !own, onSelect: () => onDates(target.key, null, null) },
      ]}
    />
  );
}

function ChildRow({ child, geometry, editable, onDates, onEdit }: RowProps & { child: TimelineChild }) {
  return (
    <div className="timeline-row timeline-row-child" role="listitem">
      <div className="timeline-side">
        <Title issue={child} />
        <Status status={child.status} />
        <RowMenu target={child} editable={editable} onEdit={onEdit} onDates={onDates} />
      </div>
      <div className="timeline-track" style={{ width: geometry.width }}>
        {child.start !== null && <Bar name={child.key} span={{ start: child.start, due: child.due, derived: NO_ESTIMATE }} geometry={geometry} thin={child.status.category} />}
      </div>
    </div>
  );
}

function EpicRow({ epic, open, onToggle, geometry, editable, onDates, onEdit }: RowProps & { epic: TimelineEpic; open: boolean; onToggle: () => void }) {
  const { counts } = epic;
  const dated = epic.start !== null || epic.due !== null;
  const points = epic.pointsTotal > 0 ? t('{done} of {total} story points done', { done: epic.pointsDone, total: epic.pointsTotal }) : undefined;

  return (
    <>
      <div className="timeline-row" role="listitem">
        <div className="timeline-side">
          {counts.total > 0 ? (
            <Button
              variant="ghost"
              size="sm"
              icon={open ? 'chevronDown' : 'chevronRight'}
              aria-expanded={open}
              aria-label={open ? t('Hide the issues of {key}', { key: epic.key }) : t('Show the issues of {key}', { key: epic.key })}
              onClick={onToggle}
            />
          ) : (
            <span className="timeline-no-toggle" />
          )}
          <div className="timeline-epic">
            <Title issue={epic} />
            <div className="timeline-meta">
              <Status status={epic.status} />
              {counts.total > 0 ? (
                <span className="timeline-progress" title={points}>
                  <ProgressBar done={counts.done} inProgress={counts.inProgress} total={counts.total} />
                  <span aria-label={t('{done} of {total} issues done', { done: counts.done, total: counts.total })}>
                    {counts.done} / {counts.total}
                  </span>
                </span>
              ) : (
                <span className="muted">{t('no issues')}</span>
              )}
            </div>
          </div>
          <RowMenu target={epic} editable={editable} onEdit={onEdit} onDates={onDates} />
        </div>
        <div className="timeline-track" style={{ width: geometry.width }}>
          <Bar
            name={epic.key}
            span={epic}
            geometry={geometry}
            progress={counts.total > 0 ? counts.done / counts.total : 0}
            onChange={editable ? (start, due) => onDates(epic.key, start, due) : undefined}
          />
          {!dated &&
            (editable ? (
              <button type="button" className="timeline-set" onClick={() => onEdit(epic)}>
                <Icon name="calendar" size={13} />
                {t('Set dates')}
              </button>
            ) : (
              <span className="timeline-set timeline-set-none">{t('No dates')}</span>
            ))}
        </div>
      </div>
      {open && epic.children.map((child) => <ChildRow key={child.id} child={child} geometry={geometry} editable={editable} onDates={onDates} onEdit={onEdit} />)}
    </>
  );
}

function Legend() {
  return (
    <div className="timeline-legend" aria-hidden="true">
      <span>
        <i className="timeline-key timeline-key-planned" />
        {t('dates set; the filled part is done')}
      </span>
      <span>
        <i className="timeline-key timeline-key-estimated" />
        {t('estimated from the epic’s issues')}
      </span>
      <span>
        <i className="timeline-key timeline-key-sprint" />
        {t('sprint')}
      </span>
      <span>
        <i className="timeline-key timeline-key-release" />
        {t('release')}
      </span>
      <span>
        <i className="timeline-key timeline-key-today" />
        {t('today')}
      </span>
    </div>
  );
}

/** Starts at today, and keeps the day in the middle of the view there when the axis changes under the viewer. */
function useAnchoredScroll(geometry: Geometry) {
  const scroller = useRef<HTMLDivElement>(null);
  const corner = useRef<HTMLDivElement>(null);
  const shown = useRef<{ from: number; dayWidth: number; left: number } | null>(null);
  /** How much of the scrolling box shows the axis: the column of issues covers the rest. */
  const visible = (box: HTMLDivElement) => box.clientWidth - (corner.current?.offsetWidth ?? 0);

  function toToday(behavior: ScrollBehavior): void {
    const box = scroller.current;

    box?.scrollTo({ left: Math.max(0, geometry.x(today()) - visible(box) / 3), behavior });
  }

  useLayoutEffect(() => {
    const box = scroller.current;
    const before = shown.current;

    if (!box) {
      return;
    }

    if (before) {
      const half = visible(box) / 2;

      box.scrollLeft = (before.from + (before.left + half) / before.dayWidth - geometry.from) * geometry.dayWidth - half;
    } else {
      toToday('instant');
    }

    shown.current = { from: geometry.from, dayWidth: geometry.dayWidth, left: box.scrollLeft };
    // Only a change of the axis moves the view; `toToday` reads the same geometry.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [geometry.from, geometry.dayWidth]);

  function remember(): void {
    if (shown.current && scroller.current) {
      shown.current.left = scroller.current.scrollLeft;
    }
  }

  return { scroller, corner, remember, toToday };
}

function Chart({ data, onDates, onEdit }: { data: TimelineProject } & Pick<RowProps, 'onDates' | 'onEdit'>) {
  const [stored, setZoom] = useStored<Zoom>('timeline:zoom', 'months');
  const [open, setOpen] = useState<string[]>([]);
  const zoom = ZOOMS.includes(stored) ? stored : 'months';
  const now = today();
  const mentioned = useMemo(
    () => [
      ...daysOf(data.epics.flatMap((epic) => [epic.start, epic.due, ...epic.children.flatMap((child) => [child.start, child.due])])),
      ...daysOf(data.sprints.flatMap((sprint) => [sprint.start, sprint.end])),
      ...daysOf(data.versions.map((version) => version.releaseDate)),
    ],
    [data],
  );
  const geometry = useMemo(() => geometryFor(zoom, [now, ...mentioned]), [zoom, now, mentioned]);
  const { major, minor } = useMemo(() => header(zoom, geometry), [zoom, geometry]);
  const { scroller, corner, remember, toToday } = useAnchoredScroll(geometry);
  const context = { geometry, sprints: data.sprints, versions: data.versions, now, major, minor };
  const toggle = (key: string) => setOpen(open.includes(key) ? open.filter((other) => other !== key) : [...open, key]);

  return (
    <section className="panel timeline">
      <div className="panel-head timeline-tools">
        <div className="timeline-switch" role="group" aria-label={t('Zoom')}>
          {ZOOMS.map((option) => (
            <button key={option} type="button" className={option === zoom ? 'timeline-switch-on' : undefined} aria-pressed={option === zoom} onClick={() => setZoom(option)}>
              {zoomLabel(option)}
            </button>
          ))}
        </div>
        <Button icon="calendar" onClick={() => toToday('smooth')}>
          {t('Today')}
        </Button>
        <Legend />
      </div>
      <div className="timeline-scroll" ref={scroller} onScroll={remember}>
        <div className="timeline-canvas">
          <div className="timeline-head">
            <div className="timeline-side timeline-corner" ref={corner}>
              {t('Epic')}
            </div>
            <Axis {...context} />
          </div>
          <div className="timeline-body" role="list">
            <Backdrop {...context} />
            {data.epics.map((epic) => (
              <EpicRow key={epic.id} epic={epic} open={open.includes(epic.key)} onToggle={() => toggle(epic.key)} geometry={geometry} editable={data.canEdit} onDates={onDates} onEdit={onEdit} />
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}

export function TimelineView() {
  const project = useProject();
  const base = '/api/m/timeline';
  const remote = useRemote(() => api.get<TimelineProject>(`${base}/projects/${project.key}`), [project.key], isOurs(project));
  const [editing, setEditing] = useState<DatesTarget | null>(null);
  const [creating, setCreating] = useState(false);
  const data = remote.data;
  const epicType = project.types.find((type) => type.level === 'epic');

  /** Shows the new dates at once; the server's answer confirms them, a refusal takes them back. */
  async function setDates(key: string, start: string | null, due: string | null): Promise<void> {
    if (!data) {
      return;
    }

    remote.set(withDates(data, key, start, due));

    try {
      const row = await api.put<TimelineEpic>(`${base}/issues/${key}/span`, { start, due });

      if (data.epics.some((epic) => epic.key === key)) {
        remote.set({ ...data, epics: data.epics.map((epic) => (epic.key === key ? row : epic)) });
      } else {
        // A child's dates can move what is estimated for its epic.
        void remote.reload();
      }
    } catch (error) {
      remote.set(data);
      toast(t('The dates of {key} were not saved: {reason}', { key, reason: errorMessage(error) }), 'error');
    }
  }

  return (
    <div className="view timeline-view">
      <header className="view-head">
        <div>
          <h1>{t('Timeline')}</h1>
          <p className="muted">
            {data?.canEdit ? t('Epics from their start date to their due date. Drag a bar to move it, drag an end to change one date.') : t('Epics from their start date to their due date.')}
          </p>
        </div>
      </header>
      {remote.error && <ErrorNote message={remote.error} />}
      {!data && !remote.error && <Spinner />}
      {data && data.epics.length === 0 && (
        <section className="panel">
          <Empty icon="layers" title={t('No epics yet')}>
            <p>{t('An epic is a larger piece of work that groups issues. On the timeline every epic is a bar from its start date to its due date, so you can see what is planned when and how far along it is.')}</p>
            {data.canEdit && epicType && (
              <Button variant="primary" icon="plus" onClick={() => setCreating(true)}>
                {t('Create an epic')}
              </Button>
            )}
          </Empty>
        </section>
      )}
      {data && data.epics.length > 0 && <Chart data={data} onDates={(key, start, due) => void setDates(key, start, due)} onEdit={setEditing} />}
      {editing && <DatesDialog target={editing} onSave={(start, due) => void setDates(editing.key, start, due)} onClose={() => setEditing(null)} />}
      {creating && epicType && <CreateIssueDialog projectKey={project.key} defaults={{ typeId: epicType.id }} onClose={() => setCreating(false)} />}
    </div>
  );
}
