import { useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode, type RefObject } from 'react';

import { currentLanguage, formatDate, formatShortDate, t } from '@/i18n/index.ts';

const HEIGHT = 280;
const MARGIN = { top: 16, right: 22, bottom: 26, left: 40 };
const LABEL_ROOM = 64;
const MAX_BAR = 24;
const BAR_GAP = 2;
const REACH = 32;

export function formatNumber(value: number): string {
  return new Intl.NumberFormat(currentLanguage() === 'pl' ? 'pl-PL' : 'en-GB', { maximumFractionDigits: 1 }).format(value);
}

/** The element's width in pixels, kept current: charts are drawn at their real size so text stays legible. */
function useWidth<T extends HTMLElement>(): [RefObject<T | null>, number] {
  const ref = useRef<T>(null);
  const [width, setWidth] = useState(0);

  useLayoutEffect(() => {
    const element = ref.current;

    if (!element) {
      return;
    }

    setWidth(Math.floor(element.clientWidth));
    const observer = new ResizeObserver((entries) => setWidth(Math.floor(entries[0].contentRect.width)));
    observer.observe(element);

    return () => observer.disconnect();
  }, []);

  return [ref, width];
}

/** Round axis values from zero to at least `max`. */
function axisTicks(max: number, integers: boolean): number[] {
  const wanted = 4;
  const rough = Math.max(max, integers ? wanted : 0.0001) / wanted;
  const power = 10 ** Math.floor(Math.log10(rough));
  const multiples = integers && power < 2 ? [1, 2, 5, 10] : [1, 2, 2.5, 5, 10];
  const step = (multiples.find((multiple) => multiple * power >= rough) ?? 10) * power;
  const ticks = [0];

  while (ticks[ticks.length - 1] < max || ticks.length < 2) {
    ticks.push(Math.round((ticks[ticks.length - 1] + step) * 1000) / 1000);
  }

  return ticks;
}

type Frame = { width: number; plotWidth: number; plotHeight: number; top: number; y: (value: number) => number; ticks: number[] };

function frameFor(width: number, max: number, integers: boolean): Frame {
  const ticks = axisTicks(max, integers);
  const top = ticks[ticks.length - 1];
  const plotWidth = Math.max(width - MARGIN.left - MARGIN.right, 10);
  const plotHeight = HEIGHT - MARGIN.top - MARGIN.bottom;

  return { width, plotWidth, plotHeight, top, ticks, y: (value) => MARGIN.top + plotHeight - (value / top) * plotHeight };
}

function Grid({ frame }: { frame: Frame }) {
  return (
    <g aria-hidden="true">
      {frame.ticks.map((tick) => (
        <g key={tick}>
          <line className={tick === 0 ? 'reports-axis' : 'reports-grid'} x1={MARGIN.left} x2={MARGIN.left + frame.plotWidth} y1={frame.y(tick)} y2={frame.y(tick)} />
          <text className="reports-tick" x={MARGIN.left - 8} y={frame.y(tick)} dy="0.32em" textAnchor="end">
            {formatNumber(tick)}
          </text>
        </g>
      ))}
    </g>
  );
}

function Tip({ x, frame, children }: { x: number; frame: Frame; children: ReactNode }) {
  return (
    <div className={x > frame.width / 2 ? 'reports-tip reports-tip-before' : 'reports-tip'} style={{ left: x, top: MARGIN.top }} role="status">
      {children}
    </div>
  );
}

export type SwatchKind = 'line' | 'dash' | 'box' | 'dot';

export function Swatch({ color, kind }: { color: string; kind: SwatchKind }) {
  return <span className={`reports-swatch reports-swatch-${kind}`} style={{ color }} aria-hidden="true" />;
}

/** One line of a tooltip: the number leads, what it is follows. */
export function TipRow({ color, kind = 'line', value, label }: { color?: string; kind?: SwatchKind; value: string; label: string }) {
  return (
    <div className="reports-tip-row">
      {color ? <Swatch color={color} kind={kind} /> : <span className="reports-swatch" />}
      <strong>{value}</strong>
      <span>{label}</span>
    </div>
  );
}

export type LegendItem = { label: string; color: string; kind: SwatchKind; value?: string };

