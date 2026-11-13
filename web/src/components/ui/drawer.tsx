import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';

import { useEscape } from './use-escape.ts';

export function Drawer({ onClose, children, label }: { onClose: () => void; children: ReactNode; label: string }) {
  useEscape(onClose);

  return createPortal(
    <div className="overlay overlay-side" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <aside className="drawer" role="dialog" aria-modal="true" aria-label={label}>
        {children}
      </aside>
    </div>,
    document.body,
  );
}
