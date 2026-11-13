import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

import { useEscape } from './use-escape.ts';

type PopoverProps = {
  anchor: HTMLElement | null;
  onClose: () => void;
  children: ReactNode;
  align?: 'start' | 'end';
  width?: number;
};

/** A panel attached to an element; closes on a click elsewhere or Escape, and stays on screen. */
export function Popover({ anchor, onClose, children, align = 'start', width }: PopoverProps) {
  const panel = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{ top: number; left: number } | null>(null);
  useEscape(onClose);

  useLayoutEffect(() => {
    if (!anchor || !panel.current) {
      return;
    }

    const box = anchor.getBoundingClientRect();
    const own = panel.current.getBoundingClientRect();
    const margin = 8;
    let left = align === 'end' ? box.right - own.width : box.left;
    let top = box.bottom + 4;

    left = Math.max(margin, Math.min(left, window.innerWidth - own.width - margin));

    if (top + own.height > window.innerHeight - margin && box.top - own.height - 4 > margin) {
      top = box.top - own.height - 4;
    }

    setPosition({ top, left });
  }, [anchor, align, children]);

  useEffect(() => {
    const handle = (event: MouseEvent) => {
      const target = event.target as Node;

      if (!panel.current?.contains(target) && !anchor?.contains(target)) {
        onClose();
      }
    };

    document.addEventListener('mousedown', handle);

    return () => document.removeEventListener('mousedown', handle);
  }, [anchor, onClose]);

  return createPortal(
    <div ref={panel} className="popover" style={{ top: position?.top ?? -9999, left: position?.left ?? -9999, width, visibility: position ? 'visible' : 'hidden' }}>
      {children}
    </div>,
    document.body,
  );
}
