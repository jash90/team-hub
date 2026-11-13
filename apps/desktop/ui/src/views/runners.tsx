// Every runner the person may see, as the web shows them: whose it is, what it takes, what
// it is carrying out now. This computer is one of them.
import type { Runner } from '@web/bindings/Runner.ts';
import type { RunnerWork } from '@web/bindings/RunnerWork.ts';
import { Icon } from '@web/components/icon.tsx';
import { Button, ErrorNote, Spinner, Tag } from '@web/components/ui.tsx';
import { t, timeAgo } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import { openExternal } from '../bridge.ts';
import { runnerKind } from '../components/where.tsx';
import { useRemote } from '../live.ts';
import { useSession } from '../session.tsx';
import { kindLabel } from '../words.ts';
import type { ThreadKind } from '../bindings/ThreadKind.ts';

function takes(work: RunnerWork): string {
  if (work.module === 'e2e') {
    return t('test suites');
  }

  return work.what.map((kind) => (['implement', 'review', 'custom'].includes(kind) ? kindLabel(kind as ThreadKind) : kind)).join(', ') || t('nothing right now');
}

function RunnerRow({ runner, here }: { runner: Runner; here: boolean }) {
  const { serverUrl, workspace } = useSession();
  const owner = workspace?.bootstrap.users.find((user) => user.id === runner.userId);

  return (
    <section className={here ? 'panel settings-card desk-runner is-here' : 'panel settings-card desk-runner'}>
      <header className="panel-head">
        <Icon name={runner.interactive ? 'user' : 'layers'} size={16} />
        <h2>{runner.name}</h2>
        {here && <Tag tone="blue">{t('This computer')}</Tag>}
        <Tag>{runnerKind(runner)}</Tag>
        <Tag tone={runner.online ? 'green' : 'amber'}>{runner.online ? t('online') : t('offline')}</Tag>
        <span className="desk-grow" />
        <span className="muted">
          {owner ? owner.name : ''} · {t('seen {when}', { when: timeAgo(runner.lastSeenAt) })}
          {runner.version ? ` · ${runner.version}` : ''}
        </span>
      </header>
      {runner.work.map((work) => (
        <div key={work.module} className="desk-runner-work">
          <strong>{work.module === 'e2e' ? t('Tests') : t('Agent runs')}</strong>
          <span>{takes(work)}</span>
          <span className="muted">{work.projects ? work.projects.join(', ') : t('every project it may edit')}</span>
          {work.pinnedOnly && <Tag title={t('It takes only what is sent to it by name.')}>{t('by name only')}</Tag>}
          {work.active.length === 0 && <span className="muted">{t('idle')}</span>}
          {work.active.map((active) => (
            <button key={active.href} type="button" className="link-button" onClick={() => void openExternal(serverUrl(active.href) ?? active.href)} title={t('Open on the web')}>
              <Icon name="play" size={12} /> {active.title}
            </button>
          ))}
        </div>
      ))}
    </section>
  );
}

export function RunnersView() {
  const { status } = useSession();
  const runners = useRemote(() => core.runners(), [], (event) => event.type === 'connection' || (event.type === 'data_changed' && (event.scope === 'runs' || event.scope === 'jobs')));
  const list = runners.data ?? [];
  const mine = list.filter((runner) => runner.id === status.runner.id);
  const others = list.filter((runner) => runner.id !== status.runner.id);

  return (
    <div className="desk-view">
      <div className="desk-view-head">
        <h1>{t('Runners')}</h1>
        <span className="desk-grow" />
        <Button size="sm" variant="ghost" icon="refresh" onClick={runners.reload} aria-label={t('Refresh')} title={t('Refresh')} />
      </div>
      <p className="muted">
        {t('A runner is a machine that has the repositories and carries out agent runs and test suites: this computer, other people’s computers, and services such as the cloud runner. Work started here runs on this computer unless you send it elsewhere.')}
      </p>
      {runners.loading && !runners.data && <Spinner />}
      {runners.error && <ErrorNote message={runners.error} />}
      {runners.data && mine.length === 0 && <div className="note">{t('The server has not heard from this computer yet. It announces itself within a minute of connecting.')}</div>}
      {[...mine, ...others].map((runner) => (
        <RunnerRow key={runner.id} runner={runner} here={runner.id === status.runner.id} />
      ))}
    </div>
  );
}
