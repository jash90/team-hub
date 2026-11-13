// The window: a bar across the top and columns under it. Threads and decisions on the left.
// Beside them the tracker, which is the server's own web interface shown in the window; or,
// for what belongs to this computer (a thread, repositories, settings), the selected view
// with my work on the server on the right.
import { useCallback, useEffect, useState, type FormEvent } from 'react';

import { Icon } from '@web/components/icon.tsx';
import { Button, ErrorNote, Spinner, Tag, Toaster, toast } from '@web/components/ui.tsx';
import { currentLanguage, setLanguage, t } from '@web/i18n/index.ts';
import { core } from './api.ts';
import type { ConnectionStatus } from './bindings/ConnectionStatus.ts';
import { inApp, listenToStage, web } from './bridge.ts';
import { issuePath } from './components/bits.tsx';
import { useLive, useRemote, useStored } from './live.ts';
import { SessionProvider, useSession, useView, type View } from './session.tsx';
import { ThreadsColumn } from './shell/threads-column.tsx';
import { WorkQueue, searchPath } from './shell/work-queue.tsx';
import { ConnectView } from './views/connect.tsx';
import { NewThreadView } from './views/new-thread.tsx';
import { ReposView } from './views/repos.tsx';
import { RunView } from './views/run.tsx';
import { SettingsView } from './views/settings.tsx';
import { ThreadView } from './views/thread.tsx';
import { TrackerView } from './views/tracker.tsx';

