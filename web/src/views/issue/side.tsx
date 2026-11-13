import { useState, type ReactNode } from 'react';

import { api } from '../../api.ts';
import type { Issue } from '../../bindings/Issue.ts';
import type { IssueRow } from '../../bindings/IssueRow.ts';
import type { Project } from '../../bindings/Project.ts';
import { PriorityPicker, SprintPicker, StatusPicker, TypePicker, UserPicker, VersionPicker } from '../../components/issue-bits.tsx';
import { Picker, Popover } from '../../components/ui.tsx';
import { formatDateTime, formatMinutes, t, timeAgo } from '../../i18n/index.ts';
import { useRemote, useSession } from '../../session.tsx';
import type { Patch } from './patch.ts';

/** The column of fields beside an issue: status, people, planning, dates. */
export function IssueSide({ issue, project, editable, update, onLogWork }: { issue: Issue; project: Project; editable: boolean; update: (patch: Patch) => Promise<void>; onLogWork: () => void }) {
  const { user } = useSession();
  const type = project.types.find((candidate) => candidate.id === issue.typeId);
  const status = project.statuses.find((candidate) => candidate.id === issue.statusId);
  const allowed = project.strictWorkflow
    ? project.transitions.filter((transition) => transition.fromStatusId === null || transition.fromStatusId === issue.statusId).map((transition) => transition.toStatusId)
    : undefined;

  return (
    <aside className="issue-side">
      <div className="side-status">{editable ? <StatusPicker project={project} value={issue.statusId} onChange={(statusId) => void update({ statusId })} allowed={allowed} /> : <span className={`status status-${status?.category}`}>{status?.name}</span>}</div>
      <dl className="fields">
        <SideField label={t('Assignee')}>
          <UserPicker value={issue.assigneeId} onChange={(assigneeId) => void update({ assigneeId })} variant="inline" disabled={!editable} />
          {editable && issue.assigneeId !== user.id && (
            <button type="button" className="link-button" onClick={() => void update({ assigneeId: user.id })}>
              {t('Assign to me')}
            </button>
          )}
        </SideField>
        <SideField label={t('Reporter')}>
          <UserPicker value={issue.reporterId} onChange={(reporterId) => void update({ reporterId })} variant="inline" disabled={!editable} />
        </SideField>
        <SideField label={t('Priority')}>
          <PriorityPicker value={issue.priority} onChange={(priority) => void update({ priority })} variant="inline" disabled={!editable} />
        </SideField>
        <SideField label={t('Type')}>
          <TypePicker project={project} value={issue.typeId} onChange={(typeId) => void update({ typeId })} variant="inline" disabled={!editable} levels={type ? [type.level] : undefined} />
        </SideField>
        {type?.level === 'standard' && <EpicField issue={issue} project={project} editable={editable} onChange={(parentKey) => void update({ parentKey })} />}
        {project.boardMode === 'scrum' && type?.level === 'standard' && (
          <SideField label={t('Sprint')}>
            <SprintPicker project={project} value={issue.sprintId} onChange={(sprintId) => void update({ sprintId })} variant="inline" disabled={!editable} />
          </SideField>
        )}
        {type?.level !== 'subtask' && (
          <SideField label={t('Fix version')}>
            <VersionPicker project={project} value={issue.versionId} onChange={(versionId) => void update({ versionId })} variant="inline" disabled={!editable} />
          </SideField>
        )}
        {type?.level !== 'epic' && (
          <SideField label={t('Story points')}>
            <NumberField value={issue.points} editable={editable} onSave={(points) => void update({ points })} />
          </SideField>
        )}
        <SideField label={t('Labels')}>
          <LabelsField labels={issue.labels} known={project.labels} editable={editable} onSave={(labels) => void update({ labels })} />
        </SideField>
        {project.components.length > 0 && (
          <SideField label={t('Components')}>
            <ComponentsField issue={issue} project={project} editable={editable} onSave={(componentIds) => void update({ componentIds })} />
          </SideField>
        )}
        <SideField label={t('Due date')}>
          <input
            type="date"
            className="input input-inline"
            value={issue.dueDate ?? ''}
            disabled={!editable}
            onChange={(event) => void update({ dueDate: event.target.value || null })}
          />
        </SideField>
        <SideField label={t('Time')}>
          <button type="button" className="link-button" disabled={!editable} onClick={() => onLogWork()}>
            {issue.spentMinutes > 0 ? t('{spent} logged', { spent: formatMinutes(issue.spentMinutes) }) : t('Log work')}
          </button>
          {issue.estimateMinutes !== null && <span className="muted"> · {t('estimate {estimate}', { estimate: formatMinutes(issue.estimateMinutes) })}</span>}
        </SideField>
      </dl>
      <p className="issue-dates muted">
        {t('Created {when}', { when: formatDateTime(issue.createdAt) })}
        <br />
        {t('Updated {when}', { when: timeAgo(issue.updatedAt) })}
        {issue.resolvedAt && (
          <>
            <br />
            {t('Resolved {when}', { when: formatDateTime(issue.resolvedAt) })}
          </>
        )}
      </p>
    </aside>
  );
}

