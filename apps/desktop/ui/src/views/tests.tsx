// A project's test jobs: start the suite on this computer or on another runner, follow a
// job while it runs, and open the finished run on the web, where its recordings are.
import { useEffect, useRef, useState, type FormEvent } from 'react';

import type { E2eJob } from '@web/bindings/E2eJob.ts';
import { Button, ErrorNote, Field, Spinner, Tag } from '@web/components/ui.tsx';
import { t, timeAgo } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import { openExternal } from '../bridge.ts';
import { ANY, HERE, namableRunners, WhereSelect } from '../components/where.tsx';
import { useLive, useRemote, useStored } from '../live.ts';
import { useSession } from '../session.tsx';

const ENDED = ['succeeded', 'failed', 'cancelled'];
const TONE: Record<string, 'green' | 'red' | 'amber' | 'blue' | undefined> = { succeeded: 'green', failed: 'red', cancelled: 'amber', running: 'blue', claimed: 'blue' };

export function jobStateLabel(state: string): string {
  const words: Record<string, string> = {
    queued: t('queued'),
    claimed: t('taken'),
    running: t('running'),
    succeeded: t('finished'),
    failed: t('failed'),
    cancelled: t('cancelled'),
  };

  return words[state] ?? state;
}

function stepLabel(step: string): string {
  const words: Record<string, string> = { cloning: t('cloning'), installing: t('installing'), running: t('running'), uploading: t('uploading') };

  return words[step] ?? step;
}

/** One job, followed while it runs: its step, the tail of its log, and how it ended. */
function JobFollow({ jobId, onChange }: { jobId: number; onChange: () => void }) {
  const { serverUrl, status } = useSession();
  const [job, setJob] = useState<E2eJob | undefined>();
  const [lines, setLines] = useState<string[]>([]);
  const [error, setError] = useState<string | undefined>();
  const seen = useRef(0);
  const tail = useRef<HTMLPreElement>(null);

  useEffect(() => {
    let stopped = false;
    let timer: number | undefined;

    seen.current = 0;
    setLines([]);
    setJob(undefined);

    const look = async (): Promise<void> => {
      try {
        const detail = await core.testJob(jobId, seen.current);

        if (stopped) {
          return;
        }

        seen.current = detail.log.reduce((last, line) => Math.max(last, line.seq), seen.current);
        setJob(detail.job);
        setError(undefined);

        if (detail.log.length > 0) {
          setLines((before) => [...before, ...detail.log.map((line) => line.text)].slice(-400));
        }

        if (!ENDED.includes(detail.job.state)) {
          timer = window.setTimeout(() => void look(), 1000);
        } else {
          onChange();
        }
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : String(reason));
        timer = window.setTimeout(() => void look(), 3000);
      }
    };

    void look();

    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobId]);

  useEffect(() => {
    tail.current?.scrollTo({ top: tail.current.scrollHeight });
  }, [lines]);

  if (!job) {
    return error ? <ErrorNote message={error} /> : <Spinner />;
  }

  const where = job.runnerName ?? job.onRunnerName ?? t('any runner that fits');
  const here = job.onRunner === status.runner.id;

  return (
    <section className="panel settings-card">
      <header className="panel-head">
        <h2>
          {t('Job')} #{job.id}
        </h2>
        <Tag tone={TONE[job.state]}>{jobStateLabel(job.state)}</Tag>
        {job.step && !ENDED.includes(job.state) && <Tag>{stepLabel(job.step)}</Tag>}
        <span className="muted">
          {job.ref}
          {job.commit ? ` · ${job.commit.slice(0, 8)}` : ''} · {here ? t('This computer') : where}
        </span>
        <span className="desk-grow" />
        {!ENDED.includes(job.state) && (
          <Button size="sm" icon="x" onClick={() => void core.cancelTestJob(job.id).then(setJob, (reason: unknown) => setError(String(reason)))} disabled={job.cancelRequested}>
            {job.cancelRequested ? t('Cancelling…') : t('Cancel')}
          </Button>
        )}
        {ENDED.includes(job.state) && (
          <Button size="sm" variant="primary" icon="external" onClick={() => void openExternal(serverUrl(`/p/${job.projectKey}/tests/jobs/${job.id}`) ?? '')}>
            {job.runId ? t('Open the session in the browser') : t('Open on the web')}
          </Button>
        )}
      </header>
      {job.error && <ErrorNote message={job.error} />}
      {job.state === 'succeeded' && <div className="note">{t('The suite ran and its report is on the server: the results, recordings and traces are on the run’s page.')}</div>}
      {error && <ErrorNote message={error} />}
      <pre className="desk-log" ref={tail} aria-label={t('Log')}>
        {lines.length === 0 ? t('Nothing printed yet.') : lines.join('\n')}
      </pre>
    </section>
  );
}

