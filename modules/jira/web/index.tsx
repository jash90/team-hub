import { useEffect, useRef, useState, type ReactNode } from 'react';

import { api } from '@/api.ts';
import type { BoardMode } from '@/bindings/BoardMode.ts';
import type { Issue } from '@/bindings/Issue.ts';
import type { JiraCsvColumn } from '@/bindings/JiraCsvColumn.ts';
import type { JiraCsvPreview } from '@/bindings/JiraCsvPreview.ts';
import type { JiraCsvRequest } from '@/bindings/JiraCsvRequest.ts';
import type { JiraImportRequest } from '@/bindings/JiraImportRequest.ts';
import type { JiraIssuePanel } from '@/bindings/JiraIssuePanel.ts';
import type { JiraJob } from '@/bindings/JiraJob.ts';
import type { JiraJobStep } from '@/bindings/JiraJobStep.ts';
import type { JiraLocalProject } from '@/bindings/JiraLocalProject.ts';
import type { JiraMapping } from '@/bindings/JiraMapping.ts';
import type { JiraNote } from '@/bindings/JiraNote.ts';
import type { JiraPreview } from '@/bindings/JiraPreview.ts';
import type { JiraRemoteProject } from '@/bindings/JiraRemoteProject.ts';
import type { JiraSettings } from '@/bindings/JiraSettings.ts';
import type { JiraTestResult } from '@/bindings/JiraTestResult.ts';
import type { StatusCategory } from '@/bindings/StatusCategory.ts';
import type { TypeLevel } from '@/bindings/TypeLevel.ts';
import { Icon } from '@/components/icon.tsx';
import { PRIORITIES } from '@/components/issue-bits.tsx';
import { Button, ErrorNote, Field, Picker, Spinner, Tag, toast, useBusy } from '@/components/ui.tsx';
import { countOf, formatDateTime, t, timeAgo } from '@/i18n/index.ts';
import type { WebModule } from '@/modules.ts';
import { Link } from '@/router.tsx';
import { useLive, useRemote, useSession } from '@/session.tsx';
import { SettingsCard } from '@/views/settings.tsx';

import { polish } from './pl.ts';
import './styles.css';

const BASE = '/api/m/jira';

// ---------- the issue panel ----------

/** Where an imported issue came from, with the way back to the original. */
function IssuePanel({ issue }: { issue: Issue }) {
  const panel = issue.panels.jira as JiraIssuePanel | undefined;

  if (!panel) {
    return null;
  }

  return (
    <section className="issue-section">
      <p className="jira-origin">
        <Icon name="download" size={14} />
        <span>{t('Imported from Jira')}</span>
        {panel.url ? (
          <>
            <span aria-hidden="true">·</span>
            <a href={panel.url} target="_blank" rel="noreferrer">
              {t('open original')}
            </a>
            <span className="muted">({panel.key})</span>
          </>
        ) : (
          <span className="muted">({panel.key})</span>
        )}
        <span className="muted jira-origin-when" title={formatDateTime(panel.importedAt)}>
          {t('read {when}', { when: timeAgo(panel.importedAt) })}
        </span>
      </p>
    </section>
  );
}

// ---------- the mapping ----------

const LEVELS: { value: TypeLevel; label: string }[] = [
  { value: 'epic', label: 'Epic: holds other issues' },
  { value: 'standard', label: 'Ordinary issue' },
  { value: 'subtask', label: 'Subtask: part of an issue' },
];

const CATEGORIES: { value: StatusCategory; label: string }[] = [
  { value: 'todo', label: 'To do' },
  { value: 'in_progress', label: 'In progress' },
  { value: 'done', label: 'Done' },
];

const JIRA_CATEGORY: Record<string, string> = { new: 'To do', indeterminate: 'In progress', done: 'Done' };

function MapTable({ title, lead, head, children, scroll }: { title: string; lead?: string; head: string[]; children: ReactNode; scroll?: boolean }) {
  return (
    <div className="jira-map">
      <h3>{title}</h3>
      {lead && <p className="muted">{lead}</p>}
      <div className={scroll ? 'jira-map-wrap jira-map-scroll' : 'jira-map-wrap'}>
        <table className="table table-plain">
          <thead>
            <tr>
              {head.map((label) => (
                <th key={label}>{label}</th>
              ))}
            </tr>
          </thead>
          <tbody>{children}</tbody>
        </table>
      </div>
    </div>
  );
}

