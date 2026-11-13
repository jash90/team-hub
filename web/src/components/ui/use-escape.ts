import { useEffect } from 'react';

/** Calls `onEscape` when Escape is pressed, before anything underneath sees the key. */
export function useEscape(onEscape: () => void, active = true): void {
  useEffect(() => {
    if (!active) {
      return undefined;
    }

    const handle = (event: globalThis.KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.stopPropagation();
        onEscape();
      }
    };

    window.addEventListener('keydown', handle, true);

    return () => window.removeEventListener('keydown', handle, true);
  }, [onEscape, active]);
}
