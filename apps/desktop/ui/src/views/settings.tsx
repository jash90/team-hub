import { useEffect, useState } from 'react';

import { Button, ConfirmDialog, ErrorNote, Field, Spinner, Tag, useBusy } from '@web/components/ui.tsx';
import { currentLanguage, setLanguage, t, timeAgo } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import { web } from '../bridge.ts';
import type { ConnectionStatus } from '../bindings/ConnectionStatus.ts';
import type { Settings } from '../bindings/Settings.ts';
import { useRemote } from '../live.ts';
import { useSession } from '../session.tsx';

/** The language and the theme are chosen here, once; the tracker in the window follows. */
function Appearance() {
  const [theme, setTheme] = useState(() => document.documentElement.dataset.theme ?? (window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'));

  function choose(next: 'light' | 'dark'): void {
    document.documentElement.dataset.theme = next;
    setTheme(next);
    void web.set('theme', next).catch(() => undefined);

    try {
      window.localStorage.setItem('team-hub:theme', next);
    } catch {
      // The theme still applies to this window.
    }
  }

  function speak(next: 'en' | 'pl'): void {
    if (next !== currentLanguage()) {
      // The tracker follows; told first, because the app's own interface reloads next.
      void web
        .set('language', next)
        .catch(() => undefined)
        .then(() => setLanguage(next));
    }
  }

  return (
    <section className="panel settings-card">
      <header className="panel-head">
        <h2>{t('Appearance')}</h2>
      </header>
      <div className="list-row">
        <span className="desk-setting-name">{t('Language')}</span>
        <Button size="sm" variant={currentLanguage() === 'pl' ? 'primary' : 'default'} onClick={() => speak('pl')}>
          Polski
        </Button>
        <Button size="sm" variant={currentLanguage() === 'en' ? 'primary' : 'default'} onClick={() => speak('en')}>
          English
        </Button>
      </div>
      <div className="list-row">
        <span className="desk-setting-name">{t('Theme')}</span>
        <Button size="sm" variant={theme === 'light' ? 'primary' : 'default'} icon="sun" onClick={() => choose('light')}>
          {t('Light')}
        </Button>
        <Button size="sm" variant={theme === 'dark' ? 'primary' : 'default'} icon="moon" onClick={() => choose('dark')}>
          {t('Dark')}
        </Button>
      </div>
    </section>
  );
}

export function SettingsView({ onStatus }: { onStatus: (status: ConnectionStatus) => void }) {
  const { status } = useSession();
  const loaded = useRemote(() => core.settings(), []);
  const models = useRemote(() => core.models(), []);
  const [draft, setDraft] = useState<Settings | undefined>();
  const [leaving, setLeaving] = useState(false);
  const [busy, run] = useBusy();

  useEffect(() => setDraft(loaded.data), [loaded.data]);

  if (!draft) {
    return loaded.error ? <ErrorNote message={loaded.error} /> : <Spinner />;
  }

  const change = <K extends keyof Settings>(name: K, value: Settings[K]) => setDraft({ ...draft, [name]: value });
  const dirty = JSON.stringify(draft) !== JSON.stringify(loaded.data);

  async function save(): Promise<void> {
    const saved = await run(() => core.saveSettings(draft as Settings), t('Saved'));

    if (saved) {
      loaded.set(saved);
      onStatus(await core.status());
    }
  }

  return (
    <div className="desk-view">
      <h1>{t('Settings')}</h1>
      <Appearance />
      <section className="panel settings-card">
        <header className="panel-head">
          <h2>{t('Connection')}</h2>
          <Tag tone={status.online ? 'green' : 'amber'}>{status.online ? t('online') : t('offline')}</Tag>
        </header>
        <p>
          <code>{status.serverUrl}</code> {status.user && <span className="muted">· {status.user.name}</span>} {status.serverVersion && <span className="muted">· team-hub {status.serverVersion}</span>}
        </p>
        <p className="muted">
          {status.cache === 'encrypted'
            ? t('The copy of the server’s data on this computer is encrypted; its key and the API token are in the system keychain.')
            : t('This computer has no keychain the app can use, so the copy of the server’s data is not kept between runs.')}
        </p>
        {status.error && <ErrorNote message={status.error} />}
        <div>
          <Button variant="danger" onClick={() => setLeaving(true)}>
            {t('Disconnect')}
          </Button>
        </div>
      </section>
      <section className="panel settings-card">
        <header className="panel-head">
          <h2>{t('Runner')}</h2>
          {status.runner.runsApi === false && <Tag tone="amber">{t('this server has no agent runs')}</Tag>}
          {status.runner.lastHeartbeat && <span className="muted">{t('last seen by the server {when}', { when: timeAgo(status.runner.lastHeartbeat) })}</span>}
        </header>
        <p className="muted">{t('As a runner this computer takes the work delegated to an AI agent on the web, for projects that have a repository mapped here, while the app is open.')}</p>
        <label className="check">
          <input type="checkbox" checked={draft.runnerEnabled} onChange={(event) => change('runnerEnabled', event.target.checked)} />
          <span>{t('Take runs delegated on the web')}</span>
        </label>
        <div className="form-row">
          <Field label={t('Name shown on the web')}>
            <input className="input" value={draft.runnerName} onChange={(event) => change('runnerName', event.target.value)} />
          </Field>
          <Field label={t('Threads working at once')} hint={t('The rest wait their turn.')}>
            <input className="input input-number" type="number" min={1} max={16} value={draft.maxRunning} onChange={(event) => change('maxRunning', Number(event.target.value))} />
          </Field>
        </div>
        {status.runner.error && <ErrorNote message={status.runner.error} />}
      </section>
      <section className="panel settings-card">
        <header className="panel-head">
          <h2>{t('Agent')}</h2>
          <Tag tone={status.sandbox === 'available' ? 'green' : 'amber'}>{status.sandbox === 'available' ? t('shell sandbox available') : t('no shell sandbox here')}</Tag>
        </header>
        <div className="form-row">
          <Field label={t('Default model')}>
            <input className="input" list="desk-models" value={draft.defaultModel} onChange={(event) => change('defaultModel', event.target.value)} />
          </Field>
          <Field label={t('Turns per run')}>
            <input className="input input-number" type="number" min={1} max={500} value={draft.maxTurns} onChange={(event) => change('maxTurns', Number(event.target.value))} />
          </Field>
          <Field label={t('Node command')} hint={t('Runs the helper that talks to the Claude Agent SDK. Takes effect after a restart.')}>
            <input className="input input-mono" value={draft.nodeCommand} onChange={(event) => change('nodeCommand', event.target.value)} />
          </Field>
        </div>
        <datalist id="desk-models">
          {(models.data ?? []).map((name) => (
            <option key={name} value={name} />
          ))}
        </datalist>
        <label className="check">
          <input type="checkbox" checked={draft.testJobs} onChange={(event) => change('testJobs', event.target.checked)} />
          <span>{t('Run test suites on this computer')}</span>
        </label>
        <label className="check">
          <input type="checkbox" checked={draft.testJobsFromQueue} disabled={!draft.testJobs} onChange={(event) => change('testJobsFromQueue', event.target.checked)} />
          <span>{t('Also take the team’s queued test jobs')}</span>
        </label>
        <p className="muted">
          {t('Running a suite is running a repository’s code on this computer, so by default it happens only for the jobs you send here by name. A job gets a folder of its own and only the variables its project’s test settings list; where the system has a sandbox it can write nowhere else and cannot read your keys.')}
        </p>
        <label className="check">
          <input type="checkbox" checked={draft.allowUnsandboxedShell} onChange={(event) => change('allowUnsandboxedShell', event.target.checked)} />
          <span>{t('Give threads a shell even where the system cannot confine it')}</span>
        </label>
        <p className="muted">
          {t('Where the operating system has a sandbox, a thread’s shell has no network and can write only inside its working tree. Where it has none, threads work without a shell unless you allow it here; then only the deny list stands between the agent and this computer. Takes effect after a restart.')}
        </p>
      </section>
      <div className="row-end">
        <span className="muted">team-hub desktop {status.appVersion}</span>
        <Button variant="primary" disabled={!dirty || busy} onClick={save}>
          {t('Save')}
        </Button>
      </div>
      {leaving && (
        <ConfirmDialog
          title={t('Disconnect from the server?')}
          body={t('The token is removed from the keychain, and on the server too when the app got it by signing in here. The stored copy of the server’s data is deleted. Threads, working trees and repositories stay.')}
          confirm={t('Disconnect')}
          danger
          onConfirm={async () => {
            const status = await core.disconnect();

            // The tracker's sign-in on this computer goes with the connection.
            await web.forget().catch(() => undefined);
            onStatus(status);
          }}
          onClose={() => setLeaving(false)}
        />
      )}
    </div>
  );
}
