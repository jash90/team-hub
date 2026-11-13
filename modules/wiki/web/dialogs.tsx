import { useEffect, useMemo, useState, type FormEvent } from 'react';

import { api } from '@/api.ts';
import type { Project } from '@/bindings/Project.ts';
import type { QuickHit } from '@/bindings/QuickHit.ts';
import type { WikiPageDetail } from '@/bindings/WikiPageDetail.ts';
import type { WikiSpace } from '@/bindings/WikiSpace.ts';
import type { WikiTasksResult } from '@/bindings/WikiTasksResult.ts';
import type { WikiTemplate } from '@/bindings/WikiTemplate.ts';
import type { WikiTreeNode } from '@/bindings/WikiTreeNode.ts';
import { IssueKey } from '@/components/issue-bits.tsx';
import { Button, ConfirmDialog, Dialog, Field, Picker, Spinner, useBusy } from '@/components/ui.tsx';
import { t } from '@/i18n/index.ts';
import { navigate } from '@/router.tsx';
import { useRemote, useSession } from '@/session.tsx';

import { BASE, openTasks, pagePath, useProjects } from './lib.ts';

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

  return (words.length === 1 ? words[0].slice(0, 5) : words.map((word) => word[0]).join('')).replace(/^[0-9]+/, '').slice(0, 8);
}

function ProjectField({ value, onChange, disabled }: { value: string | null; onChange: (value: string | null) => void; disabled?: boolean }) {
  const { boot } = useSession();

  return (
    <Field label={t('Project')} hint={t('A space of a project is visible and editable to the people the project is; a space without one belongs to everybody.')}>
      <Picker
        value={value}
        disabled={disabled}
        options={[{ value: null as string | null, label: t('No project: for everybody') }, ...boot.projects.map((project) => ({ value: project.key as string | null, label: `${project.name} (${project.key})` }))]}
        onChange={onChange}
      />
    </Field>
  );
}

/** Creates a space; `forProject` starts it as the knowledge base of that project. */
export function NewSpaceDialog({ forProject, onClose }: { forProject?: { key: string; name: string } | null; onClose: () => void }) {
  const [form, setForm] = useState({ name: forProject ? forProject.name : '', key: forProject ? forProject.key : '', description: '', projectKey: forProject?.key ?? null });
  const [keyTouched, setKeyTouched] = useState(Boolean(forProject));
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      const space = await api.post<WikiSpace>(`${BASE}/spaces`, form);
      onClose();
      navigate(`/wiki/${space.key}`);
    });
  }

  return (
    <Dialog
      title={t('New space')}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="wiki-new-space" disabled={busy || !form.name.trim() || form.key.length < 2}>
            {t('Create space')}
          </Button>
        </>
      }
    >
      <form id="wiki-new-space" className="form" onSubmit={submit}>
        <Field label={t('Name')}>
          <input className="input" autoFocus required value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value, key: keyTouched ? form.key : suggestKey(event.target.value) })} />
        </Field>
        <Field label={t('Key')} hint={t('Short and unique, e.g. DOCS. It appears in addresses and in links like [[DOCS:Page title]].')}>
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
        <ProjectField value={form.projectKey} onChange={(projectKey) => setForm({ ...form, projectKey })} />
      </form>
    </Dialog>
  );
}

