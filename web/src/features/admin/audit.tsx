import { Fragment, useEffect, useMemo, useRef, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { useInfiniteQuery } from '@tanstack/react-query';
import type { AuditEvent, User, Cluster } from '../../api/types';
import { params, request } from '../../api/client';
import { useResource } from '../../lib/query';
import {
  Button,
  Empty,
  ErrorPanel,
  Picker,
  Skeleton,
  EnvBadge,
  Badge,
} from '../../components/ui';
import { DateRange, inDateRange } from '../../components/ui/date-range';
import { RelativeTime } from '../../components/ui/time';
import { csvCell, download } from '../../lib/utils';
import { dayLabel, statementSummary } from '../../lib/activity';
import { SqlPreview } from '../../components/ui/sql-preview';
export function AuditPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const action = searchParams.get('action') ?? '',
    actor = searchParams.get('actor_id') ?? 'all',
    from = searchParams.get('from') ?? '',
    to = searchParams.get('to') ?? '';
  const update = (key: string, value: string) => {
    const next = new URLSearchParams(searchParams);
    if (value === 'all' || !value) next.delete(key);
    else next.set(key, value);
    next.delete('cursor');
    next.delete('event');
    setSearchParams(next, { replace: true });
  };
  const setActor = (value: string) => update('actor_id', value);
  // The server matches actions exactly, so wait for typing to pause before querying.
  const [actionInput, setActionInput] = useState(action);
  const updateRef = useRef(update);
  updateRef.current = update;
  useEffect(() => setActionInput(action), [action]);
  useEffect(() => {
    if (actionInput === action) return;
    const timer = setTimeout(
      () => updateRef.current('action', actionInput.trim()),
      300,
    );
    return () => clearTimeout(timer);
  }, [actionInput, action]);
  const users = useResource<{ items: User[] }>('/users');
  const clusters = useResource<{ items: Cluster[] }>('/clusters');
  const sentinel = useRef<HTMLDivElement>(null);
  const query = useInfiniteQuery({
    queryKey: ['api', 'audit', action, actor, searchParams.get('cursor')],
    queryFn: ({ pageParam, signal }) =>
      request<{ items: AuditEvent[]; next_cursor: string | null }>(
        `/audit${params({ action, actor_id: actor === 'all' ? undefined : actor, limit: 50, cursor: pageParam })}`,
        { signal },
      ),
    initialPageParam: searchParams.get('cursor') as string | null,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
  });
  const {
    hasNextPage,
    isFetchingNextPage,
    isFetchNextPageError,
    fetchNextPage,
  } = query;
  useEffect(() => {
    const element = sentinel.current;
    if (!element) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (
          entries.some((entry) => entry.isIntersecting) &&
          hasNextPage &&
          !isFetchingNextPage &&
          !isFetchNextPageError
        )
          void fetchNextPage();
      },
      { rootMargin: '150px' },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [hasNextPage, isFetchingNextPage, isFetchNextPageError, fetchNextPage]);
  const loaded = useMemo(
      () => query.data?.pages.flatMap((page) => page.items) ?? [],
      [query.data],
    ),
    items = useMemo(
      () => loaded.filter((event) => inDateRange(event.created_at, from, to)),
      [loaded, from, to],
    );
  const eventLinks = useMemo(
    () =>
      Object.fromEntries(
        (query.data?.pages ?? []).flatMap((page, index) =>
          page.items.map((event) => {
            const next = new URLSearchParams(searchParams);
            next.set('event', event.id);
            const cursor = query.data?.pageParams[index];
            if (typeof cursor === 'string') next.set('cursor', cursor);
            else next.delete('cursor');
            return [event.id, `/audit?${next}`];
          }),
        ),
      ),
    [query.data, searchParams],
  );
  const clusterById = useMemo(
    () => new Map(clusters.data?.items.map((c) => [c.id, c])),
    [clusters.data],
  );
  const exportRows = () =>
    download(
      'audit-log.csv',
      [
        [
          'id',
          'action',
          'actor',
          'target_type',
          'target_id',
          'ip',
          'details',
          'created_at',
        ]
          .map(csvCell)
          .join(','),
        ...items.map((event) =>
          [
            event.id,
            event.action,
            event.actor?.email,
            event.target_type,
            event.target_id,
            event.ip,
            JSON.stringify(event.details),
            event.created_at,
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
          <h1>Audit log</h1>
          <p>An append-only record of query and administration activity.</p>
        </div>
        <Button disabled={!items.length} onClick={exportRows}>
          Export loaded rows as CSV
        </Button>
      </div>
      <div className="filterbar">
        <input
          value={actionInput}
          onChange={(e) => setActionInput(e.target.value)}
          aria-label="Filter audit action"
          placeholder="Filter action, e.g. query.execute"
        />
        <Picker
          value={actor}
          onChange={setActor}
          label="Audit actor"
          options={[
            { value: 'all', label: 'All actors' },
            ...(users.data?.items.map((u) => ({
              value: u.id,
              label: u.name,
            })) ?? []),
          ]}
        />
        <DateRange
          onChange={(range) => {
            const next = new URLSearchParams(searchParams);
            for (const key of ['from', 'to'] as const) {
              if (range[key]) next.set(key, range[key]);
              else next.delete(key);
            }
            next.delete('cursor');
            next.delete('event');
            setSearchParams(next, { replace: true });
          }}
          from={from}
          to={to}
          onFrom={(value) => update('from', value)}
          onTo={(value) => update('to', value)}
        />
      </div>
      <p className="loaded-note" role="status">
        UTC date range · {items.length} matching of {loaded.length} loaded
        events. Export includes these rows only.
      </p>
      {query.isPending ? (
        <Skeleton />
      ) : query.error ? (
        <ErrorPanel error={query.error} />
      ) : items.length === 0 ? (
        <Empty
          title="No matching audit events"
          description="Clear the filters to see more activity."
          action={
            <Button onClick={() => setSearchParams({})}>Clear filters</Button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th scope="col">Action</th>
                <th scope="col">Actor</th>
                <th scope="col">Target</th>
                <th scope="col">IP address</th>
                <th scope="col">Time</th>
                <th scope="col">Details</th>
              </tr>
            </thead>
            <tbody>
              {items.map((event, index) => {
                const target = event.target_id
                  ? clusterById.get(event.target_id)
                  : undefined;
                return (
                  <Fragment key={event.id}>
                    {(index === 0 ||
                      items[index - 1]?.created_at.slice(0, 10) !==
                        event.created_at.slice(0, 10)) && (
                      <tr className="day-heading">
                        <th colSpan={6} scope="colgroup">
                          {dayLabel(event.created_at)}
                        </th>
                      </tr>
                    )}
                    <tr>
                      <td>
                        <AuditStatement event={event} />
                      </td>
                      <td>{event.actor?.email ?? 'System'}</td>
                      <td>
                        {event.target_type === 'cluster' && event.target_id ? (
                          <Link to={`/clusters/${event.target_id}`}>
                            {clusterById.get(event.target_id)?.name ??
                              'Cluster'}
                          </Link>
                        ) : (
                          event.target_type
                        )}
                        {target ? (
                          <small>
                            <EnvBadge environment={target.environment} />
                          </small>
                        ) : (
                          <small
                            className="mono"
                            title={event.target_id ?? undefined}
                          >
                            {event.target_id ?? '—'}
                          </small>
                        )}
                      </td>
                      <td className="mono muted">{event.ip ?? '—'}</td>
                      <td className="nowrap">
                        <RelativeTime value={event.created_at} />
                      </td>
                      <td>
                        <details open={searchParams.get('event') === event.id}>
                          <summary>JSON details</summary>
                          <Link to={eventLinks[event.id] ?? '/audit'}>
                            Link to event
                          </Link>
                          <pre className="json-preview" tabIndex={0}>
                            {JSON.stringify(event.details, null, 2)}
                          </pre>
                        </details>
                      </td>
                    </tr>
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      {query.isFetchNextPageError && (
        <ErrorPanel
          error={query.error}
          retry={() => void query.fetchNextPage()}
        />
      )}
      <div ref={sentinel}>
        {query.hasNextPage && (
          <Button
            disabled={query.isFetchingNextPage}
            onClick={() => void query.fetchNextPage()}
          >
            {query.isFetchingNextPage ? 'Loading…' : 'Load more events'}
          </Button>
        )}
      </div>
    </>
  );
}

function AuditStatement({ event }: { event: AuditEvent }) {
  const details =
    event.details &&
    typeof event.details === 'object' &&
    !Array.isArray(event.details)
      ? (event.details as Record<string, unknown>)
      : {};
  return (
    <>
      {event.action.replaceAll('.', ' · ')}
      {typeof details.sql === 'string' && (
        <div className="sql-preview">
          <div className="sql-summary">{statementSummary(details.sql)}</div>
          <SqlPreview sql={details.sql} />
        </div>
      )}
      {['allow', 'requires_approval', 'deny'].includes(
        String(details.verdict),
      ) && (
        <Badge
          variant="verdict"
          tone={
            details.verdict === 'deny'
              ? 'danger'
              : details.verdict === 'requires_approval'
                ? 'warning'
                : 'success'
          }
        >
          {String(details.verdict).replaceAll('_', ' ')}
        </Badge>
      )}
    </>
  );
}
