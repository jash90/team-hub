import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type DependencyList, type ReactNode } from 'react';

import { api, errorMessage, setCsrf } from './api.ts';
import type { Bootstrap } from './bindings/Bootstrap.ts';
import type { Project } from './bindings/Project.ts';
import type { User } from './bindings/User.ts';

export type LiveEvent = {
  id: number;
  at: string;
  actorId: number | null;
  kind: string;
  projectId: number | null;
  issueId: number | null;
  payload: Record<string, unknown>;
};

type Session = {
  boot: Bootstrap;
  user: User;
  /** Loads the bootstrap again: after signing in, creating a project, switching a module. */
  refresh: () => Promise<void>;
  userById: (id: number | null | undefined) => User | undefined;
  moduleEnabled: (id: string) => boolean;
};

const SessionContext = createContext<Session | null>(null);

export function useSession(): Session {
  const session = useContext(SessionContext);

  if (!session) {
    throw new Error('useSession outside of a signed-in session');
  }

  return session;
}

export function SessionProvider({ boot, refresh, children }: { boot: Bootstrap; refresh: () => Promise<void>; children: ReactNode }) {
  const session = useMemo<Session | null>(() => {
    if (!boot.user) {
      return null;
    }

    const users = new Map(boot.users.map((user) => [user.id, user]));
    const enabled = new Set(boot.modules.filter((module) => module.enabled).map((module) => module.id));

    return {
      boot,
      user: boot.user,
      refresh,
      userById: (id) => (id === null || id === undefined ? undefined : users.get(id)),
      moduleEnabled: (id) => enabled.has(id),
    };
  }, [boot, refresh]);

  return <SessionContext.Provider value={session}>{children}</SessionContext.Provider>;
}

export async function loadBootstrap(): Promise<Bootstrap> {
  const boot = await api.get<Bootstrap>('/api/bootstrap');
  setCsrf(boot.csrf);

  return boot;
}

// ---------- live changes ----------

const liveListeners = new Set<(event: LiveEvent) => void>();
let source: EventSource | null = null;

export function startLive(): void {
  if (source) {
    return;
  }

  source = new EventSource('/api/stream');
  source.addEventListener('change', (message) => {
    try {
      const event = JSON.parse((message as MessageEvent<string>).data) as LiveEvent;

      for (const listener of liveListeners) {
        listener(event);
      }
    } catch {
      // A malformed event is skipped; the next one is independent of it.
    }
  });
}

export function stopLive(): void {
  source?.close();
  source = null;
}

/** Calls `handler` for every change announced by the server while the component is mounted. */
export function useLive(handler: (event: LiveEvent) => void): void {
  const latest = useRef(handler);
  latest.current = handler;

  useEffect(() => {
    const listener = (event: LiveEvent) => latest.current(event);
    liveListeners.add(listener);

    return () => {
      liveListeners.delete(listener);
    };
  }, []);
}

// ---------- remote data ----------

export type Remote<T> = {
  data: T | undefined;
  error: string | null;
  loading: boolean;
  reload: () => Promise<void>;
  /** Replaces the data without asking the server, e.g. with the answer to a write. */
  set: (data: T) => void;
};

/**
 * Loads data and keeps it while it reloads. With `live`, it reloads (debounced) when the
 * server announces a change the predicate accepts.
 */
export function useRemote<T>(load: () => Promise<T>, deps: DependencyList, live?: (event: LiveEvent) => boolean): Remote<T> {
  const [data, setData] = useState<T | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const generation = useRef(0);
  const loader = useRef(load);
  loader.current = load;

  const reload = useCallback(async () => {
    const mine = ++generation.current;
    setLoading(true);

    try {
      const loaded = await loader.current();

      if (mine === generation.current) {
        setData(loaded);
        setError(null);
      }
    } catch (caught) {
      if (mine === generation.current) {
        setError(errorMessage(caught));
      }
    } finally {
      if (mine === generation.current) {
        setLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    setData(undefined);
    void reload();
    // The dependency list belongs to the caller.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  const timer = useRef<number | undefined>(undefined);
  const accepts = useRef(live);
  accepts.current = live;

  useLive((event) => {
    if (!accepts.current?.(event)) {
      return;
    }

    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => void reload(), 200);
  });

  useEffect(() => () => window.clearTimeout(timer.current), []);

  return { data, error, loading, reload, set: setData };
}

// ---------- the current project ----------

const ProjectContext = createContext<{ project: Project; reload: () => Promise<void> } | null>(null);

export function ProjectProvider({ project, reload, children }: { project: Project; reload: () => Promise<void>; children: ReactNode }) {
  const value = useMemo(() => ({ project, reload }), [project, reload]);

  return <ProjectContext.Provider value={value}>{children}</ProjectContext.Provider>;
}

export function useProject(): Project {
  const context = useContext(ProjectContext);

  if (!context) {
    throw new Error('useProject outside of a project');
  }

  return context.project;
}

export function useProjectReload(): () => Promise<void> {
  const context = useContext(ProjectContext);

  return context?.reload ?? (async () => undefined);
}

const CONFIG_EVENTS = ['project.', 'sprint.', 'version.'];

/** A project with its whole configuration, reloaded when the configuration changes. */
export function useProjectData(key: string): Remote<Project> {
  const remote: Remote<Project> = useRemote(() => api.get<Project>(`/api/projects/${encodeURIComponent(key)}`), [key], (event) => {
    return remote.data !== undefined && event.projectId === remote.data.id && CONFIG_EVENTS.some((prefix) => event.kind.startsWith(prefix));
  });

  return remote;
}

/** Remembers a small piece of interface state per browser. */
export function useStored<T>(key: string, initial: T): [T, (value: T) => void] {
  const storageKey = `team-hub:${key}`;
  const [value, setValue] = useState<T>(() => {
    try {
      const stored = window.localStorage.getItem(storageKey);

      return stored === null ? initial : (JSON.parse(stored) as T);
    } catch {
      return initial;
    }
  });

  const store = useCallback(
    (next: T) => {
      setValue(next);

      try {
        window.localStorage.setItem(storageKey, JSON.stringify(next));
      } catch {
        // The choice still holds for this page view.
      }
    },
    [storageKey],
  );

  return [value, store];
}
