import { useState } from 'react';

import { api } from '../../api.ts';
import type { Project } from '../../bindings/Project.ts';
import { UserPicker } from '../../components/issue-bits.tsx';
import { Button, InlineEdit } from '../../components/ui.tsx';
import { t } from '../../i18n/index.ts';
import { Card, useChange } from './parts.tsx';

export function Components({ project }: { project: Project }) {
  const change = useChange();
  const [name, setName] = useState('');
  const base = `/api/projects/${project.key}/components`;

  return (
    <Card title={t('Components')} lead={t('Parts of the product an issue can belong to; searchable as component = Name.')}>
      <table className="table table-plain">
        <tbody>
          {project.components.map((component) => (
            <tr key={component.id}>
              <td>
                <InlineEdit value={component.name} onSave={(next) => void change(() => api.patch(`${base}/${component.id}`, { name: next }))} />
              </td>
              <td>
                <UserPicker value={component.leadId} variant="inline" onChange={(leadId) => void change(() => api.patch(`${base}/${component.id}`, { leadId }))} />
              </td>
              <td className="cell-actions">
                <Button size="sm" variant="ghost" icon="trash" aria-label={t('Delete')} onClick={() => void change(() => api.del(`${base}/${component.id}`))} />
              </td>
            </tr>
          ))}
          {project.components.length === 0 && (
            <tr>
              <td className="muted">{t('No components yet.')}</td>
            </tr>
          )}
        </tbody>
      </table>
      <form
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault();
          void change(() => api.post(base, { name })).then(() => setName(''));
        }}
      >
        <input className="input" placeholder={t('New component')} value={name} onChange={(event) => setName(event.target.value)} />
        <Button type="submit" icon="plus" disabled={!name.trim()}>
          {t('Add')}
        </Button>
      </form>
    </Card>
  );
}