/** The proposed mapping as tables the administrator can adjust before importing. */
function MappingEditor({ mapping, onChange }: { mapping: JiraMapping; onChange: (mapping: JiraMapping) => void }) {
  const { boot } = useSession();
  const people = [
    { value: null as number | null, label: t('Nobody here yet') },
    ...boot.users.map((user) => ({ value: user.id as number | null, label: user.active ? user.name : `${user.name} (${t('deactivated')})`, keywords: user.email })),
  ];
  const unmatched = mapping.users.filter((entry) => entry.userId === null).length;

  function replace<K extends 'types' | 'statuses' | 'priorities' | 'users'>(list: K, index: number, patch: Partial<JiraMapping[K][number]>): void {
    const entries = mapping[list].map((entry, position) => (position === index ? { ...entry, ...patch } : entry));

    onChange({ ...mapping, [list]: entries });
  }

  return (
    <div className="jira-mapping">
      <Field label={t('Board')} hint={t('Scrum plans work in sprints; kanban shows one continuous flow. It can be changed later in the project settings.')}>
        <Picker
          value={mapping.boardMode ?? 'scrum'}
          options={[
            { value: 'scrum' as BoardMode, label: 'Scrum' },
            { value: 'kanban' as BoardMode, label: 'Kanban' },
          ]}
          onChange={(boardMode) => onChange({ ...mapping, boardMode })}
        />
      </Field>

      {mapping.statuses.length > 0 && (
        <MapTable
          title={t('Statuses')}
          lead={t('In workflow order. Statuses given the same name here become one status; the history keeps every move between different statuses, with its original time and author.')}
          head={[t('In Jira'), t('Name here'), t('Counts as')]}
        >
          {mapping.statuses.map((entry, index) => (
            <tr key={entry.jiraName}>
              <td>
                {entry.jiraName}
                {entry.jiraCategory && <span className="muted jira-hint">{t(JIRA_CATEGORY[entry.jiraCategory] ?? entry.jiraCategory)}</span>}
              </td>
              <td>
                <input className="input" aria-label={t('Name here')} value={entry.name} maxLength={60} onChange={(event) => replace('statuses', index, { name: event.target.value })} />
              </td>
              <td>
                <Picker value={entry.category} options={CATEGORIES.map((option) => ({ value: option.value, label: t(option.label) }))} onChange={(category) => replace('statuses', index, { category })} />
              </td>
            </tr>
          ))}
        </MapTable>
      )}

      {mapping.types.length > 0 && (
        <MapTable title={t('Issue types')} lead={t('Epics hold stories, tasks and bugs; those hold subtasks. A parent that does not fit that order is left out and named in the report.')} head={[t('In Jira'), t('Name here'), t('Level')]}>
          {mapping.types.map((entry, index) => (
            <tr key={entry.jiraName}>
              <td>{entry.jiraName}</td>
              <td>
                <input className="input" aria-label={t('Name here')} value={entry.name} maxLength={60} onChange={(event) => replace('types', index, { name: event.target.value })} />
              </td>
              <td>
                <Picker value={entry.level} options={LEVELS.map((option) => ({ value: option.value, label: t(option.label) }))} onChange={(level) => replace('types', index, { level })} />
              </td>
            </tr>
          ))}
        </MapTable>
      )}

      {mapping.priorities.length > 0 && (
        <MapTable title={t('Priorities')} head={[t('In Jira'), t('Priority here')]}>
          {mapping.priorities.map((entry, index) => (
            <tr key={entry.jiraName}>
              <td>{entry.jiraName}</td>
              <td>
                <Picker value={entry.priority} options={PRIORITIES.map((priority) => ({ value: priority.value, label: t(priority.name) }))} onChange={(priority) => replace('priorities', index, { priority })} />
              </td>
            </tr>
          ))}
        </MapTable>
      )}

      <MapTable
        title={t('People')}
        lead={t('Matched to accounts here by e-mail address. People who appear only in comments or in the history are matched the same way while importing.')}
        head={[t('In Jira'), t('Account here')]}
        scroll
      >
        {mapping.users.map((entry, index) => (
          <tr key={entry.account}>
            <td>
              {entry.name || entry.account}
              {entry.email && <span className="muted jira-hint">{entry.email}</span>}
            </td>
            <td>
              <Picker value={entry.userId} options={people} onChange={(userId) => replace('users', index, { userId })} />
            </td>
          </tr>
        ))}
        {mapping.users.length === 0 && (
          <tr>
            <td colSpan={2} className="muted">
              {t('Jira did not list the people of this project; they are matched by e-mail address while importing.')}
            </td>
          </tr>
        )}
      </MapTable>

      <label className="check jira-check">
        <input type="checkbox" checked={mapping.createUsers} onChange={(event) => onChange({ ...mapping, createUsers: event.target.checked })} />
        <span>
          {t('Create accounts for people who have none here')}
          {unmatched > 0 && <span className="muted"> ({t('{count} of the people listed', { count: unmatched })})</span>}
          <span className="muted jira-check-hint">
            {t('The accounts are switched off and have no password, so nobody can sign in with them; they keep the names on comments, history and assignments. Without them, only the names are kept, as text.')}
          </span>
        </span>
      </label>
    </div>
  );
}