function ThemeToggle() {
  const [theme, setTheme] = useState(() => document.documentElement.dataset.theme ?? (window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'));

  function toggle(): void {
    const next = theme === 'dark' ? 'light' : 'dark';

    document.documentElement.dataset.theme = next;
    setTheme(next);
    void web.set('theme', next).catch(() => undefined);

    try {
      window.localStorage.setItem('team-hub:theme', next);
    } catch {
      // The theme still applies to this window.
    }
  }

  return <Button variant="ghost" icon={theme === 'dark' ? 'sun' : 'moon'} onClick={toggle} aria-label={t('Switch theme')} title={t('Switch theme')} />;
}

function TopBar({ panel, onPanel }: { panel: boolean; onPanel: (shown: boolean) => void }) {
  const { status, go, view, openPage, workspace } = useSession();
  const [words, setWords] = useState('');
  const tracker = view.name === 'web';

  // The tests of the project the tracker shows, or else of the first project there is.
  async function openTests(): Promise<void> {
    const shown = inApp ? await web.location().catch(() => null) : null;
    const key = shown?.match(/^\/p\/([^/?#]+)/)?.[1] ?? workspace?.bootstrap.projects[0]?.key;

    openPage(key ? `/p/${key}/tests` : '/runners');
  }

  function search(event: FormEvent): void {
    event.preventDefault();

    const text = words.trim();

    if (/^[A-Za-z][A-Za-z0-9]{1,9}-\d+$/.test(text)) {
      openPage(issuePath(text.toUpperCase()));
    } else if (text) {
      openPage(searchPath(text));
    }
  }

  return (
    <header className="topbar desk-topbar">
      <button type="button" className="brand desk-brand" onClick={() => go({ name: 'web' })}>
        <span className="brand-mark" />
        <span className="brand-name">team-hub</span>
      </button>
      <Button
        variant="ghost"
        icon="menu"
        onClick={() => onPanel(!panel)}
        aria-pressed={panel}
        aria-label={panel ? t('Hide threads') : t('Show threads')}
        title={panel ? t('Hide threads') : t('Show threads')}
      />
      <Button variant={tracker ? 'default' : 'ghost'} icon="board" onClick={() => go({ name: 'web' })}>
        {t('Tracker')}
      </Button>
      {tracker && inApp && (
        // A page of the server has no browser around it here: these stand in for its buttons.
        <span className="desk-web-steps">
          <Button variant="ghost" icon="chevronLeft" onClick={() => void web.go('back')} aria-label={t('Back')} title={t('Back')} />
          <Button variant="ghost" icon="chevronRight" onClick={() => void web.go('forward')} aria-label={t('Forward')} title={t('Forward')} />
          <Button variant="ghost" icon="refresh" onClick={() => void web.go('reload')} aria-label={t('Reload')} title={t('Reload')} />
        </span>
      )}
      <form className="filter-search desk-search" onSubmit={search} role="search">
        <Icon name="search" size={14} />
        <input className="input" placeholder={t('Search issues, or type a key such as WEB-12')} value={words} onChange={(event) => setWords(event.target.value)} aria-label={t('Search')} />
      </form>
      <span className="desk-grow" />
      <Tag tone={status.online ? 'green' : 'amber'} title={status.error ?? status.serverUrl ?? undefined}>
        {status.online ? t('online') : t('offline')}
      </Tag>
      {status.runner.enabled && status.runner.runsApi !== false && <Tag tone="blue" title={status.runner.name}>{t('runner')}</Tag>}
      <Button variant="ghost" icon="layers" onClick={() => openPage('/runners')}>
        {t('Runners')}
      </Button>
      <Button variant="ghost" icon="check" onClick={() => void openTests()}>
        {t('Tests')}
      </Button>
      <Button variant={view.name === 'repos' ? 'default' : 'ghost'} icon="folder" onClick={() => go({ name: 'repos' })}>
        {t('Repositories')}
      </Button>
      <Button variant="ghost" icon="globe" onClick={() => {
          const next = currentLanguage() === 'pl' ? 'en' : 'pl';

          // The tracker follows; told first, because the app's own interface reloads next.
          void web
            .set('language', next)
            .catch(() => undefined)
            .then(() => setLanguage(next));
        }} aria-label={t('Language')} title={t('Language')}>
        {currentLanguage().toUpperCase()}
      </Button>
      <ThemeToggle />
      <Button variant={view.name === 'settings' ? 'default' : 'ghost'} icon="settings" onClick={() => go({ name: 'settings' })} aria-label={t('Settings')} title={t('Settings')} />
    </header>
  );
}

function Center({ view, onStatus }: { view: View; onStatus: (status: ConnectionStatus) => void }) {
  switch (view.name) {
    case 'thread':
      return <ThreadView threadId={view.id} />;
    case 'new-thread':
      return <NewThreadView issueKey={view.key} />;
    case 'repos':
      return <ReposView />;
    case 'run':
      return <RunView runId={view.id} />;
    case 'settings':
      return <SettingsView onStatus={onStatus} />;
    default:
      return <TrackerView />;
  }
}

export function App() {
  const [status, setStatus] = useState<ConnectionStatus | undefined>();
  const [failure, setFailure] = useState<string | undefined>();
  const [view, go] = useView();
  const [panel, setPanel] = useStored('panel', true);
  const refresh = useCallback(() => {
    core.status().then(setStatus, (reason: unknown) => setFailure(reason instanceof Error ? reason.message : String(reason)));
  }, []);
  const connected = Boolean(status?.hasToken && status.serverUrl);
  const workspace = useRemote(() => (connected ? core.workspace() : Promise.resolve(undefined)), [connected, status?.serverUrl], (event) => event.type === 'connection');

  useEffect(refresh, [refresh]);

  useLive((event) => {
    if (event.type === 'connection') {
      refresh();
    }

    // Inside the app these are system notifications; in a browser they are shown here.
    if (event.type === 'notify' && !inApp) {
      toast(`${event.title}: ${event.body}`);
    }
  });

  // A run's card in the tracker has "Open the thread in the app". It is the one thing a
  // page of the server can ask for, and all it does is change what is shown here: the
  // thread this computer made for the run, or the run itself when there is no such thread.
  useEffect(
    () =>
      listenToStage((event) => {
        const runId = event.openRun;

        if (typeof runId !== 'number' || !Number.isSafeInteger(runId) || runId <= 0) {
          return;
        }

        core.threads().then(
          (threads) => {
            const thread = threads.find((candidate) => candidate.runId === runId);

            go(thread ? { name: 'thread', id: thread.id } : { name: 'run', id: runId });
          },
          () => go({ name: 'run', id: runId }),
        );
      }),
    [go],
  );

  // The runner's state (last heartbeat, whether the server has runs) changes without an event.
  useEffect(() => {
    const timer = window.setInterval(refresh, 30_000);

    return () => window.clearInterval(timer);
  }, [refresh]);

  if (!status) {
    return <div className="center-screen">{failure ? <ErrorNote message={failure} /> : <Spinner />}</div>;
  }

  if (!connected) {
    return (
      <>
        <ConnectView status={status} onConnected={setStatus} />
        <Toaster />
      </>
    );
  }

  return (
    <SessionProvider status={status} workspace={workspace.data} view={view} go={go}>
      <div className="desk-shell">
        <TopBar panel={panel} onPanel={setPanel} />
        {view.name === 'web' ? (
          // The tracker has my work on its own start page, so it takes that column's room too.
          <div className={panel ? 'desk-columns desk-columns-web' : 'desk-columns desk-columns-web desk-columns-alone'}>
            {panel && <ThreadsColumn />}
            <main className="desk-center desk-center-web">
              <TrackerView />
            </main>
          </div>
        ) : (
          <div className={panel ? 'desk-columns' : 'desk-columns desk-columns-no-panel'}>
            {panel && <ThreadsColumn />}
            <main className="desk-center">
              {workspace.error && !workspace.data && <ErrorNote message={workspace.error} />}
              <Center view={view} onStatus={setStatus} />
            </main>
            <WorkQueue />
          </div>
        )}
      </div>
      <Toaster />
    </SessionProvider>
  );
}
