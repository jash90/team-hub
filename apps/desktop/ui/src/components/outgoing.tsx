// The two things that leave this machine: a push and a pull request. Each shows exactly what
// would go out and does nothing until the person clicks.
import { useState } from 'react';

import { Button, Dialog, ErrorNote, Field, Spinner, toast, useBusy } from '@web/components/ui.tsx';
import { Icon } from '@web/components/icon.tsx';
import { countOf, t } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import type { PullRequestPreview } from '../bindings/PullRequestPreview.ts';
import { openExternal } from '../bridge.ts';
import { useRemote } from '../live.ts';
import { CommitList, DiffFiles } from './diff.tsx';

export function PushDialog({ threadId, onClose }: { threadId: number; onClose: () => void }) {
  const preview = useRemote(() => core.pushPreview(threadId), [threadId]);
  const [confirmed, setConfirmed] = useState(false);
  const [busy, run] = useBusy();
  const data = preview.data;
  const needsConfirmation = (data?.findings.length ?? 0) > 0;
  const canPush = data && !data.blocked && (!needsConfirmation || confirmed);

  async function push(): Promise<void> {
    if (!data) {
      return;
    }

    const pushed = await run(() => core.push(threadId, data.commit, confirmed), t('Pushed'));

    if (pushed) {
      onClose();
    }
  }

  return (
    <Dialog
      title={t('Push the branch')}
      onClose={onClose}
      wide
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" icon="upload" disabled={!canPush || busy} onClick={push}>
            {data ? `${t('Push')} ${countOf(data.commits.length, 'commit', 'commits')}` : t('Push')}
          </Button>
        </>
      }
    >
      {preview.loading && !data && <Spinner label={t('Reading what would be pushed…')} />}
      {preview.error && <ErrorNote message={preview.error} />}
      {data && (
        <div className="form">
          <p>
            <Icon name="branch" size={14} /> <code>{data.branch}</code> → <code>{data.remoteUrl}</code>
          </p>
          <p className="muted">{t('Exactly this commit is pushed: {commit}. Anything the thread commits after you looked stays here.', { commit: data.commit.slice(0, 12) })}</p>
          {data.blocked && <ErrorNote message={data.blocked} />}
          {data.uncommitted > 0 && <div className="note">{t('Files in the working tree that are not committed, and so not pushed: {count}', { count: data.uncommitted })}</div>}
          {needsConfirmation && (
            <div className="desk-findings">
              <strong>{t('This push needs a second look')}</strong>
              <ul>
                {data.findings.map((found, index) => (
                  <li key={index}>
                    {found.detail} {found.commit && <code>{found.commit}</code>}
                  </li>
                ))}
              </ul>
              <label className="check">
                <input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.target.checked)} />
                <span>{t('I have looked at these and want to push them')}</span>
              </label>
            </div>
          )}
          <h3>{t('Commits')}</h3>
          <CommitList commits={data.commits} />
          <h3>{t('Changes')}</h3>
          <DiffFiles files={data.files} />
        </div>
      )}
    </Dialog>
  );
}

function PullRequestForm({ preview, onClose }: { preview: PullRequestPreview; onClose: () => void }) {
  const [title, setTitle] = useState(preview.title);
  const [body, setBody] = useState(preview.body);
  const [url, setUrl] = useState('');
  const [browserUrl, setBrowserUrl] = useState<string | null>(null);
  const [busy, run] = useBusy();

  async function open(): Promise<void> {
    const result = await run(() => core.openPullRequest(preview.threadId, title, body));

    if (result?.url) {
      toast(t('Pull request opened'));
      onClose();
    } else if (result?.browserUrl) {
      setBrowserUrl(result.browserUrl);
      await openExternal(result.browserUrl);
    }
  }

  async function record(): Promise<void> {
    if (await run(() => core.recordPullRequest(preview.threadId, url), t('Saved'))) {
      onClose();
    }
  }

  if (preview.existingUrl) {
    return (
      <p>
        {t('This branch already has a pull request:')}{' '}
        <a href={preview.existingUrl} onClick={(event) => (event.preventDefault(), void openExternal(preview.existingUrl ?? ''))}>
          {preview.existingUrl}
        </a>
      </p>
    );
  }

  return (
    <div className="form">
      <p>
        <code>{preview.head}</code> → <code>{preview.base}</code> {preview.repo && <span className="muted">· {preview.repo}</span>}
      </p>
      {!preview.pushed && <div className="note">{t('Push the branch first: the server does not have the last commit of this thread.')}</div>}
      <Field label={t('Title')}>
        <input className="input" value={title} onChange={(event) => setTitle(event.target.value)} />
      </Field>
      <Field label={t('Description')}>
        <textarea className="input" rows={8} value={body} onChange={(event) => setBody(event.target.value)} />
      </Field>
      <div className="row-end">
        <Button variant="primary" icon="pullRequest" disabled={!preview.pushed || busy || !title.trim()} onClick={open}>
          {preview.canCreate ? t('Open the pull request') : t('Continue in the browser')}
        </Button>
      </div>
      <p className="muted">
        {preview.canCreate && !browserUrl
          ? t('Opened it somewhere else already? Paste its address and the thread and its run will show it.')
          : t('The gh command is not installed, so the pull request is finished on GitHub in your browser. Paste its address here afterwards and the thread and its run will show it.')}
      </p>
      <div className="inline-form desk-inline-wide">
        <input className="input" placeholder="https://github.com/…/pull/12" value={url} onChange={(event) => setUrl(event.target.value)} aria-label={t('Address of the pull request')} />
        <Button disabled={busy || !url.trim()} onClick={record}>
          {t('Save the address')}
        </Button>
      </div>
    </div>
  );
}

export function PullRequestDialog({ threadId, onClose }: { threadId: number; onClose: () => void }) {
  const preview = useRemote(() => core.pullRequestPreview(threadId), [threadId]);

  return (
    <Dialog title={t('Open a pull request')} onClose={onClose} wide>
      {preview.loading && !preview.data && <Spinner />}
      {preview.error && <ErrorNote message={preview.error} />}
      {preview.data && <PullRequestForm preview={preview.data} onClose={onClose} />}
    </Dialog>
  );
}
