import { useState } from 'react';

import { api } from '../../api.ts';
import type { Project } from '../../bindings/Project.ts';
import type { Status } from '../../bindings/Status.ts';
import type { StatusCategory } from '../../bindings/StatusCategory.ts';
import { categoryLabel } from '../../components/issue-bits.tsx';
import { Button, InlineEdit, Picker } from '../../components/ui.tsx';
import { t } from '../../i18n/index.ts';
import { Card, RemoveDialog, useChange } from './parts.tsx';

const CATEGORIES: StatusCategory[] = ['todo', 'in_progress', 'done'];

export function Workflow({ project }: { project: Project }) {
  const change = useChange();
  const [name, setName] = useState('');
  const [category, setCategory] = useState<StatusCategory>('in_progress');
  const [removing, setRemoving] = useState<Status | null>(null);
  const base = `/api/projects/${project.key}`;
  const allowed = (from: number, to: number) => project.transitions.some((transition) => transition.toStatusId === to && (transition.fromStatusId === null || transition.fromStatusId === from));

  function toggle(from: number, to: number): void {
    const kept = project.transitions.filter((transition) => !(transition.toStatusId === to && transition.fromStatusId === from));
    const next = allowed(from, to)
      ? // Removing a move that is allowed "from anywhere" means spelling the others out.
        kept.flatMap((transition) =>
          transition.toStatusId === to && transition.fromStatusId === null
            ? project.statuses.filter((status) => status.id !== from && status.id !== to).map((status) => ({ fromStatusId: status.id as number | null, toStatusId: to, name: transition.name }))
            : [transition],
        )
      : [...kept, { fromStatusId: from as number | null, toStatusId: to, name: '' }];

    void change(() => api.put(`${base}/transitions`, next));
  }

  return (
    <>
      <Card title={t('Statuses')} lead={t('The columns of the board, in this order. The category decides what counts as started and as finished.')}>
        <table className="table table-plain">
          <thead>
            <tr>
              <th>{t('Status')}</th>
              <th>{t('Category')}</th>
              <th>{t('On the board')}</th>
              <th>{t('WIP limit')}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {project.statuses.map((status, index) => (
              <tr key={status.id}>
                <td>
                  <InlineEdit value={status.name} onSave={(next) => void change(() => api.patch(`${base}/statuses/${status.id}`, { name: next }))} />
                </td>
                <td>
                  <Picker value={status.category} options={CATEGORIES.map((value) => ({ value, label: categoryLabel(value) }))} onChange={(next) => void change(() => api.patch(`${base}/statuses/${status.id}`, { category: next }))} />
                </td>
                <td>
                  <input type="checkbox" checked={status.onBoard} aria-label={t('On the board')} onChange={(event) => void change(() => api.patch(`${base}/statuses/${status.id}`, { onBoard: event.target.checked }))} />
                </td>
                <td>
                  <input
                    className="input input-number"
                    inputMode="numeric"
                    placeholder="–"
                    defaultValue={status.wipLimit ?? ''}
                    onBlur={(event) => {
                      const limit = event.target.value.trim() ? Number(event.target.value) : null;

                      if (limit !== status.wipLimit) {
                        void change(() => api.patch(`${base}/statuses/${status.id}`, { wipLimit: limit }));
                      }
                    }}
                  />
                </td>
                <td className="cell-actions">
                  <Button size="sm" variant="ghost" icon="chevronUp" disabled={index === 0} aria-label={t('Move up')} onClick={() => void change(() => api.patch(`${base}/statuses/${status.id}`, { position: index - 1 }))} />
                  <Button size="sm" variant="ghost" icon="chevronDown" disabled={index === project.statuses.length - 1} aria-label={t('Move down')} onClick={() => void change(() => api.patch(`${base}/statuses/${status.id}`, { position: index + 1 }))} />
                  <Button size="sm" variant="ghost" icon="trash" aria-label={t('Delete')} disabled={project.statuses.length <= 1} onClick={() => setRemoving(status)} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <form
          className="inline-form"
          onSubmit={(event) => {
            event.preventDefault();
            void change(() => api.post(`${base}/statuses`, { name, category })).then(() => setName(''));
          }}
        >
          <input className="input" placeholder={t('New status')} value={name} onChange={(event) => setName(event.target.value)} />
          <Picker value={category} options={CATEGORIES.map((value) => ({ value, label: categoryLabel(value) }))} onChange={setCategory} />
          <Button type="submit" icon="plus" disabled={!name.trim()}>
            {t('Add')}
          </Button>
        </form>
      </Card>

      <Card title={t('Transitions')} lead={t('By default an issue can move from any status to any other. A strict workflow allows only the moves ticked below.')}>
        <label className="check">
          <input type="checkbox" checked={project.strictWorkflow} onChange={(event) => void change(() => api.patch(base, { strictWorkflow: event.target.checked }))} />
          <span>{t('Strict workflow')}</span>
        </label>
        {project.strictWorkflow && (
          <div className="table-wrap">
            <table className="table table-plain matrix">
              <thead>
                <tr>
                  <th>{t('From ↓ to →')}</th>
                  {project.statuses.map((status) => (
                    <th key={status.id}>{status.name}</th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {project.statuses.map((from) => (
                  <tr key={from.id}>
                    <th>{from.name}</th>
                    {project.statuses.map((to) => (
                      <td key={to.id}>{from.id === to.id ? <span className="muted">·</span> : <input type="checkbox" checked={allowed(from.id, to.id)} aria-label={`${from.name} → ${to.name}`} onChange={() => toggle(from.id, to.id)} />}</td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>

      {removing && <RemoveDialog what={removing.name} options={project.statuses.filter((status) => status.id !== removing.id)} onClose={() => setRemoving(null)} onConfirm={(moveTo) => change(() => api.del(`${base}/statuses/${removing.id}?moveTo=${moveTo}`))} />}
    </>
  );
}
