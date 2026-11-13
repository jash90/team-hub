import { useEffect, useState, type ComponentType, type ReactNode } from 'react';

import { ErrorNote, Spinner } from './components/ui.tsx';
import { t } from './i18n/index.ts';

export type LazyView<P> = ((props: P) => ReactNode) & {
  /** Fetches the view's code ahead of the first render. Fails when the fetch fails. */
  preload: () => Promise<void>;
};

/**
 * A view whose code is a chunk of its own, fetched the first time the view is needed, or
 * ahead of time with `preload`. Once the code is here the view renders at once, with no
 * placeholder in between; until then it shows `fallback` (a spinner unless told otherwise).
 */
export function lazyView<P extends object>(load: () => Promise<ComponentType<P>>, fallback: ReactNode = <Spinner />): LazyView<P> {
  let loaded: ComponentType<P> | null = null;
  let pending: Promise<void> | null = null;

  function preload(): Promise<void> {
    pending ??= load().then(
      (component) => {
        loaded = component;
      },
      (error: unknown) => {
        // The next attempt asks again. A browser may remember the failure until the page
        // is reloaded, which is what the message suggests.
        pending = null;

        throw error;
      },
    );

    return pending;
  }

  function View(props: P): ReactNode {
    const [, setReady] = useState(loaded !== null);
    const [failed, setFailed] = useState(false);

    useEffect(() => {
      if (loaded) {
        return undefined;
      }

      let mounted = true;

      preload().then(
        () => mounted && setReady(true),
        () => mounted && setFailed(true),
      );

      return () => {
        mounted = false;
      };
    }, []);

    const Ready = loaded as ComponentType<P> | null;

    if (Ready) {
      return <Ready {...props} />;
    }

    return failed ? <ErrorNote message={t('This page could not be loaded. Reload to try again.')} /> : fallback;
  }

  return Object.assign(View, { preload });
}

/** Runs `work` when the browser has nothing better to do. */
export function whenIdle(work: () => void): void {
  if (typeof window.requestIdleCallback === 'function') {
    window.requestIdleCallback(work, { timeout: 4000 });
  } else {
    window.setTimeout(work, 1500);
  }
}
