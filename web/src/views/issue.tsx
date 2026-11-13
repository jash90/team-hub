import { useRef, useState, type ReactNode } from 'react';

import { api } from '../api.ts';
import type { Change } from '../bindings/Change.ts';
import type { Comment } from '../bindings/Comment.ts';
import type { Issue } from '../bindings/Issue.ts';
import type { IssuePatch } from '../bindings/IssuePatch.ts';
import type { IssueRow } from '../bindings/IssueRow.ts';
import type { Project } from '../bindings/Project.ts';
import type { QuickHit } from '../bindings/QuickHit.ts';
import { Icon } from '../components/icon.tsx';
import {
  IssueKey,
  IssueLine,
  PriorityPicker,
  SprintPicker,
  StatusPicker,
  TypeIcon,
  TypePicker,
  UserPicker,
  VersionPicker,
  canEdit,
} from '../components/issue-bits.tsx';
import { Markdown, MarkdownEditor } from '../components/markdown.tsx';
import { Avatar, Button, ConfirmDialog, Dialog, Empty, ErrorNote, Field, InlineEdit, Menu, Picker, Popover, ProgressBar, Spinner, Tabs, attempt, copyText, useBusy } from '../components/ui.tsx';
import { formatDate, formatDateTime, formatMinutes, t, timeAgo } from '../i18n/index.ts';
import { useModules } from '../modules.ts';
import { Link, navigate, setParam } from '../router.tsx';
import { useProjectData, useRemote, useSession, useStored, type Remote } from '../session.tsx';
import { CreateIssueDialog } from './create-issue.tsx';

type Patch = Partial<IssuePatch>;