function SideField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </>
  );
}

function NumberField({ value, editable, onSave }: { value: number | null; editable: boolean; onSave: (value: number | null) => void }) {
  const [draft, setDraft] = useState<string | null>(null);

  function commit(): void {
    if (draft === null) {
      return;
    }

    const parsed = draft.trim() === '' ? null : Number(draft.replace(',', '.'));
    setDraft(null);

    if (parsed !== value && (parsed === null || Number.isFinite(parsed))) {
      onSave(parsed);
    }
  }

  return (
    <input
      className="input input-inline input-number"
      inputMode="decimal"
      disabled={!editable}
      placeholder="–"
      value={draft ?? (value === null ? '' : String(value))}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={commit}
      onKeyDown={(event) => event.key === 'Enter' && event.currentTarget.blur()}
    />
  );
}

function LabelsField({ labels, known, editable, onSave }: { labels: string[]; known: string[]; editable: boolean; onSave: (labels: string[]) => void }) {
  const [draft, setDraft] = useState('');

  function add(): void {
    const label = draft.trim().replace(/\s+/g, '-');
    setDraft('');

    if (label && !labels.some((existing) => existing.toLowerCase() === label.toLowerCase())) {
      onSave([...labels, label]);
    }
  }

  return (
    <div className="chips">
      {labels.map((label) => (
        <span key={label} className="label">
          {label}
          {editable && (
            <button type="button" aria-label={t('Remove')} onClick={() => onSave(labels.filter((existing) => existing !== label))}>
              ×
            </button>
          )}
        </span>
      ))}
      {editable && (
        <>
          <input
            className="input input-inline chips-input"
            list="known-labels"
            placeholder={labels.length === 0 ? t('Add label') : '+'}
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ' ' || event.key === ',') {
                event.preventDefault();
                add();
              }
            }}
            onBlur={add}
          />
          <datalist id="known-labels">
            {known
              .filter((label) => !labels.includes(label))
              .map((label) => (
                <option key={label} value={label} />
              ))}
          </datalist>
        </>
      )}
      {!editable && labels.length === 0 && <span className="muted">–</span>}
    </div>
  );
}

function ComponentsField({ issue, project, editable, onSave }: { issue: Issue; project: Project; editable: boolean; onSave: (ids: number[]) => void }) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const chosen = project.components.filter((component) => issue.componentIds.includes(component.id));

  return (
    <>
      <button type="button" className="picker picker-inline" disabled={!editable} onClick={(event) => setAnchor(anchor ? null : event.currentTarget)}>
        {chosen.length > 0 ? chosen.map((component) => component.name).join(', ') : <span className="muted">{t('None')}</span>}
      </button>
      {anchor && (
        <Popover anchor={anchor} onClose={() => setAnchor(null)}>
          <div className="menu">
            {project.components.map((component) => {
              const on = issue.componentIds.includes(component.id);

              return (
                <label key={component.id} className="menu-item check">
                  <input type="checkbox" checked={on} onChange={() => onSave(on ? issue.componentIds.filter((id) => id !== component.id) : [...issue.componentIds, component.id])} />
                  <span>{component.name}</span>
                </label>
              );
            })}
          </div>
        </Popover>
      )}
    </>
  );
}

function EpicField({ issue, project, editable, onChange }: { issue: Issue; project: Project; editable: boolean; onChange: (parentKey: string | null) => void }) {
  const epics = useRemote(
    () => api.get<{ issues: IssueRow[] }>(`/api/search?limit=200&q=${encodeURIComponent(`project = ${project.key} AND type = Epic ORDER BY key`)}`).then((found) => found.issues),
    [project.key],
  );
  const options = [
    { value: null as string | null, label: t('None') },
    ...(epics.data ?? []).map((epic) => ({ value: epic.key as string | null, label: `${epic.key} ${epic.summary}` })),
  ];

  return (
    <SideField label={t('Epic')}>
      <Picker value={issue.parentKey} options={options} onChange={onChange} variant="inline" disabled={!editable} display={issue.parentKey ? <span className="epic-chip">{issue.parentSummary}</span> : undefined} placeholder={t('None')} />
    </SideField>
  );
}
