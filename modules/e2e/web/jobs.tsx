import { useLayoutEffect, useRef, useState, type FormEvent, type ReactNode } from 'react';

import { api, query } from '@/api.ts';
import type { E2eJob } from '@/bindings/E2eJob.ts';
import type { E2eJobDetail } from '@/bindings/E2eJobDetail.ts';
import type { E2eJobState } from '@/bindings/E2eJobState.ts';
import type { E2eSettings } from '@/bindings/E2eSettings.ts';
import type { Project } from '@/bindings/Project.ts';
import type { Runner } from '@/bindings/Runner.ts';
import { Icon } from '@/components/icon.tsx';
import { Avatar, Button, Dialog, Empty, ErrorNote, Field, Spinner, Tag, attempt, useBusy } from '@/components/ui.tsx';
import { formatDateTime, t, timeAgo } from '@/i18n/index.ts';
import { translateServer } from '@/i18n/server.ts';
import { Link, navigate } from '@/router.tsx';
import { useProject, useRemote, useSession, type Remote } from '@/session.tsx';

import { BackIcon } from './icons.tsx';
import { BASE, Origin, duration, jobPath, runPath } from './shared.tsx';

/** As many lines as the server keeps of a job's log. */
const LOG_LINES_KEPT = 2000;
const JOBS_SHOWN = 4;

const STATE_TEXT: Record<E2eJobState, string> = {
  queued: 'Waiting for a runner',
  claimed: 'Taken by a runner',
  running: 'Tests are running',
  succeeded: 'Job finished',
  failed: 'Job failed',
  cancelled: 'Job cancelled',
};
const STATE_TONE: Record<E2eJobState, 'neutral' | 'blue' | 'green' | 'red'> = { queued: 'neutral', claimed: 'blue', running: 'blue', succeeded: 'green', failed: 'red', cancelled: 'neutral' };
/** What a runner says it is doing, in the order it does it. Any other word is shown as it came. */
const STEPS: [string, string][] = [
  ['cloning', 'Cloning the repository'],
  ['installing', 'Installing dependencies'],
  ['running', 'Running the suite'],
  ['uploading', 'Sending the report and recordings'],
];
export const VIDEO_MODES: [string, string][] = [
  ['on', 'Record every attempt'],
  ['retain-on-failure', 'Record, keep the attempts that did not pass'],
  ['on-first-retry', 'Record the first retry only'],
  ['off', 'No recording'],
];

function isActive(job: E2eJob): boolean {
  return job.state === 'queued' || job.state === 'claimed' || job.state === 'running';
}

function stepText(step: string): string {
  const known = STEPS.find(([word]) => word === step);

  return known ? t(known[1]) : step;
}

export function JobStateTag({ job }: { job: E2eJob }) {
  return <Tag tone={STATE_TONE[job.state]}>{job.cancelRequested && isActive(job) ? t('Stopping…') : t(STATE_TEXT[job.state])}</Tag>;
}

export function useJobs(project: Project): Remote<E2eJob[]> {
  return useRemote(() => api.get<E2eJob[]>(`${BASE}/projects/${project.key}/jobs`), [project.key], (event) => event.projectId === project.id && event.kind === 'e2e.job');
}

export function useTestSettings(project: Project): Remote<E2eSettings> {
  return useRemote(() => api.get<E2eSettings>(`${BASE}/projects/${project.key}/settings`), [project.key], (event) => event.projectId === project.id && event.kind === 'e2e.settings');
}

/** The runners that take test jobs for this project, connected or not. */
function useTestRunners(project: Project): Remote<Runner[]> {
  return useRemote(
    async () => {
      const all = await api.get<Runner[]>('/api/runners');

      return all.filter((runner) => runner.work.some((work) => work.module === 'e2e' && (work.projects === null || work.projects.some((key) => key.toUpperCase() === project.key))));
    },
    [project.key],
    (event) => event.projectId === project.id && event.kind === 'e2e.job',
  );
}

function canEdit(project: Project): boolean {
  return project.role !== null && project.role !== 'viewer';
}

async function cancel(job: E2eJob): Promise<void> {
  await attempt(() => api.post(`${BASE}/jobs/${job.id}/cancel`), job.state === 'queued' ? t('Job cancelled') : t('The runner is told to stop'));
}

