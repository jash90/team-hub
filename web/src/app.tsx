import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';

import { api, errorMessage } from './api.ts';
import type { Bootstrap } from './bindings/Bootstrap.ts';
import type { Notification } from './bindings/Notification.ts';
import type { QuickHit } from './bindings/QuickHit.ts';
import { Icon, type IconName } from './components/icon.tsx';
import { Avatar, Button, Drawer, Empty, ErrorNote, Menu, Popover, Spinner, Toaster, attempt, toast } from './components/ui.tsx';
import { currentLanguage, setLanguage, t, timeAgo } from './i18n/index.ts';
import { translateServer } from './i18n/server.ts';
import { whenIdle } from './lazy.tsx';
import { loadModules, useModules, type NavEntry } from './modules.ts';
import { Link, matchRoute, navigate, setParam, usePath, useSearchParams, type Params } from './router.tsx';
import { ProjectProvider, SessionProvider, loadBootstrap, startLive, stopLive, useLive, useProjectData, useRemote, useSession, useStored } from './session.tsx';
import { useClaimedProject } from './shell.ts';
import { LoginView, SetupView } from './views/auth.tsx';
import {
  BacklogView,
  BoardView,
  CreateIssueDialog,
  DashboardView,
  IssueView,
  ProjectSettingsView,
  ProjectsView,
  ReleasesView,
  RunnersView,
  ScheduleView,
  SearchView,
  SettingsView,
  preloadViews,
} from './views/lazy.ts';

export function App() {
  const [boot, setBoot] = useState<Bootstrap | null>(null);
  const [error, setError] = useState<string | null>(null);
  const latest = useRef(0);
  const refresh = useCallback(async () => {
    const mine = ++latest.current;
    // The code of the page in the address is fetched next to the bootstrap, not after it.
    const view = preloadCurrentView();
    const next = await loadBootstrap();

    if (next.user) {
      // The shell is shown once what it paints first is here: the screens of the modules
      // that are switched on (fetched side by side) and the page in the address. This is
      // also how a module switched on later gets its screens without a reload.
      const [failed] = await Promise.all([loadModules(next), view]);

      if (failed.length > 0) {
        toast(t('Some screens could not be loaded. Reload to try again.'), 'error');
      }
    }

    // An older answer that arrives late does not replace a newer one.
    if (mine === latest.current) {
      setBoot(next);
    }
  }, []);

  useEffect(() => {
    refresh().catch((caught) => setError(errorMessage(caught)));
  }, [refresh]);

  // The bootstrap lists the projects and what is switched off in each. When somebody else
  // changes a project, this browser asks again instead of waiting for its next reload.
  const again = useRef<number | undefined>(undefined);

  useLive((event) => {
    if (event.kind.startsWith('project.') || event.kind === 'instance.modules') {
      window.clearTimeout(again.current);
      again.current = window.setTimeout(() => void refresh().catch(() => undefined), 300);
    }
  });

  useEffect(() => () => window.clearTimeout(again.current), []);

  useEffect(() => {
    if (boot?.user) {
      startLive();

      return stopLive;
    }

    return undefined;
  }, [boot?.user?.id]);

  if (error) {
    return (
      <div className="center-screen">
        <ErrorNote message={error} />
      </div>
    );
  }

  if (!boot) {
    return (
      <div className="center-screen">
        <Spinner />
      </div>
    );
  }

  if (boot.setupRequired) {
    return (
      <>
        <SetupView onDone={refresh} tokenRequired={boot.setupTokenRequired} />
        <Toaster />
      </>
    );
  }

  if (!boot.user) {
    return (
      <>
        <LoginView onDone={refresh} />
        <Toaster />
      </>
    );
  }

  return (
    <SessionProvider boot={boot} refresh={refresh}>
      <Shell />
      <Toaster />
    </SessionProvider>
  );
}

type Route = { pattern: string; render: (params: Params) => ReactNode };

