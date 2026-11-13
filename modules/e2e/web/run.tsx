import { useMemo, useState } from 'react';

import { api } from '@/api.ts';
import type { E2eProjectTests } from '@/bindings/E2eProjectTests.ts';
import type { E2eRun } from '@/bindings/E2eRun.ts';
import type { E2eRunDetail } from '@/bindings/E2eRunDetail.ts';
import type { E2eRunResult } from '@/bindings/E2eRunResult.ts';
import type { E2eStatus } from '@/bindings/E2eStatus.ts';
import type { Project } from '@/bindings/Project.ts';
import { Icon } from '@/components/icon.tsx';
import { Button, ConfirmDialog, ErrorNote, Spinner, Tabs, Tag, attempt } from '@/components/ui.tsx';
import { t } from '@/i18n/index.ts';
import { Link, setParam, useSearchParams } from '@/router.tsx';
import { useRemote } from '@/session.tsx';

import { failureOfResult, type Acting } from './actions.tsx';
import { CameraIcon, VideoIcon } from './icons.tsx';
import { BASE, IssueChips, RunSummary, StatusMark, duration, isOurs, jobPath, sessionPath } from './shared.tsx';

type Filter = 'all' | E2eStatus;

const FILTERS: [Filter, string][] = [
  ['all', 'All'],
  ['failed', 'Failed tests'],
  ['flaky', 'Flaky tests'],
  ['passed', 'Passed tests'],
  ['skipped', 'Skipped tests'],
];
const RANK: Record<E2eStatus, number> = { failed: 0, flaky: 1, passed: 2, skipped: 3 };

/** Every word of the search is somewhere in the title, the file, the target or an issue key. */
function matches(result: E2eRunResult, words: string[]): boolean {
  const text = ` ${result.title} ${result.file} ${result.variant} ${result.issues.map((issue) => issue.key).join(' ')}`.toLowerCase();

  return words.every((word) => text.includes(word));
}

/** What a result has to watch: a recording, or the screenshots its report names. */
function Recorded({ result }: { result: E2eRunResult }) {
  if (result.videos > 0) {
    return (
      <span className="e2e-recorded e2e-recorded-video" title={t('Has a recording to watch')}>
        <VideoIcon size={14} />
        {t('recording')}
      </span>
    );
  }

  if (result.screenshots > 0) {
    return (
      <span className="e2e-recorded" title={t('No recording; {count} screenshots', { count: result.screenshots })}>
        <CameraIcon size={14} />
        {result.screenshots}
      </span>
    );
  }

  return <span className="e2e-recorded e2e-recorded-none" title={t('Nothing was recorded for this test')} />;
}

function ResultRow({ result, run, project, data, act }: { result: E2eRunResult; run: E2eRun; project: Project; data: E2eProjectTests | undefined; act: (acting: Acting) => void }) {
  // The describe titles name the group; the row shows the test's own.
  const parts = result.title.split(' > ');
  const own = parts.pop() ?? result.title;

  return (
    <div className={`e2e-rrow e2e-is-${result.status}`}>
      <StatusMark status={result.status} label={false} />
      <div className="e2e-rrow-main">
        <Link href={sessionPath(project.key, result.id)} className="e2e-rrow-title">
          {parts.length > 0 && <span className="muted">{parts.join(' › ')} › </span>}
          {own}
        </Link>
        {result.status !== 'passed' && result.message && <span className={`e2e-oneline e2e-is-${result.status}`}>{result.message.split('\n')[0]}</span>}
      </div>
      <span className="e2e-rrow-side">
        {result.variant && <Tag>{result.variant}</Tag>}
        <IssueChips issues={result.issues} failing={result.status === 'failed'} />
        {result.attempts > 1 && (
          <Tag tone="amber" title={t('Ran again after a failed attempt; the last attempt decides the result.')}>
            {t('{count} attempts', { count: result.attempts })}
          </Tag>
        )}
        <Recorded result={result} />
        <span className="muted e2e-rrow-time">{result.status === 'skipped' ? t('skipped') : duration(result.durationMs)}</span>
        {data?.canEdit && result.status === 'failed' && (
          <Button size="sm" icon="plus" onClick={() => act({ kind: 'bug', failure: failureOfResult(result, run) })}>
            {data.bugTypeId ? t('Create bug') : t('Create issue')}
          </Button>
        )}
      </span>
    </div>
  );
}