// ---------- starting a job ----------

function RunnerLine({ runner, owner }: { runner: Runner; owner: string | undefined }) {
  const work = runner.work.find((entry) => entry.module === 'e2e');

  return (
    <span className="e2e-runner">
      <span className="e2e-runner-name">
        <span className={runner.online ? 'e2e-dot e2e-dot-on' : 'e2e-dot'} title={runner.online ? t('online') : t('offline')} />
        <strong>{runner.name}</strong>
        <span className="e2e-runner-kind">{runner.shared ? t('shared service') : runner.interactive ? t('computer') : t('service')}</span>
      </span>
      <span className="muted">
        {[owner, work?.pinnedOnly ? t('only what is sent to it') : '', runner.online ? '' : t('last seen {when}', { when: timeAgo(runner.lastSeenAt) })].filter(Boolean).join(' · ')}
      </span>
    </span>
  );
}

function RunDialog({ project, settings, onClose }: { project: Project; settings: E2eSettings; onClose: () => void }) {
  const { user, userById } = useSession();
  const runners = useTestRunners(project);
  const [ref, setRef] = useState('');
  const [filter, setFilter] = useState('');
  const [video, setVideo] = useState<string>(settings.video);
  // A runner's id when the job is sent to that one; null leaves it to the queue.
  const [onRunner, setOnRunner] = useState<string | null>(null);
  const [busy, run] = useBusy();
  const fits = runners.data ?? [];
  // Who can be named: one's own (connected or not: the job waits), and services that are there.
  const nameable = fits.filter((runner) => runner.userId === user.id || (!runner.interactive && runner.online));
  const chosen = nameable.find((runner) => runner.id === onRunner) ?? null;
  const takesQueue = fits.some((runner) => runner.online && runner.work.some((work) => work.module === 'e2e' && !work.pinnedOnly));

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      const job = await api.post<E2eJob>(`${BASE}/projects/${project.key}/jobs`, { ref: ref.trim() || null, filter: filter.trim() || null, video, onRunner: chosen?.id ?? null });
      onClose();
      navigate(jobPath(project.key, job.id));
    });
  }

  return (
    <Dialog
      title={t('Run tests')}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" icon="play" type="submit" form="e2e-run-form" disabled={busy}>
            {t('Run tests')}
          </Button>
        </>
      }
    >
      <form id="e2e-run-form" className="form" onSubmit={submit}>
        <p className="muted">{t('A runner checks {repo} out, runs its e2e suite and sends back the report with its recordings.', { repo: settings.repo ?? '' })}</p>
        <Field label={t('Branch, tag or commit')}>
          <input className="input input-mono" value={ref} placeholder={settings.ref} onChange={(event) => setRef(event.target.value)} />
        </Field>
        <Field label={t('Only these tests (optional)')} hint={t('Test files, or a --grep pattern. Leave empty to run the whole suite.')}>
          <input className="input input-mono" value={filter} placeholder="tests/checkout.e2e.ts" onChange={(event) => setFilter(event.target.value)} />
        </Field>
        <Field label={t('Video')}>
          <select className="input" value={video} onChange={(event) => setVideo(event.target.value)}>
            {VIDEO_MODES.map(([mode, label]) => (
              <option key={mode} value={mode}>
                {t(label)}
              </option>
            ))}
          </select>
        </Field>
        <div className="e2e-where-pick" role="radiogroup" aria-label={t('Where it is carried out')}>
          <strong>{t('Where it is carried out')}</strong>
          {!runners.data && !runners.error && <Spinner />}
          <label className="e2e-where">
            <input type="radio" name="e2e-where" checked={!chosen} onChange={() => setOnRunner(null)} />
            <span>
              <strong>{t('Any runner that fits')}</strong>
              <span className="muted">{t('The first connected runner that tests this project takes the job.')}</span>
            </span>
          </label>
          {nameable.map((runner) => (
            <label key={runner.id} className="e2e-where">
              <input type="radio" name="e2e-where" checked={chosen?.id === runner.id} onChange={() => setOnRunner(runner.id)} />
              <RunnerLine runner={runner} owner={userById(runner.userId)?.name} />
            </label>
          ))}
        </div>
        {!chosen && runners.data && !takesQueue && (
          <p className="e2e-hold">
            <Icon name="alert" size={14} />
            {t('No runner that takes jobs from the queue is connected right now. The job waits until one connects.')}
          </p>
        )}
        {chosen?.shared && <p className="muted e2e-where-note">{t('A shared runner checks the repository out with your own GitHub token; store it under Settings → My keys.')}</p>}
        {chosen && !chosen.online && (
          <p className="e2e-hold">
            <Icon name="alert" size={14} />
            {t('{name} is not connected right now. The job waits for it.', { name: chosen.name })}
          </p>
        )}
      </form>
    </Dialog>
  );
}