const FIELD_NAMES: Record<string, string> = {
  status: 'status',
  assignee: 'assignee',
  reporter: 'reporter',
  summary: 'summary',
  description: 'description',
  priority: 'priority',
  type: 'type',
  sprint: 'sprint',
  version: 'fix version',
  points: 'story points',
  due: 'due date',
  estimate: 'estimate',
  labels: 'labels',
  components: 'components',
  parent: 'parent',
  link: 'link',
  attachment: 'attachment',
};

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
  const modules = useModules();
  const editable = canEdit(project);
  const type = project.types.find((candidate) => candidate.id === issue.typeId);
  const status = project.statuses.find((candidate) => candidate.id === issue.statusId);
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

  const allowed = project.strictWorkflow
    ? project.transitions.filter((transition) => transition.fromStatusId === null || transition.fromStatusId === issue.statusId).map((transition) => transition.toStatusId)
    : undefined;
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

        <aside className="issue-side">
          <div className="side-status">{editable ? <StatusPicker project={project} value={issue.statusId} onChange={(statusId) => void update({ statusId })} allowed={allowed} /> : <span className={`status status-${status?.category}`}>{status?.name}</span>}</div>
          <dl className="fields">
            <SideField label={t('Assignee')}>
              <UserPicker value={issue.assigneeId} onChange={(assigneeId) => void update({ assigneeId })} variant="inline" disabled={!editable} />
              {editable && issue.assigneeId !== user.id && (
                <button type="button" className="link-button" onClick={() => void update({ assigneeId: user.id })}>
                  {t('Assign to me')}
                </button>
              )}
            </SideField>
            <SideField label={t('Reporter')}>
              <UserPicker value={issue.reporterId} onChange={(reporterId) => void update({ reporterId })} variant="inline" disabled={!editable} />
            </SideField>
            <SideField label={t('Priority')}>
              <PriorityPicker value={issue.priority} onChange={(priority) => void update({ priority })} variant="inline" disabled={!editable} />
            </SideField>
            <SideField label={t('Type')}>
              <TypePicker project={project} value={issue.typeId} onChange={(typeId) => void update({ typeId })} variant="inline" disabled={!editable} levels={type ? [type.level] : undefined} />
            </SideField>
            {type?.level === 'standard' && <EpicField issue={issue} project={project} editable={editable} onChange={(parentKey) => void update({ parentKey })} />}
            {project.boardMode === 'scrum' && type?.level === 'standard' && (
              <SideField label={t('Sprint')}>
                <SprintPicker project={project} value={issue.sprintId} onChange={(sprintId) => void update({ sprintId })} variant="inline" disabled={!editable} />
              </SideField>
            )}
            {type?.level !== 'subtask' && (
              <SideField label={t('Fix version')}>
                <VersionPicker project={project} value={issue.versionId} onChange={(versionId) => void update({ versionId })} variant="inline" disabled={!editable} />
              </SideField>
            )}
            {type?.level !== 'epic' && (
              <SideField label={t('Story points')}>
                <NumberField value={issue.points} editable={editable} onSave={(points) => void update({ points })} />
              </SideField>
            )}
            <SideField label={t('Labels')}>
              <LabelsField labels={issue.labels} known={project.labels} editable={editable} onSave={(labels) => void update({ labels })} />
            </SideField>
            {project.components.length > 0 && (
              <SideField label={t('Components')}>
                <ComponentsField issue={issue} project={project} editable={editable} onSave={(componentIds) => void update({ componentIds })} />
              </SideField>
            )}
            <SideField label={t('Due date')}>
              <input
                type="date"
                className="input input-inline"
                value={issue.dueDate ?? ''}
                disabled={!editable}
                onChange={(event) => void update({ dueDate: event.target.value || null })}
              />
            </SideField>
            <SideField label={t('Time')}>
              <button type="button" className="link-button" disabled={!editable} onClick={() => setDialog('work')}>
                {issue.spentMinutes > 0 ? t('{spent} logged', { spent: formatMinutes(issue.spentMinutes) }) : t('Log work')}
              </button>
              {issue.estimateMinutes !== null && <span className="muted"> · {t('estimate {estimate}', { estimate: formatMinutes(issue.estimateMinutes) })}</span>}
            </SideField>
          </dl>
          <p className="issue-dates muted">
            {t('Created {when}', { when: formatDateTime(issue.createdAt) })}
            <br />
            {t('Updated {when}', { when: timeAgo(issue.updatedAt) })}
            {issue.resolvedAt && (
              <>
                <br />
                {t('Resolved {when}', { when: formatDateTime(issue.resolvedAt) })}
              </>
            )}
          </p>
        </aside>
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

function SideField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </>
  );
}

function NumberField({ value, editable, onSave }: { value: number | null; editable: boolean; onSave: (value: number | null) => void }) {
  const [draft, setDraft] = useState<string | null>(null);

  function commit(): void {
    if (draft === null) {
      return;
    }

    const parsed = draft.trim() === '' ? null : Number(draft.replace(',', '.'));
    setDraft(null);

    if (parsed !== value && (parsed === null || Number.isFinite(parsed))) {
      onSave(parsed);
    }
  }

  return (
    <input
      className="input input-inline input-number"
      inputMode="decimal"
      disabled={!editable}
      placeholder="–"
      value={draft ?? (value === null ? '' : String(value))}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={commit}
      onKeyDown={(event) => event.key === 'Enter' && event.currentTarget.blur()}
    />
  );
}

function LabelsField({ labels, known, editable, onSave }: { labels: string[]; known: string[]; editable: boolean; onSave: (labels: string[]) => void }) {
  const [draft, setDraft] = useState('');

  function add(): void {
    const label = draft.trim().replace(/\s+/g, '-');
    setDraft('');

    if (label && !labels.some((existing) => existing.toLowerCase() === label.toLowerCase())) {
      onSave([...labels, label]);
    }
  }

  return (
    <div className="chips">
      {labels.map((label) => (
        <span key={label} className="label">
          {label}
          {editable && (
            <button type="button" aria-label={t('Remove')} onClick={() => onSave(labels.filter((existing) => existing !== label))}>
              ×
            </button>
          )}
        </span>
      ))}
      {editable && (
        <>
          <input
            className="input input-inline chips-input"
            list="known-labels"
            placeholder={labels.length === 0 ? t('Add label') : '+'}
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ' ' || event.key === ',') {
                event.preventDefault();
                add();
              }
            }}
            onBlur={add}
          />
          <datalist id="known-labels">
            {known
              .filter((label) => !labels.includes(label))
              .map((label) => (
                <option key={label} value={label} />
              ))}
          </datalist>
        </>
      )}
      {!editable && labels.length === 0 && <span className="muted">–</span>}
    </div>
  );
}

