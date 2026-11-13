import { useEffect, useMemo, useRef, useState } from 'react';

import { ApiError, api, errorMessage } from '@/api.ts';
import type { WikiPageDetail } from '@/bindings/WikiPageDetail.ts';
import { Icon } from '@/components/icon.tsx';
import { MarkdownEditor } from '@/components/markdown.tsx';
import { Button, ConfirmDialog, toast } from '@/components/ui.tsx';
import { t, timeAgo } from '@/i18n/index.ts';
import { navigate } from '@/router.tsx';

import { BASE, diffLines, pagePath } from './lib.ts';
import { DiffView, PageBody, useAuthorName } from './parts.tsx';

type Props = {
  /** The page as the server has it now; it changes under the editor when somebody else saves. */
  page: WikiPageDetail;
  titles: Map<string, number>;
  spaceKeys: Set<string>;
  reload: () => Promise<void>;
  onSaved: (page: WikiPageDetail) => void;
};

/**
 * Edits title and body. The draft is based on one version of the page; when the page moves
 * on meanwhile, the draft stays and the person decides what to do with the two.
 */
export function Editor({ page, titles, spaceKeys, reload, onSaved }: Props) {
  const authorName = useAuthorName();
  const [base, setBase] = useState(page.version);
  const [title, setTitle] = useState(page.title);
  const [body, setBody] = useState(page.body);
  const [message, setMessage] = useState('');
  const [saving, setSaving] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [comparing, setComparing] = useState(false);
  const [leaving, setLeaving] = useState(false);
  const started = useRef({ title: page.title, body: page.body });
  const stale = page.version !== base;
  const dirty = title !== started.current.title || body !== started.current.body;
  const back = pagePath(page.spaceKey, page.id);
  const difference = useMemo(() => (comparing ? diffLines(page.body, body) : []), [comparing, page.body, body]);

  useEffect(() => {
    if (!dirty) {
      return undefined;
    }

    const warn = (event: BeforeUnloadEvent) => event.preventDefault();
    window.addEventListener('beforeunload', warn);

    return () => window.removeEventListener('beforeunload', warn);
  }, [dirty]);

  async function save(baseVersion: number): Promise<void> {
    setSaving(true);
    setProblem(null);

    try {
      onSaved(await api.patch<WikiPageDetail>(`${BASE}/pages/${page.id}`, { title, body, baseVersion, message }));
      toast(t('Saved'));
      navigate(back);
    } catch (error) {
      if (error instanceof ApiError && error.status === 409) {
        // Either somebody saved in between or the page was locked; the fresh page tells which.
        await reload();
        setProblem(error.message);
      } else {
        toast(errorMessage(error), 'error');
      }
    } finally {
      setSaving(false);
    }
  }

  function takeTheirs(): void {
    setTitle(page.title);
    setBody(page.body);
    setBase(page.version);
    started.current = { title: page.title, body: page.body };
    setComparing(false);
    setProblem(null);
  }

  return (
    <div className="panel wiki-article wiki-editor">
      <header className="wiki-head">
        <h1 className="wiki-title">{t('Editing “{title}”', { title: page.title })}</h1>
        <span className="muted">{t('based on version {version}', { version: base })}</span>
      </header>

      {page.locked && (
        <div className="wiki-lock" role="alert">
          <Icon name="lock" />
          <span className="wiki-lock-text">{t('This page mirrors Confluence and cannot be saved here. Detach it on the page first.')}</span>
        </div>
      )}

      {stale && (
        <div className="wiki-conflict" role="alert">
          <div className="wiki-conflict-text">
            <Icon name="alert" />
            <span>
              <strong>{t('{name} saved version {version} {when}, while you were editing.', { name: authorName(page.updatedBy, null), version: page.version, when: timeAgo(page.updatedAt) })}</strong>{' '}
              {t('Your text is still here and nothing was overwritten. Compare the two, then keep yours or take theirs.')}
            </span>
          </div>
          <div className="wiki-conflict-actions">
            <Button onClick={() => setComparing(!comparing)}>{comparing ? t('Hide the differences') : t('Show the differences')}</Button>
            <Button variant="primary" disabled={saving} onClick={() => void save(page.version)}>
              {t('Save mine as version {version}', { version: page.version + 1 })}
            </Button>
            <Button variant="danger" onClick={takeTheirs}>
              {t('Discard mine, take version {version}', { version: page.version })}
            </Button>
          </div>
          {comparing && (
            <div className="wiki-conflict-diff">
              <p className="muted">{t('− lines only in version {version} (theirs) · + lines only in your text', { version: page.version })}</p>
              {title !== page.title && (
                <p>
                  {t('Title')}: <del>{page.title}</del> → <ins>{title}</ins>
                </p>
              )}
              <DiffView lines={difference} />
            </div>
          )}
        </div>
      )}

      {problem && !stale && !page.locked && (
        <div className="note note-error" role="alert">
          <Icon name="alert" />
          <span>{problem}</span>
        </div>
      )}

      <label className="field">
        <span className="field-label">{t('Title')}</span>
        <input className="input wiki-title-input" required maxLength={255} value={title} onChange={(event) => setTitle(event.target.value)} />
      </label>

      <div className="wiki-editor-panes">
        <div className="wiki-editor-write">
          <MarkdownEditor value={body} onChange={setBody} rows={24} onSubmit={() => !stale && !page.locked && void save(base)} placeholder={t('Markdown. [[Page title]] links to a page, WEB-12 to an issue, “- [ ] text” is a task.')} />
        </div>
        <div className="wiki-editor-preview" aria-label={t('Preview')}>
          <span className="field-label">{t('Preview')}</span>
          <div className="wiki-body">
            <PageBody body={body} spaceKey={page.spaceKey} titles={titles} spaceKeys={spaceKeys} />
          </div>
        </div>
      </div>

      <div className="wiki-editor-foot">
        <label className="field">
          <span className="field-label">{t('What changed (optional)')}</span>
          <input className="input" maxLength={200} value={message} placeholder={t('Shown in the page history')} onChange={(event) => setMessage(event.target.value)} />
        </label>
        <Button onClick={() => (dirty ? setLeaving(true) : navigate(back))}>{t('Cancel')}</Button>
        <Button variant="primary" disabled={saving || !title.trim() || page.locked || stale} title={stale ? t('Decide above what to do with the newer version first') : undefined} onClick={() => void save(base)}>
          {saving ? t('Saving…') : t('Save')}
        </Button>
      </div>

      {leaving && (
        <ConfirmDialog
          title={t('Discard your changes?')}
          body={t('What you changed on this page since you opened the editor is lost.')}
          confirm={t('Discard')}
          danger
          onClose={() => setLeaving(false)}
          onConfirm={() => navigate(back)}
        />
      )}
    </div>
  );
}