/** What the import will write into, and the choices that go with it. */
function Target({ projectKey, local, options, onChange, incremental }: { projectKey: string; local: JiraLocalProject | null; options: Options; onChange: (options: Options) => void; incremental: boolean }) {
  return (
    <div className="jira-target">
      {!local && <p className="jira-info">{t('A new project {key} will be created; issues keep their keys, so {key}-123 stays {key}-123.', { key: projectKey })}</p>}
      {local && local.importedAt && (
        <p className="jira-info">
          {incremental
            ? t('{key} was last imported {when}. Only issues changed in Jira since then are read; nothing is imported twice.', { key: projectKey, when: timeAgo(local.importedAt) })
            : t('{key} was last imported {when}. Issues that are already here are updated; nothing is imported twice.', { key: projectKey, when: timeAgo(local.importedAt) })}
        </p>
      )}
      {local && !local.importedAt && (
        <label className="check jira-check">
          <input type="checkbox" checked={options.intoExisting} onChange={(event) => onChange({ ...options, intoExisting: event.target.checked })} />
          <span>
            {t('Import into the existing project “{name}”', { name: local.name })}
            <span className="muted jira-check-hint">
              {t('A project with the key {key} already exists here, with {issues}. Its own issues are left alone; where a key is taken, the issue from Jira is skipped and reported.', {
                key: projectKey,
                issues: countOf(local.issueCount, 'issue', 'issues'),
              })}
            </span>
          </span>
        </label>
      )}
      {local && incremental && (
        <label className="check jira-check">
          <input type="checkbox" checked={options.full} onChange={(event) => onChange({ ...options, full: event.target.checked })} />
          <span>{t('Read every issue again, not only those changed in Jira')}</span>
        </label>
      )}
      {local && (
        <label className="check jira-check">
          <input type="checkbox" checked={options.overwriteLocal} onChange={(event) => onChange({ ...options, overwriteLocal: event.target.checked })} />
          <span>
            {t('Overwrite local changes')}
            <span className="muted jira-check-hint">{t('Otherwise issues edited here since they were imported are skipped and listed in the report.')}</span>
          </span>
        </label>
      )}
    </div>
  );
}

type Options = { intoExisting: boolean; overwriteLocal: boolean; full: boolean };

const NO_OPTIONS: Options = { intoExisting: false, overwriteLocal: false, full: false };

function Facts({ items }: { items: [string, string | number][] }) {
  return (
    <dl className="jira-facts">
      {items.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

// ---------- connection ----------

function Connection({ settings, onSaved }: { settings: JiraSettings; onSaved: (settings: JiraSettings) => void }) {
  const [form, setForm] = useState({ baseUrl: settings.baseUrl, email: settings.email, token: '' });
  const [result, setResult] = useState<JiraTestResult | null>(null);
  const [busy, run] = useBusy();
  const changed = form.baseUrl.trim() !== settings.baseUrl || form.email.trim() !== settings.email || form.token.trim() !== '';

  async function save(): Promise<JiraSettings> {
    const saved = await api.put<JiraSettings>(`${BASE}/settings`, form);

    onSaved(saved);
    setForm({ baseUrl: saved.baseUrl, email: saved.email, token: '' });

    return saved;
  }

  return (
    <SettingsCard
      title={t('Connection')}
      lead={t('Jira Cloud: the address of the site, the e-mail of an account and an API token of that account. Jira Server or Data Center: the address and a personal access token, without an e-mail. The import only reads from Jira.')}
    >
      <form
        className="form"
        onSubmit={(event) => {
          event.preventDefault();
          setResult(null);
          void run(save, t('Saved'));
        }}
      >
        <Field label={t('Jira address')}>
          <input className="input input-mono" value={form.baseUrl} onChange={(event) => setForm({ ...form, baseUrl: event.target.value })} placeholder="https://example.atlassian.net" />
        </Field>
        <div className="form-row">
          <Field label={t('E-mail')} hint={t('Only for Jira Cloud.')}>
            <input className="input" type="email" autoComplete="off" value={form.email} onChange={(event) => setForm({ ...form, email: event.target.value })} />
          </Field>
          <Field label={t('Token')} hint={settings.tokenHint ? t('A token ending in {hint} is stored, encrypted. Leave blank to keep it.', { hint: settings.tokenHint }) : t('No token is stored. It is kept encrypted and never shown again.')}>
            <input className="input input-mono" type="password" autoComplete="off" value={form.token} onChange={(event) => setForm({ ...form, token: event.target.value })} />
          </Field>
        </div>
        <div className="inline-form">
          <Button variant="primary" type="submit" disabled={busy || !changed}>
            {t('Save')}
          </Button>
          <Button
            disabled={busy || (!settings.configured && !changed)}
            onClick={() =>
              void run(async () => {
                setResult(null);

                if (changed) {
                  await save();
                }

                setResult(await api.post<JiraTestResult>(`${BASE}/test`, {}));
              })
            }
          >
            {t('Test connection')}
          </Button>
          {settings.tokenHint && (
            <Button
              variant="danger"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  onSaved(await api.put<JiraSettings>(`${BASE}/settings`, { baseUrl: settings.baseUrl, email: settings.email, clearToken: true }));
                  setResult(null);
                }, t('Token removed'))
              }
            >
              {t('Remove token')}
            </Button>
          )}
        </div>
      </form>
      {result && (
        <p className={result.ok ? 'jira-result jira-result-ok' : 'jira-result jira-result-bad'} role="status">
          <Icon name={result.ok ? 'check' : 'alert'} />
          <span>
            {result.ok ? t('Connected.') : t(result.message)}
            {result.ok && result.deployment && ` ${result.deployment === 'cloud' ? t('This is Jira Cloud: REST v3, texts in ADF.') : t('This is Jira Server or Data Center: REST v2, texts in wiki markup.')}`}
            {result.ok && result.version && ` ${t('Version {version}.', { version: result.version })}`}
          </span>
        </p>
      )}
    </SettingsCard>
  );
}