function ComponentsField({ issue, project, editable, onSave }: { issue: Issue; project: Project; editable: boolean; onSave: (ids: number[]) => void }) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const chosen = project.components.filter((component) => issue.componentIds.includes(component.id));

  return (
    <>
      <button type="button" className="picker picker-inline" disabled={!editable} onClick={(event) => setAnchor(anchor ? null : event.currentTarget)}>
        {chosen.length > 0 ? chosen.map((component) => component.name).join(', ') : <span className="muted">{t('None')}</span>}
      </button>
      {anchor && (
        <Popover anchor={anchor} onClose={() => setAnchor(null)}>
          <div className="menu">
            {project.components.map((component) => {
              const on = issue.componentIds.includes(component.id);

              return (
                <label key={component.id} className="menu-item check">
                  <input type="checkbox" checked={on} onChange={() => onSave(on ? issue.componentIds.filter((id) => id !== component.id) : [...issue.componentIds, component.id])} />
                  <span>{component.name}</span>
                </label>
              );
            })}
          </div>
        </Popover>
      )}
    </>
  );
}

function EpicField({ issue, project, editable, onChange }: { issue: Issue; project: Project; editable: boolean; onChange: (parentKey: string | null) => void }) {
  const epics = useRemote(
    () => api.get<{ issues: IssueRow[] }>(`/api/search?limit=200&q=${encodeURIComponent(`project = ${project.key} AND type = Epic ORDER BY key`)}`).then((found) => found.issues),
    [project.key],
  );
  const options = [
    { value: null as string | null, label: t('None') },
    ...(epics.data ?? []).map((epic) => ({ value: epic.key as string | null, label: `${epic.key} ${epic.summary}` })),
  ];

  return (
    <SideField label={t('Epic')}>
      <Picker value={issue.parentKey} options={options} onChange={onChange} variant="inline" disabled={!editable} display={issue.parentKey ? <span className="epic-chip">{issue.parentSummary}</span> : undefined} placeholder={t('None')} />
    </SideField>
  );
}

