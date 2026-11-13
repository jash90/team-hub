import { useRef, useState, type ReactNode } from 'react';

import { api, query } from '@/api.ts';
import type { Project } from '@/bindings/Project.ts';
import type { ReportBurndown } from '@/bindings/ReportBurndown.ts';
import type { ReportCreatedResolved } from '@/bindings/ReportCreatedResolved.ts';
import type { ReportCumulativeFlow } from '@/bindings/ReportCumulativeFlow.ts';
import type { ReportCycleTime } from '@/bindings/ReportCycleTime.ts';
import type { ReportUnit } from '@/bindings/ReportUnit.ts';
import type { ReportVelocity } from '@/bindings/ReportVelocity.ts';
import type { ReportWorkload } from '@/bindings/ReportWorkload.ts';
import { IssueKey, categoryLabel, openIssue, sprintLabel } from '@/components/issue-bits.tsx';
import { Avatar, Empty, ErrorNote, Picker, Spinner } from '@/components/ui.tsx';
import { formatDate, formatShortDate, t } from '@/i18n/index.ts';
import { useRemote, useSession } from '@/session.tsx';

import { ColumnChart, DataTable, Legend, ScatterChart, TimeChart, TipRow, formatNumber, type TimeMark } from './charts.tsx';

const WORK = 'var(--reports-work)';
const NEW = 'var(--reports-new)';
const DONE = 'var(--reports-done)';
const CONTEXT = 'var(--reports-context)';
const DAY = 24 * 3600 * 1000;

function days(value: number): string {
  return t('{days} d', { days: formatNumber(value) });
}

type Loaded<T> = { data: T | undefined; stale: boolean; error: string | null };

/** One report of the project. While another range or sprint loads, the previous picture stays up. */
function useReport<T>(project: Project, path: string): Loaded<T> {
  const remote = useRemote(
    () => api.get<T>(`/api/m/reports/projects/${project.key}/${path}`),
    [project.key, path],
    (event) => event.projectId === project.id && (/^issue\.(created|updated|transitioned|deleted)$/.test(event.kind) || event.kind.startsWith('sprint.') || event.kind === 'project.updated'),
  );
  const kept = useRef<T | undefined>(undefined);

  if (remote.data !== undefined) {
    kept.current = remote.data;
  }

  return { data: remote.data ?? kept.current, stale: remote.data === undefined && kept.current !== undefined, error: remote.error };
}

function Switch<T extends string | number>({ label, value, options, onChange }: { label: string; value: T; options: { value: T; label: string }[]; onChange: (value: T) => void }) {
  return (
    <div className="reports-switch" role="group" aria-label={label}>
      {options.map((option) => (
        <button key={option.value} type="button" className={option.value === value ? 'reports-switch-on' : undefined} aria-pressed={option.value === value} onClick={() => onChange(option.value)}>
          {option.label}
        </button>
      ))}
    </div>
  );
}

function RangeSwitch({ value, options, onChange }: { value: number; options: number[]; onChange: (value: number) => void }) {
  return <Switch label={t('Period')} value={value} options={options.map((count) => ({ value: count, label: t('{count} days', { count }) }))} onChange={onChange} />;
}

function UnitSwitch({ value, onChange }: { value: ReportUnit; onChange: (value: ReportUnit) => void }) {
  return (
    <Switch
      label={t('Measure in')}
      value={value}
      options={[
        { value: 'points', label: t('Points') },
        { value: 'issues', label: t('Issues') },
      ]}
      onChange={onChange}
    />
  );
}

function Stats({ items }: { items: { label: string; value: string; note?: string }[] }) {
  return (
    <dl className="reports-stats">
      {items.map((item) => (
        <div key={item.label}>
          <dt>{item.label}</dt>
          <dd>
            {item.value}
            {item.note && <small>{item.note}</small>}
          </dd>
        </div>
      ))}
    </dl>
  );
}

type ReportProps<T> = { title: string; reading: string; controls?: ReactNode; report: Loaded<T>; children: (data: T) => ReactNode };

/** The frame every report shares: controls above, then the chart with a line on how to read it. */
function Report<T>({ title, reading, controls, report, children }: ReportProps<T>) {
  return (
    <>
      {controls && <div className="filter-bar">{controls}</div>}
      <section className="panel reports-panel">
        <header className="reports-head">
          <h2>{title}</h2>
          <p className="muted">{reading}</p>
        </header>
        {report.error && report.data === undefined ? (
          <ErrorNote message={report.error} />
        ) : report.data === undefined ? (
          <Spinner />
        ) : (
          <div className={report.stale ? 'reports-body reports-stale' : 'reports-body'}>{children(report.data)}</div>
        )}
      </section>
    </>
  );
}

