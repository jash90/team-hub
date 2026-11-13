import { useState, type FormEvent } from 'react';

import { api } from '../api.ts';
import type { IssueRow } from '../bindings/IssueRow.ts';
import type { Version } from '../bindings/Version.ts';
import { IssueLine, canEdit } from '../components/issue-bits.tsx';
import { Markdown } from '../components/markdown.tsx';
import { Button, ConfirmDialog, Dialog, Empty, Field, Menu, Picker, ProgressBar, Spinner, Tag, attempt, copyText, useBusy } from '../components/ui.tsx';
import { countOf, formatDate, t } from '../i18n/index.ts';
import { useProject, useProjectReload, useRemote, useStored } from '../session.tsx';

type Open = { kind: 'new' } | { kind: 'edit' | 'release' | 'notes' | 'delete'; version: Version };

export function ReleasesView() {
  const project = useProject();
  const reload = useProjectReload();
  const editable = canEdit(project);
  const [dialog, setDialog] = useState<Open | null>(null);
  const [showReleased, setShowReleased] = useStored(`releases-all:${project.key}`, true);
  const [expanded, setExpanded] = useState<number | null>(null);
  const versions = project.versions.filter((version) => !version.archived && (showReleased || !version.released));
  const today = new Date().toISOString().slice(0, 10);

  return (
    <div className="view">
      <header className="view-head">
        <div>
          <h1>{t('Releases')}</h1>
          <p className="muted">{t('Versions gather the issues that ship together.')}</p>
        </div>
        <div className="view-head-side">
          <label className="check">
            <input type="checkbox" checked={showReleased} onChange={(event) => setShowReleased(event.target.checked)} />
            <span>{t('Show released')}</span>
          </label>
          {editable && (
            <Button variant="primary" icon="plus" onClick={() => setDialog({ kind: 'new' })}>
              {t('New version')}
            </Button>
          )}
        </div>
      </header>

      {versions.length === 0 && <Empty icon="rocket" title={t('No versions yet')} />}

      {versions.map((version) => {
        const late = !version.released && version.releaseDate !== null && version.releaseDate < today;

        return (
          <section key={version.id} className="panel release">
            <header className="release-head">
              <button type="button" className="fold" onClick={() => setExpanded(expanded === version.id ? null : version.id)} aria-expanded={expanded === version.id}>
                <strong className="release-name">{version.name}</strong>
              </button>
              {version.released ? <Tag tone="green">{t('released')}</Tag> : late ? <Tag tone="red">{t('overdue')}</Tag> : <Tag tone="blue">{t('unreleased')}</Tag>}
              <span className="muted">{version.releaseDate ? formatDate(version.releaseDate) : t('no date')}</span>
              <span className="release-description muted">{version.description}</span>
              <span className="filter-spacer" />
              <span className="release-progress">
                <ProgressBar done={version.counts.done} inProgress={version.counts.inProgress} total={version.counts.total} />
                <span className="muted">
                  {version.counts.done}/{version.counts.total}
                </span>
              </span>
              <Button size="sm" onClick={() => setDialog({ kind: 'notes', version })}>
                {t('Release notes')}
              </Button>
              {editable && !version.released && (
                <Button size="sm" variant="primary" onClick={() => setDialog({ kind: 'release', version })}>
                  {t('Release')}
                </Button>
              )}
              {editable && (
                <Menu
                  label={t('Version actions')}
                  items={[
                    { label: t('Edit'), icon: 'edit', onSelect: () => setDialog({ kind: 'edit', version }) },
                    ...(version.released
                      ? [{ label: t('Mark as unreleased'), icon: 'refresh' as const, onSelect: () => void attempt(() => api.post(`/api/versions/${version.id}/release`, { released: false })).then(reload) }]
                      : []),
                    { label: t('Archive'), icon: 'folder', onSelect: () => void attempt(() => api.patch(`/api/versions/${version.id}`, { archived: true })).then(reload) },
                    { label: t('Delete'), icon: 'trash', danger: true, onSelect: () => setDialog({ kind: 'delete', version }) },
                  ]}
                />
              )}
            </header>
            {expanded === version.id && <VersionIssues version={version} />}
          </section>
        );
      })}

      {dialog && (dialog.kind === 'new' || dialog.kind === 'edit') && <VersionDialog version={dialog.kind === 'edit' ? dialog.version : null} onClose={() => setDialog(null)} />}
      {dialog?.kind === 'release' && <ReleaseDialog version={dialog.version} onClose={() => setDialog(null)} />}
      {dialog?.kind === 'notes' && <NotesDialog version={dialog.version} onClose={() => setDialog(null)} />}
      {dialog?.kind === 'delete' && (
        <ConfirmDialog
          title={t('Delete {name}?', { name: dialog.version.name })}
          body={t('Issues keep everything except their fix version.')}
          confirm={t('Delete')}
          danger
          onClose={() => setDialog(null)}
          onConfirm={async () => {
            await api.del(`/api/versions/${dialog.version.id}`);
            await reload();
          }}
        />
      )}
    </div>
  );
}

