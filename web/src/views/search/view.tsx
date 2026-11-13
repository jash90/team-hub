import { useEffect, useMemo, useState } from 'react';

import { api, query as toQuery } from '../../api.ts';
import type { QueryFieldInfo } from '../../bindings/QueryFieldInfo.ts';
import type { SavedFilter } from '../../bindings/SavedFilter.ts';
import type { SearchResult } from '../../bindings/SearchResult.ts';
import { Icon } from '../../components/icon.tsx';
import { PRIORITIES } from '../../components/issue-bits.tsx';
import { Button, Empty, ErrorNote, Menu, Spinner } from '../../components/ui.tsx';
import { countOf, t } from '../../i18n/index.ts';
import { navigate, setParam, usePath, useSearchParams } from '../../router.tsx';
import { changesIssues, useRemote, useSession } from '../../session.tsx';
import { BulkDialog, SaveFilterDialog } from './dialogs.tsx';
import { QueryBar } from './query-bar.tsx';
import { chipsQuery, type Chips } from './query.ts';
import { ResultsTable, useProjects } from './results.tsx';

const PAGE = 50;

export function SearchView({ scoped }: { scoped?: boolean }) {
  const { boot, user, userById } = useSession();
  const path = usePath();
  const params = useSearchParams();
  const scope = scoped ? (path.split('/')[2]?.toUpperCase() ?? null) : null;
  const startChips: Chips = { status: '', assignee: scoped ? '' : 'me', type: '', priority: '' };
  const initial = params.get('q') ?? chipsQuery(scope, startChips, '');
  const [text, setText] = useState(initial);
  const [ran, setRan] = useState(initial);
  const [advanced, setAdvanced] = useState(params.get('q') !== null);
  const [chips, setChips] = useState<Chips>(startChips);
  const [words, setWords] = useState('');
  const [limit, setLimit] = useState(PAGE);
  const [selected, setSelected] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  const [bulk, setBulk] = useState(false);

  const result = useRemote(() => api.get<SearchResult>(`/api/search${toQuery({ q: ran, limit })}`), [ran, limit], changesIssues);
  const fields = useRemote(() => api.get<QueryFieldInfo[]>('/api/search/fields'), []);
  const filters = useRemote(() => api.get<SavedFilter[]>('/api/filters'), []);
  const issues = result.data?.issues ?? [];
  const projects = useProjects(useMemo(() => Array.from(new Set(issues.map((issue) => issue.projectId))), [issues]));

  // A link to this page with another query (a saved filter, the quick search) takes over.
  useEffect(() => {
    const fromUrl = params.get('q');

    if (fromUrl !== null && fromUrl !== ran) {
      setText(fromUrl);
      setRan(fromUrl);
      setAdvanced(true);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params.get('q')]);

  function run(next: string): void {
    setLimit(PAGE);
    setSelected([]);
    setRan(next);
    setParam('q', next, { replace: true });
  }

  function applyChips(next: Chips, nextWords: string): void {
    setChips(next);
    setWords(nextWords);
    const built = chipsQuery(scope, next, nextWords);
    setText(built);
    run(built);
  }

  function exportCsv(): void {
    const header = ['Key', 'Summary', 'Status', 'Type', 'Priority', 'Assignee', 'Points', 'Updated'];
    const lines = issues.map((issue) => {
      const project = projects.get(issue.projectId);

      return [
        issue.key,
        issue.summary,
        project?.statuses.find((status) => status.id === issue.statusId)?.name ?? '',
        project?.types.find((type) => type.id === issue.typeId)?.name ?? '',
        PRIORITIES.find((priority) => priority.value === issue.priority)?.name ?? '',
        userById(issue.assigneeId)?.name ?? '',
        issue.points ?? '',
        issue.updatedAt,
      ]
        .map((cell) => `"${String(cell).replace(/"/g, '""')}"`)
        .join(',');
    });
    const blob = new Blob([`﻿${[header.join(','), ...lines].join('\n')}`], { type: 'text/csv;charset=utf-8' });
    const link = document.createElement('a');

    link.href = URL.createObjectURL(blob);
    link.download = 'issues.csv';
    link.click();
    URL.revokeObjectURL(link.href);
  }

  const error = result.data?.error ?? null;

  return (
    <div className="view">
      <header className="view-head">
        <div>
          <h1>{scoped ? t('Issues') : t('Search')}</h1>
        </div>
        <div className="view-head-side">
          {(filters.data?.length ?? 0) > 0 && (
            <Menu
              label={t('Saved filters')}
              align="end"
              items={(filters.data ?? []).map((filter) => ({ label: filter.ownerId === user.id ? filter.name : `${filter.name} · ${userById(filter.ownerId)?.name ?? ''}`, icon: 'star' as const, onSelect: () => navigate(`/search${toQuery({ q: filter.query })}`) }))}
            >
              <Icon name="star" /> {t('Saved filters')}
            </Menu>
          )}
          <Button icon="star" onClick={() => setSaving(true)}>
            {t('Save filter')}
          </Button>
          <Button icon="download" disabled={issues.length === 0} onClick={exportCsv}>
            CSV
          </Button>
        </div>
      </header>

      <QueryBar advanced={advanced} setAdvanced={setAdvanced} text={text} setText={setText} error={error !== null} run={run} chips={chips} words={words} setWords={setWords} applyChips={applyChips} fields={fields} />

      {error && <ErrorNote message={`${error.message} (${t('at character {position}', { position: error.position + 1 })})`} />}
      {result.error && <ErrorNote message={result.error} />}
      {!result.data && !result.error && <Spinner />}

      {result.data && !error && (
        <>
          <div className="result-bar">
            <span className="muted">{countOf(result.data.total, 'issue', 'issues')}</span>
            {selected.length > 0 && (
              <>
                <strong>{t('{count} selected', { count: selected.length })}</strong>
                <Button size="sm" onClick={() => setBulk(true)}>
                  {t('Change selected…')}
                </Button>
              </>
            )}
          </div>
          {issues.length === 0 ? (
            <Empty icon="search" title={t('No issues match')} />
          ) : (
            <ResultsTable issues={issues} projects={projects} selected={selected} setSelected={setSelected} />
          )}
          {issues.length < result.data.total && (
            <div className="row-center">
              <Button onClick={() => setLimit(limit + PAGE * 2)}>{t('Show more')}</Button>
            </div>
          )}
        </>
      )}

      {saving && <SaveFilterDialog query={ran} onClose={() => setSaving(false)} onDone={() => void filters.reload()} existing={filters.data?.filter((filter) => filter.ownerId === user.id) ?? []} />}
      {bulk && <BulkDialog keys={selected} projects={boot.projects.length} onClose={() => setBulk(false)} onDone={() => void result.reload()} project={projects.size === 1 ? Array.from(projects.values())[0] : undefined} />}
    </div>
  );
}
