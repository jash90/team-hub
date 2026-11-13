// Projects of the server tied to clones on this computer.
import { useState } from 'react';

import { Icon } from '@web/components/icon.tsx';
import { Button, Empty, ErrorNote, Spinner, Tag, useBusy } from '@web/components/ui.tsx';
import { t } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import type { RepoCheck } from '../bindings/RepoCheck.ts';
import type { RepoStatus } from '../bindings/RepoStatus.ts';
import { pickFolder } from '../bridge.ts';
import { useRemote } from '../live.ts';
import { useSession } from '../session.tsx';

function Clone({ repo, onChanged }: { repo: RepoStatus; onChanged: () => void }) {
  const { go } = useSession();
  const [busy, run] = useBusy();

  return (
    <div className="desk-clone">
      <div className="desk-clone-head">
        <Icon name="folder" size={14} />
        <code className="desk-clone-path" title={repo.mapping.path}>
          {repo.mapping.path}
        </code>
        {repo.error ? <Tag tone="red">{repo.error}</Tag> : repo.changedFiles === 0 ? <Tag tone="green">{t('clean')}</Tag> : <Tag tone="amber">{t('{count} changed', { count: repo.changedFiles })}</Tag>}
        {repo.remoteMatches === false && <Tag tone="red">{t('origin is another repository')}</Tag>}
        <span className="desk-grow" />
        <Button size="sm" variant="ghost" icon="trash" disabled={busy} onClick={async () => (await run(() => core.unmapRepo(repo.mapping.id), t('Removed')), onChanged())} aria-label={t('Remove')} title={t('Remove the mapping (the folder stays)')} />
      </div>
      <div className="desk-clone-facts muted">
        <span>
          {t('default branch')} <code>{repo.defaultBranch ?? '?'}</code>
        </span>
        {repo.currentBranch && (
          <span>
            {t('checked out')} <code>{repo.currentBranch}</code>
          </span>
        )}
        <span>{t('{count} working trees', { count: repo.worktrees.length })}</span>
      </div>
      {repo.worktrees.map((tree) => (
        <div key={tree.path} className="pr-row">
          <span className="pr-state pr-branch">
            <Icon name="branch" size={14} /> <code>{tree.head}</code>
          </span>
          <code className="pr-title" title={tree.path}>
            {tree.branch ?? t('(detached)')}
          </code>
          {tree.threadId !== null ? (
            <button type="button" className="link-button" onClick={() => go({ name: 'thread', id: tree.threadId ?? 0 })}>
              {t('thread #{id}', { id: tree.threadId })}
            </button>
          ) : (
            <span className="muted">{t('not a thread of this app')}</span>
          )}
        </div>
      ))}
    </div>
  );
}

function ProjectRepos({ projectKey, name, clones, onChanged }: { projectKey: string; name: string; clones: RepoStatus[]; onChanged: () => void }) {
  const code = useRemote(() => core.projectCode(projectKey), [projectKey]);
  const [check, setCheck] = useState<{ result: RepoCheck; githubRepo: string | null } | null>(null);
  const [busy, run] = useBusy();
  const connected = code.data?.repos ?? [];
  const mapped = new Set(clones.map((clone) => clone.mapping.githubRepo?.toLowerCase()));

  async function pick(githubRepo: string | null): Promise<void> {
    const folder = await pickFolder(githubRepo ? t('The folder with your clone of {repo}', { repo: githubRepo }) : t('The folder with your clone'));

    if (folder) {
      setCheck({ result: await core.checkRepo(folder, githubRepo), githubRepo });
    }
  }

  async function map(): Promise<void> {
    if (check && (await run(() => core.mapRepo(projectKey, check.result.path, check.githubRepo), t('Saved')))) {
      setCheck(null);
      onChanged();
    }
  }

  return (
    <section className="panel settings-card">
      <header className="panel-head">
        <span className="project-badge">{projectKey}</span>
        <h2>{name}</h2>
        <span className="desk-grow" />
        {connected.length === 0 && (
          <Button size="sm" icon="plus" onClick={() => pick(null)}>
            {t('Map a folder')}
          </Button>
        )}
      </header>
      {connected.map((repo) => (
        <div key={repo.id} className="list-row">
          <Icon name="branch" size={14} />
          <strong>{repo.fullName}</strong>
          <span className="muted">{repo.defaultBranch}</span>
          <span className="desk-grow" />
          {mapped.has(repo.fullName.toLowerCase()) ? (
            <Tag tone="green">{t('mapped')}</Tag>
          ) : (
            <Button size="sm" icon="folder" onClick={() => pick(repo.fullName)}>
              {t('Choose the folder…')}
            </Button>
          )}
        </div>
      ))}
      {connected.length === 0 && clones.length === 0 && <p className="muted">{t('No repository is connected to this project on the server. You can still map a clone; its origin is then not checked.')}</p>}
      {check && (
        <div className={check.result.matches && check.result.isRepository ? 'note desk-check-ok' : 'note note-error'}>
          <div>
            <code>{check.result.path}</code>
            <div>{check.result.problem ?? t('origin is {url}, default branch {branch}', { url: check.result.remoteUrl ?? '—', branch: check.result.defaultBranch ?? '?' })}</div>
          </div>
          <span className="desk-grow" />
          {check.result.isRepository && check.result.matches && (
            <Button variant="primary" size="sm" disabled={busy} onClick={map}>
              {t('Use this folder')}
            </Button>
          )}
          <Button size="sm" onClick={() => setCheck(null)}>
            {t('Cancel')}
          </Button>
        </div>
      )}
      {clones.map((clone) => (
        <Clone key={clone.mapping.id} repo={clone} onChanged={onChanged} />
      ))}
    </section>
  );
}

export function ReposView() {
  const { workspace } = useSession();
  const repos = useRemote(() => core.repos(), [], (event) => event.type === 'thread_changed');
  const projects = (workspace?.bootstrap.projects ?? []).filter((project) => !project.archived);

  return (
    <div className="desk-view">
      <h1>{t('Repositories')}</h1>
      <p className="muted">{t('A thread works in a clone on this computer. Tie each project to the folder of its clone; the main checkout is never touched, every thread gets a working tree of its own beside it.')}</p>
      {repos.error && <ErrorNote message={repos.error} />}
      {!repos.data && !repos.error && <Spinner />}
      {projects.length === 0 && <Empty icon="folder" title={t('No projects yet')} />}
      {repos.data &&
        projects.map((project) => (
          <ProjectRepos key={project.key} projectKey={project.key} name={project.name} clones={repos.data?.filter((repo) => repo.mapping.projectKey === project.key) ?? []} onChanged={repos.reload} />
        ))}
    </div>
  );
}