function FileGroup({ file, results, run, project, data, act }: { file: string; results: E2eRunResult[]; run: E2eRun; project: Project; data: E2eProjectTests | undefined; act: (acting: Acting) => void }) {
  const [open, setOpen] = useState(true);
  const count = (status: E2eStatus) => results.filter((result) => result.status === status).length;

  return (
    <section className="e2e-file" aria-label={file}>
      <button type="button" className="e2e-file-head" aria-expanded={open} onClick={() => setOpen(!open)}>
        <Icon name={open ? 'chevronDown' : 'chevronRight'} size={13} />
        <span className="mono">{file}</span>
        <span className="e2e-file-counts">
          {(['failed', 'flaky', 'passed', 'skipped'] as E2eStatus[])
            .filter((status) => count(status) > 0)
            .map((status) => (
              <span key={status} className={`e2e-count e2e-is-${status}`}>
                <StatusMark status={status} label={false} />
                {count(status)}
              </span>
            ))}
        </span>
      </button>
      {open && results.map((result) => <ResultRow key={result.id} result={result} run={run} project={project} data={data} act={act} />)}
    </section>
  );
}

/** One run: its summary, then its tests by file, filtered by result and by a search. */
export function RunView({ runId, project, data, act }: { runId: number; project: Project; data: E2eProjectTests | undefined; act: (acting: Acting) => void }) {
  const detail = useRemote(() => api.get<E2eRunDetail>(`${BASE}/runs/${runId}`), [runId], isOurs(project));
  const params = useSearchParams();
  const [removing, setRemoving] = useState(false);
  const [search, setSearch] = useState('');
  const wanted = params.get('status');
  const filter: Filter = FILTERS.some(([id]) => id === wanted) ? (wanted as Filter) : 'all';
  const results = detail.data?.results;
  const groups = useMemo(() => {
    const words = search.toLowerCase().split(/\s+/).filter(Boolean);
    const shown = (results ?? []).filter((result) => (filter === 'all' || result.status === filter) && matches(result, words));
    const byFile = new Map<string, E2eRunResult[]>();

    for (const result of shown) {
      byFile.set(result.file, [...(byFile.get(result.file) ?? []), result]);
    }

    // The file with the worst result first: what needs reading is at the top.
    const worst = (list: E2eRunResult[]) => Math.min(...list.map((result) => RANK[result.status]));

    return [...byFile].sort((a, b) => worst(a[1]) - worst(b[1]) || a[0].localeCompare(b[0]));
  }, [results, filter, search]);
  const back = (
    <Button icon="chevronUp" onClick={() => setParam('run', null)}>
      {t('All runs')}
    </Button>
  );

  if (!detail.data || !results) {
    return detail.error ? (
      <>
        <ErrorNote message={detail.error} />
        <div>{back}</div>
      </>
    ) : (
      <Spinner />
    );
  }

  const { run, jobId } = detail.data;
  const count = (id: Filter) => (id === 'all' ? results.length : results.filter((result) => result.status === id).length);

  return (
    <>
      <RunSummary
        run={run}
        heading={t('Run')}
        action={
          <span className="e2e-summary-actions">
            {jobId !== null && (
              <Link href={jobPath(project.key, jobId)} className="e2e-more">
                {t('Job #{id} and its output', { id: jobId })}
              </Link>
            )}
            {back}
            {data?.canAdmin && <Button variant="ghost" icon="trash" aria-label={t('Remove this run')} title={t('Remove this run')} onClick={() => setRemoving(true)} />}
          </span>
        }
      />
      <section className="panel">
        <div className="e2e-toolbar">
          <Tabs tabs={FILTERS.filter(([id]) => id === 'all' || count(id) > 0 || id === filter).map(([id, label]) => ({ id, label: t(label), count: count(id) }))} active={filter} onChange={(id) => setParam('status', id === 'all' ? null : id, { replace: true })} />
          <div className="filter-search e2e-search">
            <Icon name="search" />
            <input className="input" type="search" placeholder={t('Search tests, files, issues')} aria-label={t('Search tests, files, issues')} value={search} onChange={(event) => setSearch(event.target.value)} />
          </div>
        </div>
        {groups.length === 0 ? (
          <p className="muted panel-empty">{results.length === 0 ? t('This run has no tests.') : t('No test matches.')}</p>
        ) : (
          <div className="e2e-files">
            {groups.map(([file, list]) => (
              <FileGroup key={file} file={file} results={list} run={run} project={project} data={data} act={act} />
            ))}
          </div>
        )}
      </section>
      {removing && (
        <ConfirmDialog
          title={t('Remove this run?')}
          body={t('Its results, recordings and screenshots are removed. Tests fall back to what the run before said.')}
          confirm={t('Remove run')}
          danger
          onClose={() => setRemoving(false)}
          onConfirm={async () => {
            await attempt(() => api.del(`${BASE}/runs/${run.id}`));
            setRemoving(false);
            setParam('run', null);
          }}
        />
      )}
    </>
  );
}
