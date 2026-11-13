// Days and the axis. Every date of the timeline is a calendar day (`YYYY-MM-DD`); here a day
// is its number since 1970-01-01, so that spans are plain arithmetic and no time zone can
// move a bar by a day.

import type { TimelineSpan } from '@/bindings/TimelineSpan.ts';
import { currentLanguage, t } from '@/i18n/index.ts';

const DAY_MS = 86_400_000;

export function toDay(iso: string): number {
  const [year, month, day] = iso.split('-').map(Number);

  return Date.UTC(year, month - 1, day) / DAY_MS;
}

export function toIso(day: number): string {
  return new Date(day * DAY_MS).toISOString().slice(0, 10);
}

/** Today on the viewer's calendar. */
export function today(): number {
  const now = new Date();

  return Date.UTC(now.getFullYear(), now.getMonth(), now.getDate()) / DAY_MS;
}

function parts(day: number): { year: number; month: number } {
  const date = new Date(day * DAY_MS);

  return { year: date.getUTCFullYear(), month: date.getUTCMonth() };
}

/** The first day of the month `months` after the month of `day`. */
function monthStart(day: number, months = 0): number {
  const { year, month } = parts(day);

  return Date.UTC(year, month + months, 1) / DAY_MS;
}

function quarterStart(day: number, quarters = 0): number {
  const { year, month } = parts(day);

  return Date.UTC(year, month - (month % 3) + quarters * 3, 1) / DAY_MS;
}

/** The Monday of the week of `day`. */
function weekStart(day: number): number {
  // 1970-01-01 was a Thursday.
  return day - ((day + 3) % 7);
}

function label(day: number, options: Intl.DateTimeFormatOptions): string {
  return new Date(day * DAY_MS).toLocaleDateString(currentLanguage() === 'pl' ? 'pl-PL' : 'en-GB', { timeZone: 'UTC', ...options });
}

// ---------- zoom ----------

export type Zoom = 'weeks' | 'months' | 'quarters';

export const ZOOMS: Zoom[] = ['weeks', 'months', 'quarters'];

/** Pixels per day, and how much empty time is kept before and after everything shown. */
const SCALE: Record<Zoom, { dayWidth: number; before: number; after: number }> = {
  weeks: { dayWidth: 20, before: 21, after: 42 },
  months: { dayWidth: 7, before: 45, after: 90 },
  quarters: { dayWidth: 2.5, before: 90, after: 180 },
};

/** The axis is never narrower than this, so that a wide screen is filled. */
const MIN_WIDTH = 2000;

/** Where days are on the axis: `[from, to)`, and `x` of the left edge of a day. */
export type Geometry = { from: number; to: number; dayWidth: number; width: number; x: (day: number) => number };

/** An axis of whole months (whole quarters when zoomed out) around the days in `shown`. */
export function geometryFor(zoom: Zoom, shown: number[]): Geometry {
  const { dayWidth, before, after } = SCALE[zoom];
  const start = zoom === 'quarters' ? quarterStart : monthStart;
  const from = start(Math.min(...shown) - before);
  let to = start(Math.max(...shown) + after, 1);

  while ((to - from) * dayWidth < MIN_WIDTH) {
    to = start(to, 1);
  }

  return { from, to, dayWidth, width: (to - from) * dayWidth, x: (day) => (day - from) * dayWidth };
}

// ---------- the header ----------

/** A stretch of the axis with a caption: `[from, to)`. */
export type Cell = { from: number; to: number; label: string };

function cells(geometry: Geometry, first: number, next: (day: number) => number, caption: (day: number) => string): Cell[] {
  const found: Cell[] = [];

  for (let day = first; day < geometry.to; day = next(day)) {
    const from = Math.max(day, geometry.from);

    found.push({ from, to: Math.min(next(day), geometry.to), label: day < geometry.from ? '' : caption(day) });
  }

  return found;
}

function months(geometry: Geometry, caption: (day: number) => string): Cell[] {
  return cells(geometry, monthStart(geometry.from), (day) => monthStart(day, 1), caption);
}

function weeks(geometry: Geometry, caption: (day: number) => string): Cell[] {
  return cells(geometry, weekStart(geometry.from), (day) => day + 7, caption);
}

/** The two rows of the header: months over weeks, or quarters over months when zoomed out. */
export function header(zoom: Zoom, geometry: Geometry): { major: Cell[]; minor: Cell[] } {
  const month = (day: number) => label(day, { month: 'long', year: 'numeric' });

  if (zoom === 'quarters') {
    const quarter = (day: number) => t('Q{quarter} {year}', { quarter: Math.floor(parts(day).month / 3) + 1, year: parts(day).year });

    return { major: cells(geometry, quarterStart(geometry.from), (day) => quarterStart(day, 1), quarter), minor: months(geometry, (day) => label(day, { month: 'short' })) };
  }

  return {
    major: months(geometry, month),
    minor: weeks(geometry, (day) => (zoom === 'weeks' ? label(day, { day: 'numeric', month: 'short' }) : label(day, { day: 'numeric' }))),
  };
}

// ---------- bars ----------

/** How long a bar is drawn when only one of its ends is known. */
const OPEN_DAYS = 14;

/**
 * A bar on the axis: the days `from` to `to`, both included. `open` names the end nobody
 * knows (the bar fades out there), `estimated` the ends worked out from the children.
 */
export type Shape = { from: number; to: number; open: 'start' | 'due' | null; estimated: 'start' | 'due' | 'both' | null };

export function shapeOf(span: Pick<TimelineSpan, 'start' | 'due' | 'derived'>): Shape | null {
  const start = span.start === null ? null : toDay(span.start);
  const due = span.due === null ? null : toDay(span.due);
  const estimated = span.derived.start && span.derived.due ? 'both' : span.derived.start ? 'start' : span.derived.due ? 'due' : null;

  if (start !== null && due !== null) {
    return { from: start, to: Math.max(start, due), open: null, estimated };
  }

  if (start !== null) {
    return { from: start, to: start + OPEN_DAYS - 1, open: 'due', estimated };
  }

  return due === null ? null : { from: due - OPEN_DAYS + 1, to: due, open: 'start', estimated };
}

/** The days among `dates`, for choosing what the axis covers. */
export function daysOf(dates: (string | null)[]): number[] {
  return dates.flatMap((date) => (date ? [toDay(date)] : []));
}
