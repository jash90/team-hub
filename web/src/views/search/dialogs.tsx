import { useState } from 'react';

import { api } from '../../api.ts';
import type { Project } from '../../bindings/Project.ts';
import type { SavedFilter } from '../../bindings/SavedFilter.ts';
import { PRIORITIES, UserPicker } from '../../components/issue-bits.tsx';
import { Button, Dialog, Field, Picker, attempt, toast, useBusy } from '../../components/ui.tsx';
import { t } from '../../i18n/index.ts';

export function SaveFilterDialog({ query, existing, onClose, onDone }: { query: string; existing: SavedFilter[]; onClose: () => void; onDone: () => void }) {
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

export function BulkDialog({ keys, project, onClose, onDone }: { keys: string[]; project: Project | undefined; projects: number; onClose: () => void; onDone: () => void }) {
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
