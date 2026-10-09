import { useMemo } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import type { Cluster, HistoryEntry, Project } from '../../api/types';
import { Badge, EngineIcon, HealthBadge } from '../../components/ui';
import { RelativeTime } from '../../components/ui/time';
const environments = ['production', 'staging', 'development'] as const;
/** Environment-first inventory. Rows keep real links and support arrow/Enter navigation. */
export function ClusterCollection({
  items,
  projects,
  history,
  view,
}: {
  items: Cluster[];
  projects: Project[];
  history: HistoryEntry[];
  view: string;
}) {
  const navigate = useNavigate();
  const projectNames = useMemo(
    () => new Map(projects.map((project) => [project.id, project.name])),
    [projects],
  );
  const lastQueried = useMemo(() => {
    const latest = new Map<string, string>();
    for (const entry of history) {
      const previous = latest.get(entry.cluster_id);
      if (!previous || Date.parse(entry.created_at) > Date.parse(previous))
        latest.set(entry.cluster_id, entry.created_at);
    }
    return latest;
  }, [history]);
  return environments.map((environment) => {
    const clusters = items.filter(
      (cluster) => cluster.environment === environment,
    );
    if (!clusters.length) return null;
    const name = environment.charAt(0).toUpperCase() + environment.slice(1);
    return (
      <section
        className="environment-group"
        key={environment}
        aria-label={`${name} inventory`}
      >
        <div className={`environment-group-heading ${environment}`}>
          <h2>{name}</h2>
          <span className="muted">
            {clusters.length} {clusters.length === 1 ? 'cluster' : 'clusters'}
          </span>
        </div>
        {view === 'cards' ? (
          <div className="cluster-cards">
            {clusters.map((cluster) => (
              <Link
                className="cluster-card"
                key={cluster.id}
                to={`/clusters/${cluster.id}`}
              >
                <div className="cluster-card-top">
                  <EngineIcon engine={cluster.engine} />
                  <Badge>{`${cluster.my_access ?? 'No'} access`}</Badge>
                </div>
                <h3>{cluster.name}</h3>
                <p className="provider">
                  {projectNames.get(cluster.project_id) ?? '—'} ·{' '}
                  {cluster.region}
                  <span
                    className="cluster-card-host mono"
                    title={`${cluster.host}:${cluster.port}`}
                  >
                    {cluster.host}:{cluster.port}
                  </span>
                </p>
                <div className="cluster-card-health">
                  <HealthBadge
                    status={cluster.health.status}
                    latency={cluster.health.latency_ms}
                  />
                </div>
              </Link>
            ))}
          </div>
        ) : (
          <div className="table-wrap">
            <table
              className="cluster-inventory"
              aria-label={`${name} clusters`}
            >
              <thead>
                <tr>
                  <th>Name / engine</th>
                  <th>Health</th>
                  <th>Host</th>
                  <th>Project</th>
                  <th>Your access</th>
                  <th>Last queried</th>
                </tr>
              </thead>
              <tbody>
                {clusters.map((cluster) => (
                  <tr
                    key={cluster.id}
                    tabIndex={0}
                    aria-label={`Open ${cluster.name}`}
                    onClick={(event) => {
                      if (!(event.target as HTMLElement).closest('a'))
                        navigate(`/clusters/${cluster.id}`);
                    }}
                    onKeyDown={(event) => {
                      if (event.target !== event.currentTarget) return;
                      if (event.key === 'Enter' || event.key === ' ') {
                        event.preventDefault();
                        navigate(`/clusters/${cluster.id}`);
                      } else if (
                        ['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(
                          event.key,
                        )
                      ) {
                        event.preventDefault();
                        const rows = Array.from(
                          event.currentTarget
                            .closest('.tab-content')
                            ?.querySelectorAll<HTMLTableRowElement>(
                              '.cluster-inventory tbody tr',
                            ) ?? [],
                        );
                        const index = rows.indexOf(event.currentTarget);
                        const next =
                          event.key === 'Home'
                            ? 0
                            : event.key === 'End'
                              ? rows.length - 1
                              : Math.max(
                                  0,
                                  Math.min(
                                    rows.length - 1,
                                    index +
                                      (event.key === 'ArrowDown' ? 1 : -1),
                                  ),
                                );
                        rows[next]?.focus();
                      }
                    }}
                  >
                    <td>
                      <Link
                        className="inline strong identifier"
                        to={`/clusters/${cluster.id}`}
                      >
                        <EngineIcon engine={cluster.engine} />
                        {cluster.name}
                      </Link>
                    </td>
                    <td>
                      <HealthBadge
                        status={cluster.health.status}
                        latency={cluster.health.latency_ms}
                      />
                    </td>
                    <td>
                      <span
                        className="inventory-host mono muted"
                        title={`${cluster.host}:${cluster.port}`}
                      >
                        {cluster.host}:{cluster.port}
                      </span>
                    </td>
                    <td>{projectNames.get(cluster.project_id) ?? '—'}</td>
                    <td>
                      <Badge>{cluster.my_access ?? 'No access'}</Badge>
                    </td>
                    <td
                      title={
                        !lastQueried.has(cluster.id)
                          ? 'No query in the loaded recent history'
                          : undefined
                      }
                    >
                      <RelativeTime
                        value={lastQueried.get(cluster.id) ?? null}
                      />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    );
  });
}
