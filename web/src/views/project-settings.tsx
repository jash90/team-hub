import { useState, type FormEvent, type ReactNode } from 'react';

import { api } from '../api.ts';
import type { Project } from '../bindings/Project.ts';
import type { ProjectRole } from '../bindings/ProjectRole.ts';
import type { Status } from '../bindings/Status.ts';
import type { StatusCategory } from '../bindings/StatusCategory.ts';
import type { TypeLevel } from '../bindings/TypeLevel.ts';
import { TypeIcon, UserPicker, categoryLabel } from '../components/issue-bits.tsx';
import { Avatar, Button, ConfirmDialog, Dialog, Field, InlineEdit, Picker, Tabs, Tag, attempt, toast, useBusy } from '../components/ui.tsx';
import { t } from '../i18n/index.ts';
import { useModules } from '../modules.ts';
import { navigate } from '../router.tsx';
import { useProject, useProjectReload, useSession } from '../session.tsx';

function Card({ title, lead, children }: { title: string; lead?: string; children: ReactNode }) {
  return (
    <section className="panel settings-card">
      <header className="panel-head">
        <h2>{title}</h2>
      </header>
      {lead && <p className="muted settings-lead">{lead}</p>}
      {children}
    </section>
  );
}

/** Runs a configuration change and reloads the project so every view sees it. */
function useChange(): (work: () => Promise<unknown>, done?: string) => Promise<void> {
  const reload = useProjectReload();
  const { refresh } = useSession();

  return async (work, done) => {
    await attempt(work, done);
    await reload();
    await refresh();
  };
}

export function ProjectSettingsView({ tab }: { tab?: string }) {
  const project = useProject();
  const modules = useModules();
  const admin = project.role === 'admin';
  const moduleTabs = modules.flatMap((module) => module.projectSettings ?? []);
  const tabs = [
    { id: 'general', label: t('General') },
    { id: 'workflow', label: t('Workflow') },
    { id: 'types', label: t('Issue types') },
    { id: 'components', label: t('Components') },
    { id: 'members', label: t('Members') },
    ...moduleTabs.map((entry) => ({ id: entry.id, label: entry.label() })),
  ];
  const active = tabs.some((candidate) => candidate.id === tab) ? (tab as string) : 'general';
  const fromModule = moduleTabs.find((entry) => entry.id === active);

  return (
    <div className="view view-narrow">
      <header className="view-head">
        <div>
          <h1>{t('Project settings')}</h1>
          <p className="muted">
            {project.name} · {project.key}
          </p>
        </div>
      </header>
      {!admin && <p className="note">{t('Only project administrators can change these settings.')}</p>}
      <Tabs tabs={tabs} active={active} onChange={(id) => navigate(`/p/${project.key}/settings/${id}`)} />
      <fieldset className="settings-body" disabled={!admin && !fromModule}>
        {active === 'general' && <General project={project} />}
        {active === 'workflow' && <Workflow project={project} />}
        {active === 'types' && <Types project={project} />}
        {active === 'components' && <Components project={project} />}
        {active === 'members' && <Members project={project} />}
      </fieldset>
      {fromModule?.render(project)}
    </div>
  );
}

function General({ project }: { project: Project }) {
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

const CATEGORIES: StatusCategory[] = ['todo', 'in_progress', 'done'];

function Workflow({ project }: { project: Project }) {
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

function RemoveDialog({ what, options, onClose, onConfirm }: { what: string; options: { id: number; name: string }[]; onClose: () => void; onConfirm: (moveTo: number) => Promise<void> }) {
  const [moveTo, setMoveTo] = useState(options[0]?.id ?? 0);
  const [busy, run] = useBusy();

  return (
    <Dialog
      title={t('Delete “{name}”?', { name: what })}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button
            variant="danger"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await onConfirm(moveTo);
                onClose();
              })
            }
          >
            {t('Delete')}
          </Button>
        </>
      }
    >
      <div className="form">
        <Field label={t('Issues that have it now get')}>
          <Picker value={moveTo} options={options.map((option) => ({ value: option.id, label: option.name }))} onChange={setMoveTo} />
        </Field>
      </div>
    </Dialog>
  );
}

const LEVELS: { value: TypeLevel; label: string }[] = [
  { value: 'epic', label: 'Epic' },
  { value: 'standard', label: 'Issue' },
  { value: 'subtask', label: 'Subtask' },
];
const ICONS = ['story', 'task', 'bug', 'epic', 'subtask'];

function Types({ project }: { project: Project }) {
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

function Components({ project }: { project: Project }) {
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

const ROLES: { value: ProjectRole; label: string }[] = [
  { value: 'admin', label: 'Administrator' },
  { value: 'member', label: 'Member' },
  { value: 'viewer', label: 'Viewer' },
];

function Members({ project }: { project: Project }) {
  const { boot, userById } = useSession();
  const change = useChange();
  const [adding, setAdding] = useState<number | null>(null);
  const base = `/api/projects/${project.key}/members`;
  const outsiders = boot.users.filter((user) => user.active && !project.members.some((member) => member.userId === user.id));

  return (
    <Card
      title={t('Members')}
      lead={project.isPrivate ? t('Only these people can see this project.') : t('Everyone signed in can work in this project. Listing someone here changes what they may do: viewers only read, administrators configure.')}
    >
      <table className="table table-plain">
        <tbody>
          {project.members.map((member) => {
            const person = userById(member.userId);

            return (
              <tr key={member.userId}>
                <td>
                  <span className="with-avatar">
                    <Avatar user={person} size={24} />
                    {person?.name ?? '?'}
                    {project.leadId === member.userId && <Tag tone="purple">{t('lead')}</Tag>}
                  </span>
                </td>
                <td>
                  <Picker value={member.role} options={ROLES.map((role) => ({ value: role.value, label: t(role.label) }))} onChange={(role) => void change(() => api.put(`${base}/${member.userId}`, { role }))} />
                </td>
                <td className="cell-actions">
                  <Button size="sm" variant="ghost" icon="x" aria-label={t('Remove')} onClick={() => void change(() => api.put(`${base}/${member.userId}`, { role: null }))} />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {outsiders.length > 0 && (
        <div className="inline-form">
          <Picker value={adding} placeholder={t('Add a person…')} options={outsiders.map((user) => ({ value: user.id as number | null, label: user.name, keywords: user.email }))} onChange={setAdding} />
          <Button
            icon="plus"
            disabled={adding === null}
            onClick={() =>
              void change(() => api.put(`${base}/${adding}`, { role: 'member' })).then(() => {
                setAdding(null);
                toast(t('Added'));
              })
            }
          >
            {t('Add')}
          </Button>
        </div>
      )}
    </Card>
  );
}
