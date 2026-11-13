import { useEffect, useRef } from 'react';

/** What Escape closes, outermost first: a drawer, the dialog over it, the popover in that. */
const open: { close: () => void }[] = [];

window.addEventListener(
  'keydown',
  (event) => {
    const innermost = open[open.length - 1];

    if (event.key === 'Escape' && innermost) {
      event.stopPropagation();
      innermost.close();
    }
  },
  true,
);

/**
 * Calls `onEscape` when Escape is pressed, before anything underneath sees the key. Of the
 * things that are open at once, only the innermost hears it: Escape in a list of choices
 * closes the list, not the dialog whose form it belongs to.
 */
export function useEscape(onEscape: () => void, active = true): void {
  const latest = useRef(onEscape);
  latest.current = onEscape;

  useEffect(() => {
    if (!active) {
      return undefined;
    }

    // Its place is decided when it opens; a new `onEscape` on a later render does not move it.
    const entry = { close: () => latest.current() };
    open.push(entry);

    return () => {
      open.splice(open.indexOf(entry), 1);
    };
  }, [active]);
}