export function Legend({ items }: { items: LegendItem[] }) {
  return (
    <ul className="reports-legend">
      {items.map((item) => (
        <li key={item.label}>
          <Swatch color={item.color} kind={item.kind} />
          <span>{item.label}</span>
          {item.value !== undefined && <strong>{item.value}</strong>}
        </li>
      ))}
    </ul>
  );
}

function steps(event: KeyboardEvent): number {
  return event.key === 'ArrowLeft' ? -1 : event.key === 'ArrowRight' ? 1 : 0;
}

function linePath(values: (number | null)[], x: (index: number) => number, y: (value: number) => number): string {
  let path = '';
  let drawing = false;

  values.forEach((value, index) => {
    if (value === null) {
      drawing = false;

      return;
    }

    path += `${drawing ? 'L' : 'M'}${x(index).toFixed(1)} ${y(value).toFixed(1)}`;
    drawing = true;
  });

  return path;
}

/** The band between two lines, as far as both are known. */
function bandPath(upper: (number | null)[], lower: (number | null)[] | null, x: (index: number) => number, y: (value: number) => number): string {
  const known = upper.map((value, index) => ({ index, value })).filter((point): point is { index: number; value: number } => point.value !== null);

  if (known.length < 2) {
    return '';
  }

  const forward = known.map((point) => `${x(point.index).toFixed(1)} ${y(point.value).toFixed(1)}`);
  const back = [...known].reverse().map((point) => `${x(point.index).toFixed(1)} ${y(lower?.[point.index] ?? 0).toFixed(1)}`);

  return `M${forward.join('L')}L${back.join('L')}Z`;
}

function lastKnown(values: (number | null)[], from: number): number | null {
  for (let index = Math.min(from, values.length - 1); index >= 0; index -= 1) {
    const value = values[index];

    if (value !== null) {
      return value;
    }
  }

  return null;
}

export type TimeSeries = { id: string; label: string; color: string; values: (number | null)[]; dashed?: boolean; wash?: boolean };
export type TimeMark = { index: number; value: number; text: string };

type TimeChartProps = {
  label: string;
  /** `YYYY-MM-DD`, one per position. */
  dates: string[];
  /** In drawing order: the last is on top (and, when stacked, the top band). */
  series: TimeSeries[];
  stacked?: boolean;
  integers?: boolean;
  /** Notes on single points, e.g. scope added that day. */
  marks?: TimeMark[];
  /** The position whose values the legend shows; the last one unless given. */
  legendAt?: number;
  /** More lines for the tooltip of one day. */
  detail?: (index: number) => ReactNode;
};

