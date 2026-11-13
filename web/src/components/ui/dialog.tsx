import { useEffect, useRef, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

import { t } from '../../i18n/index.ts';
import { Button } from './button.tsx';
import { useEscape } from './use-escape.ts';

export function Dialog({ title, onClose, children, footer, wide }: { title: string; onClose: () => void; children: ReactNode; footer?: ReactNode; wide?: boolean }) {
  const panel = useRef<HTMLDivElement>(null);
  useEscape(onClose);

  useEffect(() => {
    const first = panel.current?.querySelector<HTMLElement>('[autofocus], input:not([type=hidden]), textarea, select');
    first?.focus();
  }, []);

  return createPortal(
    <div className="overlay" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className={wide ? 'dialog dialog-wide' : 'dialog'} role="dialog" aria-modal="true" aria-label={title} ref={panel}>
        <header className="dialog-head">
          <h2>{title}</h2>
          <Button variant="ghost" icon="x" onClick={onClose} aria-label={t('Close')} />
        </header>
        <div className="dialog-body">{children}</div>
        {footer && <footer className="dialog-foot">{footer}</footer>}
      </div>
    </div>,
    document.body,
  );
}