function ProjectScope({ projectKey, children }: { projectKey: string; children: ReactNode }) {
  const remote = useProjectData(projectKey);

  if (remote.error && !remote.data) {
    return <ErrorNote message={remote.error} />;
  }

  if (!remote.data) {
    return <Spinner />;
  }

  return (
    <ProjectProvider project={remote.data} reload={remote.reload}>
      {children}
    </ProjectProvider>
  );
}

const inProject = (view: ReactNode) => (found: Params) => <ProjectScope projectKey={found.key.toUpperCase()}>{view}</ProjectScope>;

/** The pages of the tracker itself, each with the view whose code it needs. */
const CORE: (Route & { view: { preload: () => Promise<void> } })[] = [
  { pattern: '/', view: DashboardView, render: () => <DashboardView /> },
  { pattern: '/projects', view: ProjectsView, render: () => <ProjectsView /> },
  { pattern: '/search', view: SearchView, render: () => <SearchView /> },
  { pattern: '/issue/:key', view: IssueView, render: (found) => <IssueView issueKey={found.key.toUpperCase()} mode="page" /> },
  { pattern: '/runners', view: RunnersView, render: () => <RunnersView /> },
  { pattern: '/settings/:tab?', view: SettingsView, render: (found) => <SettingsView tab={found.tab} /> },
  { pattern: '/p/:key/board', view: BoardView, render: inProject(<BoardView />) },
  { pattern: '/p/:key/backlog', view: BacklogView, render: inProject(<BacklogView />) },
  { pattern: '/p/:key/issues', view: SearchView, render: inProject(<SearchView scoped />) },
  { pattern: '/p/:key/releases', view: ReleasesView, render: inProject(<ReleasesView />) },
  { pattern: '/p/:key/schedule', view: ScheduleView, render: inProject(<ScheduleView />) },
  { pattern: '/p/:key/settings/:tab?', view: ProjectSettingsView, render: (found) => <ProjectScope projectKey={found.key.toUpperCase()}><ProjectSettingsView tab={found.tab} /></ProjectScope> },
];

/** Fetches the code of the page in the address (and of the issue open over it), so the first paint has it. */
function preloadCurrentView(): Promise<unknown> {
  const wanted = [CORE.find((route) => matchRoute(route.pattern, window.location.pathname))?.view];

  if (new URLSearchParams(window.location.search).has('issue')) {
    wanted.push(IssueView);
  }

  // A view that cannot be fetched says so itself when it is shown.
  return Promise.all(wanted.map((view) => view?.preload().catch(() => undefined)));
}

function projectKeyOf(path: string): string | null {
  const inProject = matchRoute('/p/:key/*', path) ?? matchRoute('/p/:key', path);

  if (inProject) {
    return inProject.key.toUpperCase();
  }

  const issue = matchRoute('/issue/:key', path);

  return issue ? issue.key.split('-')[0].toUpperCase() : null;
}

function NavLink({ entry, path }: { entry: NavEntry; path: string }) {
  const active = entry.path === '/' ? path === '/' : path === entry.path || path.startsWith(`${entry.path}/`);

  return (
    <Link href={entry.path} className={active ? 'nav-link nav-active' : 'nav-link'}>
      <Icon name={entry.icon} />
      <span>{entry.label()}</span>
    </Link>
  );
}

