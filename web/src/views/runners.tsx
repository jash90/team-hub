import { useEffect } from 'react';

import { api } from '../api.ts';
import type { Runner } from '../bindings/Runner.ts';
import type { RunnerWork } from '../bindings/RunnerWork.ts';
import { Icon } from '../components/icon.tsx';
import { Empty, ErrorNote, Spinner, Tag } from '../components/ui.tsx';
import { t, timeAgo } from '../i18n/index.ts';
import { useModules } from '../modules.ts';
import { Link } from '../router.tsx';
import { useRemote, useSession } from '../session.tsx';

/** How often the list is read again without an event: a runner going quiet announces nothing. */
const REFRESH_MS = 30_000;

/**
 * The machines that carry out the work that needs a repository on disk (`docs/runners.md`):
 * somebody's computer running the desktop app, or a service. The same list is in the
 * desktop app.
 */
export function RunnersView() {
  const { user, userById } = useSession();
  const runners = useRemote(() => api.get<Runner[]>('/api/runners'), [], (event) => event.kind.endsWith('.run') || event.kind.endsWith('.job'));

  useEffect(() => {
    const timer = window.setInterval(() => void runners.reload(), REFRESH_MS);

    return () => window.clearInterval(timer);
  }, [runners.reload]);

  const team = runners.data?.filter((runner) => runner.shared) ?? [];
  const mine = runners.data?.filter((runner) => !runner.shared && runner.userId === user.id) ?? [];
  const others = runners.data?.filter((runner) => !runner.shared && runner.userId !== user.id) ?? [];

  return (
    <div className="view view-narrow">
      <header className="view-head">
        <div>
          <h1>{t('Runners')}</h1>
          <p className="muted">
            {t('Agent runs and test suites need a repository on disk. A runner has one and carries them out: the desktop app on somebody’s computer, using that computer, or a service in a container, using the container.')}
          </p>
        </div>
      </header>
      {runners.error && <ErrorNote message={runners.error} />}
      {!runners.data && !runners.error && <Spinner />}
      {runners.data?.length === 0 && (
        <Empty icon="layers" title={t('No runner has connected yet')}>
          {t('Open the desktop app and connect it to this server with an API token, or deploy the cloud runner next to the server. Either shows up here as soon as it has said hello.')}
        </Empty>
      )}
      {runners.data?.some((runner) => runner.shared) && (
        <p className="muted runner-note">
          {t('A shared runner works with your own keys.')} <Link href="/settings/keys">{t('Set them in Settings → My keys')}</Link>
        </p>
      )}
      {team.length > 0 && (
        <section className="runner-group">
          <h2>{t('The team’s')}</h2>
          {team.map((runner) => (
            <RunnerCard key={runner.id} runner={runner} owner={null} />
          ))}
        </section>
      )}
      {mine.length > 0 && (
        <section className="runner-group">
          <h2>{t('Yours')}</h2>
          {mine.map((runner) => (
            <RunnerCard key={runner.id} runner={runner} owner={null} />
          ))}
        </section>
      )}
      {others.length > 0 && (
        <section className="runner-group">
          <h2>{t('Other people’s, connected now')}</h2>
          {others.map((runner) => (
            <RunnerCard key={`${runner.userId}:${runner.id}`} runner={runner} owner={userById(runner.userId)?.name ?? null} />
          ))}
        </section>
      )}
    </div>
  );
}

function RunnerCard({ runner, owner }: { runner: Runner; owner: string | null }) {
  return (
    <article className="panel runner-card">
      <header className="runner-head">
        <span className={runner.online ? 'runner-dot runner-dot-on' : 'runner-dot'} title={runner.online ? t('connected') : t('not connected')} />
        <h3>{runner.name}</h3>
        <Tag tone={runner.interactive ? 'neutral' : 'blue'} title={runner.interactive ? t('The desktop app on a person’s computer; it uses that computer.') : t('A service with nobody at it; what it wants to push is decided here.')}>
          {runner.interactive ? t('computer') : t('service')}
        </Tag>
        {runner.shared && <Tag tone="green" title={t('The team’s runner: it takes anyone’s work and carries it out with that person’s own keys (Settings → My keys).')}>{t('shared')}</Tag>}
        <code className="runner-id" title={t('The runner’s id: what a scheduled task names to send its work to this runner.')}>
          {runner.id}
        </code>
        <span className="filter-spacer" />
        <span className="muted">
          {[owner, runner.version ? `v${runner.version}` : null, runner.online ? t('connected') : t('last heard from {when}', { when: timeAgo(runner.lastSeenAt) })].filter(Boolean).join(' · ')}
        </span>
      </header>
      {runner.work.map((work) => (
        <WorkLine key={work.module} work={work} />
      ))}
    </article>
  );
}

function WorkLine({ work }: { work: RunnerWork }) {
  const modules = useModules();
  const described = modules.find((module) => module.id === work.module)?.runnerWork?.(work) ?? `${work.module}: ${work.what.join(', ')}`;

  return (
    <div className="runner-work">
      <div className="runner-work-line">
        <span className={work.online ? 'runner-dot runner-dot-on runner-dot-small' : 'runner-dot runner-dot-small'} />
        <strong>{described}</strong>
        <span className="muted">
          {[work.projects === null ? t('any project') : work.projects.join(', '), work.pinnedOnly ? t('only what is sent to it by name') : null].filter(Boolean).join(' · ')}
        </span>
      </div>
      {work.active.length > 0 && (
        <ul className="runner-active">
          {work.active.map((active) => (
            <li key={active.href + active.title}>
              <Icon name="play" size={12} />
              <Link href={active.href}>{active.title}</Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