function addDays(date: string, count: number): string {
  const moved = new Date(`${date}T00:00:00Z`);
  moved.setUTCDate(moved.getUTCDate() + count);

  return moved.toISOString().slice(0, 10);
}

function daysBetween(from: string, to: string): number {
  return Math.round((Date.parse(`${to}T00:00:00Z`) - Date.parse(`${from}T00:00:00Z`)) / DAY);
}

function BurndownChart({ data }: { data: ReportBurndown }) {
  const measured = data.days;
  const first = measured[0].date;
  const last = measured[measured.length - 1];
  // A running sprint is drawn up to its planned end, so the ideal line can be followed down to zero.
  const span = data.sprint?.endDate ? daysBetween(first, data.sprint.endDate) : measured.length - 1;
  const length = Math.max(measured.length, data.sprint?.state === 'active' ? span + 1 : 0);
  const dates = Array.from({ length }, (_, index) => addDays(first, index));
  const ideal = dates.map((_, index) => measured[index]?.ideal ?? (span > 0 ? (data.committed * Math.max(span - index, 0)) / span : 0));
  const scope = dates.map((_, index) => (measured[index] ? measured[index].remaining + measured[index].completed : null));
  const marks: TimeMark[] = measured.flatMap((day, index) => {
    const changes = [day.added > 0 ? `+${formatNumber(day.added)}` : '', day.removed > 0 ? `−${formatNumber(day.removed)}` : ''].filter(Boolean);

    return changes.length > 0 ? [{ index, value: day.remaining + day.completed, text: changes.join(' ') }] : [];
  });
  const added = measured.reduce((sum, day) => sum + day.added, 0);
  const removed = measured.reduce((sum, day) => sum + day.removed, 0);

  return (
    <>
      <Stats
        items={[
          { label: t('Committed'), value: formatNumber(data.committed) },
          { label: t('Remaining'), value: formatNumber(last.remaining) },
          { label: t('Completed'), value: formatNumber(last.completed) },
          { label: t('Scope changes'), value: `+${formatNumber(added)} / −${formatNumber(removed)}`, note: t('added / removed') },
        ]}
      />
      <TimeChart
        label={t('Burndown')}
        dates={dates}
        integers={Number.isInteger(data.committed)}
        legendAt={measured.length - 1}
        marks={marks}
        series={[
          { id: 'ideal', label: t('Ideal'), color: CONTEXT, values: ideal, dashed: true },
          { id: 'scope', label: t('Scope'), color: NEW, values: scope },
          { id: 'remaining', label: t('Remaining'), color: WORK, values: dates.map((_, index) => measured[index]?.remaining ?? null), wash: true },
        ]}
        detail={(index) => {
          const day = measured[index];

          return day ? (
            <>
              <TipRow value={formatNumber(day.completed)} label={t('Completed')} />
              {day.added > 0 && <TipRow value={`+${formatNumber(day.added)}`} label={t('added that day')} />}
              {day.removed > 0 && <TipRow value={`−${formatNumber(day.removed)}`} label={t('removed that day')} />}
            </>
          ) : null;
        }}
      />
      <DataTable
        head={[t('Day'), t('Remaining'), t('Ideal'), t('Completed'), t('Added'), t('Removed')]}
        rows={measured.map((day) => [formatDate(day.date), day.remaining, day.ideal, day.completed, day.added, day.removed])}
      />
    </>
  );
}

export function Burndown({ project }: { project: Project }) {
  const [sprintId, setSprintId] = useState<number | null>(null);
  const [unit, setUnit] = useState<ReportUnit | null>(null);
  const report = useReport<ReportBurndown>(project, `burndown${query({ sprint: sprintId, unit })}`);
  const started = project.sprints
    .filter((sprint) => sprint.state !== 'future')
    .sort((a, b) => Number(b.state === 'active') - Number(a.state === 'active') || (b.completedAt ?? '').localeCompare(a.completedAt ?? ''));
  const shownSprint = sprintId ?? report.data?.sprint?.id ?? null;
  const shownUnit = unit ?? report.data?.unit ?? 'points';

  if (started.length === 0) {
    return (
      <Empty icon="chart" title={t('No sprint has been started yet')}>
        {t('Start a sprint from the backlog. Its burndown appears here from the first day.')}
      </Empty>
    );
  }

  return (
    <Report
      title={t('Burndown')}
      reading={t('Work left in the sprint at the end of each day. Under the dashed line the sprint is ahead of a steady pace; a step in the scope line is work added or removed after the start.')}
      report={report}
      controls={
        <>
          <Picker
            label={t('Sprint')}
            value={shownSprint}
            options={started.map((sprint) => ({
              value: sprint.id as number | null,
              label: sprint.state === 'active' ? sprintLabel(sprint) : `${sprint.name} · ${formatShortDate(sprint.startDate)} – ${formatShortDate(sprint.completedAt ?? sprint.endDate)}`,
            }))}
            onChange={setSprintId}
          />
          <UnitSwitch value={shownUnit} onChange={setUnit} />
        </>
      }
    >
      {(data) => (data.days.length === 0 ? <Empty icon="chart" title={t('This sprint has no days to show yet')} /> : <BurndownChart data={data} />)}
    </Report>
  );
}

