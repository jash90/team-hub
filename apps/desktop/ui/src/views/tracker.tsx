import { useEffect, useRef, useState } from 'react';

import { Button, ErrorNote, Spinner } from '@web/components/ui.tsx';
import { t } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import { inApp, openExternal, web } from '../bridge.ts';
import { useSession } from '../session.tsx';
import { useWebSlot } from '../shell/web-slot.ts';

/**
 * The tracker: the server's own web interface, the same pages a browser shows. The app
 * lays it over this part of the window; here is only the place for it, and what shows
 * through until its first page is there.
 */
export function TrackerView() {
  const { status, serverUrl } = useSession();
  const slot = useRef<HTMLDivElement>(null);
  // Without the server its interface has nothing to show but the engine's own error page,
  // so it is taken off the window and this view says what is going on instead.
  const failure = useWebSlot(slot, status.serverUrl, status.online);
  const wasOnline = useRef(status.online);
  const [asking, setAsking] = useState(false);

  // Back again: the page that could not load, or that lost its live connection, loads anew.
  useEffect(() => {
    if (status.online && !wasOnline.current) {
      void web.go('reload').catch(() => undefined);
    }

    wasOnline.current = status.online;
  }, [status.online]);

  // Asking the server anything tells the core whether it is there; the core then says so
  // to the whole window, and this view follows.
  function retry(): void {
    setAsking(true);
    core
      .workspace()
      .catch(() => undefined)
      .finally(() => setAsking(false));
  }

  if (!inApp) {
    const address = serverUrl('/');

    return (
      <div className="desk-web desk-web-note">
        <h1>{t('Tracker')}</h1>
        <p className="muted">{t('Inside the app this place shows the web interface of your team-hub: board, backlog, issues, the knowledge base. A browser cannot show one site inside another, so open it in a tab.')}</p>
        {address && (
          <div>
            <Button variant="primary" icon="external" onClick={() => void openExternal(address)}>
              {t('Open team-hub in a tab')}
            </Button>
          </div>
        )}
      </div>
    );
  }

  if (!status.online) {
    return (
      <div className="desk-web desk-web-note" ref={slot}>
        <h1>{t('The server cannot be reached')}</h1>
        <p className="muted">{t('The server cannot be reached, so its pages cannot be shown. Threads keep working on this computer.')}</p>
        {status.error && <ErrorNote message={status.error} />}
        <div>
          <Button variant="primary" icon="refresh" disabled={asking} onClick={retry}>
            {asking ? t('Connecting…') : t('Try again')}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="desk-web" ref={slot}>
      {failure ? <ErrorNote message={failure} /> : <Spinner />}
    </div>
  );
}
