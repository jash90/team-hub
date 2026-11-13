import { useState, type FormEvent } from 'react';

import { Button, ErrorNote, Field } from '@web/components/ui.tsx';
import { t } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import type { ConnectionStatus } from '../bindings/ConnectionStatus.ts';

/** The first screen: the server's address and an API token. */
export function ConnectView({ status, onConnected }: { status: ConnectionStatus; onConnected: (status: ConnectionStatus) => void }) {
  const [url, setUrl] = useState(status.serverUrl ?? '');
  const [token, setToken] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | undefined>(status.serverUrl && !status.hasToken ? (status.error ?? undefined) : undefined);

  async function submit(event: FormEvent): Promise<void> {
    event.preventDefault();
    setBusy(true);
    setError(undefined);

    try {
      onConnected(await core.connect(url, token));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="center-screen">
      <form className="auth-card form" onSubmit={submit}>
        <div className="auth-brand">
          <span className="brand-mark" />
          team-hub
        </div>
        <h1>{t('Connect to your team-hub')}</h1>
        <p className="muted">{t('The token is kept in the keychain of this computer and nowhere else. Create one in team-hub under Settings → API tokens.')}</p>
        <Field label={t('Server address')}>
          <input className="input" placeholder="https://hub.example.com" value={url} onChange={(event) => setUrl(event.target.value)} autoFocus required />
        </Field>
        <Field label={t('API token')}>
          <input className="input input-mono" type="password" placeholder="th_…" value={token} onChange={(event) => setToken(event.target.value)} autoComplete="off" required />
        </Field>
        {error && <ErrorNote message={error} />}
        <Button variant="primary" type="submit" disabled={busy}>
          {busy ? t('Connecting…') : t('Connect')}
        </Button>
      </form>
    </div>
  );
}