export function Velocity({ project }: { project: Project }) {
  const [unit, setUnit] = useState<ReportUnit>('points');
  const report = useReport<ReportVelocity>(project, 'velocity?sprints=8');
  const committed = (sprint: ReportVelocity['sprints'][number]) => (unit === 'points' ? sprint.committedPoints : sprint.committedIssues);
  const completed = (sprint: ReportVelocity['sprints'][number]) => (unit === 'points' ? sprint.completedPoints : sprint.completedIssues);

  return (
    <Report
      title={t('Velocity')}
      reading={t('What each completed sprint took on at its start and what it finished. The line is the average of finished work: a fair guide to how much to plan next.')}
      report={report}
      controls={<UnitSwitch value={unit} onChange={setUnit} />}
    >
      {(data) => {
        if (data.sprints.length === 0) {
          return (
            <Empty icon="chart" title={t('No sprint has been completed yet')}>
              {t('Velocity compares completed sprints; the first one shows up here when it is completed.')}
            </Empty>
          );
        }

        const average = unit === 'points' ? data.averageCompletedPoints : data.sprints.reduce((sum, sprint) => sum + completed(sprint), 0) / data.sprints.length;
        const latest = data.sprints[data.sprints.length - 1];

        return (
          <>
            <Stats
              items={[
                { label: t('Average completed'), value: formatNumber(average), note: t('per sprint') },
                { label: t('Last sprint'), value: `${formatNumber(completed(latest))} / ${formatNumber(committed(latest))}`, note: t('completed / committed') },
              ]}
            />
            <ColumnChart
              label={t('Velocity')}
              groups={data.sprints.map((sprint) => ({ key: String(sprint.sprint.id), label: sprint.sprint.name, values: [committed(sprint), completed(sprint)] }))}
              series={[
                { label: t('Committed'), color: CONTEXT },
                { label: t('Completed'), color: WORK },
              ]}
              reference={{ value: average, label: t('Average completed') }}
              detail={(index) => {
                const sprint = data.sprints[index];

                return (
                  <>
                    <div className="muted">
                      {formatShortDate(sprint.sprint.startDate)} – {formatShortDate(sprint.sprint.completedAt)}
                    </div>
                    <TipRow color={CONTEXT} kind="box" value={formatNumber(committed(sprint))} label={t('Committed')} />
                    <TipRow color={WORK} kind="box" value={formatNumber(completed(sprint))} label={t('Completed')} />
                  </>
                );
              }}
            />
            <DataTable
              head={[t('Sprint'), t('Committed points'), t('Completed points'), t('Committed issues'), t('Completed issues')]}
              rows={data.sprints.map((sprint) => [sprint.sprint.name, sprint.committedPoints, sprint.completedPoints, sprint.committedIssues, sprint.completedIssues])}
            />
          </>
        );
      }}
    </Report>
  );
}

