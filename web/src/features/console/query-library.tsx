import { History, Bookmark, BookmarkPlus } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useInfiniteQuery } from '@tanstack/react-query';
import { params, request } from '../../api/client';
import type { HistoryEntry, Page } from '../../api/types';
import { useUser } from '../auth/session';
import { useToast } from '../../components/ui/toast';
import {
  Button,
  Empty,
  ErrorPanel,
  Field,
  Modal,
  Skeleton,
  Tip,
} from '../../components/ui';
import { formatTime } from '../../lib/utils';
type Favorite = { id: string; name: string; sql: string };
function load(key: string): Favorite[] {
  try {
    const items: unknown = JSON.parse(localStorage.getItem(key) ?? '[]');
    return Array.isArray(items)
      ? items.filter(
          (item): item is Favorite =>
            item &&
            typeof item.id === 'string' &&
            typeof item.name === 'string' &&
            typeof item.sql === 'string',
        )
      : [];
  } catch {
    return [];
  }
}
export function QueryLibrary({
  clusterId,
  sql,
  onRestore,
  onSaved,
  busy = false,
}: {
  busy?: boolean;
  clusterId: string;
  sql: string;
  onRestore: (sql: string) => void;
  onSaved: () => void;
}) {
  const user = useUser(),
    toast = useToast(),
    key = `vda.favorites.${user?.id}.${clusterId}`;
  const [mode, setMode] = useState<'history' | 'favorites' | null>(null),
    [search, setSearch] = useState(''),
    [saving, setSaving] = useState(false),
    [favorites, setFavorites] = useState(() => load(key));
  const history = useInfiniteQuery({
    queryKey: ['api', 'inline-history', clusterId, user?.id],
    queryFn: ({ pageParam, signal }) =>
      request<Page<HistoryEntry>>(
        `/history${params({ cluster_id: clusterId, user_id: user?.id, limit: 50, cursor: pageParam })}`,
        { signal },
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: mode === 'history',
  });
  const save = (items: Favorite[]) => {
    try {
      localStorage.setItem(key, JSON.stringify(items));
      setFavorites(items);
      toast('Favorites saved in this browser');
      return true;
    } catch {
      toast(
        'Favorites could not be saved. Browser storage is unavailable.',
        'error',
      );
      return false;
    }
  };
  const historyPages = history.data?.pages;
  // The list is only shown in the drawer, so skip the work while it is closed
  // (this component re-renders on every editor keystroke through `sql`).
  const filtered = useMemo(() => {
    if (!mode) return [];
    const entries =
      mode === 'history'
        ? (historyPages?.flatMap((page) => page.items) ?? []).map((item) => ({
            id: item.id,
            name: `${item.status} · ${formatTime(item.created_at)}`,
            sql: item.sql,
          }))
        : favorites;
    const needle = search.toLocaleLowerCase();
    return entries.filter((item) =>
      `${item.name} ${item.sql}`.toLocaleLowerCase().includes(needle),
    );
  }, [mode, historyPages, favorites, search]);
  const toggleMode = (next: 'history' | 'favorites') => {
    setSearch('');
    setMode((value) => (value === next ? null : next));
  };
  return (
    <>
      <div className="query-library-toolbar">
        <Tip text="Query history">
          <Button
            className="icon"
            aria-label="History"
            aria-expanded={mode === 'history'}
            onClick={() => toggleMode('history')}
          >
            <History size={16} />
          </Button>
        </Tip>
        <Tip text="Favorites · saved in this browser">
          <Button
            className="icon"
            aria-label={`Favorites (${favorites.length})`}
            aria-expanded={mode === 'favorites'}
            onClick={() => toggleMode('favorites')}
          >
            <Bookmark size={16} />
          </Button>
        </Tip>
        <Tip text="Save favorite · this browser only">
          <Button
            className="icon"
            aria-label="Save favorite"
            disabled={busy || !sql.trim()}
            onClick={() => setSaving(true)}
          >
            <BookmarkPlus size={16} />
          </Button>
        </Tip>
      </div>
      <Modal
        open={!!mode}
        onOpenChange={(open) => {
          if (!open) setMode(null);
        }}
        drawer
        title={mode === 'history' ? 'Query history' : 'Local favorites'}
        description="Restore SQL into a new tab. Favorites are saved only in this browser."
      >
        <section
          className="query-library"
          aria-label={
            mode === 'history' ? 'Inline query history' : 'Local favorites'
          }
        >
          <input
            aria-label="Search query library"
            placeholder="Search loaded queries…"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
          {mode === 'history' && history.isPending ? (
            <Skeleton />
          ) : mode === 'history' && history.error ? (
            <ErrorPanel
              error={history.error}
              retry={() => void history.refetch()}
            />
          ) : filtered.length === 0 ? (
            <Empty
              title="No matching queries"
              description={
                mode === 'history'
                  ? 'Run a query or clear the search to see your history.'
                  : 'Save useful SQL as a favorite in this browser.'
              }
              action={
                <Button
                  onClick={() =>
                    mode === 'favorites' ? setSaving(true) : setSearch('')
                  }
                >
                  {mode === 'favorites' ? 'Save favorite' : 'Clear search'}
                </Button>
              }
            />
          ) : (
            filtered.map((item) => (
              <article key={item.id}>
                <div className="inline wrap">
                  <strong>{item.name}</strong>
                  <Button
                    disabled={busy}
                    onClick={() => {
                      onRestore(item.sql);
                      setMode(null);
                    }}
                  >
                    Restore into tab
                  </Button>
                  {mode === 'favorites' && (
                    <Button
                      variant="ghost"
                      onClick={() =>
                        save(
                          favorites.filter(
                            (favorite) => favorite.id !== item.id,
                          ),
                        )
                      }
                    >
                      Remove
                    </Button>
                  )}
                </div>
                <pre>{item.sql}</pre>
              </article>
            ))
          )}
          {mode === 'history' && history.hasNextPage && (
            <Button
              disabled={history.isFetchingNextPage}
              onClick={() => void history.fetchNextPage()}
            >
              {history.isFetchingNextPage ? 'Loading…' : 'Load more queries'}
            </Button>
          )}
        </section>
      </Modal>
      <Modal
        open={saving}
        onOpenChange={setSaving}
        title="Save a favorite"
        description="Stored only in this browser, for your account and this cluster. SQL can contain sensitive values; use placeholders when possible."
      >
        <form
          onSubmit={(event) => {
            event.preventDefault();
            const name = String(
              new FormData(event.currentTarget).get('name'),
            ).trim();
            if (!name) return;
            if (
              save([
                ...favorites,
                {
                  id: crypto.randomUUID(),
                  name,
                  sql,
                },
              ])
            ) {
              onSaved();
              setSaving(false);
            }
          }}
        >
          <Field label="Favorite name">
            <input name="name" required maxLength={80} />
          </Field>
          <div className="dialog-actions">
            <Button type="submit" variant="primary">
              Save favorite
            </Button>
          </div>
        </form>
      </Modal>
    </>
  );
}
