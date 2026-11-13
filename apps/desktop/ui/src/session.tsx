// Who is connected to which server, where the person is in the app, and how to go elsewhere.
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';

import type { Project } from '@web/bindings/Project.ts';
import type { User } from '@web/bindings/User.ts';
import type { ConnectionStatus } from './bindings/ConnectionStatus.ts';
import type { WorkspaceView } from './bindings/WorkspaceView.ts';

export type View =
  | { name: 'home' }
  | { name: 'issue'; key: string }
  | { name: 'thread'; id: number }
  | { name: 'new-thread'; key?: string }
  | { name: 'search'; query: string }
  | { name: 'repos' }
  | { name: 'runners' }
  | { name: 'tests'; project?: string }
  | { name: 'run'; id: number }
  | { name: 'settings' };

function toHash(view: View): string {
  switch (view.name) {
    case 'issue':
      return `#/issue/${encodeURIComponent(view.key)}`;
    case 'thread':
      return `#/thread/${view.id}`;
    case 'new-thread':
      return view.key ? `#/new-thread/${encodeURIComponent(view.key)}` : '#/new-thread';
    case 'search':
      return `#/search/${encodeURIComponent(view.query)}`;
    case 'repos':
      return '#/repos';
    case 'runners':
      return '#/runners';
    case 'tests':
      return view.project ? `#/tests/${encodeURIComponent(view.project)}` : '#/tests';
    case 'run':
      return `#/run/${view.id}`;
    case 'settings':
      return '#/settings';
    default:
      return '#/';
  }
}

function fromHash(hash: string): View {
  const [, name, rest] = hash.split('/');
  const value = rest ? decodeURIComponent(rest) : '';

  switch (name) {
    case 'issue':
      return value ? { name: 'issue', key: value } : { name: 'home' };
    case 'thread':
      return Number.isInteger(Number(value)) && value ? { name: 'thread', id: Number(value) } : { name: 'home' };
    case 'new-thread':
      return { name: 'new-thread', key: value || undefined };
    case 'search':
      return { name: 'search', query: value };
    case 'repos':
      return { name: 'repos' };
    case 'runners':
      return { name: 'runners' };
    case 'tests':
      return { name: 'tests', project: value || undefined };
    case 'run':
      return Number.isInteger(Number(value)) && value ? { name: 'run', id: Number(value) } : { name: 'home' };
    case 'settings':
      return { name: 'settings' };
    default:
      return { name: 'home' };
  }
}

/** The view named by the address, and a way to go to another one. */
export function useView(): [View, (view: View) => void] {
  const [view, setView] = useState<View>(() => fromHash(window.location.hash));

  useEffect(() => {
    const changed = () => setView(fromHash(window.location.hash));

    window.addEventListener('hashchange', changed);

    return () => window.removeEventListener('hashchange', changed);
  }, []);

  return [
    view,
    useCallback((next: View) => {
      window.location.hash = toHash(next);
    }, []),
  ];
}

type Session = {
  status: ConnectionStatus;
  workspace: WorkspaceView | undefined;
  me: User | null;
  userById: (id: number | null | undefined) => User | undefined;
  go: (view: View) => void;
  view: View;
  /** The server's address of an issue or a page, to open in the browser. */
  serverUrl: (path: string) => string | null;
};

const SessionContext = createContext<Session | null>(null);

export function useSession(): Session {
  const session = useContext(SessionContext);

  if (!session) {
    throw new Error('useSession outside the session');
  }

  return session;
}

export function SessionProvider({ status, workspace, view, go, children }: { status: ConnectionStatus; workspace: WorkspaceView | undefined; view: View; go: (view: View) => void; children: ReactNode }) {
  const session = useMemo<Session>(() => {
    const users = new Map((workspace?.bootstrap.users ?? []).map((user) => [user.id, user]));

    return {
      status,
      workspace,
      me: status.user,
      userById: (id) => (id == null ? undefined : users.get(id)),
      go,
      view,
      serverUrl: (path) => (status.serverUrl ? `${status.serverUrl}${path}` : null),
    };
  }, [status, workspace, view, go]);

  return <SessionContext.Provider value={session}>{children}</SessionContext.Provider>;
}

/** The status and the type of an issue's row, from its project's configuration. */
export function partsOf(projects: Project[], row: { projectId: number; statusId: number; typeId: number }) {
  const project = projects.find((candidate) => candidate.id === row.projectId);

  return {
    project,
    status: project?.statuses.find((status) => status.id === row.statusId),
    type: project?.types.find((type) => type.id === row.typeId),
  };
}
