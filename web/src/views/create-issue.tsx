import { useEffect, useState, type FormEvent } from 'react';

import { api } from '../api.ts';
import type { IssueRow } from '../bindings/IssueRow.ts';
import type { Project } from '../bindings/Project.ts';
import { PriorityPicker, SprintPicker, TypePicker, UserPicker, canEdit, openIssue } from '../components/issue-bits.tsx';
import { MarkdownEditor } from '../components/markdown.tsx';
import { Button, Dialog, Field, Picker, Spinner, toast, useBusy } from '../components/ui.tsx';
import { t } from '../i18n/index.ts';
import { useRemote, useSession } from '../session.tsx';

type Draft = {
  summary: string;
  description: string;
  typeId: number | null;
  priority: number;
  assigneeId: number | null;
  sprintId: number | null;
  parentKey: string | null;
  points: string;
  labels: string;
};

const EMPTY: Draft = { summary: '', description: '', typeId: null, priority: 3, assigneeId: null, sprintId: null, parentKey: null, points: '', labels: '' };

export type CreateDefaults = Partial<Pick<Draft, 'sprintId' | 'parentKey' | 'typeId'>> & { statusId?: number };

export function CreateIssueDialog({ projectKey, onClose, defaults }: { projectKey: string | null; onClose: () => void; defaults?: CreateDefaults }) {
  const { boot } = useSession();
  const projects = boot.projects.filter((project) => !project.archived && canEdit(project));
  const [key, setKey] = useState(projectKey && projects.some((project) => project.key === projectKey) ? projectKey : (projects[0]?.key ?? ''));
  const project = useRemote<Project | null>(() => (key ? api.get<Project>(`/api/projects/${key}`) : Promise.resolve(null)), [key]);
  const [draft, setDraft] = useState<Draft>({ ...EMPTY, ...defaults });
  const [another, setAnother] = useState(false);
  const [busy, run] = useBusy();
  const loaded = project.data ?? null;
  const type = loaded?.types.find((candidate) => candidate.id === draft.typeId);

  // A type, sprint or parent chosen for one project means nothing in another.
  useEffect(() => {
    if (!loaded) {
      return;
    }

    setDraft((current) => ({
      ...current,
      typeId: loaded.types.some((candidate) => candidate.id === current.typeId)
        ? current.typeId
        : (loaded.types.find((candidate) => candidate.level === (defaults?.parentKey ? 'subtask' : 'standard') && (defaults?.parentKey || candidate.name === 'Task'))?.id ??
          loaded.types.find((candidate) => candidate.level === 'standard')?.id ??
          null),
      sprintId: loaded.sprints.some((sprint) => sprint.id === current.sprintId) ? current.sprintId : null,
      parentKey: current.parentKey?.startsWith(`${loaded.key}-`) ? current.parentKey : null,
    }));
  }, [loaded?.id]);

  const epics = useRemote<IssueRow[]>(
    () =>
      loaded && type?.level === 'standard'
        ? api.get<{ issues: IssueRow[] }>(`/api/search?limit=200&q=${encodeURIComponent(`project = ${loaded.key} AND type = Epic AND category != done ORDER BY key`)}`).then((found) => found.issues)
        : Promise.resolve([]),
    [loaded?.id, type?.level],
  );

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      const created = await api.post<IssueRow>('/api/issues', {
        projectKey: key,
        summary: draft.summary,
        description: draft.description,
        typeId: draft.typeId,
        priority: draft.priority,
        assigneeId: draft.assigneeId,
        sprintId: draft.sprintId,
        parentKey: draft.parentKey,
        statusId: defaults?.statusId,
        points: draft.points.trim() ? Number(draft.points.replace(',', '.')) : null,
        labels: draft.labels.split(/[\s,]+/).filter(Boolean),
      });

      toast(t('{key} created', { key: created.key }));

      if (another) {
        setDraft({ ...draft, summary: '', description: '' });
      } else {
        onClose();
        openIssue(created.key);
      }
    });
  }

  return (
    <Dialog
      title={t('Create issue')}
      onClose={onClose}
      wide
      footer={
        <>
          <label className="check dialog-foot-left">
            <input type="checkbox" checked={another} onChange={(event) => setAnother(event.target.checked)} />
            <span>{t('Create another')}</span>
          </label>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="create-issue" disabled={busy || !loaded || !draft.summary.trim()}>
            {t('Create')}
          </Button>
        </>
      }
    >
      {projects.length === 0 ? (
        <p className="muted">{t('There is no project you can create issues in yet.')}</p>
      ) : (
        <form id="create-issue" className="form" onSubmit={submit}>
          <div className="form-row">
            <Field label={t('Project')}>
              <Picker value={key} options={projects.map((candidate) => ({ value: candidate.key, label: `${candidate.name} (${candidate.key})` }))} onChange={setKey} />
            </Field>
            <Field label={t('Type')}>{loaded && draft.typeId !== null ? <TypePicker project={loaded} value={draft.typeId} onChange={(typeId) => setDraft({ ...draft, typeId })} levels={draft.parentKey && type?.level === 'subtask' ? ['subtask'] : ['epic', 'standard']} /> : <Spinner />}</Field>
          </div>
          <Field label={t('Summary')}>
            <input className="input" autoFocus required maxLength={255} value={draft.summary} onChange={(event) => setDraft({ ...draft, summary: event.target.value })} />
          </Field>
          <Field label={t('Description')}>
            <MarkdownEditor value={draft.description} onChange={(description) => setDraft({ ...draft, description })} rows={7} placeholder={t('What needs to happen, and how will we know it is done?')} />
          </Field>
          <div className="form-row">
            <Field label={t('Assignee')}>
              <UserPicker value={draft.assigneeId} onChange={(assigneeId) => setDraft({ ...draft, assigneeId })} />
            </Field>
            <Field label={t('Priority')}>
              <PriorityPicker value={draft.priority} onChange={(priority) => setDraft({ ...draft, priority })} />
            </Field>
          </div>
          {loaded && type?.level !== 'epic' && (
            <div className="form-row">
              {loaded.boardMode === 'scrum' && type?.level === 'standard' && (
                <Field label={t('Sprint')}>
                  <SprintPicker project={loaded} value={draft.sprintId} onChange={(sprintId) => setDraft({ ...draft, sprintId })} />
                </Field>
              )}
              {type?.level === 'standard' ? (
                <Field label={t('Epic')}>
                  <Picker
                    value={draft.parentKey}
                    options={[{ value: null, label: t('None') }, ...(epics.data ?? []).map((epic) => ({ value: epic.key as string | null, label: `${epic.key} ${epic.summary}` }))]}
                    onChange={(parentKey) => setDraft({ ...draft, parentKey })}
                  />
                </Field>
              ) : (
                <Field label={t('Parent')}>
                  <input className="input" value={draft.parentKey ?? ''} placeholder={`${loaded.key}-12`} onChange={(event) => setDraft({ ...draft, parentKey: event.target.value.trim() || null })} />
                </Field>
              )}
            </div>
          )}
          <div className="form-row">
            <Field label={t('Story points')}>
              <input className="input" inputMode="decimal" value={draft.points} onChange={(event) => setDraft({ ...draft, points: event.target.value })} />
            </Field>
            <Field label={t('Labels')} hint={t('Separated by spaces')}>
              <input className="input" value={draft.labels} onChange={(event) => setDraft({ ...draft, labels: event.target.value })} />
            </Field>
          </div>
        </form>
      )}
    </Dialog>
  );
}
