// The window: the views as tabs across the top, the selected view under them, and threads
// and decisions in a column on the right. The first view is the tracker, which is the
// server's own web interface shown in the window; the others belong to this computer.
import { useCallback, useEffect, useState } from 'react';

import { Icon } from '@web/components/icon.tsx';
import { Button, ErrorNote, Spinner, Tag, Toaster, toast } from '@web/components/ui.tsx';
import { t } from '@web/i18n/index.ts';
import { core } from './api.ts';
import type { ConnectionStatus } from './bindings/ConnectionStatus.ts';
import { inApp, listenToStage, web } from './bridge.ts';
import { useLive, useRemote, useStored } from './live.ts';
import { SessionProvider, useSession, useView, type View } from './session.tsx';
import { Inspector, useThreadWork } from './shell/inspector.tsx';
import { ConnectView } from './views/connect.tsx';
import { NewThreadView } from './views/new-thread.tsx';
import { ReposView } from './views/repos.tsx';
import { RunView } from './views/run.tsx';
import { SettingsView } from './views/settings.tsx';
import { ThreadView } from './views/thread.tsx';
import { ThreadsView } from './views/threads.tsx';
import { TrackerView } from './views/tracker.tsx';

function TabBar({ panel, onPanel }: { panel: boolean; onPanel: (shown: boolean) => void }) {
  const { status, go, view } = useSession();
  const work = useThreadWork();
  const tracker = view.name === 'web';
  const threads = view.name === 'threads' || view.name === 'thread' || view.name === 'new-thread' || view.name === 'run';
  const tab = (on: boolean) => (on ? 'tab tab-active' : 'tab');

  return (
    <header className="desk-tabbar">
      <nav className="tabs desk-tabs" aria-label={t('Views')}>
        <button type="button" className={tab(tracker)} onClick={() => go({ name: 'web' })}>
          <Icon name="board" size={15} /> {t('Tracker')}
        </button>
        <button type="button" className={tab(threads)} onClick={() => go({ name: 'threads' })}>
          <Icon name="bot" size={15} /> {t('Threads')}
          {work.waiting > 0 && <Tag tone="amber">{work.waiting}</Tag>}
        </button>
        <button type="button" className={tab(view.name === 'repos')} onClick={() => go({ name: 'repos' })}>
          <Icon name="folder" size={15} /> {t('Repositories')}
        </button>
      </nav>
      <span className="desk-grow" />
      {tracker && inApp && (
        // A page of the server has no browser around it here: these stand in for its buttons.
        <span className="desk-web-steps">
          <Button size="sm" variant="ghost" icon="chevronLeft" onClick={() => void web.go('back')} aria-label={t('Back')} title={t('Back')} />
          <Button size="sm" variant="ghost" icon="chevronRight" onClick={() => void web.go('forward')} aria-label={t('Forward')} title={t('Forward')} />
          <Button size="sm" variant="ghost" icon="refresh" onClick={() => void web.go('reload')} aria-label={t('Reload')} title={t('Reload')} />
        </span>
      )}
      {status.online ? (
        <span className="desk-status muted" title={status.serverUrl ?? undefined}>
          <span className="desk-dot" />
          {status.runner.enabled && status.runner.runsApi !== false ? status.runner.name : t('online')}
        </span>
      ) : (
        <Tag tone="amber" title={status.error ?? status.serverUrl ?? undefined}>
          {t('offline')}
        </Tag>
      )}
      <Button
        size="sm"
        variant="ghost"
        icon="menu"
        onClick={() => onPanel(!panel)}
        aria-pressed={panel}
        aria-label={panel ? t('Hide threads') : t('Show threads')}
        title={panel ? t('Hide threads') : t('Show threads')}
      />
      <Button size="sm" variant={view.name === 'settings' ? 'default' : 'ghost'} icon="settings" onClick={() => go({ name: 'settings' })} aria-label={t('Settings')} title={t('Settings')} />
    </header>
  );
}

function Center({ view, onStatus }: { view: View; onStatus: (status: ConnectionStatus) => void }) {
  switch (view.name) {
    case 'threads':
      return <ThreadsView />;
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
        <TabBar panel={panel} onPanel={setPanel} />
        <div className="desk-body">
          {view.name === 'web' ? (
            <main className="desk-main desk-main-web">
              <TrackerView />
            </main>
          ) : (
            <main className="desk-main">
              {workspace.error && !workspace.data && <ErrorNote message={workspace.error} />}
              <Center view={view} onStatus={setStatus} />
            </main>
          )}
          {panel && <Inspector />}
        </div>
      </div>
      <Toaster />
    </SessionProvider>
  );
}
