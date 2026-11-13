import { useCallback, useState, useSyncExternalStore } from 'react';

import { errorMessage } from '../../api.ts';
import { Icon } from '../icon.tsx';

type Toast = { id: number; text: string; kind: 'info' | 'error' };

let toasts: Toast[] = [];
let nextToast = 1;
const toastListeners = new Set<() => void>();

function publishToasts(next: Toast[]): void {
  toasts = next;

  for (const listener of toastListeners) {
    listener();
  }
}

export function toast(text: string, kind: Toast['kind'] = 'info'): void {
  const id = nextToast++;
  publishToasts([...toasts, { id, text, kind }]);
  window.setTimeout(() => publishToasts(toasts.filter((item) => item.id !== id)), kind === 'error' ? 7000 : 3500);
}

export function Toaster() {
  const items = useSyncExternalStore(
    (listener) => {
      toastListeners.add(listener);

      return () => toastListeners.delete(listener);
    },
    () => toasts,
  );

  return (
    <div className="toaster" role="status" aria-live="polite">
      {items.map((item) => (
        <div key={item.id} className={`toast toast-${item.kind}`}>
          <Icon name={item.kind === 'error' ? 'alert' : 'check'} />
          <span>{item.text}</span>
        </div>
      ))}
    </div>
  );
}

/** Runs something that talks to the server; a failure becomes a toast instead of an unhandled rejection. */
export async function attempt<T>(work: () => Promise<T>, done?: string): Promise<T | undefined> {
  try {
    const result = await work();

    if (done) {
      toast(done);
    }

    return result;
  } catch (error) {
    toast(errorMessage(error), 'error');

    return undefined;
  }
}

/** `[busy, run]`: `run` wraps `attempt` and tells whether it is still going. */
export function useBusy(): [boolean, <T>(work: () => Promise<T>, done?: string) => Promise<T | undefined>] {
  const [busy, setBusy] = useState(false);
  const run = useCallback(async <T,>(work: () => Promise<T>, done?: string) => {
    setBusy(true);

    try {
      return await attempt(work, done);
    } finally {
      setBusy(false);
    }
  }, []);

  return [busy, run];
}