// ---------- import from Jira ----------

function FromJira({ blocked, onStarted }: { blocked: boolean; onStarted: (job: JiraJob) => void }) {
  const [projects, setProjects] = useState<JiraRemoteProject[] | null>(null);
  const [preview, setPreview] = useState<JiraPreview | null>(null);
  const [mapping, setMapping] = useState<JiraMapping | null>(null);
  const [options, setOptions] = useState<Options>(NO_OPTIONS);
  const [choosing, setChoosing] = useState<string | null>(null);
  const [busy, run] = useBusy();

  async function choose(key: string): Promise<void> {
    setChoosing(key);

    try {
      const loaded = await api.get<JiraPreview>(`${BASE}/projects/${encodeURIComponent(key)}/preview`);

      setPreview(loaded);
      setMapping(loaded.mapping);
      setOptions(NO_OPTIONS);
    } finally {
      setChoosing(null);
    }
  }

  const needsConsent = preview?.local !== null && preview?.local !== undefined && !preview.local.importedAt;

  return (
    <SettingsCard
      title={t('Import a project')}
      lead={t('Brings a project over with its issue keys, history, comments, work logs, links, sprints and versions. Attachments stay in Jira and are linked from the description.')}
      action={
        <Button icon="refresh" disabled={busy} onClick={() => void run(async () => setProjects(await api.get<JiraRemoteProject[]>(`${BASE}/projects`)))}>
          {projects ? t('Refresh') : t('Show projects')}
        </Button>
      }
    >
      {busy && !projects && <Spinner label={t('Asking Jira…')} />}
      {projects && projects.length === 0 && <p className="muted">{t('The account behind the token sees no projects in Jira.')}</p>}
      {projects && projects.length > 0 && (
        <div className="jira-map-wrap jira-map-scroll">
          <table className="table table-plain">
            <thead>
              <tr>
                <th>{t('Key')}</th>
                <th>{t('Project')}</th>
                <th>{t('Issues')}</th>
                <th />
                <th />
              </tr>
            </thead>
            <tbody>
              {projects.map((project) => (
                <tr key={project.key} className={preview?.projectKey === project.key ? 'row-selected' : undefined}>
                  <td className="mono">{project.key}</td>
                  <td>{project.name}</td>
                  <td>{project.issueCount ?? '–'}</td>
                  <td>
                    {project.importedAt ? (
                      <Tag tone="green" title={formatDateTime(project.importedAt)}>
                        {t('imported {when}', { when: timeAgo(project.importedAt) })}
                      </Tag>
                    ) : (
                      project.exists && <Tag tone="amber">{t('key in use here')}</Tag>
                    )}
                  </td>
                  <td className="cell-actions">
                    <Button size="sm" disabled={choosing !== null} onClick={() => void run(() => choose(project.key))}>
                      {choosing === project.key ? t('Reading…') : preview?.projectKey === project.key ? t('Read again') : t('Choose')}
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {preview && mapping && (
        <div className="jira-preview">
          <p className="jira-preview-title">
            <span className="mono">{preview.projectKey}</span> {preview.projectName}
          </p>
          <Facts
            items={[
              [t('Issues'), preview.issueCount ?? '–'],
              [t('Sprints'), preview.sprints],
              [t('Versions'), preview.versions],
              [t('Components'), preview.components],
              [t('Jira'), preview.deployment === 'cloud' ? 'Cloud' : 'Server / Data Center'],
            ]}
          />
          {!preview.agile && <p className="jira-info">{t('Jira’s board API did not answer, so sprints are taken from the issues themselves: sprints without issues are not imported.')}</p>}
          {!preview.storyPoints && <p className="jira-info">{t('This Jira has no “Story Points” field, so estimates are not imported.')}</p>}
          <Target projectKey={preview.projectKey} local={preview.local} options={options} onChange={setOptions} incremental={Boolean(preview.local?.importedAt)} />
          <MappingEditor mapping={mapping} onChange={setMapping} />
          <div className="inline-form">
            <Button
              variant="primary"
              icon="download"
              disabled={busy || blocked || (needsConsent && !options.intoExisting)}
              onClick={() =>
                void run(async () => {
                  const request: JiraImportRequest = { projectKey: preview.projectKey, ...options, mapping };

                  onStarted(await api.post<JiraJob>(`${BASE}/import`, request));
                })
              }
            >
              {t('Import {key}', { key: preview.projectKey })}
            </Button>
            {blocked && <span className="muted">{t('An import is running; wait for it to finish.')}</span>}
          </div>
        </div>
      )}
    </SettingsCard>
  );
}

// ---------- import from a CSV file ----------

const TARGET_TEXT: Record<string, string> = {
  ignore: 'Not imported',
  key: 'Issue key',
  id: 'Issue id in Jira',
  projectKey: 'Project key',
  projectName: 'Project name',
  summary: 'Summary',
  description: 'Description',
  type: 'Issue type',
  status: 'Status',
  statusCategory: 'Status category',
  priority: 'Priority',
  assignee: 'Assignee',
  assigneeId: 'Assignee (account id)',
  reporter: 'Reporter',
  reporterId: 'Reporter (account id)',
  created: 'Created',
  updated: 'Updated',
  resolved: 'Resolved',
  due: 'Due date',
  labels: 'Labels',
  components: 'Components',
  fixVersion: 'Fix version',
  sprint: 'Sprint',
  points: 'Story points',
  estimate: 'Original estimate (seconds)',
  parent: 'Parent (id or key)',
  comment: 'A comment',
  worklog: 'Work log',
  attachment: 'Attachment',
  linkInward: 'Link from another issue',
  linkOutward: 'Link to another issue',
};

function FromCsv({ blocked, onStarted }: { blocked: boolean; onStarted: (job: JiraJob) => void }) {
  const [preview, setPreview] = useState<JiraCsvPreview | null>(null);
  const [mapping, setMapping] = useState<JiraMapping | null>(null);
  const [name, setName] = useState('');
  const [options, setOptions] = useState<Options>(NO_OPTIONS);
  const [allColumns, setAllColumns] = useState(false);
  const [busy, run] = useBusy();
  const file = useRef<HTMLInputElement>(null);

  function show(loaded: JiraCsvPreview): void {
    setPreview(loaded);
    setMapping(loaded.mapping);
    setName(loaded.projectName);
  }

  /** Reads the file again with other columns or another project; the mapping keeps what was adjusted. */
  function reread(change: { columns?: JiraCsvColumn[]; projectKey?: string }): void {
    if (!preview || !mapping) {
      return;
    }

    const switched = change.projectKey !== undefined && change.projectKey !== preview.projectKey;
    const request: JiraCsvRequest = {
      columns: change.columns ?? preview.columns,
      projectKey: change.projectKey ?? preview.projectKey,
      projectName: switched ? undefined : name,
      mapping: switched ? undefined : mapping,
    };

    void run(async () => {
      show(await api.post<JiraCsvPreview>(`${BASE}/csv/${preview.uploadId}/preview`, request));

      if (switched) {
        setOptions(NO_OPTIONS);
      }
    });
  }

  const ignored = preview?.columns.filter((column) => column.target === 'ignore').length ?? 0;
  const shown = preview?.columns.filter((column) => allColumns || column.target !== 'ignore') ?? [];
  const needsConsent = preview?.local !== null && preview?.local !== undefined && !preview.local.importedAt;

  return (
    <SettingsCard
      title={t('Import a CSV file')}
      lead={t('For when there is no API access. In Jira, search for the project’s issues and choose Export → CSV (all fields). A file has no history: each issue gets one status change, at the time it was resolved or last updated.')}
    >
      <div className="inline-form">
        <input
          ref={file}
          type="file"
          accept=".csv,text/csv"
          className="jira-file"
          aria-label={t('CSV file')}
          onChange={(event) => {
            const chosen = event.target.files?.[0];

            event.target.value = '';

            if (chosen) {
              void run(async () => {
                show(await api.upload<JiraCsvPreview>(`${BASE}/csv`, chosen));
                setOptions(NO_OPTIONS);
              });
            }
          }}
        />
        <Button icon="upload" disabled={busy} onClick={() => file.current?.click()}>
          {preview ? t('Choose another file') : t('Choose a file')}
        </Button>
        {preview && (
          <span className="muted">
            {preview.filename} · {t('{count} rows', { count: preview.rows })}
          </span>
        )}
      </div>

      {preview && mapping && (
        <div className="jira-preview">
          <div className="form-row">
            <Field label={t('Project in the file')}>
              <Picker
                value={preview.projectKey}
                options={preview.projects.map((project) => ({ value: project.key, label: `${project.key} · ${countOf(project.issues, 'issue', 'issues')}` }))}
                onChange={(projectKey) => reread({ projectKey })}
              />
            </Field>
            <Field label={t('Project name')}>
              <input className="input" value={name} disabled={preview.local !== null} onChange={(event) => setName(event.target.value)} />
            </Field>
          </div>
          <Facts
            items={[
              [t('Issues'), preview.issueCount],
              [t('Sprints'), preview.sprints],
              [t('Versions'), preview.versions],
              [t('Components'), preview.components],
            ]}
          />
          {preview.problems.length > 0 && (
            <div className="jira-problems">
              <strong>{t('Rows that need a look')}</strong>
              <ul>
                {preview.problems.map((problem) => (
                  <li key={problem}>{problem}</li>
                ))}
              </ul>
            </div>
          )}

          <MapTable title={t('Columns')} lead={t('Jira repeats a column for every value of a field with several (labels, sprints, comments); each of them is read.')} head={[t('Column'), t('Example'), t('Read as')]} scroll>
            {shown.map((column) => (
              <tr key={column.index}>
                <td>{column.name || t('(no name)')}</td>
                <td className="muted jira-sample">{column.sample}</td>
                <td>
                  <Picker
                    value={column.target}
                    options={preview.targets.map((target) => ({ value: target, label: t(TARGET_TEXT[target] ?? target) }))}
                    onChange={(target) => reread({ columns: preview.columns.map((other) => (other.index === column.index ? { ...other, target } : other)) })}
                  />
                </td>
              </tr>
            ))}
          </MapTable>
          {ignored > 0 && (
            <Button size="sm" variant="ghost" onClick={() => setAllColumns(!allColumns)}>
              {allColumns ? t('Hide the columns that are not imported') : t('Show the {count} columns that are not imported', { count: ignored })}
            </Button>
          )}

          <Target projectKey={preview.projectKey} local={preview.local} options={options} onChange={setOptions} incremental={false} />
          <MappingEditor mapping={mapping} onChange={setMapping} />
          <div className="inline-form">
            <Button
              variant="primary"
              icon="download"
              disabled={busy || blocked || preview.issueCount === 0 || (needsConsent && !options.intoExisting)}
              onClick={() =>
                void run(async () => {
                  const request: JiraCsvRequest = {
                    columns: preview.columns,
                    projectKey: preview.projectKey,
                    projectName: name,
                    intoExisting: options.intoExisting,
                    overwriteLocal: options.overwriteLocal,
                    mapping,
                  };

                  onStarted(await api.post<JiraJob>(`${BASE}/csv/${preview.uploadId}/import`, request));
                })
              }
            >
              {t('Import {key}', { key: preview.projectKey })}
            </Button>
            {blocked && <span className="muted">{t('An import is running; wait for it to finish.')}</span>}
          </div>
        </div>
      )}
    </SettingsCard>
  );
}

// ---------- progress and report ----------

const STEP_TEXT: Record<JiraJobStep, string> = {
  waiting: 'Waiting to start',
  connecting: 'Connecting to Jira',
  structure: 'Creating the project: statuses, types, components, versions, sprints',
  issues: 'Importing issues',
  relations: 'Connecting parents and links',
  finished: 'Finished',
};

const STATE_TEXT = { queued: 'waiting to start', running: 'importing', done: 'imported', failed: 'import failed', cancelled: 'import cancelled' } as const;
const STATE_TONE = { queued: 'blue', running: 'blue', done: 'green', failed: 'red', cancelled: 'neutral' } as const;

function isRunning(job: JiraJob): boolean {
  return job.state === 'queued' || job.state === 'running';
}

function Notes({ title, notes }: { title: string; notes: JiraNote[] }) {
  if (notes.length === 0) {
    return null;
  }

  return (
    <div className="jira-notes">
      <strong>
        {title} <span className="tab-count">{notes.length}</span>
      </strong>
      <div className="jira-map-wrap jira-map-scroll">
        <table className="table table-plain">
          <tbody>
            {notes.map((note) => (
              <tr key={`${note.key}:${note.reason}`}>
                <td className="mono jira-note-key">{note.key}</td>
                <td>{t(note.reason)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function JobCard({ job, onClose }: { job: JiraJob; onClose: () => void }) {
  const { boot } = useSession();
  const [busy, run] = useBusy();
  const report = job.report;
  const running = isRunning(job);
  const total = job.total ?? 0;
  const share = job.state === 'done' ? 100 : total > 0 ? Math.min(100, Math.round((job.processed / total) * 100)) : 0;
  const exists = boot.projects.some((project) => project.key === job.projectKey);
  const written = report.created + report.updated;

  return (
    <SettingsCard
      title={running ? t('Importing {key}', { key: job.projectKey }) : t('Import of {key}', { key: job.projectKey })}
      action={
        running ? (
          <Button size="sm" variant="danger" disabled={busy} onClick={() => void run(() => api.post(`${BASE}/jobs/${job.id}/cancel`, {}), t('The import stops after the issues it is on'))}>
            {t('Cancel')}
          </Button>
        ) : (
          <Button size="sm" variant="ghost" icon="x" aria-label={t('Close')} onClick={onClose} />
        )
      }
    >
      <div className="jira-progress">
        <div className="jira-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={share} aria-label={t('Progress')}>
          <span className={`jira-bar-fill jira-bar-${job.state}`} style={{ width: `${share}%` }} />
        </div>
        <p className="jira-step">
          <Tag tone={STATE_TONE[job.state]}>{t(STATE_TEXT[job.state])}</Tag>
          <span>{running ? t(STEP_TEXT[job.step]) : t('{when}, from {source}', { when: formatDateTime(job.finishedAt ?? job.createdAt), source: job.kind === 'csv' ? t('a CSV file') : 'Jira' })}</span>
          {(running || job.processed > 0) && (
            <span className="muted">{job.total !== null ? t('{done} of {total} issues', { done: job.processed, total: Math.max(job.total, job.processed) }) : countOf(job.processed, 'issue', 'issues')}</span>
          )}
        </p>
      </div>

      {job.error && <ErrorNote message={t(job.error)} />}

      <Facts
        items={[
          [t('Created'), report.created],
          [t('Updated'), report.updated],
          [t('Unchanged'), report.unchanged],
          [t('Skipped'), report.skipped.length],
          [t('Failed'), report.failed.length],
        ]}
      />

      {!running && (
        <div className="jira-report">
          <Facts
            items={[
              [t('Comments'), report.comments],
              [t('History entries'), report.changes],
              [t('Work logs'), report.worklogs],
              [t('Links'), report.links],
              [t('Sprints'), report.sprints],
              [t('Versions'), report.versions],
              [t('Components'), report.components],
              [t('People matched'), report.usersMatched],
              [t('Accounts created'), report.usersCreated.length],
            ]}
          />
          {report.usersCreated.length > 0 && (
            <p className="muted">
              {t('Accounts created, switched off:')} {report.usersCreated.join(', ')}
            </p>
          )}
          <Notes title={t('Skipped')} notes={report.skipped} />
          <Notes title={t('Failed')} notes={report.failed} />
          {report.unmapped.length > 0 && (
            <div className="jira-notes">
              <strong>
                {t('Could not be mapped as it was')} <span className="tab-count">{report.unmapped.length}</span>
              </strong>
              <ul>
                {report.unmapped.map((line) => (
                  <li key={line}>{line}</li>
                ))}
              </ul>
            </div>
          )}
          {exists && (written > 0 || report.unchanged > 0) && (
            <p className="jira-open">
              <Link href={`/p/${job.projectKey}/board`}>{t('Open {key}', { key: job.projectKey })}</Link>
              <Link href={`/p/${job.projectKey}/backlog`}>{t('Backlog')}</Link>
            </p>
          )}
        </div>
      )}
    </SettingsCard>
  );
}

function PastJobs({ jobs, shown, onShow }: { jobs: JiraJob[]; shown: number | null; onShow: (id: number) => void }) {
  const { userById } = useSession();

  if (jobs.length === 0) {
    return null;
  }

  return (
    <SettingsCard title={t('Past imports')}>
      <div className="jira-map-wrap">
        <table className="table table-plain">
          <thead>
            <tr>
              <th>{t('When')}</th>
              <th>{t('Project')}</th>
              <th />
              <th>{t('Created')}</th>
              <th>{t('Updated')}</th>
              <th>{t('Skipped')}</th>
              <th>{t('Failed')}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {jobs.map((job) => (
              <tr key={job.id} className={job.id === shown ? 'row-selected' : undefined}>
                <td className="cell-date" title={userById(job.startedBy)?.name}>
                  {formatDateTime(job.createdAt)}
                </td>
                <td>
                  <span className="mono">{job.projectKey}</span> <span className="muted">{job.kind === 'csv' ? 'CSV' : 'Jira'}</span>
                </td>
                <td>
                  <Tag tone={STATE_TONE[job.state]}>{t(STATE_TEXT[job.state])}</Tag>
                </td>
                <td>{job.report.created}</td>
                <td>{job.report.updated}</td>
                <td>{job.report.skipped.length}</td>
                <td>{job.report.failed.length}</td>
                <td className="cell-actions">
                  <Button size="sm" variant="ghost" onClick={() => onShow(job.id)}>
                    {t('Report')}
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </SettingsCard>
  );
}

// ---------- the settings tab ----------

/** What the server announces each time a job moves (`jira.job`). */
type Pulse = { jobId: number; state: JiraJob['state']; step: JiraJobStep; processed: number; total: number | null; created: number; updated: number; unchanged: number };

function AdminSettings() {
  const { refresh } = useSession();
  const settings = useRemote(() => api.get<JiraSettings>(`${BASE}/settings`), []);
  const jobs = useRemote(
    () => api.get<JiraJob[]>(`${BASE}/jobs`),
    [],
    (event) => event.kind === 'jira.job',
  );
  const [shown, setShown] = useState<number | null>(null);
  const [pulse, setPulse] = useState<Pulse | null>(null);
  const card = useRef<HTMLDivElement>(null);
  const wasRunning = useRef<number | null>(null);
  const list = jobs.data ?? [];
  const running = list.find(isRunning);
  const loaded = list.find((candidate) => candidate.id === shown) ?? running;
  // The list reloads when the announcements pause; while they keep coming, the newest one
  // says where the running job is.
  const ahead = loaded && pulse && pulse.jobId === loaded.id && isRunning(loaded) && pulse.processed >= loaded.processed;
  const job: JiraJob | undefined =
    loaded && ahead
      ? {
          ...loaded,
          // The end of a job is taken from the list, which then has the whole report.
          state: pulse.state === 'running' ? 'running' : loaded.state,
          step: pulse.step,
          processed: pulse.processed,
          total: pulse.total,
          report: { ...loaded.report, created: pulse.created, updated: pulse.updated, unchanged: pulse.unchanged },
        }
      : loaded;

  useLive((event) => {
    if (event.kind === 'jira.job' && typeof event.payload.jobId === 'number') {
      setPulse(event.payload as Pulse);
    }
  });

  // When an import ends, the project and the people it created exist: the rest of the
  // application learns about them.
  useEffect(() => {
    if (running) {
      wasRunning.current = running.id;
    } else if (wasRunning.current !== null) {
      const finished = list.find((candidate) => candidate.id === wasRunning.current);

      wasRunning.current = null;
      setShown(finished?.id ?? null);
      void refresh();

      if (finished?.state === 'done') {
        toast(t('Import of {key} finished', { key: finished.projectKey }));
      }
    }
    // Only the running job decides; the list is read when it changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [running?.id]);

  function started(next: JiraJob): void {
    setShown(next.id);
    jobs.set([next, ...list.filter((candidate) => candidate.id !== next.id)]);
    window.setTimeout(() => card.current?.scrollIntoView({ behavior: 'smooth', block: 'start' }), 50);
  }

  if (!settings.data) {
    return settings.error ? <ErrorNote message={settings.error} /> : <Spinner />;
  }

  return (
    <>
      <Connection settings={settings.data} onSaved={settings.set} />
      {settings.data.configured && <FromJira blocked={Boolean(running)} onStarted={started} />}
      <FromCsv blocked={Boolean(running)} onStarted={started} />
      <div ref={card} className="jira-job">
        {job && <JobCard job={job} onClose={() => setShown(null)} />}
      </div>
      <PastJobs
        jobs={list}
        shown={job?.id ?? null}
        onShow={(id) => {
          setShown(id);
          window.setTimeout(() => card.current?.scrollIntoView({ behavior: 'smooth', block: 'start' }), 50);
        }}
      />
    </>
  );
}

export const jiraModule: WebModule = {
  id: 'jira',
  issuePanel: IssuePanel,
  settings: [{ id: 'jira', label: () => t('Jira import'), adminOnly: true, render: () => <AdminSettings /> }],
  polish,
};
