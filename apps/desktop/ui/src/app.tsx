// The window: a bar across the top and three columns under it. Threads and decisions on the
// left, the selected view in the middle, my work on the server on the right.
import { useCallback, useEffect, useState, type FormEvent } from 'react';

import { Icon } from '@web/components/icon.tsx';
import { Button, ErrorNote, Spinner, Tag, Toaster, toast } from '@web/components/ui.tsx';
import { currentLanguage, setLanguage, t } from '@web/i18n/index.ts';
import { core } from './api.ts';
import type { ConnectionStatus } from './bindings/ConnectionStatus.ts';
import { inApp } from './bridge.ts';
import { useLive, useRemote } from './live.ts';
import { SessionProvider, useSession, useView, type View } from './session.tsx';
import { ThreadsColumn } from './shell/threads-column.tsx';
import { WorkQueue } from './shell/work-queue.tsx';
import { ConnectView } from './views/connect.tsx';
import { HomeView } from './views/home.tsx';
import { IssueView } from './views/issue.tsx';
import { NewThreadView } from './views/new-thread.tsx';
import { ReposView } from './views/repos.tsx';
import { RunView } from './views/run.tsx';
import { RunnersView } from './views/runners.tsx';
import { TestsView } from './views/tests.tsx';
import { SearchView } from './views/search.tsx';
import { SettingsView } from './views/settings.tsx';
import { ThreadView } from './views/thread.tsx';

function ThemeToggle() {
  const [theme, setTheme] = useState(() => document.documentElement.dataset.theme ?? (window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'));

  function toggle(): void {
    const next = theme === 'dark' ? 'light' : 'dark';

    document.documentElement.dataset.theme = next;
    setTheme(next);

    try {
      window.localStorage.setItem('team-hub:theme', next);
    } catch {
      // The theme still applies to this window.
    }
  }

  return <Button variant="ghost" icon={theme === 'dark' ? 'sun' : 'moon'} onClick={toggle} aria-label={t('Switch theme')} title={t('Switch theme')} />;
}

function TopBar() {
  const { status, go, view } = useSession();
  const [words, setWords] = useState('');

  function search(event: FormEvent): void {
    event.preventDefault();

    const text = words.trim();

    if (/^[A-Za-z][A-Za-z0-9]{1,9}-\d+$/.test(text)) {
      go({ name: 'issue', key: text.toUpperCase() });
    } else if (text) {
      go({ name: 'search', query: text });
    }
  }

  return (
    <header className="topbar desk-topbar">
      <button type="button" className="brand desk-brand" onClick={() => go({ name: 'home' })}>
        <span className="brand-mark" />
        <span className="brand-name">team-hub</span>
      </button>
      <form className="filter-search desk-search" onSubmit={search} role="search">
        <Icon name="search" size={14} />
        <input className="input" placeholder={t('Search issues, or type a key such as WEB-12')} value={words} onChange={(event) => setWords(event.target.value)} aria-label={t('Search')} />
      </form>
      <span className="desk-grow" />
      <Tag tone={status.online ? 'green' : 'amber'} title={status.error ?? status.serverUrl ?? undefined}>
        {status.online ? t('online') : t('offline')}
      </Tag>
      {status.runner.enabled && status.runner.runsApi !== false && <Tag tone="blue" title={status.runner.name}>{t('runner')}</Tag>}
      <Button variant={view.name === 'runners' ? 'default' : 'ghost'} icon="layers" onClick={() => go({ name: 'runners' })}>
        {t('Runners')}
      </Button>
      <Button variant={view.name === 'tests' ? 'default' : 'ghost'} icon="check" onClick={() => go({ name: 'tests' })}>
        {t('Tests')}
      </Button>
      <Button variant={view.name === 'repos' ? 'default' : 'ghost'} icon="folder" onClick={() => go({ name: 'repos' })}>
        {t('Repositories')}
      </Button>
      <Button variant="ghost" icon="globe" onClick={() => setLanguage(currentLanguage() === 'pl' ? 'en' : 'pl')} aria-label={t('Language')} title={t('Language')}>
        {currentLanguage().toUpperCase()}
      </Button>
      <ThemeToggle />
      <Button variant={view.name === 'settings' ? 'default' : 'ghost'} icon="settings" onClick={() => go({ name: 'settings' })} aria-label={t('Settings')} title={t('Settings')} />
    </header>
  );
}

function Center({ view, onStatus }: { view: View; onStatus: (status: ConnectionStatus) => void }) {
  switch (view.name) {
    case 'issue':
      return <IssueView issueKey={view.key} />;
    case 'thread':
      return <ThreadView threadId={view.id} />;
    case 'new-thread':
      return <NewThreadView issueKey={view.key} />;
    case 'search':
      return <SearchView query={view.query} />;
    case 'repos':
      return <ReposView />;
    case 'runners':
      return <RunnersView />;
    case 'tests':
      return <TestsView project={view.project} />;
    case 'run':
      return <RunView runId={view.id} />;
    case 'settings':
      return <SettingsView onStatus={onStatus} />;
    default:
      return <HomeView />;
  }
}

export function App() {
  const [status, setStatus] = useState<ConnectionStatus | undefined>();
  const [failure, setFailure] = useState<string | undefined>();
  const [view, go] = useView();
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
        <TopBar />
        <div className="desk-columns">
          <ThreadsColumn />
          <main className="desk-center">
            {workspace.error && !workspace.data && <ErrorNote message={workspace.error} />}
            <Center view={view} onStatus={setStatus} />
          </main>
          <WorkQueue />
        </div>
      </div>
      <Toaster />
    </SessionProvider>
  );
}
