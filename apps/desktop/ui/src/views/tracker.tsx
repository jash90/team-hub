import { useLayoutEffect, useRef, useState } from 'react';

import { Button, ErrorNote, Spinner } from '@web/components/ui.tsx';
import { t } from '@web/i18n/index.ts';
import { inApp, openExternal, web } from '../bridge.ts';
import { useSession } from '../session.tsx';

/**
 * The tracker: the server's own web interface, the same pages a browser shows. The app
 * lays it over this part of the window; here is only the place for it, and what shows
 * through until its first page is there.
 */
export function TrackerView() {
  const { status, serverUrl } = useSession();
  const slot = useRef<HTMLDivElement>(null);
  const [failure, setFailure] = useState<string | undefined>();

  useLayoutEffect(() => {
    const element = slot.current;

    if (!inApp || !element) {
      return undefined;
    }

    let frame = 0;
    let gone = false;
    // Where the place is, in the window's own units, each time it moves or changes size.
    const place = () => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => {
        const box = element.getBoundingClientRect();

        web.show({ x: box.left, y: box.top, width: box.width, height: box.height }).then(
          () => !gone && setFailure(undefined),
          (reason: unknown) => !gone && setFailure(reason instanceof Error ? reason.message : String(reason)),
        );
      });
    };
    const observer = new ResizeObserver(place);

    observer.observe(element);
    window.addEventListener('resize', place);
    place();

    return () => {
      gone = true;
      observer.disconnect();
      window.removeEventListener('resize', place);
      window.cancelAnimationFrame(frame);
      void web.hide();
    };
  }, [status.serverUrl]);

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

  return (
    <div className="desk-web" ref={slot}>
      {failure ? <ErrorNote message={failure} /> : <Spinner />}
      {!failure && !status.online && <p className="muted">{t('The server cannot be reached, so its pages cannot be shown. Threads keep working on this computer.')}</p>}
    </div>
  );
}
