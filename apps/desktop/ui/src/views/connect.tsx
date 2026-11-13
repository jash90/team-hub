import { useEffect, useRef, useState, type FormEvent } from 'react';

import { Button, ErrorNote, Field, Spinner } from '@web/components/ui.tsx';
import { t } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import type { ConnectionStatus } from '../bindings/ConnectionStatus.ts';
import type { PairingView } from '../bindings/PairingView.ts';
import { inApp, openExternal, signIn, web } from '../bridge.ts';
import { useWebSlot } from '../shell/web-slot.ts';

function messageOf(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

/** `ABCD2345` as two groups, the way the server's page shows it. */
function grouped(code: string): string {
  return code.length === 8 ? `${code.slice(0, 4)}-${code.slice(4)}` : code;
}

/**
 * The request to connect is made and waits for its person: the code on this side, and the
 * server's own pages beside it, where they sign in and say yes (`docs/desktop-pairing.md`).
 */
function Pairing({ pairing, onCancel }: { pairing: PairingView; onCancel: () => void }) {
  const slot = useRef<HTMLDivElement>(null);
  const failure = useWebSlot(slot, pairing.serverUrl);

  return (
    <div className="desk-pair">
      <aside className="desk-pair-side">
        <div className="auth-brand">
          <span className="brand-mark" />
          team-hub
        </div>
        <h1>{t('Sign in and confirm')}</h1>
        <p className="muted">{inApp ? t('Sign in to {server} on the right, then confirm that the page shows this code.', { server: pairing.serverUrl }) : t('Sign in to {server} in the tab that opened, then confirm that the page shows this code.', { server: pairing.serverUrl })}</p>
        <p className="desk-pair-code">{grouped(pairing.code)}</p>
        <p className="muted">{t('Your password stays between you and the server: the app never sees it. Once you confirm, the server hands the app a token of its own, which is kept in the keychain of this computer.')}</p>
        <div className="desk-pair-waiting">
          <Spinner />
          <span className="muted">{t('Waiting for you to confirm…')}</span>
        </div>
        <div>
          <Button onClick={onCancel}>{t('Cancel')}</Button>
        </div>
      </aside>
      <div className="desk-web desk-pair-web" ref={slot}>
        {failure ? <ErrorNote message={failure} /> : inApp ? <Spinner /> : <p className="muted">{t('The page is open in a tab of your browser.')}</p>}
      </div>
    </div>
  );
}

/** The first screen: the server's address, and the e-mail address and password of an account there. */
export function ConnectView({ status, onConnected }: { status: ConnectionStatus; onConnected: (status: ConnectionStatus) => void }) {
  const [url, setUrl] = useState(status.serverUrl ?? '');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [token, setToken] = useState('');
  const [busy, setBusy] = useState(false);
  const [pairing, setPairing] = useState<PairingView | undefined>();
  const [error, setError] = useState<string | undefined>(status.serverUrl && !status.hasToken ? (status.error ?? undefined) : undefined);
  // The request this view is waiting for; an answer to one that was given up is not shown.
  const current = useRef<string | undefined>(undefined);

  // Leaving the screen gives the request up.
  useEffect(
    () => () => {
      if (current.current) {
        void core.pairCancel().catch(() => undefined);
      }
    },
    [],
  );

  async function withPassword(event: FormEvent): Promise<void> {
    event.preventDefault();
    setBusy(true);
    setError(undefined);

    try {
      const connected = await signIn<ConnectionStatus>(url, email, password);

      setPassword('');
      onConnected(connected);
    } catch (reason) {
      setError(messageOf(reason));
    } finally {
      setBusy(false);
    }
  }

  async function atTracker(): Promise<void> {
    setBusy(true);
    setError(undefined);

    try {
      const started = await core.pairStart(url);

      current.current = started.code;

      // The page is opened before its place is shown, so that it is the first page the
      // server's interface loads here.
      if (inApp) {
        await web.open(started.path);
      } else {
        await openExternal(`${started.serverUrl}${started.path}`);
      }

      setPairing(started);

      const connected = await core.pairWait();

      if (current.current === started.code) {
        current.current = undefined;
        // The page that said yes has nothing more to say.
        await web.open('/').catch(() => undefined);
        onConnected(connected);
      }
    } catch (reason) {
      // Giving up is not an error to show.
      if (current.current) {
        setError(messageOf(reason));
      }

      current.current = undefined;
      setPairing(undefined);
    } finally {
      setBusy(false);
    }
  }

  function cancel(): void {
    current.current = undefined;
    setPairing(undefined);
    void core.pairCancel().catch(() => undefined);
  }

  async function withToken(event: FormEvent): Promise<void> {
    event.preventDefault();
    setBusy(true);
    setError(undefined);

    try {
      onConnected(await core.connect(url, token));
    } catch (reason) {
      setError(messageOf(reason));
    } finally {
      setBusy(false);
    }
  }

  if (pairing) {
    return <Pairing pairing={pairing} onCancel={cancel} />;
  }

  return (
    <div className="center-screen">
      <div className="auth-card">
        <div className="auth-brand">
          <span className="brand-mark" />
          team-hub
        </div>
        <h1>{t('Connect to your team-hub')}</h1>
        <p className="muted">{t('Sign in with the account you use in the browser. The password goes to your server and is kept nowhere; the app gets a token of its own, in the keychain of this computer.')}</p>
        <form className="form" onSubmit={withPassword}>
          <Field label={t('Server address')}>
            <input className="input" placeholder="https://hub.example.com" value={url} onChange={(event) => setUrl(event.target.value)} autoFocus={!url} required />
          </Field>
          <Field label={t('E-mail')}>
            <input className="input" type="email" value={email} onChange={(event) => setEmail(event.target.value)} autoComplete="username" autoFocus={Boolean(url)} required />
          </Field>
          <Field label={t('Password')}>
            <input className="input" type="password" value={password} onChange={(event) => setPassword(event.target.value)} autoComplete="current-password" required />
          </Field>
          {error && <ErrorNote message={error} />}
          <Button variant="primary" type="submit" disabled={busy}>
            {busy ? t('Connecting…') : t('Sign in')}
          </Button>
        </form>
        <details className="desk-advanced">
          <summary>{t('Other ways to connect')}</summary>
          <div className="form">
            <p className="muted">{t('Sign in at the tracker instead: the app shows a code, you sign in on the server’s own page and confirm it there, and the app never sees your password.')}</p>
            <div>
              <Button disabled={busy || !url.trim()} onClick={() => void atTracker()}>
                {t('Sign in at the tracker')}
              </Button>
            </div>
          </div>
          <form className="form" onSubmit={withToken}>
            <p className="muted">{t('Or paste an API token, for a computer where nobody can sign in. Create one in team-hub under Settings → API tokens; it is kept in the keychain of this computer and nowhere else.')}</p>
            <Field label={t('API token')}>
              <input className="input input-mono" type="password" placeholder="th_…" value={token} onChange={(event) => setToken(event.target.value)} autoComplete="off" required />
            </Field>
            <div>
              <Button type="submit" disabled={busy || !url.trim()}>
                {t('Connect')}
              </Button>
            </div>
          </form>
        </details>
      </div>
    </div>
  );
}