/** The button that starts a job, with why it cannot when the project names no repository. */
export function RunTestsButton({ project, settings }: { project: Project; settings: E2eSettings | undefined }) {
  const [open, setOpen] = useState(false);

  if (!canEdit(project) || !settings) {
    return null;
  }

  const missing = settings.repo === null;

  return (
    <span className="e2e-run-tests">
      {missing && (
        <span className="muted e2e-run-tests-why">
          {t('No repository is set for this project’s tests.')} {project.role === 'admin' ? <Link href={`/p/${project.key}/settings/e2e`}>{t('Set it up')}</Link> : t('A project administrator sets it in the project’s settings.')}
        </span>
      )}
      <Button variant="primary" icon="play" disabled={missing} title={missing ? t('No repository is set for this project’s tests.') : undefined} onClick={() => setOpen(true)}>
        {t('Run tests')}
      </Button>
      {open && <RunDialog project={project} settings={settings} onClose={() => setOpen(false)} />}
    </span>
  );
}

// ---------- the project's jobs ----------

/** For whom a job that was sent to one runner waits, and whether that runner is there. */
function WaitsFor({ job, runners }: { job: E2eJob; runners: Runner[] | undefined }) {
  if (job.state !== 'queued' || job.onRunner === null) {
    return null;
  }

  const connected = runners?.find((runner) => runner.id === job.onRunner)?.online ?? false;

  return (
    <span className={connected || !runners ? 'e2e-job-waits' : 'e2e-job-waits e2e-job-waits-off'}>
      {t('waits for {name}', { name: job.onRunnerName ?? job.onRunner })}
      {runners && !connected && ` · ${t('not connected')}`}
    </span>
  );
}

function JobRow({ job, project, runners }: { job: E2eJob; project: Project; runners: Runner[] | undefined }) {
  const { userById } = useSession();
  const who = userById(job.requestedBy);
  const took = job.startedAt && job.finishedAt ? duration(Date.parse(job.finishedAt) - Date.parse(job.startedAt)) : '';

  return (
    <div className="e2e-job">
      <Link href={jobPath(project.key, job.id)} className="e2e-job-main">
        <JobStateTag job={job} />
        <span className="e2e-job-what">
          <strong>#{job.id}</strong>
          <Origin branch={job.ref} commit={job.commit} />
          {job.filter && (
            <span className="mono e2e-job-filter" title={job.filter}>
              {job.filter}
            </span>
          )}
        </span>
        <span className="muted e2e-job-note">
          {isActive(job) && job.step && <span>{stepText(job.step)}</span>}
          <WaitsFor job={job} runners={runners} />
          {job.state === 'failed' && job.error && <span className="e2e-job-error">{translateServer(job.error)}</span>}
          {job.runnerName && <span>{job.runnerName}</span>}
          {took && <span>{took}</span>}
        </span>
        <span className="muted e2e-job-who">
          <Avatar user={who} size={18} />
          <span>{who?.name ?? t('Somebody')}</span>
          <span title={formatDateTime(job.createdAt)}>{timeAgo(job.createdAt)}</span>
        </span>
      </Link>
      <span className="e2e-job-actions">
        {job.runId !== null && (
          <Link href={runPath(project.key, job.runId)} className="e2e-more">
            {t('Open run')}
          </Link>
        )}
        {isActive(job) && canEdit(project) && !job.cancelRequested && (
          <Button size="sm" onClick={() => void cancel(job)}>
            {t('Cancel job')}
          </Button>
        )}
      </span>
    </div>
  );
}