export function CumulativeFlow({ project }: { project: Project }) {
  const [range, setRange] = useState(30);
  const report = useReport<ReportCumulativeFlow>(project, `cumulative-flow?days=${range}`);

  return (
    <Report
      title={t('Cumulative flow')}
      reading={t('How many issues were in each stage at the end of each day. A band that keeps widening is where work piles up; the top edge is everything ever created.')}
      report={report}
      controls={<RangeSwitch value={range} options={[14, 30, 90]} onChange={setRange} />}
    >
      {(data) => {
        const latest = data.days[data.days.length - 1];

        if (!latest || latest.todo + latest.inProgress + latest.done === 0) {
          return <Empty icon="chart" title={t('There are no issues to count yet')} />;
        }

        return (
          <>
            <TimeChart
              label={t('Cumulative flow')}
              dates={data.days.map((day) => day.date)}
              stacked
              series={[
                { id: 'done', label: categoryLabel('done'), color: DONE, values: data.days.map((day) => day.done) },
                { id: 'in_progress', label: categoryLabel('in_progress'), color: WORK, values: data.days.map((day) => day.inProgress) },
                { id: 'todo', label: categoryLabel('todo'), color: CONTEXT, values: data.days.map((day) => day.todo) },
              ]}
              detail={(index) => <TipRow value={formatNumber(data.days[index].todo + data.days[index].inProgress + data.days[index].done)} label={t('in total')} />}
            />
            <DataTable
              head={[t('Day'), categoryLabel('todo'), categoryLabel('in_progress'), categoryLabel('done')]}
              rows={data.days.map((day) => [formatDate(day.date), day.todo, day.inProgress, day.done])}
            />
          </>
        );
      }}
    </Report>
  );
}

export function CreatedResolved({ project }: { project: Project }) {
  const [range, setRange] = useState(30);
  const report = useReport<ReportCreatedResolved>(project, `created-vs-resolved?days=${range}`);

  return (
    <Report
      title={t('Created vs resolved')}
      reading={t('Issues created and issues resolved, added up from the first day shown. While the lines drift apart, more work arrives than gets finished.')}
      report={report}
      controls={<RangeSwitch value={range} options={[14, 30, 90]} onChange={setRange} />}
    >
      {(data) => {
        if (data.created + data.resolved === 0) {
          return <Empty icon="chart" title={t('Nothing was created or resolved in this period')} />;
        }

        const net = data.created - data.resolved;

        return (
          <>
            <Stats
              items={[
                { label: t('Created'), value: formatNumber(data.created) },
                { label: t('Resolved'), value: formatNumber(data.resolved) },
                { label: t('Open work changed by'), value: `${net > 0 ? '+' : net < 0 ? '−' : ''}${formatNumber(Math.abs(net))}` },
              ]}
            />
            <TimeChart
              label={t('Created vs resolved')}
              dates={data.days.map((day) => day.date)}
              series={[
                { id: 'resolved', label: t('Resolved'), color: DONE, values: data.days.map((day) => day.resolvedTotal) },
                { id: 'created', label: t('Created'), color: NEW, values: data.days.map((day) => day.createdTotal) },
              ]}
              detail={(index) => (
                <>
                  <TipRow value={`+${formatNumber(data.days[index].created)}`} label={t('created that day')} />
                  <TipRow value={`+${formatNumber(data.days[index].resolved)}`} label={t('resolved that day')} />
                </>
              )}
            />
            <DataTable
              head={[t('Day'), t('Created'), t('Resolved'), t('Created in total'), t('Resolved in total')]}
              rows={data.days.map((day) => [formatDate(day.date), day.created, day.resolved, day.createdTotal, day.resolvedTotal])}
            />
          </>
        );
      }}
    </Report>
  );
}

type Timing = 'lead' | 'cycle';