/** Lines over days, or stacked bands, with a crosshair that reads every series at once. */
export function TimeChart({ label, dates, series, stacked = false, integers = true, marks = [], legendAt, detail }: TimeChartProps) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [active, setActive] = useState<number | null>(null);
  const count = dates.length;
  const tops = series.map((entry, position) =>
    stacked ? entry.values.map((_, index) => series.slice(0, position + 1).reduce((sum, below) => sum + (below.values[index] ?? 0), 0)) : entry.values,
  );
  const highest = Math.max(0, ...tops.flat().filter((value): value is number => value !== null), ...marks.map((mark) => mark.value));
  const frame = frameFor(width, highest, integers);
  const x = (index: number) => MARGIN.left + (count > 1 ? (index / (count - 1)) * frame.plotWidth : frame.plotWidth / 2);
  const every = Math.max(1, Math.ceil(count / Math.max(2, Math.floor(frame.plotWidth / LABEL_ROOM))));
  const shown = [...series].reverse();
  const legendIndex = legendAt ?? count - 1;
  const kind = (entry: TimeSeries): SwatchKind => (stacked ? 'box' : entry.dashed ? 'dash' : 'line');

  function locate(event: PointerEvent<SVGSVGElement>): void {
    const position = event.clientX - event.currentTarget.getBoundingClientRect().left;
    const index = count > 1 ? Math.round(((position - MARGIN.left) / frame.plotWidth) * (count - 1)) : 0;

    setActive(Math.min(count - 1, Math.max(0, index)));
  }

  function onKey(event: KeyboardEvent): void {
    const move = steps(event);

    if (move !== 0) {
      event.preventDefault();
      setActive(Math.min(count - 1, Math.max(0, (active ?? legendIndex) + move)));
    }
  }

  return (
    <div className="reports-figure">
      <Legend
        items={shown.map((entry) => {
          const value = lastKnown(entry.values, legendIndex);

          return { label: entry.label, color: entry.color, kind: kind(entry), value: value === null ? undefined : formatNumber(value) };
        })}
      />
      <div
        ref={ref}
        className="reports-chart"
        tabIndex={0}
        role="group"
        aria-label={label}
        onKeyDown={onKey}
        onFocus={() => setActive(active ?? legendIndex)}
        onBlur={() => setActive(null)}
      >
        {width > 0 && (
          <svg viewBox={`0 0 ${width} ${HEIGHT}`} width={width} height={HEIGHT} onPointerMove={locate} onPointerLeave={() => setActive(null)}>
            <Grid frame={frame} />
            {dates.map(
              (date, index) =>
                index % every === 0 && (
                  <text key={date} className="reports-tick" x={x(index)} y={HEIGHT - 8} textAnchor={index === count - 1 && count > 1 ? 'end' : 'middle'}>
                    {formatShortDate(date)}
                  </text>
                ),
            )}
            {series.map((entry, position) => (
              <g key={entry.id} style={{ color: entry.color }}>
                {stacked && <path className="reports-band" d={bandPath(tops[position], position > 0 ? tops[position - 1] : null, x, frame.y)} />}
                {entry.wash && <path className="reports-wash" d={bandPath(tops[position], null, x, frame.y)} />}
                <path className={entry.dashed ? 'reports-line reports-line-dashed' : 'reports-line'} d={linePath(tops[position], x, frame.y)} />
              </g>
            ))}
            {marks.map((mark) => (
              <g key={mark.index}>
                <circle className="reports-mark-dot" cx={x(mark.index)} cy={frame.y(mark.value)} r={4} />
                <text className="reports-mark" x={x(mark.index)} y={frame.y(mark.value) - 9} textAnchor="middle">
                  {mark.text}
                </text>
              </g>
            ))}
            {active !== null && (
              <g aria-hidden="true">
                <line className="reports-cross" x1={x(active)} x2={x(active)} y1={MARGIN.top} y2={MARGIN.top + frame.plotHeight} />
                {series.map((entry, position) => {
                  const value = tops[position][active];

                  return value === null ? null : <circle key={entry.id} className="reports-dot" style={{ color: entry.color }} cx={x(active)} cy={frame.y(value)} r={4.5} />;
                })}
              </g>
            )}
          </svg>
        )}
        {active !== null && width > 0 && (
          <Tip x={x(active)} frame={frame}>
            <div className="reports-tip-title">{formatDate(dates[active])}</div>
            {shown.map((entry) => {
              const value = entry.values[active];

              return value === null ? null : <TipRow key={entry.id} color={entry.color} kind={kind(entry)} value={formatNumber(value)} label={entry.label} />;
            })}
            {detail?.(active)}
          </Tip>
        )}
      </div>
    </div>
  );
}

/** A column that is square on the baseline and rounded where the value ends. */
function columnPath(left: number, top: number, width: number, bottom: number): string {
  const radius = Math.min(4, width / 2, bottom - top);

  return `M${left} ${bottom}V${top + radius}Q${left} ${top} ${left + radius} ${top}H${left + width - radius}Q${left + width} ${top} ${left + width} ${top + radius}V${bottom}Z`;
}

function fitted(text: string, room: number): string {
  const letters = Math.max(3, Math.floor(room / 6.4));

  return text.length > letters ? `${text.slice(0, letters - 1).trimEnd()}…` : text;
}

export type ColumnGroup = { key: string; label: string; values: number[] };

type ColumnChartProps = {
  label: string;
  groups: ColumnGroup[];
  series: { label: string; color: string }[];
  /** A level drawn across all groups, e.g. the average. */
  reference?: { value: number; label: string };
  detail: (index: number) => ReactNode;
};

