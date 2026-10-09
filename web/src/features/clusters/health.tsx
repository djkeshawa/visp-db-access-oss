import type { Health, Cluster } from '../../api/types';
import { useAction, useResource } from '../../lib/query';
import { Button, ErrorPanel, HealthBadge, Skeleton } from '../../components/ui';
import { formatTime } from '../../lib/utils';
type HealthResponse = {
  current: Health;
  history: {
    status: Health['status'];
    latency_ms: number | null;
    checked_at: string;
  }[];
};
export function HealthPage({ cluster }: { cluster: Cluster }) {
  const query = useResource<HealthResponse>(
    `/clusters/${cluster.id}/health?hours=24`,
  );
  const check = useAction('Health check completed');
  if (query.isPending) return <Skeleton />;
  if (query.error)
    return (
      <ErrorPanel error={query.error} retry={() => void query.refetch()} />
    );
  const { current, history } = query.data;
  const max = Math.max(10, ...history.map((h) => h.latency_ms ?? 0)) * 1.25;
  const paths: string[] = [];
  let path = '';
  history.forEach((point, i) => {
    if (point.latency_ms === null) {
      if (path) paths.push(path);
      path = '';
    } else
      path += `${path ? ' L' : 'M'} ${36 + (i / Math.max(1, history.length - 1)) * 704} ${190 - (point.latency_ms / max) * 154}`;
  });
  if (path) paths.push(path);
  const connections = current.active_connections ?? 0,
    capacity = current.max_connections ?? 0;
  return (
    <>
      <div className="section-toolbar">
        <h2>Cluster health</h2>
        {cluster.my_access === 'admin' && (
          <Button
            disabled={check.isPending}
            onClick={() =>
              check.mutate({ path: `/clusters/${cluster.id}/health/check` })
            }
          >
            {check.isPending ? 'Checking…' : 'Check now'}
          </Button>
        )}
      </div>
      <div className="health-summary">
        <div>
          <span className="muted">Current status</span>
          <HealthBadge status={current.status} latency={current.latency_ms} />
          <small>Checked {formatTime(current.checked_at)}</small>
        </div>
        <div>
          <span className="muted">Server version</span>
          <strong>{current.server_version ?? 'Unavailable'}</strong>
          <small>
            {current.is_replica ? 'Read replica' : 'Primary server'}
          </small>
        </div>
        <div>
          <span className="muted">Connections</span>
          <strong>
            {current.active_connections ?? '—'} /{' '}
            {current.max_connections ?? '—'}
          </strong>
          <div className="connections-bar">
            <i
              style={{
                width: `${capacity ? Math.min(100, (connections / capacity) * 100) : 0}%`,
              }}
            />
          </div>
        </div>
      </div>
      {current.error && (
        <div className="callout danger" role="alert">
          {current.error}
        </div>
      )}
      <div className="chart-card">
        <h3>Connection latency</h3>
        <p className="muted">
          Last 24 hours · milliseconds · gaps indicate unavailable probes
        </p>
        <svg
          className="latency-chart"
          viewBox="0 0 780 235"
          role="img"
          aria-label="Latency over the last 24 hours"
        >
          {[0, 0.5, 1].map((fraction) => (
            <g key={fraction}>
              <line
                x1="36"
                x2="740"
                y1={190 - fraction * 154}
                y2={190 - fraction * 154}
              />
              <text x="2" y={194 - fraction * 154}>
                {Math.round(max * fraction)}
              </text>
            </g>
          ))}
          {paths.map((d, i) => (
            <path d={d} key={i} fill="none" />
          ))}
          <text x="36" y="223">
            24 h ago
          </text>
          <text x="376" y="223">
            12 h ago
          </text>
          <text x="710" y="223">
            Now
          </text>
        </svg>
      </div>
    </>
  );
}