/** The project's jobs: what is waiting or running, and the last ones that ended. */
export function JobsPanel({ project, jobs }: { project: Project; jobs: E2eJob[] }) {
  const [all, setAll] = useState(false);
  const runners = useTestRunners(project);
  const active = jobs.filter(isActive).length;
  // Whatever is under way is always shown; of what has ended, the newest few.
  const shown = all ? jobs : jobs.filter((job, index) => isActive(job) || index < JOBS_SHOWN);

  if (jobs.length === 0) {
    return null;
  }

  return (
    <section className="panel e2e-jobs" aria-label={t('Test jobs')}>
      <header className="panel-head">
        <h2>{t('Test jobs')}</h2>
        {active > 0 && <span className="tab-count">{t('{count} under way', { count: active })}</span>}
        <span className="filter-spacer" />
        {jobs.length > shown.length || all ? (
          <button type="button" className="link-button" onClick={() => setAll(!all)}>
            {all ? t('Show fewer') : t('Show all {count}', { count: jobs.length })}
          </button>
        ) : null}
      </header>
      {shown.map((job) => (
        <JobRow key={job.id} job={job} project={project} runners={runners.data} />
      ))}
    </section>
  );
}

// ---------- one job, live ----------

/** The steps a runner goes through, with the one it is at. */
function Steps({ job }: { job: E2eJob }) {
  const at = STEPS.findIndex(([word]) => word === job.step);

  if (!isActive(job) || job.state === 'queued') {
    return null;
  }

  if (job.step !== null && at === -1) {
    return <p className="e2e-job-step-free">{job.step}</p>;
  }

  return (
    <ol className="e2e-steps-line" aria-label={t('What the runner is doing')}>
      {STEPS.map(([word, label], index) => (
        <li key={word} className={index < at ? 'e2e-step-done' : index === at ? 'e2e-step-now' : ''} aria-current={index === at ? 'step' : undefined}>
          {index < at ? <Icon name="check" size={12} /> : <span className="e2e-step-number">{index + 1}</span>}
          {t(label)}
        </li>
      ))}
    </ol>
  );
}

/**
 * The job's output. It follows the newest line while the reader is at the bottom, and stays
 * where it is once they scroll up to read.
 */
