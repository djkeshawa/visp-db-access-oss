import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { ArrowUpRight, CheckCheck, Clock3 } from 'lucide-react';
import type { Approval, Cluster, HistoryEntry, Page } from '../../api/types';
import { useResource } from '../../lib/query';
import { useUser } from '../auth/session';
import {
  Empty,
  EnvBadge,
  ErrorPanel,
  HealthBadge,
  SegmentedControl,
  Skeleton,
} from '../../components/ui';
import { RelativeTime, ExpiryTime } from '../../components/ui/time';
import { HistoryTable } from '../history/history';
/** Ticks every 30s so expired approvals drop off without waiting for a refetch. */
function useMinuteClock() {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30000);
    return () => clearInterval(timer);
  }, []);
  return now;
}
function RequestRow({ approval, mine }: { approval: Approval; mine: boolean }) {
  return (
    <Link className="needs-row" to={`/approvals?id=${approval.id}&tab=all`}>
      {mine ? <Clock3 size={16} /> : <CheckCheck size={16} />}
      <div>
        <strong>
          {mine ? 'Your pending request' : 'Waiting for your review'} ·{' '}
          {approval.cluster_name}
        </strong>
        <small>
          {approval.reason} · {approval.requester.name} ·{' '}
          <RelativeTime value={approval.created_at} />
        </small>
      </div>
      <ExpiryTime value={approval.expires_at} />
      <ArrowUpRight size={16} />
    </Link>
  );
}
/** Actionable work comes from scoped approval and cluster APIs, never fabricated totals. */
export function NeedsYou({ unhealthy }: { unhealthy: Cluster[] }) {
  const user = useUser();
  const mine = useResource<Page<Approval>>(
      '/approvals?mine=true&status=pending&limit=5',
    ),
    inbox = useResource<Page<Approval>>('/approvals?status=pending&limit=50'),
    clusters = useResource<{ items: Cluster[] }>('/clusters');
  const now = useMinuteClock();
  const waiting = useMemo(() => {
    const adminClusters = new Set(
      clusters.data?.items
        .filter((cluster) => cluster.my_access === 'admin')
        .map((cluster) => cluster.id),
    );
    return (
      inbox.data?.items.filter(
        (approval) =>
          approval.requester.id !== user?.id &&
          Date.parse(approval.expires_at) > now &&
          adminClusters.has(approval.cluster_id),
      ) ?? []
    );
  }, [inbox.data, clusters.data, user?.id, now]);
  const pending = useMemo(
    () =>
      mine.data?.items.filter(
        (approval) => Date.parse(approval.expires_at) > now,
      ) ?? [],
    [mine.data, now],
  );
  const unhealthyTargets = useMemo(
    () =>
      unhealthy.filter((cluster) =>
        ['down', 'degraded'].includes(cluster.health.status),
      ),
    [unhealthy],
  );
  const error = mine.error ?? inbox.error ?? clusters.error;
  return (
    <section className="needs-you">
      <h2>Needs you</h2>
      {mine.isPending || inbox.isPending || clusters.isPending ? (
        <Skeleton />
      ) : error ? (
        <ErrorPanel error={error} />
      ) : (
        <>
          {!waiting.length && !pending.length && !unhealthyTargets.length && (
            <Empty
              title="You're all caught up"
              description="No pending reviews, requests, or unhealthy clusters."
            />
          )}
          {waiting.map((approval) => (
            <RequestRow key={approval.id} approval={approval} mine={false} />
          ))}
          {pending.map((approval) => (
            <RequestRow key={approval.id} approval={approval} mine />
          ))}
          {unhealthyTargets.map((cluster) => (
            <Link
              className="needs-row"
              key={cluster.id}
              to={`/clusters/${cluster.id}?tab=health`}
            >
              <span className={`status-dot ${cluster.health.status}`} />
              <div>
                <strong>{cluster.name}</strong>{' '}
                <EnvBadge environment={cluster.environment} />
                <small>
                  {cluster.health.error ?? 'Elevated connection latency'}
                </small>
              </div>
              <HealthBadge
                status={cluster.health.status}
                latency={cluster.health.latency_ms}
              />
              <ArrowUpRight size={16} />
            </Link>
          ))}
          {(mine.data?.next_cursor || inbox.data?.next_cursor) && (
            <Link to="/approvals">More requests in the inbox</Link>
          )}
        </>
      )}
    </section>
  );
}
export function MyWork() {
  const user = useUser(),
    [mode, setMode] = useState('mine');
  const everyone = user?.org_role === 'admin' && mode === 'everyone';
  const clusters = useResource<{ items: Cluster[] }>('/clusters');
  const history = useResource<Page<HistoryEntry>>(
    `/history?${everyone ? '' : `user_id=${user?.id}&`}limit=5`,
    everyone || !!user?.id,
  );
  return (
    <section className="recent-queries">
      <div className="section-heading">
        <h2>Recent queries</h2>
        <div className="inline wrap">
          {user?.org_role === 'admin' && (
            <SegmentedControl
              label="Recent query scope"
              value={mode}
              onChange={setMode}
              items={[
                { value: 'mine', label: 'Mine' },
                { value: 'everyone', label: 'Everyone' },
              ]}
            />
          )}
          <Link
            to={everyone || !user ? '/history' : `/history?user_id=${user.id}`}
          >
            View history
          </Link>
        </div>
      </div>
      {history.isPending ? (
        <Skeleton />
      ) : history.error ? (
        <ErrorPanel
          error={history.error}
          retry={() => void history.refetch()}
        />
      ) : (
        <HistoryTable
          items={history.data.items}
          clusters={clusters.data?.items ?? []}
          showUser={everyone}
        />
      )}
    </section>
  );
}
