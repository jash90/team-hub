import { useState, type FormEvent } from 'react';

import { api } from '../../api.ts';
import type { Project } from '../../bindings/Project.ts';
import { UserPicker } from '../../components/issue-bits.tsx';
import { Button, ConfirmDialog, Field, Picker } from '../../components/ui.tsx';
import { t } from '../../i18n/index.ts';
import { Card, useChange } from './parts.tsx';

export function General({ project }: { project: Project }) {
  const change = useChange();
  const [form, setForm] = useState({ name: project.name, description: project.description });
  const [archiving, setArchiving] = useState(false);

  function submit(event: FormEvent): void {
    event.preventDefault();
    void change(() => api.patch(`/api/projects/${project.key}`, form), t('Saved'));
  }

  return (
    <>
      <Card title={t('Details')}>
        <form className="form" onSubmit={submit}>
          <Field label={t('Name')}>
            <input className="input" required value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} />
          </Field>
          <Field label={t('Description')}>
            <textarea className="input" rows={2} value={form.description} onChange={(event) => setForm({ ...form, description: event.target.value })} />
          </Field>
          <div className="form-row">
            <Field label={t('Project lead')}>
              <UserPicker value={project.leadId} allowNone={false} onChange={(leadId) => void change(() => api.patch(`/api/projects/${project.key}`, { leadId }))} />
            </Field>
            <Field label={t('How the team works')}>
              <Picker
                value={project.boardMode}
                onChange={(boardMode) => void change(() => api.patch(`/api/projects/${project.key}`, { boardMode }))}
                options={[
                  { value: 'scrum', label: 'Scrum' },
                  { value: 'kanban', label: 'Kanban' },
                ]}
              />
            </Field>
          </div>
          <label className="check">
            <input type="checkbox" checked={project.isPrivate} onChange={(event) => void change(() => api.patch(`/api/projects/${project.key}`, { isPrivate: event.target.checked }))} />
            <span>{t('Private: only members can see it')}</span>
          </label>
          <div>
            <Button variant="primary" type="submit">
              {t('Save')}
            </Button>
          </div>
        </form>
      </Card>
      <Card title={t('Archive')} lead={t('An archived project is read as before but disappears from pickers and “My work”.')}>
        <Button variant={project.archived ? 'default' : 'danger'} onClick={() => (project.archived ? void change(() => api.patch(`/api/projects/${project.key}`, { archived: false })) : setArchiving(true))}>
          {project.archived ? t('Restore project') : t('Archive project')}
        </Button>
      </Card>
      {archiving && (
        <ConfirmDialog
          title={t('Archive {name}?', { name: project.name })}
          body={t('Nothing is deleted; the project can be restored here at any time.')}
          confirm={t('Archive project')}
          danger
          onClose={() => setArchiving(false)}
          onConfirm={() => change(() => api.patch(`/api/projects/${project.key}`, { archived: true }))}
        />
      )}
    </>
  );
}
