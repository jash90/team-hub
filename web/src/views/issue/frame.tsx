import { useRef, useState } from 'react';

import { api } from '../../api.ts';
import type { Issue } from '../../bindings/Issue.ts';
import type { Project } from '../../bindings/Project.ts';
import { Icon } from '../../components/icon.tsx';
import { IssueKey, IssueLine, TypeIcon, canEdit } from '../../components/issue-bits.tsx';
import { Markdown, MarkdownEditor } from '../../components/markdown.tsx';
import { Button, ConfirmDialog, Empty, ErrorNote, InlineEdit, Menu, ProgressBar, Spinner, attempt, copyText } from '../../components/ui.tsx';
import { t, timeAgo } from '../../i18n/index.ts';
import { useModules } from '../../modules.ts';
import { Link, navigate, setParam } from '../../router.tsx';
import { useProjectData, useRemote, useSession, type Remote } from '../../session.tsx';
import { CreateIssueDialog } from '../lazy.ts';
import { Activity } from './activity.tsx';
import { LinkDialog, WorkDialog } from './dialogs.tsx';
import type { Patch } from './patch.ts';
import { IssueSide } from './side.tsx';

export function IssueView({ issueKey, mode }: { issueKey: string; mode: 'page' | 'drawer' }) {
  const projectKey = issueKey.split('-')[0];
  const project = useProjectData(projectKey);
  const issue: Remote<Issue> = useRemote(
    () => api.get<Issue>(`/api/issues/${issueKey}`),
    [issueKey],
    (event) => issue.data !== undefined && (event.issueId === issue.data.id || (event.kind === 'issue.deleted' && event.payload.key === issueKey)),
  );

  if ((issue.error && !issue.data) || (project.error && !project.data)) {
    return (
      <div className="issue-frame">
        <ErrorNote message={issue.error ?? project.error ?? ''} />
        {mode === 'drawer' && <Button onClick={() => setParam('issue', null)}>{t('Close')}</Button>}
      </div>
    );
  }

  if (!issue.data || !project.data) {
    return (
      <div className="issue-frame">
        <Spinner />
      </div>
    );
  }

  return <IssueBody issue={issue.data} project={project.data} mode={mode} reload={() => void issue.reload()} />;
}

