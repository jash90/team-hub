import { useState } from 'react';

import { api } from '../../api.ts';
import type { Project } from '../../bindings/Project.ts';
import type { TypeLevel } from '../../bindings/TypeLevel.ts';
import { TypeIcon } from '../../components/issue-bits.tsx';
import { Button, InlineEdit, Picker } from '../../components/ui.tsx';
import { t } from '../../i18n/index.ts';
import { Card, RemoveDialog, useChange } from './parts.tsx';

const LEVELS: { value: TypeLevel; label: string }[] = [
  { value: 'epic', label: 'Epic' },
  { value: 'standard', label: 'Issue' },
  { value: 'subtask', label: 'Subtask' },
];
const ICONS = ['story', 'task', 'bug', 'epic', 'subtask'];

export function Types({ project }: { project: Project }) {
  const change = useChange();
  const [name, setName] = useState('');
  const [level, setLevel] = useState<TypeLevel>('standard');
  const [removing, setRemoving] = useState<{ id: number; name: string } | null>(null);
  const base = `/api/projects/${project.key}/types`;

  return (
    <Card title={t('Issue types')} lead={t('An epic groups issues; a subtask splits one. Everything else is an ordinary issue.')}>
      <table className="table table-plain">
        <thead>
          <tr>
            <th>{t('Type')}</th>
            <th>{t('Level')}</th>
            <th>{t('Icon')}</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {project.types.map((type) => (
            <tr key={type.id}>
              <td>
                <span className="with-avatar">
                  <TypeIcon type={type} />
                  <InlineEdit value={type.name} onSave={(next) => void change(() => api.patch(`${base}/${type.id}`, { name: next }))} />
                </span>
              </td>
              <td className="muted">{t(LEVELS.find((entry) => entry.value === type.level)?.label ?? '')}</td>
              <td>
                <Picker value={type.icon} options={ICONS.map((icon) => ({ value: icon, label: icon, render: <TypeIcon type={{ ...type, icon }} /> }))} onChange={(icon) => void change(() => api.patch(`${base}/${type.id}`, { icon }))} />
              </td>
              <td className="cell-actions">
                <Button size="sm" variant="ghost" icon="trash" aria-label={t('Delete')} onClick={() => setRemoving(type)} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <form
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault();
          void change(() => api.post(base, { name, level, icon: level === 'epic' ? 'epic' : level === 'subtask' ? 'subtask' : 'task' })).then(() => setName(''));
        }}
      >
        <input className="input" placeholder={t('New type')} value={name} onChange={(event) => setName(event.target.value)} />
        <Picker value={level} options={LEVELS.map((entry) => ({ value: entry.value, label: t(entry.label) }))} onChange={setLevel} />
        <Button type="submit" icon="plus" disabled={!name.trim()}>
          {t('Add')}
        </Button>
      </form>
      {removing && (
        <RemoveDialog
          what={removing.name}
          options={project.types.filter((type) => type.id !== removing.id && type.level === project.types.find((candidate) => candidate.id === removing.id)?.level)}
          onClose={() => setRemoving(null)}
          onConfirm={(moveTo) => change(() => api.del(`${base}/${removing.id}?moveTo=${moveTo}`))}
        />
      )}
    </Card>
  );
}