export function CycleTime({ project }: { project: Project }) {
  const [range, setRange] = useState(60);
  const [timing, setTiming] = useState<Timing>('lead');
  const report = useReport<ReportCycleTime>(project, `cycle-time?days=${range}`);

  return (
    <Report
      title={t('Cycle time')}
      reading={t('Each dot is a resolved issue; the higher it sits, the longer it took. 85 of 100 issues were finished within the p85 line, which makes it a fair promise for the next one.')}
      report={report}
      controls={
        <>
          <Switch
            label={t('Measure')}
            value={timing}
            options={[
              { value: 'lead', label: t('From creation') },
              { value: 'cycle', label: t('From start of work') },
            ]}
            onChange={setTiming}
          />
          <RangeSwitch value={range} options={[30, 60, 90, 180]} onChange={setRange} />
        </>
      }
    >
      {(data) => {
        if (data.issues.length === 0) {
          return <Empty icon="chart" title={t('No issue was resolved in this period')} />;
        }

        const percentiles = timing === 'lead' ? data.lead : data.cycle;
        const measured = data.issues.flatMap((issue) => {
          const value = timing === 'lead' ? issue.leadDays : issue.cycleDays;

          return value === null ? [] : [{ issue, value }];
        });
        const guides = [
          { name: 'p50', value: percentiles.p50 },
          { name: 'p85', value: percentiles.p85 },
          { name: 'p95', value: percentiles.p95 },
        ].flatMap((guide) => (guide.value === null ? [] : [{ name: guide.name, value: guide.value }]));
        const until = Date.now();

        return (
          <>
            <Stats
              items={[
                ...guides.map((guide) => ({ label: guide.name, value: days(guide.value), note: guide.name === 'p50' ? t('half of the issues') : undefined })),
                { label: t('Issues'), value: formatNumber(measured.length), note: measured.length < data.issues.length ? t('{count} never in progress', { count: data.issues.length - measured.length }) : undefined },
              ]}
            />
            <Legend items={[{ label: timing === 'lead' ? t('Days from creation to resolution') : t('Days from the start of work to resolution'), color: WORK, kind: 'dot' }]} />
            <ScatterChart
              label={t('Cycle time')}
              points={measured.map(({ issue, value }) => ({ key: issue.key, at: Date.parse(issue.resolvedAt), value }))}
              from={until - range * DAY}
              until={until}
              color={WORK}
              guides={guides.map((guide) => ({ value: guide.value, label: `${guide.name} · ${days(guide.value)}` }))}
              onSelect={(index) => openIssue(measured[index].issue.key)}
              detail={(index) => {
                const { issue } = measured[index];

                return (
                  <>
                    <div className="reports-tip-title">
                      {issue.key} · {issue.summary}
                    </div>
                    <div className="muted">{t('resolved {when}', { when: formatDate(issue.resolvedAt) })}</div>
                    <TipRow bare value={days(issue.leadDays)} label={t('from creation')} />
                    {issue.cycleDays !== null && <TipRow bare value={days(issue.cycleDays)} label={t('from start of work')} />}
                  </>
                );
              }}
            />
            <div className="reports-data-scroll">
              <table className="table">
                <thead>
                  <tr>
                    <th>{t('Issue')}</th>
                    <th>{t('Summary')}</th>
                    <th>{t('Resolved')}</th>
                    <th className="reports-number">{t('From creation')}</th>
                    <th className="reports-number">{t('From start of work')}</th>
                  </tr>
                </thead>
                <tbody>
                  {[...data.issues].reverse().map((issue) => (
                    <tr key={issue.key}>
                      <td className="cell-key">
                        <IssueKey issue={issue} />
                      </td>
                      <td>{issue.summary}</td>
                      <td className="cell-date muted">{formatDate(issue.resolvedAt)}</td>
                      <td className="reports-number">{days(issue.leadDays)}</td>
                      <td className="reports-number">{issue.cycleDays === null ? '–' : days(issue.cycleDays)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </>
        );
      }}
    </Report>
  );
}

export function Workload({ project }: { project: Project }) {
  const report = useReport<ReportWorkload>(project, 'workload');
  const { userById } = useSession();

  return (
    <Report title={t('Workload')} reading={t('Open issues per person right now. The solid part of a bar is already in progress, the pale part has not been started.')} report={report}>
      {(data) => {
        if (data.assignees.length === 0) {
          return <Empty icon="chart" title={t('There is no open work')} />;
        }

        const most = Math.max(...data.assignees.map((row) => row.openIssues));

        return (
          <>
            <Legend
              items={[
                { label: categoryLabel('in_progress'), color: WORK, kind: 'box' },
                { label: t('Not started'), color: 'var(--reports-work-pale)', kind: 'box' },
              ]}
            />
            <div className="reports-load">
              <div className="reports-load-row reports-load-head">
                <span />
                <span className="reports-load-track" />
                <span>{t('Open issues')}</span>
                <span>{categoryLabel('in_progress')}</span>
                <span>{t('Points')}</span>
              </div>
              {data.assignees.map((row) => {
                const waiting = row.openIssues - row.inProgress;
                const name = row.assigneeId === null ? t('Unassigned') : row.name;

                return (
                  <div key={row.assigneeId ?? 0} className="reports-load-row" title={`${name}: ${categoryLabel('in_progress')} ${row.inProgress}, ${t('Not started')} ${waiting}`}>
                    <span className="with-avatar reports-load-name">
                      <Avatar user={userById(row.assigneeId)} size={22} label={name} />
                      <span>{name}</span>
                    </span>
                    <span className="reports-load-track">
                      {row.inProgress > 0 && <span className="reports-load-doing" style={{ width: `${(row.inProgress / most) * 100}%` }} />}
                      {waiting > 0 && <span className="reports-load-waiting" style={{ width: `${(waiting / most) * 100}%` }} />}
                    </span>
                    <strong>{row.openIssues}</strong>
                    <span>{row.inProgress}</span>
                    <span>{formatNumber(row.openPoints)}</span>
                  </div>
                );
              })}
            </div>
          </>
        );
      }}
    </Report>
  );
}
