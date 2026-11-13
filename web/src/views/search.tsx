import { useEffect, useMemo, useState } from 'react';

import { api, query as toQuery } from '../api.ts';
import type { IssueRow } from '../bindings/IssueRow.ts';
import type { Project } from '../bindings/Project.ts';
import type { QueryFieldInfo } from '../bindings/QueryFieldInfo.ts';
import type { SavedFilter } from '../bindings/SavedFilter.ts';
import type { SearchResult } from '../bindings/SearchResult.ts';
import { Icon } from '../components/icon.tsx';
import { DueDate, IssueKey, ModuleBadges, PRIORITIES, PriorityIcon, StatusPill, TypeIcon, UserPicker } from '../components/issue-bits.tsx';
import { Avatar, Button, Dialog, Empty, ErrorNote, Field, Menu, Picker, Popover, Spinner, attempt, toast, useBusy } from '../components/ui.tsx';
import { countOf, t, timeAgo } from '../i18n/index.ts';
import { navigate, setParam, usePath, useSearchParams } from '../router.tsx';
import { useRemote, useSession } from '../session.tsx';

const PAGE = 50;
const FUNCTIONS = ['currentUser()', 'openSprints()', 'now()', 'startOfDay()', 'startOfWeek()', 'startOfMonth()'];

function quote(text: string): string {
  return /^[\p{L}\p{N}_.@+-]+$/u.test(text) ? text : `"${text.replace(/"/g, '\\"')}"`;
}