export function SpaceSettingsDialog({ space, onClose, onSaved }: { space: WikiSpace; onClose: () => void; onSaved: () => void }) {
  const [form, setForm] = useState({ name: space.name, description: space.description, projectKey: space.projectKey });
  const [deleting, setDeleting] = useState(false);
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      const patch = { name: form.name, description: form.description, ...(form.projectKey !== space.projectKey ? { projectKey: form.projectKey } : {}) };
      await api.patch<WikiSpace>(`${BASE}/spaces/${space.key}`, patch);
      onSaved();
      onClose();
    }, t('Saved'));
  }

  if (deleting) {
    return (
      <ConfirmDialog
        title={t('Delete the space {name}?', { name: space.name })}
        body={t('All {count} pages of the space, their history and their links to issues are deleted. This cannot be undone.', { count: space.pageCount })}
        confirm={t('Delete space')}
        danger
        onClose={() => setDeleting(false)}
        onConfirm={async () => {
          await api.del(`${BASE}/spaces/${space.key}`);
          navigate('/wiki');
        }}
      />
    );
  }

  return (
    <Dialog
      title={t('Space settings')}
      onClose={onClose}
      footer={
        <>
          {space.canAdmin && (
            <span className="dialog-foot-left">
              <Button variant="danger" onClick={() => setDeleting(true)}>
                {t('Delete space')}
              </Button>
            </span>
          )}
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="wiki-space-settings" disabled={busy || !form.name.trim()}>
            {t('Save')}
          </Button>
        </>
      }
    >
      <form id="wiki-space-settings" className="form" onSubmit={submit}>
        <Field label={t('Name')}>
          <input className="input" required value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} />
        </Field>
        <Field label={t('Description')}>
          <textarea className="input" rows={2} value={form.description} onChange={(event) => setForm({ ...form, description: event.target.value })} />
        </Field>
        <ProjectField value={form.projectKey} disabled={!space.canAdmin} onChange={(projectKey) => setForm({ ...form, projectKey })} />
      </form>
    </Dialog>
  );
}

/** Creates a page, optionally from a template, and opens it for editing. */
export function NewPageDialog({ spaceKey, parent, title, onClose }: { spaceKey: string; parent: WikiTreeNode | null; title?: string; onClose: () => void }) {
  const templates = useRemote(() => api.get<WikiTemplate[]>(`${BASE}/templates`), []);
  const [name, setName] = useState(title ?? '');
  const [templateId, setTemplateId] = useState<string | null>(null);
  const [busy, run] = useBusy();
  const chosen = templates.data?.find((template) => template.id === templateId);

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      const page = await api.post<WikiPageDetail>(`${BASE}/spaces/${spaceKey}/pages`, { title: name, parentId: parent?.id ?? null, ...(templateId ? { templateId } : {}) });
      onClose();
      navigate(pagePath(page.spaceKey, page.id, 'edit'));
    });
  }

  return (
    <Dialog
      title={parent ? t('New page under “{title}”', { title: parent.title }) : t('New page')}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="wiki-new-page" disabled={busy || !name.trim()}>
            {t('Create page')}
          </Button>
        </>
      }
    >
      <form id="wiki-new-page" className="form" onSubmit={submit}>
        <Field label={t('Title')}>
          <input className="input" autoFocus required maxLength={255} value={name} onChange={(event) => setName(event.target.value)} />
        </Field>
        <Field label={t('Start from')} hint={chosen?.description}>
          <Picker
            value={templateId}
            options={[{ value: null as string | null, label: t('An empty page') }, ...(templates.data ?? []).map((template) => ({ value: template.id as string | null, label: t(template.name) }))]}
            onChange={setTemplateId}
          />
        </Field>
      </form>
    </Dialog>
  );
}

function subtree(nodes: WikiTreeNode[], rootId: number): Set<number> {
  const inside = new Set([rootId]);

  // The list is in reading order, so a parent always comes before its children.
  for (const node of nodes) {
    if (node.parentId !== null && inside.has(node.parentId)) {
      inside.add(node.id);
    }
  }

  return inside;
}

function depthOf(nodes: WikiTreeNode[]): Map<number, number> {
  const depth = new Map<number, number>();

  for (const node of nodes) {
    depth.set(node.id, node.parentId !== null && depth.has(node.parentId) ? (depth.get(node.parentId) as number) + 1 : 0);
  }

  return depth;
}

