import { useState } from 'react';

import { api } from '../../api.ts';
import type { Change } from '../../bindings/Change.ts';
import type { Comment } from '../../bindings/Comment.ts';
import type { Issue } from '../../bindings/Issue.ts';
import { Markdown, MarkdownEditor } from '../../components/markdown.tsx';
import { Avatar, Button, Tabs, attempt, useBusy } from '../../components/ui.tsx';
import { formatDate, formatDateTime, formatMinutes, t, timeAgo } from '../../i18n/index.ts';
import { useSession, useStored } from '../../session.tsx';

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

export function Activity({ issue, editable, reload }: { issue: Issue; editable: boolean; reload: () => void }) {
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