function VersionIssues({ version }: { version: Version }) {
  const project = useProject();
  const issues = useRemote(
    () => api.get<{ issues: IssueRow[] }>(`/api/search?limit=200&q=${encodeURIComponent(`project = ${project.key} AND version = "${version.name}" ORDER BY status, rank`)}`).then((result) => result.issues),
    [version.id],
    (event) => event.projectId === project.id && event.kind.startsWith('issue.'),
  );

  if (!issues.data) {
    return <Spinner />;
  }

  return <div className="release-issues">{issues.data.length === 0 ? <p className="muted panel-empty">{t('No issues have this fix version yet.')}</p> : issues.data.map((issue) => <IssueLine key={issue.id} issue={issue} project={project} />)}</div>;
}

function VersionDialog({ version, onClose }: { version: Version | null; onClose: () => void }) {
  const project = useProject();
  const reload = useProjectReload();
  const [form, setForm] = useState({ name: version?.name ?? '', description: version?.description ?? '', startDate: version?.startDate ?? '', releaseDate: version?.releaseDate ?? '' });
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      if (version) {
        await api.patch(`/api/versions/${version.id}`, form);
      } else {
        await api.post(`/api/projects/${project.key}/versions`, form);
      }

      await reload();
      onClose();
    });
  }

  return (
    <Dialog
      title={version ? t('Edit version') : t('New version')}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="version-form" disabled={busy || !form.name.trim()}>
            {t('Save')}
          </Button>
        </>
      }
    >
      <form id="version-form" className="form" onSubmit={submit}>
        <Field label={t('Name')}>
          <input className="input" autoFocus required value={form.name} placeholder="1.2.0" onChange={(event) => setForm({ ...form, name: event.target.value })} />
        </Field>
        <Field label={t('Description')}>
          <input className="input" value={form.description} onChange={(event) => setForm({ ...form, description: event.target.value })} />
        </Field>
        <div className="form-row">
          <Field label={t('Starts')}>
            <input className="input" type="date" value={form.startDate} onChange={(event) => setForm({ ...form, startDate: event.target.value })} />
          </Field>
          <Field label={t('Release date')}>
            <input className="input" type="date" value={form.releaseDate} onChange={(event) => setForm({ ...form, releaseDate: event.target.value })} />
          </Field>
        </div>
      </form>
    </Dialog>
  );
}

function ReleaseDialog({ version, onClose }: { version: Version; onClose: () => void }) {
  const project = useProject();
  const reload = useProjectReload();
  const others = project.versions.filter((candidate) => !candidate.released && !candidate.archived && candidate.id !== version.id);
  const open = version.counts.total - version.counts.done;
  const [moveTo, setMoveTo] = useState<number | null>(open > 0 ? (others[0]?.id ?? null) : null);
  const [busy, run] = useBusy();

  return (
    <Dialog
      title={t('Release {name}', { name: version.name })}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button
            variant="primary"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await api.post(`/api/versions/${version.id}/release`, { released: true, moveTo });
                await reload();
                onClose();
              }, t('Version released'))
            }
          >
            {t('Release')}
          </Button>
        </>
      }
    >
      <div className="form">
        <p>
          <strong>{countOf(version.counts.done, 'issue', 'issues')}</strong> {t('finished')}, <strong>{countOf(open, 'issue', 'issues')}</strong> {t('not finished')}.
        </p>
        {open > 0 && (
          <Field label={t('Unfinished issues')}>
            <Picker value={moveTo} options={[...others.map((candidate) => ({ value: candidate.id as number | null, label: t('Move to {name}', { name: candidate.name }) })), { value: null, label: t('Leave them in this version') }]} onChange={setMoveTo} />
          </Field>
        )}
      </div>
    </Dialog>
  );
}

function NotesDialog({ version, onClose }: { version: Version; onClose: () => void }) {
  const notes = useRemote(() => api.get<{ markdown: string }>(`/api/versions/${version.id}/notes`), [version.id]);

  return (
    <Dialog
      title={t('Release notes')}
      onClose={onClose}
      wide
      footer={
        <>
          <Button icon="copy" disabled={!notes.data} onClick={() => void copyText(notes.data?.markdown ?? '')}>
            {t('Copy Markdown')}
          </Button>
          <Button variant="primary" onClick={onClose}>
            {t('Close')}
          </Button>
        </>
      }
    >
      {notes.data ? <Markdown text={notes.data.markdown} /> : <Spinner />}
    </Dialog>
  );
}