function Shell() {
  const { boot, user } = useSession();
  const modules = useModules();
  const path = usePath();
  const params = useSearchParams();
  const openKey = params.get('issue');
  const [creating, setCreating] = useState(false);
  const [lastProject, setLastProject] = useStored<string | null>('last-project', null);
  const [collapsed, setCollapsed] = useStored('nav-collapsed', false);
  // The address names the project, or else the page on screen may (see `shell.ts`).
  const claimedProject = useClaimedProject();
  const pathProject = projectKeyOf(path) ?? claimedProject;
  const projectKey = [pathProject, lastProject, boot.projects.find((project) => !project.archived)?.key ?? null].find(
    (key) => key && boot.projects.some((project) => project.key === key),
  );
  const project = boot.projects.find((candidate) => candidate.key === projectKey);
  const projectModules = useModules(project?.key);

  useEffect(() => {
    if (pathProject && pathProject !== lastProject && boot.projects.some((candidate) => candidate.key === pathProject)) {
      setLastProject(pathProject);
    }
  }, [pathProject, lastProject, boot.projects, setLastProject]);

  useEffect(() => {
    const handle = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement;
      const typing = target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName);

      if (typing || event.metaKey || event.ctrlKey || event.altKey) {
        return;
      }

      if (event.key === 'c') {
        event.preventDefault();
        setCreating(true);
      } else if (event.key === '/') {
        event.preventDefault();
        document.querySelector<HTMLInputElement>('#quick-search')?.focus();
      }
    };

    window.addEventListener('keydown', handle);

    return () => window.removeEventListener('keydown', handle);
  }, []);

  // The first page is on screen; the code of the others is fetched while nothing else happens.
  useEffect(() => whenIdle(preloadViews), []);

  const routes = useMemo<Route[]>(() => {
    // A module switched off in a project has no pages there.
    const offIn = (key: string, id: string) => boot.projects.find((candidate) => candidate.key === key)?.disabledModules.includes(id) ?? false;
    const fromModules = modules.flatMap((module) =>
      (module.routes ?? []).map((route) => ({
        pattern: route.pattern,
        render: (found: Params) => {
          if (!route.pattern.startsWith('/p/:key/')) {
            return route.render(found);
          }

          return offIn(found.key.toUpperCase(), module.id) ? null : <ProjectScope projectKey={found.key.toUpperCase()}>{route.render(found)}</ProjectScope>;
        },
      })),
    );

    return [...CORE, ...fromModules];
  }, [modules, boot.projects]);

  let view: ReactNode = null;

  for (const route of routes) {
    const found = matchRoute(route.pattern, path);

    if (found) {
      view = route.render(found);
      break;
    }
  }

  const bareProject = matchRoute('/p/:key', path);

  useEffect(() => {
    if (bareProject) {
      navigate(`/p/${bareProject.key}/board`, { replace: true });
    }
  }, [bareProject?.key]);

  const mainNav: NavEntry[] = [
    { id: 'home', label: () => t('My work'), icon: 'home', path: '/' },
    { id: 'projects', label: () => t('Projects'), icon: 'folder', path: '/projects' },
    { id: 'search', label: () => t('Search'), icon: 'search', path: '/search' },
    ...modules.flatMap((module) => module.globalNav?.() ?? []),
  ];
  const projectNav: NavEntry[] = project
    ? [
        { id: 'board', label: () => t('Board'), icon: 'board', path: `/p/${project.key}/board` },
        { id: 'backlog', label: () => t('Backlog'), icon: 'list', path: `/p/${project.key}/backlog` },
        { id: 'issues', label: () => t('Issues'), icon: 'filter', path: `/p/${project.key}/issues` },
        { id: 'releases', label: () => t('Releases'), icon: 'rocket', path: `/p/${project.key}/releases` },
        { id: 'schedule', label: () => t('Schedule'), icon: 'clock', path: `/p/${project.key}/schedule` },
        ...projectModules.flatMap((module) => module.projectNav?.(project.key) ?? []),
        { id: 'settings', label: () => t('Project settings'), icon: 'settings', path: `/p/${project.key}/settings` },
      ]
    : [];

  return (
    <div className={collapsed ? 'shell shell-collapsed' : 'shell'}>
      <aside className="sidebar">
        <div className="brand">
          <Link href="/" className="brand-mark" aria-label="team-hub">
            <svg viewBox="0 0 32 32" width="26" height="26">
              <rect width="32" height="32" rx="8" fill="var(--accent)" />
              <path d="M9 16.5l4.5 4.5L23 11" fill="none" stroke="#fff" strokeWidth="3.2" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </Link>
          <span className="brand-name">team-hub</span>
          <Button variant="ghost" icon="menu" onClick={() => setCollapsed(!collapsed)} aria-label={t('Toggle navigation')} />
        </div>
        <nav className="nav">
          {mainNav.map((entry) => (
            <NavLink key={entry.id} entry={entry} path={path} />
          ))}
        </nav>
        {project && (
          <>
            <div className="nav-project">
              <span className="project-badge">{project.key}</span>
              <select className="project-select" value={project.key} aria-label={t('Project')} onChange={(event) => navigate(`/p/${event.target.value}/board`)}>
                {boot.projects
                  .filter((candidate) => !candidate.archived || candidate.key === project.key)
                  .map((candidate) => (
                    <option key={candidate.key} value={candidate.key}>
                      {candidate.name}
                    </option>
                  ))}
              </select>
            </div>
            <nav className="nav">
              {projectNav.map((entry) => (
                <NavLink key={entry.id} entry={entry} path={path} />
              ))}
            </nav>
          </>
        )}
        <div className="nav-foot">
          <NavLink entry={{ id: 'runners', label: () => t('Runners'), icon: 'layers', path: '/runners' }} path={path} />
          <NavLink entry={{ id: 'settings', label: () => t('Settings'), icon: 'puzzle', path: '/settings' }} path={path} />
        </div>
      </aside>
      <div className="main">
        <header className="topbar">
          <QuickSearch />
          <Button variant="primary" icon="plus" onClick={() => setCreating(true)} title={t('Create issue (c)')}>
            {t('Create')}
          </Button>
          <Notifications />
          <ThemeToggle />
          <Menu
            label={user.name}
            items={[
              { label: t('Profile and tokens'), icon: 'user', onSelect: () => navigate('/settings/profile') },
              { label: currentLanguage() === 'pl' ? 'English' : 'Polski', icon: 'globe', onSelect: () => setLanguage(currentLanguage() === 'pl' ? 'en' : 'pl') },
              { label: t('Sign out'), icon: 'logout', onSelect: () => void api.post('/api/logout').then(() => window.location.assign('/')) },
            ]}
          >
            <Avatar user={boot.users.find((candidate) => candidate.id === user.id)} size={28} />
          </Menu>
        </header>
        <main className="content">{view ?? <Empty icon="search" title={t('Page not found')} />}</main>
      </div>
      {openKey && (
        <Drawer onClose={() => setParam('issue', null)} label={openKey}>
          <IssueView issueKey={openKey.toUpperCase()} mode="drawer" />
        </Drawer>
      )}
      {creating && <CreateIssueDialog projectKey={project?.key ?? null} onClose={() => setCreating(false)} />}
    </div>
  );
}