function Activity({ issue, editable, reload }: { issue: Issue; editable: boolean; reload: () => void }) {
  const { user, userById } = useSession();
  const [tab, setTab] = useStored('issue-activity', 'comments');
  const [draft, setDraft] = useState('');
  const [editing, setEditing] = useState<{ id: number; body: string } | null>(null);
  const [busy, run] = useBusy();

  function send(): void {
    if (!draft.trim()) {
      return;
    }

    void run(async () => {
      await api.post(`/api/issues/${issue.key}/comments`, { body: draft });
      setDraft('');
      reload();
    });
  }

  const author = (comment: Comment | Change, id: number | null, label: string | null) => {
    const person = userById(id);

    return (
      <>
        <Avatar user={person} size={26} label={person?.name ?? label ?? undefined} />
        <strong>{person?.name ?? label ?? t('Someone')}</strong>
        {person && label && <span className="tag tag-neutral">{label}</span>}
        <span className="muted" title={formatDateTime(comment.createdAt)}>
          {timeAgo(comment.createdAt)}
        </span>
      </>
    );
  };

  return (
    <section className="issue-section">
      <div className="section-head">
        <h3>{t('Activity')}</h3>
      </div>
      <Tabs
        tabs={[
          { id: 'comments', label: t('Comments'), count: issue.comments.length },
          { id: 'history', label: t('History'), count: issue.changes.length },
          { id: 'work', label: t('Work log'), count: issue.worklogs.length },
        ]}
        active={tab}
        onChange={setTab}
      />

      {tab === 'comments' && (
        <div className="comments">
          {issue.comments.map((comment) => (
            <article key={comment.id} className="comment">
              <header>
                {author(comment, comment.authorId, comment.authorLabel)}
                {comment.updatedAt !== comment.createdAt && <span className="muted">· {t('edited')}</span>}
                {comment.authorId === user.id && editing?.id !== comment.id && (
                  <span className="comment-actions">
                    <Button size="sm" variant="ghost" icon="edit" aria-label={t('Edit')} onClick={() => setEditing({ id: comment.id, body: comment.body })} />
                    <Button size="sm" variant="ghost" icon="trash" aria-label={t('Delete')} onClick={() => void attempt(() => api.del(`/api/comments/${comment.id}`)).then(reload)} />
                  </span>
                )}
              </header>
              {editing?.id === comment.id ? (
                <>
                  <MarkdownEditor value={editing.body} onChange={(body) => setEditing({ id: comment.id, body })} rows={4} autoFocus />
                  <div className="row-end">
                    <Button onClick={() => setEditing(null)}>{t('Cancel')}</Button>
                    <Button
                      variant="primary"
                      onClick={() =>
                        void attempt(() => api.patch(`/api/comments/${comment.id}`, { body: editing.body })).then(() => {
                          setEditing(null);
                          reload();
                        })
                      }
                    >
                      {t('Save')}
                    </Button>
                  </div>
                </>
              ) : (
                <Markdown text={comment.body} />
              )}
            </article>
          ))}
          {editable && (
            <div className="composer">
              <MarkdownEditor value={draft} onChange={setDraft} rows={3} placeholder={t('Add a comment… @name mentions someone, WEB-12 links an issue')} onSubmit={send} />
              <div className="row-end">
                <span className="muted">{t('Ctrl+Enter sends')}</span>
                <Button variant="primary" disabled={busy || !draft.trim()} onClick={send}>
                  {t('Comment')}
                </Button>
              </div>
            </div>
          )}
        </div>
      )}

      {tab === 'history' && (
        <div className="history">
          {issue.changes.length === 0 && <p className="muted">{t('Nothing has changed since the issue was created.')}</p>}
          {[...issue.changes].reverse().map((change) => (
            <div key={change.id} className="history-row">
              {author(change, change.actorId, change.actorLabel)}
              <span className="history-what">
                {t(FIELD_NAMES[change.field] ?? change.field)}:{' '}
                {change.oldValue && <del>{change.oldValue}</del>}
                {change.oldValue && change.newValue && ' → '}
                {change.newValue && <ins>{change.newValue}</ins>}
                {!change.newValue && change.oldValue && ` (${t('removed')})`}
              </span>
            </div>
          ))}
        </div>
      )}

      {tab === 'work' && (
        <div className="history">
          {issue.worklogs.length === 0 && <p className="muted">{t('No time logged yet.')}</p>}
          {issue.worklogs.map((worklog) => (
            <div key={worklog.id} className="history-row">
              <Avatar user={userById(worklog.userId)} size={26} />
              <strong>{userById(worklog.userId)?.name ?? t('Someone')}</strong>
              <span className="muted">{formatDate(worklog.startedAt)}</span>
              <span className="history-what">
                <strong>{formatMinutes(worklog.minutes)}</strong> {worklog.note}
              </span>
              {worklog.userId === user.id && <Button size="sm" variant="ghost" icon="trash" aria-label={t('Delete')} onClick={() => void attempt(() => api.del(`/api/worklogs/${worklog.id}`)).then(reload)} />}
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

function LinkDialog({ issue, onClose, onDone }: { issue: Issue; onClose: () => void; onDone: () => void }) {
  const [kind, setKind] = useState('blocks');
  const [target, setTarget] = useState('');
  const [busy, run] = useBusy();
  const hits = useRemote(() => (target.trim().length > 1 ? api.get<QuickHit[]>(`/api/quick?q=${encodeURIComponent(target)}`) : Promise.resolve([])), [target]);
  const kinds = [
    { value: 'blocks', label: t('blocks') },
    { value: 'blocked', label: t('is blocked by') },
    { value: 'relates', label: t('relates to') },
    { value: 'duplicates', label: t('duplicates') },
  ];

  function save(): void {
    const key = target.trim().toUpperCase();

    void run(async () => {
      if (kind === 'blocked') {
        await api.post(`/api/issues/${key}/links`, { type: 'blocks', targetKey: issue.key });
      } else {
        await api.post(`/api/issues/${issue.key}/links`, { type: kind, targetKey: key });
      }

      onDone();
      onClose();
    });
  }

  return (
    <Dialog
      title={t('Link {key}', { key: issue.key })}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" disabled={busy || !target.trim()} onClick={save}>
            {t('Link')}
          </Button>
        </>
      }
    >
      <div className="form">
        <Field label={t('This issue')}>
          <Picker value={kind} options={kinds} onChange={setKind} />
        </Field>
        <Field label={t('Issue')} hint={t('A key such as WEB-12, or words from the summary')}>
          <input className="input" autoFocus value={target} onChange={(event) => setTarget(event.target.value)} onKeyDown={(event) => event.key === 'Enter' && save()} />
        </Field>
        <div className="menu menu-flat">
          {(hits.data ?? [])
            .filter((hit) => hit.kind === 'issue' && hit.id !== issue.key)
            .slice(0, 6)
            .map((hit) => (
              <button key={hit.id} type="button" className="menu-item quick-hit" onClick={() => setTarget(hit.id)}>
                <span className="issue-key">{hit.id}</span>
                <span className="quick-hit-title">{hit.title}</span>
              </button>
            ))}
        </div>
      </div>
    </Dialog>
  );
}

function WorkDialog({ issue, onClose, onDone }: { issue: Issue; onClose: () => void; onDone: () => void }) {
  const [spent, setSpent] = useState('');
  const [note, setNote] = useState('');
  const [estimate, setEstimate] = useState('');
  const [busy, run] = useBusy();

  return (
    <Dialog
      title={t('Log work on {key}', { key: issue.key })}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button
            variant="primary"
            disabled={busy || (!spent.trim() && !estimate.trim())}
            onClick={() =>
              void run(async () => {
                if (spent.trim()) {
                  await api.post(`/api/issues/${issue.key}/worklogs`, { spent, note });
                }

                if (estimate.trim()) {
                  const hours = Number(estimate.replace(',', '.'));
                  await api.patch(`/api/issues/${issue.key}`, { estimateMinutes: Number.isFinite(hours) ? Math.round(hours * 60) : null });
                }

                onDone();
                onClose();
              })
            }
          >
            {t('Save')}
          </Button>
        </>
      }
    >
      <div className="form">
        <Field label={t('Time spent')} hint={t('For example 1h 30m, 45m or 2d (a day is 8 hours)')}>
          <input className="input" autoFocus value={spent} onChange={(event) => setSpent(event.target.value)} placeholder="1h 30m" />
        </Field>
        <Field label={t('What was done')}>
          <input className="input" value={note} onChange={(event) => setNote(event.target.value)} />
        </Field>
        <Field label={t('Original estimate (hours)')} hint={issue.estimateMinutes !== null ? t('Currently {estimate}', { estimate: formatMinutes(issue.estimateMinutes) }) : undefined}>
          <input className="input" inputMode="decimal" value={estimate} onChange={(event) => setEstimate(event.target.value)} />
        </Field>
      </div>
    </Dialog>
  );
}

export function IssueListEmpty({ title }: { title: string }) {
  return <Empty icon="check" title={title} />;
}
