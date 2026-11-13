// What the core says while the window is open, and data that follows it.
import { useCallback, useEffect, useRef, useState, type DependencyList } from 'react';

import type { AppEvent } from './bindings/AppEvent.ts';
import { listen } from './bridge.ts';

const handlers = new Set<(event: AppEvent) => void>();
let started = false;

function start(): void {
  if (!started) {
    started = true;
    listen((event) => handlers.forEach((handler) => handler(event)));
  }
}

/** Calls `handler` for every event of the core while the component is mounted. */
export function useLive(handler: (event: AppEvent) => void): void {
  const latest = useRef(handler);

  latest.current = handler;

  useEffect(() => {
    const relay = (event: AppEvent) => latest.current(event);

    start();
    handlers.add(relay);

    return () => {
      handlers.delete(relay);
    };
  }, []);
}

export type Remote<T> = { data: T | undefined; error: string | undefined; loading: boolean; reload: () => void; set: (value: T) => void };

/**
 * Loads something from the core and loads it again when `refreshOn` says an event concerns it.
 * What is on screen stays while the newer version is on its way.
 */
export function useRemote<T>(load: () => Promise<T>, deps: DependencyList, refreshOn?: (event: AppEvent) => boolean): Remote<T> {
  const [state, setState] = useState<{ data: T | undefined; error: string | undefined; loading: boolean }>({ data: undefined, error: undefined, loading: true });
  const request = useRef(0);
  const loader = useRef(load);
  const pending = useRef<number | undefined>(undefined);

  loader.current = load;

  const run = useCallback((keep: boolean) => {
    const mine = ++request.current;

    setState((before) => ({ data: keep ? before.data : undefined, error: keep ? before.error : undefined, loading: true }));
    loader.current().then(
      (data) => mine === request.current && setState({ data, error: undefined, loading: false }),
      (reason: unknown) => mine === request.current && setState((before) => ({ data: before.data, error: reason instanceof Error ? reason.message : String(reason), loading: false })),
    );
  }, []);

  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => run(false), deps);

  useLive((event) => {
    if (refreshOn?.(event)) {
      // Events come in bursts (an agent at work); one reload answers them all.
      window.clearTimeout(pending.current);
      pending.current = window.setTimeout(() => run(true), 120);
    }
  });

  useEffect(() => () => window.clearTimeout(pending.current), []);

  return {
    ...state,
    reload: useCallback(() => run(true), [run]),
    set: useCallback((data: T) => setState({ data, error: undefined, loading: false }), []),
  };
}

/** A value kept in the window's storage, e.g. which panel is folded. */
export function useStored<T>(key: string, initial: T): [T, (value: T) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const stored = window.localStorage.getItem(`team-hub-desktop:${key}`);

      return stored === null ? initial : (JSON.parse(stored) as T);
    } catch {
      return initial;
    }
  });

  return [
    value,
    useCallback(
      (next: T) => {
        setValue(next);

        try {
          window.localStorage.setItem(`team-hub-desktop:${key}`, JSON.stringify(next));
        } catch {
          // Without storage the choice lasts as long as the window.
        }
      },
      [key],
    ),
  ];
}
