import { useMemo, useState } from 'react';
import { Search, Database, Terminal, ArrowRight, Command } from 'lucide-react';
import { useLocation, useNavigate } from 'react-router-dom';
import type { Cluster, HistoryEntry, Page } from '../api/types';
import { useResource } from '../lib/query';
import { useUser } from '../features/auth/session';
import { useTheme } from '../lib/theme';
import { fuzzyScore } from '../lib/command';
import { Button, Modal, Skeleton, ErrorPanel } from './ui';
type Destination = { to: string; text: string };
export function CommandPalette({
  open,
  onOpenChange,
  pages,
  clusters,
}: {
  open: boolean;
  onOpenChange: (value: boolean) => void;
  pages: Destination[];
  clusters: Cluster[];
}) {
  const [search, setSearch] = useState(''),
    [cursor, setCursor] = useState(0);
  const user = useUser(),
    navigate = useNavigate(),
    location = useLocation(),
    { resolved, setTheme } = useTheme();
  const history = useResource<Page<HistoryEntry>>(
    `/history?user_id=${user?.id}&limit=10`,
    open,
  );
  const items = useMemo(() => {
    const go = (to: string) => () => navigate(to);
    const clusterId =
      new URLSearchParams(location.search).get('cluster_id') ??
      location.pathname.match(/\/clusters\/([^/]+)/)?.[1];
    return [
      ...pages.map((page) => ({
        group: 'Pages',
        id: page.to,
        label: page.text,
        search: page.text,
        action: go(page.to),
      })),
      ...clusters.map((c) => ({
        group: 'Clusters',
        id: c.id,
        label: c.name,
        search: `${c.name} ${c.environment} ${c.host}`,
        action: go(`/console?cluster_id=${c.id}`),
      })),
      ...(history.data?.items ?? []).map((q) => ({
        group: 'Recent queries',
        id: q.id,
        label: q.sql,
        search: `${q.sql} ${q.cluster_name}`,
        action: go(
          `/console?cluster_id=${q.cluster_id}&sql=${encodeURIComponent(q.sql)}`,
        ),
      })),
      ...(user?.org_role === 'admin'
        ? [
            {
              group: 'Actions',
              id: 'add',
              label: 'Add cluster',
              search: 'Add cluster database',
              action: go('/clusters?add=true'),
            },
          ]
        : []),
      {
        group: 'Actions',
        id: 'new',
        label: 'New query tab',
        search: 'New query tab SQL',
        action: () => {
          if (document.getElementById('active-query-panel'))
            window.dispatchEvent(new Event('vda:new-tab'));
          else if (clusterId) navigate(`/console?cluster_id=${clusterId}`);
          else
            navigate(
              `/console${clusters[0] ? `?cluster_id=${clusters[0].id}` : ''}`,
            );
        },
      },
      {
        group: 'Actions',
        id: 'theme',
        label: 'Toggle theme',
        search: 'Toggle theme dark light',
        action: () => setTheme(resolved === 'dark' ? 'light' : 'dark'),
      },
    ]
      .map((item) => ({ ...item, score: fuzzyScore(item.search, search) }))
      .filter((item) => item.score > 0)
      .sort(
        (a, b) =>
          ['Pages', 'Actions', 'Clusters', 'Recent queries'].indexOf(a.group) -
            ['Pages', 'Actions', 'Clusters', 'Recent queries'].indexOf(
              b.group,
            ) || b.score - a.score,
      );
  }, [
    pages,
    clusters,
    history.data,
    search,
    user,
    location,
    navigate,
    resolved,
    setTheme,
  ]);
  const selected = Math.min(cursor, Math.max(0, items.length - 1));
  const run = (index: number) => {
    items[index]?.action();
    onOpenChange(false);
  };
  return (
    <Modal
      open={open}
      onOpenChange={onOpenChange}
      className="palette-dialog"
      title="Jump to a page or cluster"
      description="Pages, databases, recent queries and actions. ↑ ↓ to choose, Enter to open."
    >
      <div className="palette-search">
        <Search size={20} />
        <input
          autoFocus
          aria-label="Search pages and clusters"
          aria-controls="command-results"
          aria-activedescendant={
            items.length > 0 ? `command-${selected}` : undefined
          }
          placeholder="Search commands…"
          value={search}
          onChange={(event) => {
            setSearch(event.target.value);
            setCursor(0);
          }}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
              event.preventDefault();
              const next =
                (selected +
                  (event.key === 'ArrowDown' ? 1 : -1) +
                  items.length) %
                Math.max(1, items.length);
              setCursor(next);
              document
                .getElementById(`command-${next}`)
                ?.scrollIntoView({ block: 'nearest' });
            }
            if (event.key === 'Enter') {
              event.preventDefault();
              run(selected);
            }
          }}
        />
      </div>
      <div
        className="command-results"
        id="command-results"
        role="group"
        aria-label="Commands"
      >
        {items.length === 0 && (
          <p className="muted" role="status">
            No matches. Try a page or cluster name.
          </p>
        )}
        {items.map((item, index) => (
          <div key={item.id}>
            {items[index - 1]?.group !== item.group && (
              <h2 className="command-section">{item.group}</h2>
            )}
            <Button
              id={`command-${index}`}
              variant="ghost"
              aria-current={selected === index ? 'true' : undefined}
              className={selected === index ? 'command-selected' : ''}
              onClick={() => run(index)}
              title={item.label}
            >
              {item.group === 'Clusters' ? (
                <Database size={16} />
              ) : item.group === 'Recent queries' ? (
                <Terminal size={16} />
              ) : item.group === 'Actions' ? (
                <Command size={16} />
              ) : (
                <ArrowRight size={16} />
              )}
              <span>{item.label}</span>
              {selected === index && <kbd aria-hidden="true">↵</kbd>}
            </Button>
          </div>
        ))}
        {history.isPending && <Skeleton />}
        {history.error && (
          <ErrorPanel
            error={history.error}
            retry={() => void history.refetch()}
          />
        )}
      </div>
    </Modal>
  );
}
