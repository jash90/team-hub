import { useState } from 'react';

import { api, errorMessage } from '../api.ts';
import type { DesktopPairing } from '../bindings/DesktopPairing.ts';
import { Button, ErrorNote, Spinner } from '../components/ui.tsx';
import { formatDateTime, t } from '../i18n/index.ts';
import { Link, useSearchParams } from '../router.tsx';
import { useRemote, useSession } from '../session.tsx';

/** `ABCD2345` as two groups, the way the app shows it. */
function grouped(code: string): string {
  return code.length === 8 ? `${code.slice(0, 4)}-${code.slice(4)}` : code;
}

/**
 * A desktop app asks to be connected to the account of whoever is signed in here
 * (`docs/desktop-pairing.md`). The app shows a code and so does this page; the person
 * compares them and says yes or no. The app gets a token of theirs only after a yes.
 */
export function PairView() {
  const { user } = useSession();
  const code = useSearchParams().get('code') ?? '';
  const pairing = useRemote(() => api.get<DesktopPairing>(`/api/desktop/pairings/${encodeURIComponent(code)}`), [code]);
  const [done, setDone] = useState<'approved' | 'declined' | undefined>();
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | undefined>();

  async function answer(what: 'approve' | 'decline'): Promise<void> {
    setBusy(true);
    setFailure(undefined);

    try {
      await api.post(`/api/desktop/pairings/${encodeURIComponent(code)}/${what}`);
      setDone(what === 'approve' ? 'approved' : 'declined');
    } catch (caught) {
      setFailure(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  }

  if (done) {
    return (
      <div className="view view-narrow">
        <header className="view-head">
          <div>
            <h1>{done === 'approved' ? t('The app is connected') : t('The app was not connected')}</h1>
            <p className="muted">
              {done === 'approved'
                ? t('It works as you from now on. You can take that back at any time: Settings → Profile and tokens lists it by its name.')
                : t('Nothing was given to it. To connect it after all, start again in the app.')}
            </p>
          </div>
        </header>
        <Link href="/">{t('To the start page')}</Link>
      </div>
    );
  }

  if (pairing.error && !pairing.data) {
    return (
      <div className="view view-narrow">
        <header className="view-head">
          <h1>{t('Connect the desktop app')}</h1>
        </header>
        <ErrorNote message={pairing.error} />
      </div>
    );
  }

  if (!pairing.data) {
    return <Spinner />;
  }

  return (
    <div className="view view-narrow">
      <header className="view-head">
        <div>
          <h1>{t('Connect the desktop app')}</h1>
          <p className="muted">{t('Connect the desktop app “{name}” to your account, {user}? It will work as you: read and change what you can, and run agents in your name.', { name: pairing.data.name, user: user.name })}</p>
        </div>
      </header>
      <p>{t('The app shows this code. Go on only if you started this yourself, on your own computer, a moment ago, and the codes are the same.')}</p>
      <p className="pair-code" aria-label={t('Code')}>
        {grouped(pairing.data.code)}
      </p>
      <p className="muted">{t('The app chose its own name, so the name proves nothing; the code does. This request ends {when}.', { when: formatDateTime(pairing.data.expiresAt) })}</p>
      {failure && <ErrorNote message={failure} />}
      <div className="pair-actions">
        <Button variant="primary" disabled={busy} onClick={() => void answer('approve')}>
          {t('Connect')}
        </Button>
        <Button disabled={busy} onClick={() => void answer('decline')}>
          {t('Do not connect')}
        </Button>
      </div>
    </div>
  );
}