function IssueBody({ issue, project, mode, reload }: { issue: Issue; project: Project; mode: 'page' | 'drawer'; reload: () => void }) {
  const { user, userById } = useSession();
  const modules = useModules(project.key);
  const editable = canEdit(project);
  const type = project.types.find((candidate) => candidate.id === issue.typeId);
  const watching = issue.watchers.includes(user.id);
  const [dialog, setDialog] = useState<'link' | 'work' | 'delete' | 'child' | null>(null);
  const [editingDescription, setEditingDescription] = useState(false);
  const [description, setDescription] = useState(issue.description);
  const fileInput = useRef<HTMLInputElement>(null);

  async function update(patch: Patch): Promise<void> {
    await attempt(() => api.patch(`/api/issues/${issue.key}`, patch));
    reload();
  }

  async function upload(files: FileList | null): Promise<void> {
    for (const file of Array.from(files ?? [])) {
      await attempt(() => api.upload(`/api/issues/${issue.key}/attachments`, file));
    }

    reload();
  }

  const childLevel = type?.level === 'epic' ? 'standard' : 'subtask';
  const childType = project.types.find((candidate) => candidate.level === childLevel && (childLevel === 'subtask' || candidate.name === 'Story')) ?? project.types.find((candidate) => candidate.level === childLevel);

  return (
    <div className={`issue-frame issue-${mode}`}>
      <header className="issue-head">
        <div className="crumbs">
          <Link href={`/p/${project.key}/board`}>{project.name}</Link>
          {issue.parentKey && (
            <>
              <Icon name="chevronRight" size={12} />
              <IssueKey issue={{ key: issue.parentKey }} />
            </>
          )}
          <Icon name="chevronRight" size={12} />
          <span className="crumb-current">
            <TypeIcon type={type} />
            {issue.key}
          </span>
        </div>
        <div className="issue-head-actions">
          <Button variant="ghost" size="sm" icon="eye" className={watching ? 'is-on' : ''} onClick={() => void attempt(() => api.put(`/api/issues/${issue.key}/watch`, { watching: !watching })).then(reload)} title={t('People watching get notified about changes')}>
            {watching ? t('Watching') : t('Watch')} · {issue.watchers.length}
          </Button>
          <Button variant="ghost" size="sm" icon="link" onClick={() => void copyText(`${window.location.origin}/issue/${issue.key}`)} aria-label={t('Copy link')} title={t('Copy link')} />
          <Menu
            label={t('More')}
            items={[
              ...(mode === 'drawer' ? [{ label: t('Open full page'), icon: 'external' as const, onSelect: () => navigate(`/issue/${issue.key}`) }] : []),
              { label: t('Link issue'), icon: 'link', onSelect: () => setDialog('link'), disabled: !editable },
              { label: t('Log work'), icon: 'clock', onSelect: () => setDialog('work'), disabled: !editable },
              { label: t('Attach file'), icon: 'paperclip', onSelect: () => fileInput.current?.click(), disabled: !editable },
              { label: t('Delete issue'), icon: 'trash', danger: true, onSelect: () => setDialog('delete'), disabled: !editable },
            ]}
          />
          {mode === 'drawer' && <Button variant="ghost" icon="x" onClick={() => setParam('issue', null)} aria-label={t('Close')} />}
        </div>
      </header>

      <div className="issue-columns">
        <div className="issue-main">
          {editable ? <InlineEdit className="issue-title" value={issue.summary} onSave={(summary) => void update({ summary })} multiline /> : <h1 className="issue-title">{issue.summary}</h1>}

          <section className="issue-section">
            <div className="section-head">
              <h3>{t('Description')}</h3>
              {editable && !editingDescription && (
                <Button
                  size="sm"
                  variant="ghost"
                  icon="edit"
                  onClick={() => {
                    setDescription(issue.description);
                    setEditingDescription(true);
                  }}
                >
                  {t('Edit')}
                </Button>
              )}
            </div>
            {editingDescription ? (
              <>
                <MarkdownEditor value={description} onChange={setDescription} rows={10} autoFocus onSubmit={() => void update({ description }).then(() => setEditingDescription(false))} />
                <div className="row-end">
                  <Button onClick={() => setEditingDescription(false)}>{t('Cancel')}</Button>
                  <Button variant="primary" onClick={() => void update({ description }).then(() => setEditingDescription(false))}>
                    {t('Save')}
                  </Button>
                </div>
              </>
            ) : issue.description.trim() ? (
              <Markdown text={issue.description} />
            ) : (
              <p className="muted">{t('No description yet.')}</p>
            )}
          </section>

          {(type?.level !== 'subtask' || issue.children.length > 0) && (
            <section className="issue-section">
              <div className="section-head">
                <h3>{type?.level === 'epic' ? t('Issues in this epic') : t('Subtasks')}</h3>
                {issue.children.length > 0 && (
                  <span className="section-progress">
                    <ProgressBar done={issue.subtasks.done} total={issue.subtasks.total} />
                    <span className="muted">
                      {issue.subtasks.done}/{issue.subtasks.total}
                    </span>
                  </span>
                )}
                {editable && childType && (
                  <Button size="sm" variant="ghost" icon="plus" onClick={() => setDialog('child')}>
                    {t('Add')}
                  </Button>
                )}
              </div>
              {issue.children.map((child) => (
                <IssueLine key={child.id} issue={child} project={project} compact />
              ))}
              {issue.children.length === 0 && <p className="muted">{type?.level === 'epic' ? t('No issues in this epic yet.') : t('No subtasks.')}</p>}
            </section>
          )}

          {issue.links.length > 0 && (
            <section className="issue-section">
              <div className="section-head">
                <h3>{t('Linked issues')}</h3>
              </div>
              {issue.links.map((link) => (
                <IssueLine
                  key={link.id}
                  compact
                  issue={link.issue}
                  project={link.issue.projectId === project.id ? project : undefined}
                  trailing={
                    <>
                      <span className="link-kind">{linkLabel(link.type, link.direction)}</span>
                      {editable && <Button size="sm" variant="ghost" icon="x" aria-label={t('Remove link')} onClick={() => void attempt(() => api.del(`/api/links/${link.id}`)).then(reload)} />}
                    </>
                  }
                />
              ))}
            </section>
          )}

          {issue.attachments.length > 0 && (
            <section className="issue-section">
              <div className="section-head">
                <h3>{t('Attachments')}</h3>
              </div>
              <div className="attachments">
                {issue.attachments.map((attachment) => (
                  <div key={attachment.id} className="attachment">
                    <Icon name="paperclip" />
                    <a href={`/api/attachments/${attachment.id}`} target="_blank" rel="noreferrer">
                      {attachment.filename}
                    </a>
                    <span className="muted">
                      {Math.max(1, Math.round(attachment.size / 1024))} kB · {userById(attachment.uploaderId)?.name ?? ''} · {timeAgo(attachment.createdAt)}
                    </span>
                    {editable && <Button size="sm" variant="ghost" icon="trash" aria-label={t('Delete')} onClick={() => void attempt(() => api.del(`/api/attachments/${attachment.id}`)).then(reload)} />}
                  </div>
                ))}
              </div>
            </section>
          )}

          {modules.map((module) => {
            const Panel = module.issuePanel;

            return Panel && issue.panels[module.id] !== undefined ? <Panel key={module.id} issue={issue} reload={reload} /> : null;
          })}

          <Activity issue={issue} editable={editable} reload={reload} />
        </div>

        <IssueSide issue={issue} project={project} editable={editable} update={update} onLogWork={() => setDialog('work')} />
      </div>

      <input ref={fileInput} type="file" multiple hidden onChange={(event) => void upload(event.target.files).then(() => (event.target.value = ''))} />
      {dialog === 'link' && <LinkDialog issue={issue} onClose={() => setDialog(null)} onDone={reload} />}
      {dialog === 'work' && <WorkDialog issue={issue} onClose={() => setDialog(null)} onDone={reload} />}
      {dialog === 'child' && childType && (
        <CreateIssueDialog projectKey={project.key} defaults={{ parentKey: issue.key, typeId: childType.id, sprintId: childLevel === 'subtask' ? issue.sprintId : null }} onClose={() => setDialog(null)} />
      )}
      {dialog === 'delete' && (
        <ConfirmDialog
          title={t('Delete {key}?', { key: issue.key })}
          body={t('The issue, its comments, history and attachments are removed for good. Subtasks stay, without a parent.')}
          confirm={t('Delete issue')}
          danger
          onClose={() => setDialog(null)}
          onConfirm={async () => {
            await api.del(`/api/issues/${issue.key}`);

            if (mode === 'drawer') {
              setParam('issue', null);
            } else {
              navigate(`/p/${project.key}/board`);
            }
          }}
        />
      )}
    </div>
  );
}

function linkLabel(type: string, direction: string): string {
  if (type === 'blocks') {
    return direction === 'outward' ? t('blocks') : t('is blocked by');
  }

  if (type === 'duplicates') {
    return direction === 'outward' ? t('duplicates') : t('is duplicated by');
  }

  return t('relates to');
}

export function IssueListEmpty({ title }: { title: string }) {
  return <Empty icon="check" title={title} />;
}