function ThemeToggle() {
  const [theme, setTheme] = useState(() => document.documentElement.dataset.theme ?? (window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'));

  function toggle(): void {
    const next = theme === 'dark' ? 'light' : 'dark';
    document.documentElement.dataset.theme = next;
    setTheme(next);

    try {
      window.localStorage.setItem('team-hub:theme', next);
    } catch {
      // The theme still applies to this page view.
    }
  }

  return <Button variant="ghost" icon={theme === 'dark' ? 'sun' : 'moon'} onClick={toggle} aria-label={t('Switch theme')} title={t('Switch theme')} />;
}

const HIT_ICONS: Record<string, IconName> = { issue: 'check', page: 'page', pr: 'pullRequest' };

function QuickSearch() {
  const [text, setText] = useState('');
  const [hits, setHits] = useState<QuickHit[]>([]);
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const [cursor, setCursor] = useState(0);

  useEffect(() => {
    if (!text.trim()) {
      setHits([]);

      return undefined;
    }

    const timer = window.setTimeout(() => {
      api
        .get<QuickHit[]>(`/api/quick?q=${encodeURIComponent(text)}`)
        .then((found) => {
          setHits(found);
          setCursor(0);
        })
        .catch(() => setHits([]));
    }, 150);

    return () => window.clearTimeout(timer);
  }, [text]);

  function go(hit: QuickHit | undefined): void {
    if (hit) {
      if (hit.kind === 'issue') {
        setParam('issue', hit.id);
      } else {
        navigate(hit.url);
      }
    } else if (text.trim()) {
      navigate(`/search?q=${encodeURIComponent(text.trim())}`);
    }

    setText('');
    setAnchor(null);
  }

  return (
    <div className="quick">
      <Icon name="search" className="quick-icon" />
      <input
        id="quick-search"
        className="input quick-input"
        placeholder={t('Search issues, pages… ( / )')}
        value={text}
        onFocus={(event) => setAnchor(event.currentTarget)}
        onChange={(event) => {
          setText(event.target.value);
          setAnchor(event.currentTarget);
        }}
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown') {
            event.preventDefault();
            setCursor((current) => Math.min(current + 1, hits.length - 1));
          } else if (event.key === 'ArrowUp') {
            event.preventDefault();
            setCursor((current) => Math.max(current - 1, -1));
          } else if (event.key === 'Enter') {
            go(hits[cursor]);
          } else if (event.key === 'Escape') {
            event.currentTarget.blur();
            setAnchor(null);
          }
        }}
      />
      {anchor && text.trim() && (
        <Popover anchor={anchor} onClose={() => setAnchor(null)} width={Math.max(anchor.offsetWidth, 420)}>
          <div className="menu">
            {hits.map((hit, index) => (
              <button key={`${hit.kind}:${hit.id}`} type="button" className={index === cursor ? 'menu-item quick-hit menu-cursor' : 'menu-item quick-hit'} onMouseEnter={() => setCursor(index)} onClick={() => go(hit)}>
                <Icon name={HIT_ICONS[hit.kind] ?? 'page'} />
                <span className="quick-hit-title">{hit.title}</span>
                <span className="muted">{hit.subtitle}</span>
              </button>
            ))}
            <button type="button" className="menu-item quick-hit" onClick={() => go(undefined)}>
              <Icon name="search" />
              <span className="quick-hit-title">{t('Search all issues for “{text}”', { text: text.trim() })}</span>
            </button>
          </div>
        </Popover>
      )}
    </div>
  );
}

