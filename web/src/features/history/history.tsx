import { Fragment, useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { useInfiniteQuery } from '@tanstack/react-query';
import { ChevronDown, Terminal } from 'lucide-react';
import type { Cluster, HistoryEntry, User } from '../../api/types';
import { params, request } from '../../api/client';
import { useResource } from '../../lib/query';
import { useUser } from '../auth/session';
import {
  Badge,
  Button,
  Empty,
  ErrorPanel,
  Picker,
  Skeleton,
  EnvBadge,
} from '../../components/ui';
import { DateRange, inDateRange } from '../../components/ui/date-range';
import { RelativeTime } from '../../components/ui/time';
import { csvCell, download } from '../../lib/utils';
import { dayLabel, statementSummary } from '../../lib/activity';
import { SqlPreview } from '../../components/ui/sql-preview';
export function HistoryTable({
  items,
  showUser = true,
  initialExpanded = null,
  entryLinks = {},
  clusters = [],
}: {
  items: HistoryEntry[];
  showUser?: boolean;
  initialExpanded?: string | null;
  entryLinks?: Record<string, string>;
  clusters?: Cluster[];
}) {
  const [expanded, setExpanded] = useState<string | null>(initialExpanded);
  const clusterById = useMemo(
    () => new Map(clusters.map((cluster) => [cluster.id, cluster])),
    [clusters],
  );
  if (!items.length)
    return (
      <Empty
        title="No queries yet"
        description="Run a guarded query from the console to see it here."
        action={
          <Link className="button secondary" to="/console">
            Open console
          </Link>
        }
      />
    );
  return (
    <div className="table-wrap">
      <table className="history-table">
        <thead>
          <tr>
            <th>Query</th>
            <th>Cluster</th>
            {showUser && <th>User</th>}
            <th>Status</th>
            <th>Rows</th>
            <th>Duration</th>
            <th>Time</th>
            <th>
              <span className="sr-only">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {items.map((entry, index) => {
            const entryCluster = clusterById.get(entry.cluster_id);
            return (
              <Fragment key={entry.id}>
                {(index === 0 ||
                  items[index - 1]?.created_at.slice(0, 10) !==
                    entry.created_at.slice(0, 10)) && (
                  <tr className="day-heading">
                    <th colSpan={showUser ? 8 : 7} scope="colgroup">
                      {dayLabel(entry.created_at)}
                    </th>
                  </tr>
                )}
                <tr>
                  <td className="sql-preview">
                    <button
                      type="button"
                      onClick={() =>
                        setExpanded(expanded === entry.id ? null : entry.id)
                      }
                      aria-expanded={expanded === entry.id}
                      aria-controls={
                        expanded === entry.id
                          ? `history-expanded-${entry.id}`
                          : undefined
                      }
                    >
                      <span>
                        <span className="sql-summary">
                          {statementSummary(entry.sql)}
                        </span>
                        <SqlPreview sql={entry.sql} />
                      </span>
                      <ChevronDown size={14} />
                    </button>
                    {expanded === entry.id && (
                      <div
                        className="history-expanded"
                        id={`history-expanded-${entry.id}`}
                      >
                        <pre>{entry.sql}</pre>
                        {entry.error && (
                          <p className="error-text">{entry.error}</p>
                        )}
                        <Link
                          className="button secondary"
                          to={
                            entryLinks[entry.id] ??
                            `/history?cluster_id=${entry.cluster_id}&entry=${entry.id}`
                          }
                        >
                          Link to query
                        </Link>
                        <Link
                          className="button secondary"
                          to={`/console?cluster_id=${entry.cluster_id}&sql=${encodeURIComponent(entry.sql)}`}
                        >
                          <Terminal size={13} />
                          Open in console
                        </Link>
                      </div>
                    )}
                  </td>
                  <td>
                    <Link to={`/clusters/${entry.cluster_id}`}>
                      {entry.cluster_name}
                    </Link>
                    {entryCluster && (
                      <small>
                        <EnvBadge environment={entryCluster.environment} />
                      </small>
                    )}
                  </td>
                  {showUser && <td className="muted">{entry.user_email}</td>}
                  <td>
                    <Badge
                      variant="verdict"
                      tone={
                        entry.verdict === 'allow'
                          ? 'success'
                          : entry.verdict === 'requires_approval'
                            ? 'warning'
                            : 'danger'
                      }
                    >
                      {entry.verdict === 'deny'
                        ? 'Blocked'
                        : entry.verdict === 'requires_approval'
                          ? 'Needs approval'
                          : 'Allowed'}
                    </Badge>
                    {/* A denied query's status is "blocked" too; say it once. */}
                    {!(
                      entry.verdict === 'deny' && entry.status === 'blocked'
                    ) && (
                      <small className="muted">
                        {entry.status === 'ok'
                          ? 'Completed'
                          : entry.status.charAt(0).toUpperCase() +
                            entry.status.slice(1)}
                      </small>
                    )}
                  </td>
                  <td className="numeric">
                    {entry.row_count?.toLocaleString() ?? '—'}
                  </td>
                  <td className="numeric muted">
                    {entry.elapsed_ms !== null ? `${entry.elapsed_ms} ms` : '—'}
                  </td>
                  <td className="muted nowrap">
                    <RelativeTime value={entry.created_at} />
                  </td>
                  <td>
                    <Link
                      aria-label="Open query in console"
                      to={`/console?cluster_id=${entry.cluster_id}&sql=${encodeURIComponent(entry.sql)}`}
                    >
                      <Terminal size={15} />
                    </Link>
                  </td>
                </tr>
              </Fragment>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
export function HistoryPage() {
  const [searchParams, setSearchParams] = useSearchParams(),
    user = useUser();
  const cluster = searchParams.get('cluster_id') ?? 'all',
    status = searchParams.get('status') ?? 'all',
    actor = searchParams.get('user_id') ?? 'all',
    from = searchParams.get('from') ?? '',
    to = searchParams.get('to') ?? '';
  const update = (key: string, value: string) => {
    const next = new URLSearchParams(searchParams);
    if (value === 'all' || !value) next.delete(key);
    else next.set(key, value);
    next.delete('cursor');
    next.delete('entry');
    setSearchParams(next, { replace: true });
  };
  const setCluster = (value: string) => update('cluster_id', value),
    setStatus = (value: string) => update('status', value),
    setActor = (value: string) => update('user_id', value);
  const clusters = useResource<{ items: Cluster[] }>('/clusters'),
    users = useResource<{ items: User[] }>(
      '/users',
      user?.org_role === 'admin',
    );
  const filter = {
    cluster_id: cluster === 'all' ? undefined : cluster,
    status: status === 'all' ? undefined : status,
    user_id: actor === 'all' ? undefined : actor,
  };
  const query = useInfiniteQuery({
    queryKey: ['api', 'history', filter, searchParams.get('cursor')],
    queryFn: ({ pageParam, signal }) =>
      request<{ items: HistoryEntry[]; next_cursor: string | null }>(
        `/history${params({ ...filter, limit: 50, cursor: pageParam })}`,
        { signal },
      ),
    initialPageParam: searchParams.get('cursor') as string | null,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
  });
  const loaded = useMemo(
      () => query.data?.pages.flatMap((page) => page.items) ?? [],
      [query.data],
    ),
    items = useMemo(
      () => loaded.filter((entry) => inDateRange(entry.created_at, from, to)),
      [loaded, from, to],
    );
  const entryLinks = useMemo(
    () =>
      Object.fromEntries(
        (query.data?.pages ?? []).flatMap((page, index) =>
          page.items.map((entry) => {
            const next = new URLSearchParams(searchParams);
            next.set('entry', entry.id);
            const cursor = query.data?.pageParams[index];
            if (typeof cursor === 'string') next.set('cursor', cursor);
            else next.delete('cursor');
            return [entry.id, `/history?${next}`];
          }),
        ),
      ),
    [query.data, searchParams],
  );
  const exportRows = () =>
    download(
      'query-history.csv',
      [
        [
          'id',
          'cluster',
          'user',
          'sql',
          'status',
          'rows',
          'duration_ms',
          'created_at',
        ]
          .map(csvCell)
          .join(','),
        ...items.map((entry) =>
          [
            entry.id,
            entry.cluster_name,
            entry.user_email,
            entry.sql,
            entry.status,
            entry.row_count,
            entry.elapsed_ms,
            entry.created_at,
          ]
            .map(csvCell)
            .join(','),
        ),
      ].join('\r\n'),
      'text/csv;charset=utf-8',
    );
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Query history</h1>
          <p>Every execution, including blocked and cancelled queries.</p>
        </div>
        <Button disabled={!items.length} onClick={exportRows}>
          Export loaded rows as CSV
        </Button>
      </div>
      <div className="filterbar">
        <Picker
          value={cluster}
          onChange={setCluster}
          label="Cluster"
          options={[
            { value: 'all', label: 'All clusters' },
            ...(clusters.data?.items.map((c) => ({
              value: c.id,
              label: c.name,
            })) ?? []),
          ]}
        />
        <Picker
          value={status}
          onChange={setStatus}
          label="Query status"
          options={[
            { value: 'all', label: 'All statuses' },
            ...[
              'running',
              'unknown',
              'ok',
              'blocked',
              'error',
              'cancelled',
            ].map((value) => ({
              value,
              label: value,
            })),
          ]}
        />
        {user?.org_role === 'admin' && (
          <Picker
            value={actor}
            onChange={setActor}
            label="User"
            options={[
              { value: 'all', label: 'All users' },
              ...(users.data?.items.map((u) => ({
                value: u.id,
                label: u.name,
              })) ?? []),
            ]}
          />
        )}
        <DateRange
          onChange={(range) => {
            const next = new URLSearchParams(searchParams);
            for (const key of ['from', 'to'] as const) {
              if (range[key]) next.set(key, range[key]);
              else next.delete(key);
            }
            next.delete('cursor');
            next.delete('entry');
            setSearchParams(next, { replace: true });
          }}
          from={from}
          to={to}
          onFrom={(value) => update('from', value)}
          onTo={(value) => update('to', value)}
        />
      </div>
      <p className="loaded-note">
        UTC date range · {items.length} matching of {loaded.length} loaded
        queries. Load more to include older rows.
      </p>
      {searchParams.get('entry') &&
        !loaded.some((entry) => entry.id === searchParams.get('entry')) &&
        !query.isPending && (
          <p className="callout warning">
            This linked query is not in the loaded rows. Load more, or clear
            filters.
          </p>
        )}
      {query.isPending ? (
        <Skeleton />
      ) : query.error ? (
        <ErrorPanel error={query.error} retry={() => void query.refetch()} />
      ) : (
        <HistoryTable
          clusters={clusters.data?.items ?? []}
          items={items}
          key={searchParams.get('entry') ?? 'list'}
          initialExpanded={searchParams.get('entry')}
          entryLinks={entryLinks}
          showUser={user?.org_role === 'admin'}
        />
      )}
      {query.isFetchNextPageError && (
        <ErrorPanel
          error={query.error}
          retry={() => void query.fetchNextPage()}
        />
      )}
      {query.hasNextPage && (
        <Button
          disabled={query.isFetchingNextPage}
          onClick={() => void query.fetchNextPage()}
        >
          {query.isFetchingNextPage ? 'Loading…' : 'Load more queries'}
        </Button>
      )}
    </>
  );
}