/** Groups of thin columns side by side, one colour per series. */
export function ColumnChart({ label, groups, series, reference, detail }: ColumnChartProps) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [active, setActive] = useState<number | null>(null);
  const highest = Math.max(0, reference?.value ?? 0, ...groups.flatMap((group) => group.values));
  const frame = frameFor(width, highest, true);
  const band = frame.plotWidth / Math.max(groups.length, 1);
  const thickness = Math.max(3, Math.min(MAX_BAR, (band * 0.6 - BAR_GAP * (series.length - 1)) / series.length));
  const groupWidth = thickness * series.length + BAR_GAP * (series.length - 1);
  const bottom = frame.y(0);
  const center = (index: number) => MARGIN.left + band * (index + 0.5);
  const capped = band >= 56;

  return (
    <div className="reports-figure">
      <Legend
        items={[
          ...series.map((entry) => ({ label: entry.label, color: entry.color, kind: 'box' as const })),
          ...(reference ? [{ label: reference.label, color: 'var(--muted)', kind: 'line' as const, value: formatNumber(reference.value) }] : []),
        ]}
      />
      <div ref={ref} className="reports-chart" role="group" aria-label={label}>
        {width > 0 && (
          <svg viewBox={`0 0 ${width} ${HEIGHT}`} width={width} height={HEIGHT} onPointerLeave={() => setActive(null)}>
            <Grid frame={frame} />
            {groups.map((group, index) => (
              <g key={group.key} className="reports-group" tabIndex={0} onFocus={() => setActive(index)} onBlur={() => setActive(null)} onPointerEnter={() => setActive(index)}>
                <rect className={active === index ? 'reports-slot reports-slot-on' : 'reports-slot'} x={MARGIN.left + band * index} y={MARGIN.top} width={band} height={frame.plotHeight} />
                {group.values.map((value, position) => {
                  const left = center(index) - groupWidth / 2 + position * (thickness + BAR_GAP);

                  return (
                    <g key={series[position].label} style={{ color: series[position].color }}>
                      {value > 0 && <path className="reports-bar" d={columnPath(left, frame.y(value), thickness, bottom)} />}
                      {capped && (
                        <text className="reports-cap" x={left + thickness / 2} y={frame.y(value) - 5} textAnchor="middle">
                          {formatNumber(value)}
                        </text>
                      )}
                    </g>
                  );
                })}
                <text className="reports-tick" x={center(index)} y={HEIGHT - 8} textAnchor="middle">
                  {fitted(group.label, band - 8)}
                </text>
                <title>{`${group.label}: ${group.values.map((value, position) => `${series[position].label} ${formatNumber(value)}`).join(', ')}`}</title>
              </g>
            ))}
            {reference && <line className="reports-reference" x1={MARGIN.left} x2={MARGIN.left + frame.plotWidth} y1={frame.y(reference.value)} y2={frame.y(reference.value)} />}
          </svg>
        )}
        {active !== null && width > 0 && (
          <Tip x={center(active)} frame={frame}>
            <div className="reports-tip-title">{groups[active].label}</div>
            {detail(active)}
          </Tip>
        )}
      </div>
    </div>
  );
}

export type ScatterPoint = { key: string; at: number; value: number };

type ScatterChartProps = {
  label: string;
  /** In time order. */
  points: ScatterPoint[];
  from: number;
  until: number;
  color: string;
  /** Levels worth reading the dots against, e.g. percentiles. */
  guides: { value: number; label: string }[];
  detail: (index: number) => ReactNode;
  onSelect: (index: number) => void;
};