export function TestsView({ project }: { project?: string }) {
  const { workspace, status, go } = useSession();
  const projects = workspace?.bootstrap.projects ?? [];
  const [stored, setStored] = useStored<string>('tests-project', '');
  const key = project ?? (stored || projects[0]?.key || '');
  const settings = useRemote(() => (key ? core.testSettings(key) : Promise.resolve(null)), [key]);
  const jobs = useRemote(() => (key ? core.testJobs(key) : Promise.resolve([])), [key], (event) => event.type === 'data_changed' && event.scope === 'jobs');
  const runners = useRemote(() => core.runners(), []);
  const [reference, setReference] = useState('');
  const [filter, setFilter] = useState('');
  const [where, setWhere] = useState(HERE);
  const [following, setFollowing] = useState<number | undefined>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | undefined>();
  const me = workspace?.bootstrap.user?.id;
  const elsewhere = namableRunners(runners.data ?? [], status.runner.id, me, 'e2e', undefined, key);

  useEffect(() => setFollowing(undefined), [key]);
  useLive((event) => event.type === 'data_changed' && event.scope === 'jobs' && runners.reload());

  async function start(event: FormEvent): Promise<void> {
    event.preventDefault();
    setBusy(true);
    setError(undefined);

    try {
      const onRunner = where === HERE ? status.runner.id : where === ANY ? null : where;
      const job = await core.startTestJob({ projectKey: key, reference: reference.trim() || null, filter: filter.trim() || null, onRunner });

      setFollowing(job.id);
      jobs.reload();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="desk-view">
      <div className="desk-view-head">
        <h1>{t('Tests')}</h1>
        <select
          className="input"
          value={key}
          aria-label={t('Project')}
          onChange={(event) => {
            setStored(event.target.value);
            go({ name: 'tests', project: event.target.value });
          }}
        >
          {projects.map((candidate) => (
            <option key={candidate.key} value={candidate.key}>
              {candidate.key} · {candidate.name}
            </option>
          ))}
        </select>
      </div>
      {settings.error && <ErrorNote message={settings.error} />}
      {settings.data === null && !settings.loading && <div className="note">{t('This server has no tests module, or it is switched off for this project.')}</div>}
      {settings.data && !settings.data.repo && <div className="note">{t('This project has no repository set for its tests. A project administrator sets it on the web, under the project’s settings → Tests.')}</div>}
      {settings.data?.repo && (
        <form className="panel settings-card form" onSubmit={start}>
          <header className="panel-head">
            <h2>{t('Run the suite')}</h2>
            <span className="muted">
              {settings.data.repo}
              {settings.data.directory ? ` · ${settings.data.directory}` : ''}
            </span>
          </header>
          <div className="form-row">
            <Field label={t('Branch, tag or commit')}>
              <input className="input input-mono" value={reference} placeholder={settings.data.ref} onChange={(event) => setReference(event.target.value)} />
            </Field>
            <Field label={t('Only these tests')} hint={t('Test files or --grep "…"; empty runs everything.')}>
              <input className="input input-mono" value={filter} onChange={(event) => setFilter(event.target.value)} />
            </Field>
            <Field label={t('Where')}>
              <WhereSelect value={where} onChange={setWhere} runners={elsewhere} />
            </Field>
          </div>
          {where === HERE && <p className="muted">{t('On this computer: the suite runs here, with this computer’s processor and memory, in a folder of its own. The server only stores what comes back.')}</p>}
          {error && <ErrorNote message={error} />}
          <div className="row-end">
            <Button variant="primary" type="submit" icon="play" disabled={busy}>
              {busy ? t('Starting…') : t('Run the tests')}
            </Button>
          </div>
        </form>
      )}
      {following !== undefined && <JobFollow key={following} jobId={following} onChange={jobs.reload} />}
      <section className="panel settings-card">
        <header className="panel-head">
          <h2>{t('Jobs')}</h2>
          <span className="desk-grow" />
          <Button size="sm" variant="ghost" icon="refresh" onClick={jobs.reload} aria-label={t('Refresh')} title={t('Refresh')} />
        </header>
        {jobs.loading && !jobs.data && <Spinner />}
        {jobs.error && <ErrorNote message={jobs.error} />}
        {jobs.data?.length === 0 && <div className="panel-empty muted">{t('No job has been run for this project yet.')}</div>}
        {(jobs.data ?? []).map((job) => (
          <div key={job.id} className={following === job.id ? 'issue-line desk-row desk-row-selected' : 'issue-line desk-row'} onClick={() => setFollowing(job.id)}>
            <span className="input-mono">#{job.id}</span>
            <Tag tone={TONE[job.state]}>{jobStateLabel(job.state)}</Tag>
            <span className="issue-line-summary">
              {job.ref}
              {job.filter ? ` · ${job.filter}` : ''}
            </span>
            <span className="muted">{job.onRunner === status.runner.id ? t('This computer') : (job.runnerName ?? job.onRunnerName ?? t('any runner that fits'))}</span>
            <span className="muted">{timeAgo(job.createdAt)}</span>
          </div>
        ))}
      </section>
    </div>
  );
}
