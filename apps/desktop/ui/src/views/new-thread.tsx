import { useEffect, useState, type FormEvent } from 'react';

import { Icon } from '@web/components/icon.tsx';
import { Button, ErrorNote, Field } from '@web/components/ui.tsx';
import { t } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import { web } from '../bridge.ts';
import type { ThreadKind } from '../bindings/ThreadKind.ts';
import { ANY, HERE, namableRunners, WhereSelect } from '../components/where.tsx';
import { useRemote } from '../live.ts';
import { useSession } from '../session.tsx';
import { kindHint, kindIcon, kindLabel, THREAD_KINDS } from '../words.ts';

/** Starts an agent thread for an issue: what kind of work, with which instructions, where. */
export function NewThreadView({ issueKey }: { issueKey?: string }) {
  const { go, status, workspace } = useSession();
  const [where, setWhere] = useState(HERE);
  const [key, setKey] = useState(issueKey ?? '');
  const [kind, setKind] = useState<ThreadKind>('implement');
  const [instructions, setInstructions] = useState('');
  const [model, setModel] = useState('');
  const [repoId, setRepoId] = useState<number | null>(null);
  const [branch, setBranch] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | undefined>();
  const settings = useRemote(() => core.settings(), []);
  const models = useRemote(() => core.models(), []);
  const repos = useRemote(() => core.repos(), []);
  const dashboard = useRemote(() => core.dashboard(), []);
  const project = key.trim().toUpperCase().split('-')[0];
  const ofProject = (repos.data ?? []).filter((repo) => repo.mapping.projectKey === project);
  const runners = useRemote(() => core.runners(), []);
  const code = useRemote(() => (project ? core.projectCode(project) : Promise.resolve(null)), [project]);
  // The runners that take this kind of work for this project, besides this computer.
  const elsewhere = namableRunners(runners.data ?? [], status.runner.id, workspace?.bootstrap.user?.id, 'agents', kind === 'analyse' ? 'custom' : kind, project);
  const here = where === HERE;

  useEffect(() => setKey(issueKey ?? ''), [issueKey]);

  // Started with no issue named while the tracker shows one: that is the issue meant.
  useEffect(() => {
    if (issueKey) {
      return;
    }

    void web.location().then(
      (path) => {
        const shown = path?.match(/^\/issue\/([A-Za-z][A-Za-z0-9]*-\d+)/)?.[1];

        if (shown) {
          setKey((typed) => typed || shown.toUpperCase());
        }
      },
      () => undefined,
    );
  }, [issueKey]);
  useEffect(() => setRepoId(null), [project]);

  async function submit(event: FormEvent): Promise<void> {
    event.preventDefault();
    setBusy(true);
    setError(undefined);

    try {
      if (!here) {
        // Carried out by another runner: nothing starts on this computer, the app follows the run.
        const chosen = ofProject.find((repo) => repo.mapping.id === repoId) ?? ofProject[0];
        const repo = chosen?.mapping.githubRepo ?? code.data?.repos[0]?.fullName ?? null;
        const run = await core.delegate({ issueKey: key.trim(), kind, instructions, model: model || null, repo, onRunner: where === ANY ? null : where });

        go({ name: 'run', id: run.id });

        return;
      }

      const thread = await core.startThread({ issueKey: key.trim(), kind, instructions, model: model || null, repoId: repoId ?? (ofProject.length === 1 ? ofProject[0].mapping.id : null), branch: branch.trim() || null });

      go({ name: 'thread', id: thread.id });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="desk-view form" onSubmit={submit}>
      <h1>{t('New thread')}</h1>
      <p className="muted">{t('The agent works in a git working tree and a branch of its own. Nothing leaves this computer until you approve it.')}</p>
      <section className="panel settings-card">
        <header className="panel-head">
          <h2>{t('Issue')}</h2>
        </header>
        <Field label={t('Key')} hint={t('A key such as WEB-12')}>
          <input className="input input-mono" list="desk-my-issues" value={key} onChange={(event) => setKey(event.target.value)} required autoFocus={!issueKey} />
        </Field>
        <datalist id="desk-my-issues">
          {(dashboard.data?.dashboard.assigned ?? []).map((row) => (
            <option key={row.id} value={row.key}>
              {row.summary}
            </option>
          ))}
        </datalist>
      </section>
      <section className="panel settings-card">
        <header className="panel-head">
          <h2>{t('What the agent does')}</h2>
        </header>
        <div className="desk-kinds" role="radiogroup" aria-label={t('What the agent does')}>
          {THREAD_KINDS.map((option) => (
            <button key={option} type="button" role="radio" aria-checked={kind === option} className={kind === option ? 'desk-kind is-on' : 'desk-kind'} onClick={() => setKind(option)}>
              <Icon name={kindIcon(option)} size={16} />
              <strong>{kindLabel(option)}</strong>
              <span className="muted">{kindHint(option)}</span>
            </button>
          ))}
        </div>
        <Field label={t('Extra instructions')} hint={t('Optional. The issue itself is given to the agent as it is on the server.')}>
          <textarea className="input" rows={4} value={instructions} onChange={(event) => setInstructions(event.target.value)} />
        </Field>
      </section>
      <section className="panel settings-card">
        <header className="panel-head">
          <h2>{t('Where and with what')}</h2>
        </header>
        <Field
          label={t('Where')}
          hint={here ? t('On this computer: the agent works here, in a working tree of its own, with your own model sign-in.') : t('On another runner: nothing runs on this computer, and its pushes and pull requests are decided where that runner says.')}
        >
          <WhereSelect value={where} onChange={setWhere} runners={elsewhere} />
        </Field>
        <div className="form-row">
          <Field label={t('Repository')}>
            <select className="input" value={repoId ?? ''} onChange={(event) => setRepoId(event.target.value ? Number(event.target.value) : null)} disabled={ofProject.length <= 1}>
              {ofProject.length !== 1 && <option value="">{ofProject.length === 0 ? t('None mapped for this project') : t('Choose…')}</option>}
              {ofProject.map((repo) => (
                <option key={repo.mapping.id} value={repo.mapping.id}>
                  {repo.mapping.githubRepo ?? repo.mapping.path}
                </option>
              ))}
            </select>
          </Field>
          <Field label={t('Model')}>
            <select className="input" value={model} onChange={(event) => setModel(event.target.value)}>
              <option value="">
                {t('Default')} ({settings.data?.defaultModel ?? '…'})
              </option>
              {(models.data ?? []).map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <Field label={t('Branch')} hint={t('Leave empty for the branch suggested for the issue (feature/KEY-summary). Name an existing branch to continue it.')}>
          <input className="input input-mono" value={branch} onChange={(event) => setBranch(event.target.value)} />
        </Field>
        {here && project && ofProject.length === 0 && repos.data && (
          <div className="note">
            {t('No local repository is mapped to project {project}.', { project })}{' '}
            <button type="button" className="link-button" onClick={() => go({ name: 'repos' })}>
              {t('Map one')}
            </button>
          </div>
        )}
      </section>
      {error && <ErrorNote message={error} />}
      <div className="row-end">
        <Button onClick={() => window.history.back()}>{t('Cancel')}</Button>
        <Button variant="primary" type="submit" icon="play" disabled={busy || !key.trim()}>
          {busy ? t('Starting…') : here ? t('Start the thread') : t('Send the work')}
        </Button>
      </div>
    </form>
  );
}
