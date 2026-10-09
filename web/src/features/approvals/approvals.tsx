import { useEffect, useMemo, useState } from 'react';
import { X } from 'lucide-react';
import { Link, useSearchParams } from 'react-router-dom';
import { expiryLabel, sqlDiff, guardedSqlChanged } from '../../lib/sql-diff';
import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import type { Approval, Cluster, Page, Policy } from '../../api/types';
import { params, request } from '../../api/client';
import { useAction, useResource } from '../../lib/query';
import {
  Badge,
  Button,
  Empty,
  EnvBadge,
  ErrorPanel,
  Field,
  Modal,
  Skeleton,
  TabBar,
  Confirm,
  Picker,
} from '../../components/ui';
import { useUser } from '../auth/session';
import { RelativeTime } from '../../components/ui/time';
import { formatTime } from '../../lib/utils';

import { SafetyPanel } from '../console/safety';
import { ApprovalResult } from './result';
import { useMedia } from '../../lib/media';
import { ExpiryTime } from '../../components/ui/time';
export function ApprovalsPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const [notes, setNotes] = useState<Record<string, string>>({});
  const phone = useMedia('(max-width: 600px)'),
    user = useUser();
  const counts = useResource<Page<Approval>>('/approvals?limit=50');
  const tab = searchParams.get('tab') ?? 'pending',
    selected = searchParams.get('id');
  const clusterFilter = searchParams.get('cluster_id') ?? 'all',
    search = searchParams.get('q') ?? '';
  const update = (key: string, value: string | null) => {
    const next = new URLSearchParams(searchParams);
    if (!value || (value === 'all' && key !== 'tab')) next.delete(key);
    else next.set(key, value);
    setSearchParams(next, { replace: true });
  };
  const setTab = (value: string) => update('tab', value),
    setSelected = (value: string | null) => update('id', value);
  const clusters = useResource<{ items: Cluster[] }>('/clusters');
  const filter = {
    cluster_id: clusterFilter === 'all' ? undefined : clusterFilter,
    status: tab === 'pending' || tab === 'approved' ? tab : undefined,
    mine: tab === 'mine' ? 'true' : undefined,
  };
  const query = useInfiniteQuery({
    queryKey: ['api', 'approvals', filter],
    queryFn: ({ pageParam, signal }) =>
      request<Page<Approval>>(
        `/approvals${params({ ...filter, limit: 50, cursor: pageParam })}`,
        { signal },
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    refetchInterval: 5000,
  });
  const loaded = useMemo(
    () => query.data?.pages.flatMap((page) => page.items) ?? [],
    [query.data],
  );
  const items = useMemo(() => {
    const needle = search.toLowerCase();
    return loaded.filter((item) =>
      `${item.reason} ${item.sql} ${item.requester.name}`
        .toLowerCase()
        .includes(needle),
    );
  }, [loaded, search]);
  const clusterById = useMemo(
    () => new Map(clusters.data?.items.map((c) => [c.id, c]) ?? []),
    [clusters.data],
  );
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Approvals</h1>
          <p>Requests for a second person's review.</p>
        </div>
      </div>
      <div className="filterbar">
        <input
          aria-label="Search loaded approvals"
          placeholder="Search loaded requests…"
          value={search}
          onChange={(event) => update('q', event.target.value)}
        />
        <Picker
          label="Approval cluster"
          value={clusterFilter}
          onChange={(value) => update('cluster_id', value)}
          options={[
            { value: 'all', label: 'All clusters' },
            ...(clusters.data?.items.map((cluster) => ({
              value: cluster.id,
              label: cluster.name,
            })) ?? []),
          ]}
        />
      </div>
      <p className="loaded-note">
        {items.length} matching of {loaded.length} loaded requests
        {query.hasNextPage ? ' · More available' : ''}. Filter counts cover{' '}
        {counts.data?.items.length ?? '—'} loaded accessible requests. You
        cannot approve your own request.
        {counts.error ? ' Filter counts are unavailable.' : ''}
      </p>
      <TabBar
        value={tab}
        onChange={setTab}
        items={['pending', 'approved', 'mine', 'all'].map((value) => ({
          value,
          label: (
            <span>
              {value[0]?.toUpperCase() + value.slice(1)}{' '}
              <span className="muted">
                {counts.data?.items.filter(
                  (item) =>
                    value === 'all' ||
                    (value === 'mine'
                      ? item.requester.id === user?.id
                      : item.status === value),
                ).length ?? '—'}
                {counts.data?.next_cursor ? '+' : ''}
              </span>
            </span>
          ),
        }))}
      >
        <div className="approval-inbox-layout">
          <div>
            {query.isPending ? (
              <Skeleton />
            ) : query.error ? (
              <ErrorPanel
                error={query.error}
                retry={() => void query.refetch()}
              />
            ) : items.length === 0 ? (
              <Empty
                title={
                  tab === 'pending'
                    ? 'Nothing waiting for review'
                    : 'No matching requests'
                }
                description="Write queries can be submitted for approval from the console."
                action={
                  <Link className="button secondary" to="/console">
                    Open console
                  </Link>
                }
              />
            ) : (
              <ul className="approval-inbox-list" aria-label="Approval inbox">
                {items.map((approval) => {
                  const cluster = clusterById.get(approval.cluster_id);
                  return (
                    <li key={approval.id}>
                      <button
                        type="button"
                        className="approval-inbox-item"
                        aria-label="Review"
                        aria-describedby={`request-${approval.id}`}
                        aria-pressed={selected === approval.id}
                        onClick={() => setSelected(approval.id)}
                      >
                        <span className="requester-avatar" aria-hidden="true">
                          {approval.requester.name
                            .split(' ')
                            .map((part) => part[0])
                            .join('')
                            .slice(0, 2)}
                        </span>
                        <span>
                          <strong id={`request-${approval.id}`}>
                            {approval.reason}
                          </strong>
                          <small>
                            {approval.requester.name} · {approval.cluster_name}
                            {cluster && (
                              <>
                                {' '}
                                · <EnvBadge environment={cluster.environment} />
                              </>
                            )}
                          </small>
                          <small>
                            <RelativeTime value={approval.created_at} /> ·{' '}
                            <ExpiryTime value={approval.expires_at} />
                          </small>
                          <small>
                            <ApprovalBadge status={approval.status} />
                            {approval.error && <span>{approval.error}</span>}
                          </small>
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
          <section
            className="approval-inbox-detail"
            aria-label="Query approval"
          >
            <div className="section-heading">
              <h2>Query approval</h2>
              {selected && (
                <Button
                  variant="ghost"
                  aria-label="Close approval"
                  onClick={() => setSelected(null)}
                >
                  <X size={16} />
                </Button>
              )}
            </div>
            {!phone && selected ? (
              <ApprovalDetail
                key={selected}
                id={selected}
                note={notes[selected] ?? ''}
                setNote={(note) =>
                  setNotes((notes) => ({ ...notes, [selected]: note }))
                }
              />
            ) : (
              <Empty
                title="Select a request to review"
                description="Choose a request from the inbox."
              />
            )}
          </section>
        </div>
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
            {query.isFetchingNextPage ? 'Loading…' : 'Load more'}
          </Button>
        )}
      </TabBar>
      <Modal
        open={phone && !!selected}
        onOpenChange={(open) => {
          if (!open) setSelected(null);
        }}
        drawer
        className="approval-sheet"
        title="Query approval"
        description="Review the SQL, target environment, and reason before deciding."
        wide
      >
        {phone && selected && (
          <ApprovalDetail
            key={selected}
            id={selected}
            note={notes[selected] ?? ''}
            setNote={(note) =>
              setNotes((notes) => ({ ...notes, [selected]: note }))
            }
          />
        )}
      </Modal>
    </>
  );
}
export function ApprovalBadge({ status }: { status: Approval['status'] }) {
  return (
    <Badge
      variant="status"
      tone={
        status === 'pending'
          ? 'warning'
          : status === 'approved' || status === 'executed'
            ? 'success'
            : status === 'rejected' || status === 'failed'
              ? 'danger'
              : status === 'executing'
                ? 'info'
                : 'neutral'
      }
    >
      {status}
    </Badge>
  );
}
function ExpiryNotice({ expiresAt }: { expiresAt: string }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  return (
    <div className="approval-expiry" role="status">
      {expiryLabel(expiresAt, now)} · {formatTime(expiresAt)}
    </div>
  );
}
function ApprovalDetail({
  id,
  note,
  setNote,
}: {
  id: string;
  note: string;
  setNote: (note: string) => void;
}) {
  const path = `/approvals/${id}`;
  const approval = useQuery({
    queryKey: ['api', path],
    queryFn: ({ signal }) => request<Approval>(path, { signal }),
    refetchOnMount: 'always',
    refetchInterval: (query) =>
      query.state.data?.status === 'executing' ? 2000 : false,
  });
  const user = useUser();
  // Re-render once at the moment of expiry so review/execute controls are
  // withdrawn on time; the visible countdown lives in <ExpiryNotice />.
  const [now, setNow] = useState(() => Date.now());
  const expiresAt = approval.data?.expires_at;
  useEffect(() => {
    if (!expiresAt) return;
    const delay = Date.parse(expiresAt) - Date.now();
    setNow(Date.now());
    if (!Number.isFinite(delay) || delay <= 0 || delay > 2_147_483_000) return;
    const timer = setTimeout(() => setNow(Date.now()), delay + 20);
    return () => clearTimeout(timer);
  }, [expiresAt]);
  const [executeConfirm, setExecuteConfirm] = useState(false);
  const action = useAction<Approval>('Approval updated');
  const cluster = useResource<Cluster>(
    `/clusters/${approval.data?.cluster_id}`,
    !!approval.data,
  );
  const policy = useResource<Policy>(
    `/clusters/${approval.data?.cluster_id}/policy`,
    !!approval.data,
  );
  if (approval.isPending) return <Skeleton />;
  if (approval.error) return <ErrorPanel error={approval.error} />;
  const data = approval.data;
  const canReview =
    data.requester.id !== user?.id &&
    cluster.data?.my_access === 'admin' &&
    data.status === 'pending' &&
    Date.parse(data.expires_at) > now;
  const canExecute =
    data.status === 'approved' &&
    (data.requester.id === user?.id || data.reviewer?.id === user?.id) &&
    Date.parse(data.expires_at) > now;
  return (
    <div className="approval-detail">
      <div className="approval-scroll">
        <div className="section-toolbar">
          <strong>{data.cluster_name}</strong>
          <div className="inline">
            {cluster.data && (
              <EnvBadge environment={cluster.data.environment} />
            )}
            <ApprovalBadge status={data.status} />
          </div>
        </div>
        <div className="approval-reason">
          <strong>{data.requester.name}</strong>
          <span className="muted">{data.requester.email}</span>
          <p>{data.reason}</p>
        </div>
        <ExpiryNotice expiresAt={data.expires_at} />
        <Link to={`/approvals?id=${id}`}>Link to approval</Link>
        {data.error && (
          <div className="callout danger" role="alert">
            {data.error}
          </div>
        )}
        <pre
          className="sql-preview-block"
          tabIndex={0}
          aria-label="SQL preview"
        >
          {data.sql}
        </pre>
        {data.analysis.rewritten_sql &&
          guardedSqlChanged(data.sql, data.analysis.rewritten_sql) && (
            <details className="sql-comparison" open>
              <summary>Submitted SQL compared with guarded SQL</summary>
              <p className="muted">− removed · + added by analysis</p>
              <pre tabIndex={0}>
                {sqlDiff(data.sql, data.analysis.rewritten_sql).map(
                  (line, index) => (
                    <span className={`diff-line ${line.kind}`} key={index}>
                      {line.kind === 'added'
                        ? '+ '
                        : line.kind === 'removed'
                          ? '− '
                          : '  '}
                      {line.text}
                      {'\n'}
                    </span>
                  ),
                )}
              </pre>
            </details>
          )}
        <SafetyPanel
          analysis={data.analysis}
          maxRows={policy.data?.max_rows}
          clusterId={data.cluster_id}
        />
        {data.requester.id === user?.id && data.status === 'pending' && (
          <div className="callout warning">
            You requested this query. Another cluster administrator must review
            it.
          </div>
        )}
        {data.result && <ApprovalResult result={data.result} />}
        <ol className="timeline">
          <li>
            <strong>Created</strong>
            <span>
              <RelativeTime value={data.created_at} /> · {data.requester.name}
            </span>
          </li>
          <li className={data.reviewed_at ? '' : 'future'}>
            <strong>
              {data.status === 'rejected' ? 'Rejected' : 'Reviewed'}
            </strong>
            <span>
              {data.reviewed_at ? (
                <>
                  <RelativeTime value={data.reviewed_at} /> ·{' '}
                  {data.reviewer?.name}
                </>
              ) : (
                'Waiting for review'
              )}
            </span>
            {data.review_note && <p>{data.review_note}</p>}
          </li>
          <li className={data.executed_at ? '' : 'future'}>
            <strong>
              {data.status === 'failed'
                ? 'Failed'
                : data.status === 'executing'
                  ? 'Executing'
                  : 'Executed'}
            </strong>
            <span>
              {data.executed_at ? (
                <RelativeTime value={data.executed_at} />
              ) : (
                `Valid until ${formatTime(data.expires_at)}`
              )}
            </span>
          </li>
        </ol>
      </div>
      {(canReview || canExecute) && (
        <div className="approval-actions">
          {canReview && (
            <>
              <Field
                label="Review note"
                hint="Required when rejecting a request."
              >
                <textarea
                  value={note}
                  onChange={(e) => setNote(e.target.value)}
                  rows={2}
                />
              </Field>
              <div className="dialog-actions">
                <Button
                  variant="danger"
                  disabled={action.isPending || !note.trim()}
                  onClick={() =>
                    action.mutate({
                      path: `/approvals/${id}/reject`,
                      body: { note },
                    })
                  }
                >
                  {action.isPending ? 'Saving…' : 'Reject'}
                </Button>
                <Button
                  variant="primary"
                  disabled={action.isPending}
                  onClick={() =>
                    action.mutate({
                      path: `/approvals/${id}/approve`,
                      body: { note },
                    })
                  }
                >
                  {action.isPending ? 'Saving…' : 'Approve'}
                </Button>
              </div>
            </>
          )}
          {canExecute && (
            <Button
              variant="primary"
              disabled={action.isPending}
              onClick={() => setExecuteConfirm(true)}
            >
              Execute approved query
            </Button>
          )}
        </div>
      )}
      <Confirm
        open={executeConfirm}
        onOpenChange={setExecuteConfirm}
        title="Execute this approved query?"
        description={`This will apply the approved SQL on ${data.cluster_name}. Current policy and access are checked again by the server.`}
        busy={action.isPending}
        onConfirm={() =>
          action.mutate(
            { path: `/approvals/${id}/execute` },
            {
              onSuccess: () => setExecuteConfirm(false),
              onError: () => {
                setExecuteConfirm(false);
                void approval.refetch();
              },
            },
          )
        }
      />
    </div>
  );
}
