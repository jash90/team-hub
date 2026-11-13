// The one place that knows where the core is. Inside the app's window it is the Tauri command
// `hub`; in a browser (development) it is the core's development server, which runs the very
// same commands (`hub-dev-server`). Nothing else in the interface talks to anything.
import type { AppEvent } from './bindings/AppEvent.ts';

const IN_APP = '__TAURI_INTERNALS__' in window;
const DEV_SERVER = (import.meta.env.VITE_HUB_DEV_SERVER as string | undefined) ?? 'http://127.0.0.1:4455';

export const inApp = IN_APP;

/** Runs a command of the core. Rejects with a message for the person. */
export async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (IN_APP) {
    const { invoke } = await import('@tauri-apps/api/core');

    try {
      return await invoke<T>('hub', { command, args: args ?? null });
    } catch (reason) {
      throw new Error(typeof reason === 'string' ? reason : String(reason));
    }
  }

  const response = await fetch(`${DEV_SERVER}/invoke`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ command, args: args ?? null }),
  }).catch(() => {
    throw new Error(`The development server of the core is not running at ${DEV_SERVER}.`);
  });
  const answer = (await response.json()) as { ok: true; value: T } | { ok: false; error: string };

  if (!answer.ok) {
    throw new Error(answer.error);
  }

  return answer.value;
}

/** Calls `handler` for every event of the core. Returns how to stop. */
export function listen(handler: (event: AppEvent) => void): () => void {
  let stopped = false;

  if (IN_APP) {
    let unlisten: (() => void) | undefined;

    void import('@tauri-apps/api/event').then(async ({ listen: listenTo }) => {
      unlisten = await listenTo<AppEvent>('hub', (message) => handler(message.payload));

      if (stopped) {
        unlisten();
      }
    });

    return () => {
      stopped = true;
      unlisten?.();
    };
  }

  const poll = async () => {
    // Without a position the server answers with where it is now: what happened before
    // this page was opened is not news.
    let after: number | undefined;

    while (!stopped) {
      try {
        const response = await fetch(after === undefined ? `${DEV_SERVER}/events` : `${DEV_SERVER}/events?after=${after}`);
        const batch = (await response.json()) as { last: number; events: AppEvent[] };

        after = batch.last;
        batch.events.forEach(handler);
      } catch {
        await new Promise((resolve) => setTimeout(resolve, 2000));
      }
    }
  };

  void poll();

  return () => {
    stopped = true;
  };
}

/** What the window itself tells the local interface (`src-tauri/src/stage.rs`). */
export type StageEvent = {
  /** A page of the server asked for the thread of this run to be shown. */
  openRun?: number;
};

/** Calls `handler` for what the window says. In a browser the window says nothing. */
export function listenToStage(handler: (event: StageEvent) => void): () => void {
  if (!IN_APP) {
    return () => undefined;
  }

  let stopped = false;
  let unlisten: (() => void) | undefined;

  void import('@tauri-apps/api/event').then(async ({ listen: listenTo }) => {
    unlisten = await listenTo<StageEvent>('stage', (message) => handler(message.payload));

    if (stopped) {
      unlisten();
    }
  });

  return () => {
    stopped = true;
    unlisten?.();
  };
}

/** A command of the window itself, not of the core. In a browser there is no such thing. */
async function shell<T>(command: string, args?: Record<string, unknown>): Promise<T | undefined> {
  if (!IN_APP) {
    return undefined;
  }

  const { invoke } = await import('@tauri-apps/api/core');

  try {
    return await invoke<T>(command, args);
  } catch (reason) {
    throw new Error(typeof reason === 'string' ? reason : String(reason));
  }
}

/**
 * The server's web interface, which the app shows in the same window, over the place the
 * local interface gives it (`src-tauri/src/stage.rs`). It is told where to be and which
 * page to show; it cannot call anything here.
 */
export const web = {
  /** Shows it over this rectangle of the window. */
  show: (box: { x: number; y: number; width: number; height: number }) => shell<null>('web_show', box),
  hide: () => shell<null>('web_hide'),
  /** Opens a page of the server, such as `/issue/WEB-12`. */
  open: (path: string) => shell<null>('web_open', { path }),
  /** The page it shows now, as a path; null when it shows none. */
  location: async () => (await shell<string | null>('web_location')) ?? null,
  go: (step: 'back' | 'forward' | 'reload') => shell<null>('web_go', { step }),
  /** The language and the theme chosen in the app's own bar apply to it too. */
  set: (what: 'language' | 'theme', value: string) => shell<null>('web_set', { what, value }),
  /** Closes it and forgets its sign-in. */
  forget: () => shell<null>('web_forget'),
};

/**
 * Signs in with the e-mail address and password of an account. Inside the app the window
 * does it, so that the tracker it shows is signed in by the same sign-in; in a browser
 * the core does, and the tracker is a tab that signs in for itself.
 */
export async function signIn<T>(url: string, email: string, password: string): Promise<T> {
  const args = { url, email, password };

  return IN_APP ? ((await shell<T>('sign_in', args)) as T) : call<T>('sign_in', args);
}

/** Lets the person pick a folder. In a browser there is no folder dialog, so it asks for the path. */
export async function pickFolder(title: string): Promise<string | null> {
  if (IN_APP) {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const picked = await open({ directory: true, multiple: false, title });

    return typeof picked === 'string' ? picked : null;
  }

  return window.prompt(title);
}

/** Opens an address in the person's browser. Only http(s). */
export async function openExternal(url: string): Promise<void> {
  if (!/^https?:\/\//.test(url)) {
    return;
  }

  if (IN_APP) {
    const { openUrl } = await import('@tauri-apps/plugin-opener');

    await openUrl(url);

    return;
  }

  window.open(url, '_blank', 'noopener,noreferrer');
}
