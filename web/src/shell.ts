import { useEffect, useSyncExternalStore } from 'react';

// The shell reads the current project from the address (`/p/:key/…`, `/issue/KEY-1`). A page
// that lives elsewhere but belongs to a project (a knowledge-base space bound to one) says
// so here, and the shell shows that project's navigation instead of the last one used.

let claimed: string | null = null;
const listeners = new Set<() => void>();

function claim(key: string | null): void {
  if (claimed === key) {
    return;
  }

  claimed = key;

  for (const listener of listeners) {
    listener();
  }
}

/**
 * Tells the shell which project the page on screen belongs to. Pass the project's key, or
 * null while it is not known or when the page belongs to none.
 */
export function useShellProject(key: string | null | undefined): void {
  useEffect(() => {
    if (!key) {
      return undefined;
    }

    const wanted = key.toUpperCase();
    claim(wanted);

    return () => {
      if (claimed === wanted) {
        claim(null);
      }
    };
  }, [key]);
}

/** For the shell: the project the page on screen says it belongs to. */
export function useClaimedProject(): string | null {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);

      return () => listeners.delete(listener);
    },
    () => claimed,
  );
}
