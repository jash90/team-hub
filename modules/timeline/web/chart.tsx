// What is drawn on the axis: its header, what lies behind the rows (sprints, releases,
// today), and the bars, which a person who may edit drags to move and to resize.

import { useState, type PointerEvent } from 'react';

import type { StatusCategory } from '@/bindings/StatusCategory.ts';
import type { TimelineSpan } from '@/bindings/TimelineSpan.ts';
import type { TimelineSprint } from '@/bindings/TimelineSprint.ts';
import type { TimelineVersion } from '@/bindings/TimelineVersion.ts';
import { Icon } from '@/components/icon.tsx';
import { countOf, formatDate, formatShortDate, t } from '@/i18n/index.ts';

import { shapeOf, toDay, toIso, type Cell, type Geometry, type Shape } from './dates.ts';

type Dated<T, K extends keyof T> = T & { [P in K]: string };

/** Sprints that have both dates: only those can be drawn. */
export function datedSprints(sprints: TimelineSprint[]): Dated<TimelineSprint, 'start' | 'end'>[] {
  return sprints.filter((sprint): sprint is Dated<TimelineSprint, 'start' | 'end'> => sprint.start !== null && sprint.end !== null && sprint.start <= sprint.end);
}

export function datedVersions(versions: TimelineVersion[]): Dated<TimelineVersion, 'releaseDate'>[] {
  return versions.filter((version): version is Dated<TimelineVersion, 'releaseDate'> => version.releaseDate !== null);
}

function sprintState(sprint: TimelineSprint): string {
  return sprint.state === 'active' ? t('running') : sprint.state === 'closed' ? t('completed') : t('planned');
}

/** Left edge and width of the days `from` to `to`, both included. */
function box(geometry: Geometry, from: number, to: number): { left: number; width: number } {
  return { left: geometry.x(from), width: (to - from + 1) * geometry.dayWidth };
}

/** The middle of a day: where a line for "on this day" goes. */
function middle(geometry: Geometry, day: number): number {
  return geometry.x(day) + geometry.dayWidth / 2;
}

function Cells({ cells, geometry, className }: { cells: Cell[]; geometry: Geometry; className: string }) {
  return (
    <div className={`timeline-lane ${className}`}>
      {cells.map((cell) => (
        <div key={cell.from} className="timeline-cell" style={box(geometry, cell.from, cell.to - 1)}>
          <span>{cell.label}</span>
        </div>
      ))}
    </div>
  );
}

type Context = { geometry: Geometry; sprints: TimelineSprint[]; versions: TimelineVersion[]; now: number };

/** A sprint narrower than this keeps its name for the tooltip: a stump of it would only mislead. */
const NAMED_FROM = 64;

/** Roughly how wide the flag of a version is, without measuring it. */
function flagWidth(name: string): number {
  return 34 + name.length * 7;
}

/** The versions with a row each, so that flags close together sit one above the other, not on top. */
function stacked(versions: Dated<TimelineVersion, 'releaseDate'>[], geometry: Geometry): { version: Dated<TimelineVersion, 'releaseDate'>; left: number; row: number }[] {
  const ends: number[] = [];

  return [...versions]
    .sort((first, second) => first.releaseDate.localeCompare(second.releaseDate))
    .map((version) => {
      const left = middle(geometry, toDay(version.releaseDate));
      const free = ends.findIndex((end) => end <= left);
      const row = free === -1 ? ends.length : free;
      ends[row] = left + flagWidth(version.name);

      return { version, left, row };
    });
}

