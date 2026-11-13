import { useState, type FormEvent } from 'react';

import { api } from '../api.ts';
import type { Project } from '../bindings/Project.ts';
import { Avatar, Button, Dialog, Empty, Field, Picker, Tag, useBusy } from '../components/ui.tsx';
import { countOf, t } from '../i18n/index.ts';
import { Link, navigate } from '../router.tsx';
import { useSession } from '../session.tsx';

function suggestKey(name: string): string {
  const words = name
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/ł/gi, 'l')
    .toUpperCase()
    .split(/[^A-Z0-9]+/)
    .filter(Boolean);

  if (words.length === 0) {
    return '';
  }

  return (words.length === 1 ? words[0].slice(0, 4) : words.map((word) => word[0]).join('')).replace(/^[0-9]+/, '').slice(0, 6);
}

export function ProjectsView() {
  const { boot, userById } = useSession();
  const [creating, setCreating] = useState(false);

  return (
    <div className="view">
      <header className="view-head">
        <div>
          <h1>{t('Projects')}</h1>
        </div>
        <Button variant="primary" icon="plus" onClick={() => setCreating(true)}>
          {t('New project')}
        </Button>
      </header>

      {boot.projects.length === 0 ? (
        <Empty icon="folder" title={t('No projects yet')}>
          {t('A project holds issues, a board, sprints and releases.')}
        </Empty>
      ) : (
        <div className="project-grid">
          {boot.projects.map((project) => (
            <Link key={project.id} href={`/p/${project.key}/board`} className="project-card">
              <span className="project-card-head">
                <span className="project-badge">{project.key}</span>
                <strong>{project.name}</strong>
              </span>
              <span className="muted project-card-text">{project.description}</span>
              <span className="project-card-foot">
                <Tag>{project.boardMode === 'kanban' ? 'Kanban' : 'Scrum'}</Tag>
                {project.isPrivate && <Tag tone="amber">{t('private')}</Tag>}
                {project.archived && <Tag tone="red">{t('archived')}</Tag>}
                <span className="muted">{countOf(project.openCount, 'open issue', 'open issues')}</span>
                <span className="filter-spacer" />
                <Avatar user={userById(project.leadId)} size={22} />
              </span>
            </Link>
          ))}
        </div>
      )}

      {creating && <NewProjectDialog onClose={() => setCreating(false)} />}
    </div>
  );
}

function NewProjectDialog({ onClose }: { onClose: () => void }) {
  const { refresh } = useSession();
  const [form, setForm] = useState({ name: '', key: '', description: '', boardMode: 'scrum', isPrivate: false });
  const [keyTouched, setKeyTouched] = useState(false);
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      const project = await api.post<Project>('/api/projects', form);
      await refresh();
      onClose();
      navigate(`/p/${project.key}/${form.boardMode === 'scrum' ? 'backlog' : 'board'}`);
    });
  }

  return (
    <Dialog
      title={t('New project')}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="new-project" disabled={busy || !form.name.trim() || !form.key.trim()}>
            {t('Create project')}
          </Button>
        </>
      }
    >
      <form id="new-project" className="form" onSubmit={submit}>
        <Field label={t('Name')}>
          <input className="input" autoFocus required value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value, key: keyTouched ? form.key : suggestKey(event.target.value) })} />
        </Field>
        <Field label={t('Key')} hint={t('The prefix of issue keys, e.g. WEB-12. It cannot be changed later.')}>
          <input
            className="input input-mono"
            required
            maxLength={10}
            value={form.key}
            onChange={(event) => {
              setKeyTouched(true);
              setForm({ ...form, key: event.target.value.toUpperCase().replace(/[^A-Z0-9]/g, '') });
            }}
          />
        </Field>
        <Field label={t('Description')}>
          <textarea className="input" rows={2} value={form.description} onChange={(event) => setForm({ ...form, description: event.target.value })} />
        </Field>
        <Field label={t('How the team works')}>
          <Picker
            value={form.boardMode}
            onChange={(boardMode) => setForm({ ...form, boardMode })}
            options={[
              { value: 'scrum', label: t('Scrum: sprints, a backlog to plan them from') },
              { value: 'kanban', label: t('Kanban: a continuous flow, no sprints') },
            ]}
          />
        </Field>
        <label className="check">
          <input type="checkbox" checked={form.isPrivate} onChange={(event) => setForm({ ...form, isPrivate: event.target.checked })} />
          <span>{t('Private: only members can see it')}</span>
        </label>
      </form>
    </Dialog>
  );
}
