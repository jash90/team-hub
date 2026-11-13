import { useEffect, useMemo, useState, type FormEvent } from 'react';

import { api } from '@/api.ts';
import type { WikiSpace } from '@/bindings/WikiSpace.ts';
import type { WikiTreeNode } from '@/bindings/WikiTreeNode.ts';
import { Icon } from '@/components/icon.tsx';
import { Button, Empty, ErrorNote, Menu, Spinner, Tag } from '@/components/ui.tsx';
import { t } from '@/i18n/index.ts';
import { Link, navigate } from '@/router.tsx';
import { useRemote, useStored } from '@/session.tsx';
import { useShellProject } from '@/shell.ts';

import { NewPageDialog, SpaceSettingsDialog } from './dialogs.tsx';
import { BASE, isPageEvent, pagePath, titleIndex, useSpaces } from './lib.ts';
import { PageArea, type PageMode } from './page.tsx';
import { LockIcon } from './parts.tsx';

type TreeProps = {
  space: WikiSpace;
  nodes: WikiTreeNode[];
  currentId: number | null;
  onAdd: (parent: WikiTreeNode | null) => void;
};

/** The pages of a space as a tree. Which branches are open is remembered per space. */
function Tree({ space, nodes, currentId, onAdd }: TreeProps) {
  const [open, setOpen] = useStored<Record<string, boolean>>(`wiki-open:${space.key}`, {});
  const byId = useMemo(() => new Map(nodes.map((node) => [node.id, node])), [nodes]);
  const parents = useMemo(() => new Set(nodes.map((node) => node.parentId).filter((id): id is number => id !== null)), [nodes]);

  // Opening a page shows where it is: the branches above it unfold.
  useEffect(() => {
    const unfolded: Record<string, boolean> = {};
    let above = currentId !== null ? (byId.get(currentId)?.parentId ?? null) : null;

    while (above !== null && !(above in unfolded)) {
      if (open[above] !== true) {
        unfolded[above] = true;
      }

      above = byId.get(above)?.parentId ?? null;
    }

    if (Object.values(unfolded).length > 0) {
      setOpen({ ...open, ...unfolded });
    }
    // Only a change of page unfolds; folding by hand afterwards stays.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentId, byId]);

  const depth = new Map<number, number>();
  const shown: WikiTreeNode[] = [];
  const visible = new Set<number>();

  for (const node of nodes) {
    const parent = node.parentId !== null ? byId.get(node.parentId) : undefined;
    depth.set(node.id, parent ? (depth.get(parent.id) ?? 0) + 1 : 0);

    // Top-level pages show their children until folded; deeper ones until unfolded.
    const parentOpen = !parent || (open[parent.id] ?? (depth.get(parent.id) ?? 0) === 0);

    if (parentOpen && (!parent || visible.has(parent.id))) {
      shown.push(node);
      visible.add(node.id);
    }
  }

  return (
    <nav className="wiki-nodes" aria-label={t('Pages')}>
      {shown.map((node) => {
        const level = depth.get(node.id) ?? 0;
        const folded = !(open[node.id] ?? level === 0);

        return (
          <div key={node.id} className={node.id === currentId ? 'wiki-node wiki-node-current' : 'wiki-node'} style={{ paddingLeft: 6 + level * 16 }}>
            {parents.has(node.id) ? (
              <button type="button" className="wiki-node-fold" aria-expanded={!folded} aria-label={folded ? t('Unfold “{title}”', { title: node.title }) : t('Fold “{title}”', { title: node.title })} onClick={() => setOpen({ ...open, [node.id]: folded })}>
                <Icon name={folded ? 'chevronRight' : 'chevronDown'} size={14} />
              </button>
            ) : (
              <span className="wiki-node-fold" />
            )}
            <Link href={pagePath(space.key, node.id)} className="wiki-node-link" aria-current={node.id === currentId ? 'page' : undefined} title={node.title}>
              {node.title}
            </Link>
            {node.locked && <LockIcon />}
            {space.canEdit && (
              <button type="button" className="wiki-node-add" aria-label={t('New page under “{title}”', { title: node.title })} title={t('New page under “{title}”', { title: node.title })} onClick={() => onAdd(node)}>
                <Icon name="plus" size={14} />
              </button>
            )}
          </div>
        );
      })}
    </nav>
  );
}

function SpaceSearch({ spaceKey }: { spaceKey: string }) {
  const [text, setText] = useState('');

  function submit(event: FormEvent): void {
    event.preventDefault();

    if (text.trim()) {
      navigate(`/wiki?q=${encodeURIComponent(text.trim())}&space=${spaceKey}`);
    }
  }

  return (
    <form className="filter-search wiki-space-search" onSubmit={submit} role="search">
      <Icon name="search" />
      <input className="input" placeholder={t('Search this space')} aria-label={t('Search this space')} value={text} onChange={(event) => setText(event.target.value)} />
    </form>
  );
}

/** A space: its page tree beside the page that is open. */
export function SpaceView({ spaceKey, pageId, mode }: { spaceKey: string; pageId: number | null; mode: PageMode }) {
  const key = spaceKey.toUpperCase();
  const space = useRemote(() => api.get<WikiSpace>(`${BASE}/spaces/${encodeURIComponent(key)}`), [key], (event) => isPageEvent(event, key) || event.kind === 'confluence.imported');
  const tree = useRemote(() => api.get<WikiTreeNode[]>(`${BASE}/spaces/${encodeURIComponent(key)}/tree`), [key], (event) => isPageEvent(event, key) || event.kind === 'confluence.imported');
  const spaces = useSpaces();
  const [treeHidden, setTreeHidden] = useStored('wiki-tree-hidden', false);
  const [adding, setAdding] = useState<{ parent: WikiTreeNode | null } | null>(null);
  const [settings, setSettings] = useState(false);
  const nodes = tree.data ?? [];
  const titles = useMemo(() => titleIndex(tree.data ?? []), [tree.data]);
  const spaceKeys = useMemo(() => new Set((spaces.data ?? []).map((candidate) => candidate.key)), [spaces.data]);

  // A space bound to a project is part of it: the shell shows that project's navigation.
  useShellProject(space.data?.projectKey);

  if ((space.error && !space.data) || (tree.error && !tree.data)) {
    return (
      <div className="view">
        <ErrorNote message={space.error ?? tree.error ?? ''} />
        <p>
          <Link href="/wiki">{t('All spaces')}</Link>
        </p>
      </div>
    );
  }

  if (!space.data || !tree.data) {
    return <Spinner />;
  }

  const current = pageId ?? nodes[0]?.id ?? null;
  const data = space.data;

  return (
    <div className="view wiki">
      <div className="wiki-bar">
        <Button variant="ghost" icon="list" className={treeHidden ? '' : 'is-on'} aria-pressed={!treeHidden} onClick={() => setTreeHidden(!treeHidden)}>
          {t('Pages')}
        </Button>
        <nav className="crumbs" aria-label={t('Breadcrumbs')}>
          <Link href="/wiki">{t('Knowledge base')}</Link>
          <Icon name="chevronRight" size={12} />
          <Link href={`/wiki/${data.key}`} className="crumb-current">
            {data.name}
          </Link>
          {data.source === 'confluence' && <Tag tone="blue">Confluence</Tag>}
          {data.projectKey && (
            <Link href={`/p/${data.projectKey}/board`} className="wiki-project-link" title={t('The project this space belongs to')}>
              <Tag>{data.projectKey}</Tag>
            </Link>
          )}
        </nav>
        <span className="filter-spacer" />
        <SpaceSearch spaceKey={data.key} />
      </div>

      <div className={treeHidden ? 'wiki-layout wiki-layout-full' : 'wiki-layout'}>
        {!treeHidden && (
          <aside className="panel wiki-tree">
            <header className="panel-head">
              <span className="project-badge">{data.key}</span>
              <strong className="wiki-tree-name" title={data.description || data.name}>
                {data.name}
              </strong>
              <span className="filter-spacer" />
              {data.canEdit && <Button size="sm" variant="ghost" icon="plus" aria-label={t('New page')} title={t('New page')} onClick={() => setAdding({ parent: null })} />}
              {data.canEdit && <Menu label={t('Space actions')} items={[{ label: t('Space settings'), icon: 'settings', onSelect: () => setSettings(true) }]} />}
            </header>
            {nodes.length === 0 ? <p className="muted panel-empty">{t('No pages yet.')}</p> : <Tree space={data} nodes={nodes} currentId={current} onAdd={(parent) => setAdding({ parent })} />}
          </aside>
        )}
        <div className="wiki-main">
          {current === null ? (
            <div className="panel">
              <Empty icon="page" title={t('This space has no pages yet')}>
                {data.description && <p>{data.description}</p>}
                {data.canEdit && (
                  <p className="wiki-empty-action">
                    <Button variant="primary" icon="plus" onClick={() => setAdding({ parent: null })}>
                      {t('Create the first page')}
                    </Button>
                  </p>
                )}
              </Empty>
            </div>
          ) : (
            <PageArea key={current} space={data} pageId={current} mode={mode} nodes={nodes} titles={titles} spaceKeys={spaceKeys} onAdd={(parent) => setAdding({ parent })} reloadTree={() => void tree.reload()} />
          )}
        </div>
      </div>

      {adding && <NewPageDialog spaceKey={data.key} parent={adding.parent} onClose={() => setAdding(null)} />}
      {settings && (
        <SpaceSettingsDialog
          space={data}
          onClose={() => setSettings(false)}
          onSaved={() => {
            void space.reload();
            void spaces.reload();
          }}
        />
      )}
    </div>
  );
}