/** The header of the axis: two rows of dates, then the sprints and the releases by name. */
export function Axis({ major, minor, geometry, sprints, versions, now }: Context & { major: Cell[]; minor: Cell[] }) {
  const named = datedSprints(sprints);
  const flags = stacked(datedVersions(versions), geometry);
  const flagRows = Math.max(0, ...flags.map((flag) => flag.row + 1));

  return (
    <div className="timeline-axis" style={{ width: geometry.width }}>
      <Cells cells={major} geometry={geometry} className="timeline-major" />
      <Cells cells={minor} geometry={geometry} className="timeline-minor" />
      {named.length > 0 && (
        <div className="timeline-lane timeline-sprints">
          {named.map((sprint) => (
            <div
              key={sprint.id}
              className={`timeline-sprint-name timeline-sprint-${sprint.state}`}
              style={box(geometry, toDay(sprint.start), toDay(sprint.end))}
              title={`${sprint.name} (${sprintState(sprint)}): ${formatDate(sprint.start)} – ${formatDate(sprint.end)}`}
            >
              {box(geometry, toDay(sprint.start), toDay(sprint.end)).width >= NAMED_FROM && <span>{sprint.name}</span>}
            </div>
          ))}
        </div>
      )}
      {flags.length > 0 && (
        <div className="timeline-lane timeline-versions" style={{ height: flagRows * 22 + 2 }}>
          {flags.map(({ version, left, row }) => (
            <div
              key={version.id}
              className={version.released ? 'timeline-version timeline-version-released' : 'timeline-version'}
              style={{ left, top: row * 22 + 2 }}
              title={`${version.name}: ${version.released ? t('released {date}', { date: formatDate(version.releaseDate) }) : t('release planned for {date}', { date: formatDate(version.releaseDate) })}`}
            >
              <Icon name={version.released ? 'check' : 'flag'} size={11} />
              <span>{version.name}</span>
            </div>
          ))}
        </div>
      )}
      <div className="timeline-today-mark" style={{ left: middle(geometry, now) }} title={t('Today')} />
    </div>
  );
}

/** What lies behind every row: the grid, the sprints as bands, a line per release, today. */
export function Backdrop({ major, minor, geometry, sprints, versions, now }: Context & { major: Cell[]; minor: Cell[] }) {
  return (
    <div className="timeline-backdrop" style={{ width: geometry.width }} aria-hidden="true">
      {datedSprints(sprints).map((sprint) => (
        <div key={sprint.id} className={`timeline-band timeline-band-${sprint.state}`} style={box(geometry, toDay(sprint.start), toDay(sprint.end))} />
      ))}
      {minor.map((cell) => (
        <div key={cell.from} className="timeline-gridline" style={{ left: geometry.x(cell.from) }} />
      ))}
      {major.map((cell) => (
        <div key={cell.from} className="timeline-gridline timeline-gridline-major" style={{ left: geometry.x(cell.from) }} />
      ))}
      {datedVersions(versions).map((version) => (
        <div key={version.id} className={version.released ? 'timeline-release-line timeline-release-line-done' : 'timeline-release-line'} style={{ left: middle(geometry, toDay(version.releaseDate)) }} />
      ))}
      <div className="timeline-today" style={{ left: middle(geometry, now) }} />
    </div>
  );
}

// ---------- bars ----------

type Grip = 'move' | 'start' | 'due';
type Drag = { grip: Grip; x: number; days: number };

/** The bar as it would be if the drag ended now. A bar is never shorter than a day. */
function dragged(shape: Shape, drag: Drag | null): Shape {
  if (!drag) {
    return shape;
  }

  if (drag.grip === 'move') {
    return { ...shape, from: shape.from + drag.days, to: shape.to + drag.days };
  }

  return drag.grip === 'start' ? { ...shape, from: Math.min(shape.from + drag.days, shape.to) } : { ...shape, to: Math.max(shape.to + drag.days, shape.from) };
}

/** What a bar says about itself beside the dates: which of them nobody set. */
function caveats(shape: Shape): string[] {
  const estimated = shape.estimated === 'both' ? t('estimated') : shape.estimated === 'start' ? t('start estimated') : shape.estimated === 'due' ? t('due date estimated') : null;
  const open = shape.open === 'start' ? t('no start date') : shape.open === 'due' ? t('no due date') : null;

  return [estimated, open].flatMap((text) => (text ? [text] : []));
}