function Notifications() {
  const { boot, refresh, user } = useSession();
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const [unread, setUnread] = useState(boot.unreadNotifications);
  const list = useRemote(() => (anchor ? api.get<Notification[]>('/api/notifications') : Promise.resolve([])), [anchor !== null]);

  useEffect(() => setUnread(boot.unreadNotifications), [boot.unreadNotifications]);

  // Anything somebody else did to an issue may have produced a notification for me.
  useLive((event) => {
    if (event.issueId !== null && event.actorId !== user.id) {
      void api.get<Bootstrap>('/api/bootstrap').then((fresh) => setUnread(fresh.unreadNotifications));
    }
  });

  async function markAll(): Promise<void> {
    await attempt(() => api.post('/api/notifications/read', {}));
    setUnread(0);
    await list.reload();
    await refresh();
  }

  return (
    <>
      <button type="button" className="button button-ghost button-md button-icon bell" aria-label={t('Notifications')} title={t('Notifications')} onClick={(event) => setAnchor(anchor ? null : event.currentTarget)}>
        <Icon name="bell" />
        {unread > 0 && <span className="bell-count">{unread > 99 ? '99+' : unread}</span>}
      </button>
      {anchor && (
        <Popover anchor={anchor} onClose={() => setAnchor(null)} align="end" width={400}>
          <div className="notifications">
            <header>
              <strong>{t('Notifications')}</strong>
              <Button size="sm" variant="ghost" disabled={unread === 0} onClick={() => void markAll()}>
                {t('Mark all read')}
              </Button>
            </header>
            {!list.data && <Spinner />}
            {list.data?.length === 0 && <Empty icon="bell" title={t('Nothing new')} />}
            {list.data?.map((item) => (
              <button
                key={item.id}
                type="button"
                className={item.readAt ? 'notification' : 'notification notification-unread'}
                onClick={() => {
                  setAnchor(null);
                  void api.post('/api/notifications/read', { id: item.id }).then(() => setUnread((count) => Math.max(0, count - (item.readAt ? 0 : 1))));

                  if (item.issueKey) {
                    setParam('issue', item.issueKey);
                  }
                }}
              >
                <span>{translateServer(item.text)}</span>
                <span className="muted">{timeAgo(item.createdAt)}</span>
              </button>
            ))}
          </div>
        </Popover>
      )}
    </>
  );
}