function Log({ detail, live }: { detail: E2eJobDetail; live: boolean }) {
  const box = useRef<HTMLDivElement>(null);
  const [following, setFollowing] = useState(true);
  const newest = detail.log.at(-1)?.seq ?? 0;

  useLayoutEffect(() => {
    if (following && box.current) {
      box.current.scrollTop = box.current.scrollHeight;
    }
  }, [newest, following]);

  function onScroll(): void {
    const element = box.current;

    if (element) {
      setFollowing(element.scrollHeight - element.scrollTop - element.clientHeight < 24);
    }
  }

  return (
    <section className="panel e2e-log-panel">
      <header className="panel-head">
        <h2>{t('Output')}</h2>
        {live && (
          <span className={following ? 'e2e-live' : 'e2e-live e2e-live-off'}>
            <span className="e2e-live-dot" />
            {following ? t('following') : t('paused')}
          </span>
        )}
        <span className="filter-spacer" />
        {!following && (
          <Button size="sm" icon="chevronDown" onClick={() => setFollowing(true)}>
            {t('Newest lines')}
          </Button>
        )}
      </header>
      {detail.log.length === 0 ? (
        <p className="muted panel-empty">{live ? t('Nothing has been written yet.') : t('This job wrote nothing.')}</p>
      ) : (
        <div className="e2e-log" ref={box} onScroll={onScroll} role="log" aria-live="off" tabIndex={0}>
          {detail.logTruncated && <div className="e2e-log-cut">{t('Earlier lines are gone: only the newest {count} are kept.', { count: LOG_LINES_KEPT })}</div>}
          {detail.log.map((line) => (
            <div key={line.seq} className="e2e-log-line">
              <span className="e2e-log-seq">{line.seq}</span>
              <span className="e2e-log-text">{line.text}</span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

function useJob(id: number): Remote<E2eJobDetail> {
  // What has been read so far. Each announcement fetches only the lines after it.
  const held = useRef<E2eJobDetail | null>(null);

  return useRemote(
    async () => {
      const before = held.current?.job.id === id ? held.current : null;
      const after = before?.log.at(-1)?.seq ?? 0;
      const next = await api.get<E2eJobDetail>(`${BASE}/jobs/${id}${query({ after: after || null })}`);
      // Two fetches can overlap; a line is kept once.
      const known = held.current?.job.id === id ? held.current.log : [];
      const last = known.at(-1)?.seq ?? 0;
      const log = [...known, ...next.log.filter((line) => line.seq > last)];
      const merged = { job: next.job, log: log.slice(-LOG_LINES_KEPT), logTruncated: next.logTruncated || log.length > LOG_LINES_KEPT };

      held.current = merged;

      return merged;
    },
    [id],
    (event) => (event.kind === 'e2e.job' || event.kind === 'e2e.job.log') && event.payload.jobId === id,
  );
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="e2e-fact">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

/** One job: its state and step, its output as it comes, and how to stop it. */
export function JobView({ id }: { id: number }) {
  const project = useProject();
  const { userById } = useSession();
  const detail = useJob(id);
  const runners = useTestRunners(project);
  const back = (
    <Link href={`/p/${project.key}/tests`} className="e2e-back">
      <BackIcon size={14} />
      {t('Tests')}
    </Link>
  );

  if (!detail.data) {
    return (
      <div className="view e2e-view">
        {back}
        {detail.error ? <Empty icon="alert" title={detail.error} /> : <Spinner />}
      </div>
    );
  }

  const { job } = detail.data;
  const who = userById(job.requestedBy);
  const active = isActive(job);

  return (
    <div className="view e2e-view">
      <header className="view-head">
        <div className="e2e-head-title">
          {back}
          <h1>
            {t('Test job #{id}', { id: job.id })} <JobStateTag job={job} />
          </h1>
        </div>
        <div className="view-head-side">
          {job.runId !== null && (
            <Button variant="primary" icon="check" onClick={() => navigate(runPath(project.key, job.runId ?? 0))}>
              {t('Open the results')}
            </Button>
          )}
          {active && canEdit(project) && (
            <Button icon="x" disabled={job.cancelRequested} onClick={() => void cancel(job)}>
              {job.cancelRequested ? t('Stopping…') : t('Cancel job')}
            </Button>
          )}
        </div>
      </header>

      {job.state === 'failed' && job.error && <ErrorNote message={translateServer(job.error)} />}
      {job.state === 'queued' && (
        <p className="e2e-hold">
          <Icon name="clock" size={14} />
          {job.onRunner === null ? t('The job waits for a runner to take it.') : <WaitsFor job={job} runners={runners.data} />}
        </p>
      )}
      <Steps job={job} />

      <section className="panel e2e-job-facts">
        <dl>
          <Fact label={t('Repository')}>
            <span className="mono">{job.repo}</span> <Origin branch={job.ref} commit={job.commit} />
          </Fact>
          {job.filter && (
            <Fact label={t('Only these tests')}>
              <span className="mono">{job.filter}</span>
            </Fact>
          )}
          <Fact label={t('Recording')}>
            {t('video: {video}', { video: job.video })} · {t('trace: {trace}', { trace: job.trace })}
          </Fact>
          <Fact label={t('Asked for by')}>
            <span className="e2e-job-who">
              <Avatar user={who} size={18} />
              {who?.name ?? t('Somebody')}
              <span className="muted" title={formatDateTime(job.createdAt)}>
                {timeAgo(job.createdAt)}
              </span>
            </span>
          </Fact>
          {job.runnerName && <Fact label={t('Runner')}>{job.runnerName}</Fact>}
          {job.startedAt && (
            <Fact label={job.finishedAt ? t('Took') : t('Started')}>
              {job.finishedAt ? duration(Date.parse(job.finishedAt) - Date.parse(job.startedAt)) : <span title={formatDateTime(job.startedAt)}>{timeAgo(job.startedAt)}</span>}
            </Fact>
          )}
        </dl>
      </section>

      <Log detail={detail.data} live={active} />
    </div>
  );
}