function period(shape: Shape, format: (iso: string) => string): string {
  if (shape.open) {
    return shape.open === 'due' ? t('from {date}', { date: format(toIso(shape.from)) }) : t('until {date}', { date: format(toIso(shape.to)) });
  }

  return `${format(toIso(shape.from))} – ${format(toIso(shape.to))}`;
}

type BarProps = {
  /** Who the bar belongs to, for people who do not see it. */
  name: string;
  span: Pick<TimelineSpan, 'start' | 'due' | 'derived'>;
  geometry: Geometry;
  /** The share that is done, 0 to 1; a thin bar has none. */
  progress?: number;
  /** A child's bar: thin, in the colour of its status, not for dragging. */
  thin?: StatusCategory;
  /** Called with the new dates when a person has dragged the bar; without it the bar stays put. */
  onChange?: (start: string, due: string) => void;
};

/**
 * One span on the axis. Solid when its dates are set; hatched with a dashed outline, and
 * captioned, when they are estimated; fading out towards an end that is not known.
 * Nothing is drawn for a span without any date.
 */
export function Bar(props: BarProps) {
  const shape = shapeOf(props.span);

  return shape && <ShapedBar {...props} settled={shape} />;
}

function ShapedBar({ name, settled, geometry, progress, thin, onChange }: BarProps & { settled: Shape }) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const shape = dragged(settled, drag);
  const draggable = onChange !== undefined && settled.open === null;
  const notes = caveats(settled);
  const { left, width } = box(geometry, shape.from, shape.to);
  const classes = [
    'timeline-bar',
    thin ? `timeline-bar-thin timeline-bar-${thin}` : '',
    settled.estimated ? 'timeline-bar-estimated' : '',
    settled.open ? `timeline-bar-open-${settled.open}` : '',
    draggable ? 'timeline-bar-draggable' : '',
    drag ? 'timeline-bar-dragging' : '',
    width < 28 ? 'timeline-bar-short' : '',
  ];
  const daysAt = (event: PointerEvent, from: Drag) => Math.round((event.clientX - from.x) / geometry.dayWidth);

  function grab(event: PointerEvent<HTMLDivElement>): void {
    if (!draggable || event.button !== 0) {
      return;
    }

    const grip = (event.target as HTMLElement).dataset.grip as Grip | undefined;

    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    setDrag({ grip: grip ?? 'move', x: event.clientX, days: 0 });
  }

  function release(event: PointerEvent<HTMLDivElement>): void {
    if (!drag) {
      return;
    }

    const dropped = dragged(settled, { ...drag, days: daysAt(event, drag) });
    setDrag(null);

    if (dropped.from !== settled.from || dropped.to !== settled.to) {
      onChange?.(toIso(dropped.from), toIso(dropped.to));
    }
  }

  return (
    <>
      <div
        className={classes.filter(Boolean).join(' ')}
        style={{ left, width }}
        role="img"
        aria-label={[name, period(shape, formatDate), ...notes].join(', ')}
        title={drag ? undefined : [period(shape, formatDate), ...notes].join(' · ')}
        onPointerDown={grab}
        onPointerMove={(event) => drag && setDrag({ ...drag, days: daysAt(event, drag) })}
        onPointerUp={release}
        onPointerCancel={() => setDrag(null)}
      >
        {progress !== undefined && progress > 0 && <span className="timeline-fill" style={{ width: `${Math.min(progress, 1) * 100}%` }} />}
        {draggable && <span className="timeline-grip timeline-grip-start" data-grip="start" />}
        {draggable && <span className="timeline-grip timeline-grip-due" data-grip="due" />}
      </div>
      {drag ? (
        <span className="timeline-bar-dates" style={{ left: left + width / 2 }}>
          {period(shape, formatShortDate)} · {countOf(shape.to - shape.from + 1, 'day', 'days')}
        </span>
      ) : (
        !thin && notes.length > 0 && (
          <span className="timeline-bar-note" style={{ left: left + width + 4 }}>
            {notes.join(' · ')}
          </span>
        )
      )}
    </>
  );
}