/** Moves a page: under which page, and after which of its new siblings. */
export function MoveDialog({ page, nodes, onClose, onMoved }: { page: WikiTreeNode; nodes: WikiTreeNode[]; onClose: () => void; onMoved: () => void }) {
  const [parentId, setParentId] = useState<number | null>(page.parentId);
  const siblings = useMemo(() => nodes.filter((node) => node.parentId === parentId && node.id !== page.id), [nodes, parentId, page.id]);
  const before = nodes.filter((node) => node.parentId === page.parentId);
  const current = before.findIndex((node) => node.id === page.id);
  const [position, setPosition] = useState(Math.max(0, current));
  const [busy, run] = useBusy();
  const moved = useMemo(() => subtree(nodes, page.id), [nodes, page.id]);
  const depth = useMemo(() => depthOf(nodes), [nodes]);
  const place = Math.min(position, siblings.length);

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      await api.post(`${BASE}/pages/${page.id}/move`, { parentId, position: place });
      onMoved();
      onClose();
    }, t('Page moved'));
  }

  return (
    <Dialog
      title={t('Move “{title}”', { title: page.title })}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="wiki-move" disabled={busy}>
            {t('Move')}
          </Button>
        </>
      }
    >
      <form id="wiki-move" className="form" onSubmit={submit}>
        <Field label={t('Under')} hint={t('The pages under it move along.')}>
          <Picker
            value={parentId}
            options={[
              { value: null as number | null, label: t('The top level of the space') },
              ...nodes.filter((node) => !moved.has(node.id)).map((node) => ({ value: node.id as number | null, label: `${'– '.repeat(depth.get(node.id) ?? 0)}${node.title}` })),
            ]}
            onChange={(next) => {
              setParentId(next);
              setPosition(Number.MAX_SAFE_INTEGER);
            }}
          />
        </Field>
        <Field label={t('Position')}>
          <Picker
            value={place}
            options={[{ value: 0, label: t('First') }, ...siblings.map((sibling, index) => ({ value: index + 1, label: t('After “{title}”', { title: sibling.title }) }))]}
            onChange={setPosition}
          />
        </Field>
      </form>
    </Dialog>
  );
}