/** Project configurations by id, loaded as rows from those projects show up. */
function useProjects(ids: number[]): Map<number, Project> {
  const { boot } = useSession();
  const [projects, setProjects] = useState<Map<number, Project>>(new Map());
  const wanted = ids.join(',');

  useEffect(() => {
    for (const id of ids) {
      const summary = boot.projects.find((project) => project.id === id);

      if (summary && !projects.has(id)) {
        void api.get<Project>(`/api/projects/${summary.key}`).then((project) => setProjects((current) => new Map(current).set(id, project)));
      }
    }
    // Only the set of ids matters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wanted]);

  return projects;
}

type Chips = { status: string; assignee: string; type: string; priority: string };

function chipsQuery(scope: string | null, chips: Chips, text: string): string {
  const parts = [
    scope ? `project = ${scope}` : '',
    chips.status ? `category = ${quote(chips.status)}` : '',
    chips.assignee === 'me' ? 'assignee = currentUser()' : chips.assignee === 'none' ? 'assignee is empty' : '',
    chips.type ? `type = ${quote(chips.type)}` : '',
    chips.priority ? `priority = ${chips.priority}` : '',
    text.trim() ? `text ~ ${quote(text.trim()).startsWith('"') ? quote(text.trim()) : `"${text.trim()}"`}` : '',
  ].filter(Boolean);

  return `${parts.join(' AND ')}${parts.length > 0 ? ' ' : ''}ORDER BY updated DESC`;
}

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
  const [hints, setHints] = useState<HTMLElement | null>(null);

  const result = useRemote(() => api.get<SearchResult>(`/api/search${toQuery({ q: ran, limit })}`), [ran, limit], (event) => event.kind.startsWith('issue.'));
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
  const allSelected = issues.length > 0 && selected.length === issues.length;

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

      <div className="search-box">
        {advanced ? (
          <form
            className="search-query"
            onSubmit={(event) => {
              event.preventDefault();
              run(text);
            }}
          >
            <input
              className={error ? 'input input-mono input-error' : 'input input-mono'}
              spellCheck={false}
              value={text}
              onChange={(event) => setText(event.target.value)}
              placeholder='project = WEB AND status = "In Progress" ORDER BY priority DESC'
              aria-label={t('Query')}
            />
            <Button variant="primary" type="submit" icon="search">
              {t('Search')}
            </Button>
            <Button icon="info" onClick={(event) => setHints(hints ? null : event.currentTarget)}>
              {t('Fields')}
            </Button>
          </form>
        ) : (
          <div className="search-chips">
            <div className="filter-search">
              <Icon name="search" />
              <input className="input" placeholder={t('Words in the summary, description or comments')} value={words} onChange={(event) => setWords(event.target.value)} onKeyDown={(event) => event.key === 'Enter' && applyChips(chips, words)} />
            </div>
            <Picker
              value={chips.status}
              onChange={(status) => applyChips({ ...chips, status }, words)}
              options={[
                { value: '', label: t('Any status') },
                { value: 'todo', label: t('To do') },
                { value: 'in progress', label: t('In progress') },
                { value: 'done', label: t('Done') },
              ]}
            />
            <Picker
              value={chips.assignee}
              onChange={(assignee) => applyChips({ ...chips, assignee }, words)}
              options={[
                { value: '', label: t('Anyone') },
                { value: 'me', label: t('Assigned to me') },
                { value: 'none', label: t('Unassigned') },
              ]}
            />
            <Picker
              value={chips.type}
              onChange={(type) => applyChips({ ...chips, type }, words)}
              options={[{ value: '', label: t('Any type') }, ...['Epic', 'Story', 'Task', 'Bug', 'Subtask'].map((name) => ({ value: name, label: name }))]}
            />
            <Picker
              value={chips.priority}
              onChange={(priority) => applyChips({ ...chips, priority }, words)}
              options={[{ value: '', label: t('Any priority') }, ...PRIORITIES.map((priority) => ({ value: priority.name, label: t(priority.name) }))]}
            />
          </div>
        )}
        <button type="button" className="link-button" onClick={() => setAdvanced(!advanced)}>
          {advanced ? t('Simple filters') : t('Write a query')}
        </button>
      </div>

      {hints && fields.data && (
        <Popover anchor={hints} onClose={() => setHints(null)} align="end" width={460}>
          <div className="hints">
            <p className="muted">{t('Combine clauses with AND, OR, NOT and parentheses; finish with ORDER BY. Click an example to add it.')}</p>
            <div className="hints-grid">
              {fields.data.map((field) => (
                <button key={field.name} type="button" className="hint" onClick={() => setText((current) => (current.trim() ? `${field.example} AND ${current}` : field.example))}>
                  <strong>{field.name}</strong>
                  <code>{field.example}</code>
                </button>
              ))}
            </div>
            <p className="muted">
              {t('Functions')}: {FUNCTIONS.join(', ')} · {t('dates')}: 2026-11-30, -7d, 2w
            </p>
          </div>
        </Popover>
      )}

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
            <div className="table-wrap">
              <table className="table">
                <thead>
                  <tr>
                    <th className="cell-check">
                      <input type="checkbox" checked={allSelected} aria-label={t('Select all')} onChange={() => setSelected(allSelected ? [] : issues.map((issue) => issue.key))} />
                    </th>
                    <th>{t('Key')}</th>
                    <th>{t('Summary')}</th>
                    <th>{t('Status')}</th>
                    <th>{t('Assignee')}</th>
                    <th className="cell-narrow">{t('Priority')}</th>
                    <th className="cell-narrow">{t('Points')}</th>
                    <th>{t('Updated')}</th>
                  </tr>
                </thead>
                <tbody>
                  {issues.map((issue) => (
                    <ResultRow key={issue.id} issue={issue} project={projects.get(issue.projectId)} selected={selected.includes(issue.key)} onToggle={() => setSelected(selected.includes(issue.key) ? selected.filter((key) => key !== issue.key) : [...selected, issue.key])} />
                  ))}
                </tbody>
              </table>
            </div>
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

function ResultRow({ issue, project, selected, onToggle }: { issue: IssueRow; project: Project | undefined; selected: boolean; onToggle: () => void }) {
  const { userById } = useSession();
  const status = project?.statuses.find((candidate) => candidate.id === issue.statusId);
  const assignee = userById(issue.assigneeId);

  return (
    <tr className={selected ? 'row-selected' : undefined}>
      <td className="cell-check">
        <input type="checkbox" checked={selected} onChange={onToggle} aria-label={issue.key} />
      </td>
      <td className="cell-key">
        <span className="with-avatar">
          <TypeIcon type={project?.types.find((candidate) => candidate.id === issue.typeId)} />
          <IssueKey issue={issue} done={status?.category === 'done'} />
        </span>
      </td>
      <td className="cell-summary">
        <span className="summary-text">{issue.summary}</span>
        <span className="summary-extra">
          {issue.labels.map((label) => (
            <span key={label} className="label">
              {label}
            </span>
          ))}
          <ModuleBadges issue={issue} />
          <DueDate date={issue.dueDate} done={status?.category === 'done'} />
        </span>
      </td>
      <td>{status ? <StatusPill status={status} /> : null}</td>
      <td className="cell-date">
        <span className="with-avatar">
          <Avatar user={assignee} size={20} />
          <span className={assignee ? '' : 'muted'}>{assignee?.name ?? t('Unassigned')}</span>
        </span>
      </td>
      <td className="cell-narrow">
        <PriorityIcon priority={issue.priority} />
      </td>
      <td className="cell-narrow">{issue.points ?? ''}</td>
      <td className="muted cell-date">{timeAgo(issue.updatedAt)}</td>
    </tr>
  );
}

function SaveFilterDialog({ query, existing, onClose, onDone }: { query: string; existing: SavedFilter[]; onClose: () => void; onDone: () => void }) {
  const [name, setName] = useState('');
  const [shared, setShared] = useState(false);
  const [busy, run] = useBusy();

  return (
    <Dialog
      title={t('Save filter')}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button
            variant="primary"
            disabled={busy || !name.trim()}
            onClick={() =>
              void run(async () => {
                await api.post('/api/filters', { name, query, shared });
                onDone();
                onClose();
              }, t('Filter saved'))
            }
          >
            {t('Save')}
          </Button>
        </>
      }
    >
      <div className="form">
        <Field label={t('Name')}>
          <input className="input" autoFocus value={name} onChange={(event) => setName(event.target.value)} />
        </Field>
        <Field label={t('Query')}>
          <code className="code-block">{query}</code>
        </Field>
        <label className="check">
          <input type="checkbox" checked={shared} onChange={(event) => setShared(event.target.checked)} />
          <span>{t('Share with everyone')}</span>
        </label>
        {existing.length > 0 && (
          <div>
            <span className="field-label">{t('Your filters')}</span>
            {existing.map((filter) => (
              <div key={filter.id} className="list-row">
                <span>{filter.name}</span>
                <Button size="sm" variant="ghost" icon="trash" aria-label={t('Delete')} onClick={() => void attempt(() => api.del(`/api/filters/${filter.id}`)).then(onDone)} />
              </div>
            ))}
          </div>
        )}
      </div>
    </Dialog>
  );
}

function BulkDialog({ keys, project, onClose, onDone }: { keys: string[]; project: Project | undefined; projects: number; onClose: () => void; onDone: () => void }) {
  const [assignee, setAssignee] = useState<number | null | undefined>(undefined);
  const [statusId, setStatusId] = useState<number | null>(null);
  const [priority, setPriority] = useState<number | null>(null);
  const [sprintId, setSprintId] = useState<number | null | undefined>(undefined);
  const [busy, run] = useBusy();
  const patch = {
    ...(assignee !== undefined ? { assigneeId: assignee } : {}),
    ...(statusId !== null ? { statusId } : {}),
    ...(priority !== null ? { priority } : {}),
    ...(sprintId !== undefined ? { sprintId } : {}),
  };

  return (
    <Dialog
      title={t('Change {count} issues', { count: keys.length })}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button
            variant="primary"
            disabled={busy || Object.keys(patch).length === 0}
            onClick={() =>
              void run(async () => {
                await api.post('/api/issues/bulk', { keys, patch });
                toast(t('{count} issues changed', { count: keys.length }));
                onDone();
                onClose();
              })
            }
          >
            {t('Apply')}
          </Button>
        </>
      }
    >
      <div className="form">
        <p className="muted">{t('Only the fields you set here change. Either every issue changes or none does.')}</p>
        <Field label={t('Assignee')}>
          {assignee === undefined ? (
            <Button onClick={() => setAssignee(null)}>{t('Change…')}</Button>
          ) : (
            <UserPicker value={assignee} onChange={setAssignee} />
          )}
        </Field>
        <Field label={t('Priority')}>
          <Picker value={priority} options={[{ value: null as number | null, label: t('Leave as is') }, ...PRIORITIES.map((item) => ({ value: item.value as number | null, label: t(item.name) }))]} onChange={setPriority} />
        </Field>
        {project ? (
          <>
            <Field label={t('Status')}>
              <Picker value={statusId} options={[{ value: null as number | null, label: t('Leave as is') }, ...project.statuses.map((status) => ({ value: status.id as number | null, label: status.name }))]} onChange={setStatusId} />
            </Field>
            {project.boardMode === 'scrum' && (
              <Field label={t('Sprint')}>
                <Picker
                  value={sprintId === undefined ? -1 : sprintId}
                  options={[
                    { value: -1 as number | null, label: t('Leave as is') },
                    { value: null, label: t('Backlog') },
                    ...project.sprints.filter((sprint) => sprint.state !== 'closed').map((sprint) => ({ value: sprint.id as number | null, label: sprint.name })),
                  ]}
                  onChange={(value) => setSprintId(value === -1 ? undefined : value)}
                />
              </Field>
            )}
          </>
        ) : (
          <p className="muted">{t('Status and sprint can be changed when all selected issues are in one project.')}</p>
        )}
      </div>
    </Dialog>
  );
}