/** One dot per item over time. The pointer only has to be near a dot, not on it. */
export function ScatterChart({ label, points, from, until, color, guides, detail, onSelect }: ScatterChartProps) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [active, setActive] = useState<number | null>(null);
  const highest = Math.max(0, ...points.map((point) => point.value), ...guides.map((guide) => guide.value));
  const frame = frameFor(width, highest, false);
  const span = Math.max(until - from, 1);
  const x = (at: number) => MARGIN.left + (Math.min(Math.max(at - from, 0), span) / span) * frame.plotWidth;
  const labels = Math.max(2, Math.floor(frame.plotWidth / LABEL_ROOM));
  const lines = guides
    .map((guide) => ({ ...guide, y: frame.y(guide.value) }))
    .sort((a, b) => b.y - a.y)
    .reduce<{ y: number; label: string }[]>((merged, guide) => {
      const previous = merged[merged.length - 1];

      // Levels too close to label apart share one label; each still gets its line.
      if (previous && previous.y - guide.y < 14) {
        previous.label = `${previous.label} · ${guide.label}`;
        previous.y = guide.y;
      } else {
        merged.push({ y: guide.y, label: guide.label });
      }

      return merged;
    }, []);

  function locate(event: PointerEvent<SVGSVGElement>): void {
    const box = event.currentTarget.getBoundingClientRect();
    const [pointerX, pointerY] = [event.clientX - box.left, event.clientY - box.top];
    let nearest: number | null = null;
    let reach = REACH;

    points.forEach((point, index) => {
      const distance = Math.hypot(x(point.at) - pointerX, frame.y(point.value) - pointerY);

      if (distance < reach) {
        [nearest, reach] = [index, distance];
      }
    });

    setActive(nearest);
  }

  function onKey(event: KeyboardEvent): void {
    const move = steps(event);

    if (move !== 0 && points.length > 0) {
      event.preventDefault();
      setActive(Math.min(points.length - 1, Math.max(0, (active ?? points.length - 1) + move)));
    } else if (event.key === 'Enter' && active !== null) {
      onSelect(active);
    }
  }

  return (
    <div
      ref={ref}
      className="reports-chart"
      tabIndex={0}
      role="group"
      aria-label={label}
      onKeyDown={onKey}
      onFocus={() => setActive(active ?? (points.length > 0 ? points.length - 1 : null))}
      onBlur={() => setActive(null)}
    >
      {width > 0 && (
        <svg
          viewBox={`0 0 ${width} ${HEIGHT}`}
          width={width}
          height={HEIGHT}
          className={active === null ? undefined : 'reports-pointing'}
          onPointerMove={locate}
          onPointerLeave={() => setActive(null)}
          onClick={() => active !== null && onSelect(active)}
        >
          <Grid frame={frame} />
          {Array.from({ length: labels + 1 }, (_, index) => {
            const at = from + (span * index) / labels;

            return (
              <text key={index} className="reports-tick" x={x(at)} y={HEIGHT - 8} textAnchor={index === labels ? 'end' : index === 0 ? 'start' : 'middle'}>
                {formatShortDate(new Date(at).toISOString())}
              </text>
            );
          })}
          {guides.map((guide) => (
            <line key={guide.label} className="reports-reference" x1={MARGIN.left} x2={MARGIN.left + frame.plotWidth} y1={frame.y(guide.value)} y2={frame.y(guide.value)} />
          ))}
          {lines.map((line) => (
            <text key={line.label} className="reports-guide" x={MARGIN.left + frame.plotWidth} y={line.y - 5} textAnchor="end">
              {line.label}
            </text>
          ))}
          <g style={{ color }}>
            {points.map((point, index) => (
              <circle key={point.key} className={active === index ? 'reports-dot reports-dot-on' : 'reports-dot'} cx={x(point.at)} cy={frame.y(point.value)} r={active === index ? 6 : 4.5}>
                <title>{`${point.key}: ${formatNumber(point.value)}`}</title>
              </circle>
            ))}
          </g>
        </svg>
      )}
      {active !== null && width > 0 && points[active] && (
        <Tip x={x(points[active].at)} frame={frame}>
          {detail(active)}
        </Tip>
      )}
    </div>
  );
}

/** The numbers behind a chart, for reading exactly and for screen readers. */
export function DataTable({ head, rows }: { head: string[]; rows: (string | number)[][] }) {
  return (
    <details className="reports-data">
      <summary>{t('Show the numbers')}</summary>
      <div className="reports-data-scroll">
        <table className="table">
          <thead>
            <tr>
              {head.map((cell) => (
                <th key={cell}>{cell}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, index) => (
              <tr key={index}>
                {row.map((cell, position) => (
                  <td key={position} className={position === 0 ? 'cell-date' : 'reports-number'}>
                    {typeof cell === 'number' ? formatNumber(cell) : cell}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </details>
  );
}