/** Turns chosen task lines of a page into issues. */
export function TasksDialog({ page, onClose, onDone }: { page: WikiPageDetail; onClose: () => void; onDone: (page: WikiPageDetail) => void }) {
  const { boot } = useSession();
  const projects = boot.projects.filter((project) => !project.archived && project.role !== 'viewer');
  const tasks = useMemo(
    () =>
      openTasks(
        page.body,
        boot.projects.map((project) => project.key),
      ),
    [page.body, boot.projects],
  );
  const [chosen, setChosen] = useState<number[]>(() => tasks.map((task) => task.index));
  const [projectKey, setProjectKey] = useState<string | null>(projects.find((project) => project.key === page.space.projectKey)?.key ?? projects[0]?.key ?? null);
  const [typeId, setTypeId] = useState<number | null>(null);
  const [created, setCreated] = useState<string[] | null>(null);
  const [busy, run] = useBusy();
  const loaded = useProjects(projectKey ? [projectKey] : []);
  const project: Project | undefined = projectKey ? loaded[projectKey] : undefined;

  useEffect(() => setTypeId(null), [projectKey]);

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      const result = await api.post<WikiTasksResult>(`${BASE}/pages/${page.id}/issues`, { projectKey, lines: chosen, baseVersion: page.version, ...(typeId !== null ? { typeId } : {}) });
      onDone(result.page);
      setCreated(result.keys);
    });
  }

  if (created) {
    return (
      <Dialog title={t('Issues created')} onClose={onClose} footer={<Button variant="primary" onClick={onClose}>{t('Done')}</Button>}>
        <div className="form">
          <p>{t('Each task now starts with the key of its issue, and the issues are linked to this page.')}</p>
          <div className="wiki-keys">
            {created.map((key) => (
              <IssueKey key={key} issue={{ key }} />
            ))}
          </div>
        </div>
      </Dialog>
    );
  }

  return (
    <Dialog
      title={t('Create issues from tasks')}
      onClose={onClose}
      wide
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="wiki-tasks" disabled={busy || chosen.length === 0 || !projectKey}>
            {busy ? t('Creating…') : t('Create {count} issues', { count: chosen.length })}
          </Button>
        </>
      }
    >
      <form id="wiki-tasks" className="form" onSubmit={submit}>
        <div className="form-row">
          <Field label={t('Project')}>
            <Picker value={projectKey} options={projects.map((candidate) => ({ value: candidate.key as string | null, label: `${candidate.name} (${candidate.key})` }))} onChange={setProjectKey} />
          </Field>
          <Field label={t('Type')}>
            {projectKey && !project ? (
              <Spinner />
            ) : (
              <Picker
                value={typeId}
                options={[
                  { value: null as number | null, label: t('Task (the default)') },
                  ...(project?.types ?? []).filter((type) => type.level === 'standard').map((type) => ({ value: type.id as number | null, label: type.name })),
                ]}
                onChange={setTypeId}
              />
            )}
          </Field>
        </div>
        <fieldset className="wiki-task-list">
          <legend className="field-label">{t('Tasks on this page')}</legend>
          {tasks.length === 0 && <p className="muted">{t('There are no open tasks without an issue on this page.')}</p>}
          {tasks.map((task) => (
            <label key={task.index} className="check">
              <input
                type="checkbox"
                checked={chosen.includes(task.index)}
                onChange={(event) => setChosen(event.target.checked ? [...chosen, task.index].sort((left, right) => left - right) : chosen.filter((index) => index !== task.index))}
              />
              <span>{task.text}</span>
            </label>
          ))}
        </fieldset>
      </form>
    </Dialog>
  );
}

/** Finds an issue by key or words and links it to the page. */
export function LinkIssueDialog({ page, onClose, onDone }: { page: WikiPageDetail; onClose: () => void; onDone: (page: WikiPageDetail) => void }) {
  const [text, setText] = useState('');
  const [hits, setHits] = useState<QuickHit[] | null>(null);
  const [busy, run] = useBusy();
  const linked = new Set(page.issues.filter((entry) => entry.relation === 'documents').map((entry) => entry.issue.key));

  useEffect(() => {
    if (!text.trim()) {
      setHits(null);

      return undefined;
    }

    const timer = window.setTimeout(() => {
      api
        .get<QuickHit[]>(`/api/quick?q=${encodeURIComponent(text)}`)
        .then((found) => setHits(found.filter((hit) => hit.kind === 'issue')))
        .catch(() => setHits([]));
    }, 150);

    return () => window.clearTimeout(timer);
  }, [text]);

  function link(key: string): void {
    void run(async () => {
      onDone(await api.put<WikiPageDetail>(`${BASE}/pages/${page.id}/issues/${key}`, {}));
      onClose();
    }, t('Issue linked'));
  }

  return (
    <Dialog title={t('Link an issue to this page')} onClose={onClose}>
      <div className="form">
        <Field label={t('Issue')} hint={t('Type a key such as WEB-12, or words from the summary.')}>
          <input className="input" autoFocus value={text} onChange={(event) => setText(event.target.value)} />
        </Field>
        <div className="wiki-results">
          {hits?.length === 0 && <p className="muted">{t('No issue matches.')}</p>}
          {hits?.map((hit) => (
            <button key={hit.id} type="button" className="menu-item wiki-result" disabled={busy || linked.has(hit.id)} onClick={() => link(hit.id)}>
              <span className="issue-key">{hit.id}</span>
              <span className="wiki-result-title">{hit.title}</span>
              {linked.has(hit.id) && <span className="muted">{t('already linked')}</span>}
            </button>
          ))}
        </div>
      </div>
    </Dialog>
  );
}
