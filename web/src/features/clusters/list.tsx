import { useMemo } from 'react';
import { useSearchParams } from 'react-router-dom';
import { Plus, LayoutGrid, List, Search } from 'lucide-react';
import { DiscoveredResources } from '../discovery/resources';
import type {
  Cluster,
  Project,
  ResourcePage,
  Page,
  HistoryEntry,
} from '../../api/types';
import { useResource } from '../../lib/query';
import { useUser } from '../auth/session';
import {
  Badge,
  Button,
  Empty,
  ErrorPanel,
  Picker,
  Skeleton,
  TabBar,
  SegmentedControl,
} from '../../components/ui';
import { AddCluster } from './forms';
import { ClusterCollection } from './collection';
const healthOrder = ['down', 'degraded', 'unknown', 'healthy'];
export function Clusters() {
  const [params, setParams] = useSearchParams();
  const clusters = useResource<{ items: Cluster[] }>('/clusters'),
    projects = useResource<{ items: Project[] }>('/projects');
  const user = useUser();
  const history = useResource<Page<HistoryEntry>>('/history?limit=200');
  const discovered = useResource<ResourcePage>(
    '/discovery/resources?limit=1',
    user?.org_role === 'admin',
  );
  const tab =
    user?.org_role === 'admin' && params.get('tab') === 'discovered'
      ? 'discovered'
      : 'managed';
  const update = (key: string, value: string) => {
    const next = new URLSearchParams(params);
    if (value === 'all' || value === '') next.delete(key);
    else next.set(key, value);
    setParams(next, { replace: true });
  };
  const search = params.get('q') ?? '',
    environment = params.get('environment') ?? 'all',
    engine = params.get('engine') ?? 'all',
    provider = params.get('provider') ?? 'all',
    health = params.get('health') ?? 'all',
    view = params.get('view') ?? 'table',
    sort = params.get('sort') ?? 'name';
  const setSearch = (value: string) => update('q', value),
    setEnvironment = (value: string) => update('environment', value),
    setEngine = (value: string) => update('engine', value),
    setProvider = (value: string) => update('provider', value),
    setHealth = (value: string) => update('health', value),
    setView = (value: string) => update('view', value);
  const adding = params.get('add') === 'true',
    setAdding = (value: boolean) => update('add', value ? 'true' : '');
  const projectId = params.get('project_id');
  const items = useMemo(() => {
    const needle = search.toLowerCase();
    return (
      clusters.data?.items
        .filter(
          (c) =>
            (!projectId || c.project_id === projectId) &&
            `${c.name} ${c.host} ${Object.values(c.tags).join(' ')}`
              .toLowerCase()
              .includes(needle) &&
            (environment === 'all' || environment === c.environment) &&
            (engine === 'all' || engine === c.engine) &&
            (provider === 'all' || provider === c.provider) &&
            (health === 'all' || health === c.health.status),
        )
        .sort((a, b) =>
          sort === 'health'
            ? healthOrder.indexOf(a.health.status) -
                healthOrder.indexOf(b.health.status) ||
              a.name.localeCompare(b.name)
            : a.name.localeCompare(b.name),
        ) ?? []
    );
  }, [
    clusters.data,
    projectId,
    search,
    environment,
    engine,
    provider,
    health,
    sort,
  ]);
  const filter = (
    value: string,
    onChange: (value: string) => void,
    label: string,
    values: string[],
  ) => (
    <Picker
      value={value}
      onChange={onChange}
      label={label}
      options={[
        { value: 'all', label: `All ${label.toLowerCase()}` },
        ...values.map((value) => ({
          value,
          label:
            (
              {
                postgres: 'PostgreSQL',
                mysql: 'MySQL',
                aws: 'AWS',
                gcp: 'GCP',
                onprem: 'On-premises',
              } as Record<string, string>
            )[value] ?? value.charAt(0).toUpperCase() + value.slice(1),
        })),
      ]}
    />
  );
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>
            Clusters{' '}
            <span className="heading-count">
              {clusters.data?.items.length ?? '—'}
            </span>
          </h1>
          <p>Your databases, with access and health in one place.</p>
        </div>
        {user?.org_role === 'admin' && (
          <Button variant="primary" onClick={() => setAdding(true)}>
            <Plus size={16} />
            Add cluster
          </Button>
        )}
      </div>
      <TabBar
        value={tab}
        onChange={(value) => {
          const next = new URLSearchParams(params);
          next.set('tab', value);
          setParams(next, { replace: true });
        }}
        items={[
          { value: 'managed', label: 'Connected' },
          ...(user?.org_role === 'admin'
            ? [
                {
                  value: 'discovered',
                  label: (
                    <span className="inline">
                      Discovered{' '}
                      <Badge tone="info">
                        {discovered.data?.counts.new ?? '—'}
                      </Badge>
                    </span>
                  ),
                },
              ]
            : []),
        ]}
      >
        {tab === 'discovered' ? (
          <DiscoveredResources />
        ) : (
          <>
            <div className="filterbar">
              <div className="search-input">
                <Search size={16} />
                <input
                  aria-label="Search clusters"
                  placeholder="Search clusters or hosts…"
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                />
              </div>
              <Picker
                value={projectId ?? 'all'}
                onChange={(value) => update('project_id', value)}
                label="Filter project"
                options={[
                  { value: 'all', label: 'All projects' },
                  ...(projects.data?.items ?? []).map((project) => ({
                    value: project.id,
                    label: project.name,
                  })),
                ]}
              />
              {filter(environment, setEnvironment, 'Environments', [
                'production',
                'staging',
                'development',
              ])}
              {filter(engine, setEngine, 'Engines', ['postgres', 'mysql'])}
              {filter(provider, setProvider, 'Providers', [
                'aws',
                'gcp',
                'azure',
                'onprem',
                'other',
              ])}
              {filter(health, setHealth, 'Health states', [
                'healthy',
                'degraded',
                'down',
                'unknown',
              ])}
              <Picker
                value={sort}
                onChange={(value) => update('sort', value)}
                label="Sort clusters"
                options={[
                  { value: 'name', label: 'Name A–Z' },
                  { value: 'health', label: 'Needs attention first' },
                ]}
              />
              <SegmentedControl
                label="Cluster view"
                value={view}
                onChange={setView}
                items={[
                  {
                    value: 'cards',
                    label: <LayoutGrid size={16} />,
                    accessibleLabel: 'Card view',
                  },
                  {
                    value: 'table',
                    label: <List size={16} />,
                    accessibleLabel: 'Table view',
                  },
                ]}
              />
            </div>
            {clusters.isPending || projects.isPending ? (
              <Skeleton />
            ) : clusters.error || projects.error ? (
              <ErrorPanel
                error={clusters.error ?? projects.error}
                retry={() => {
                  void clusters.refetch();
                  void projects.refetch();
                }}
              />
            ) : items.length === 0 ? (
              <Empty
                title="No matching clusters"
                description="Try fewer filters, or add a database to your workspace."
                action={
                  <Button
                    onClick={() => {
                      const next = new URLSearchParams(params);
                      for (const key of [
                        'q',
                        'environment',
                        'engine',
                        'provider',
                        'health',
                        'project_id',
                      ])
                        next.delete(key);
                      setParams(next, { replace: true });
                    }}
                  >
                    Clear filters
                  </Button>
                }
              />
            ) : (
              <ClusterCollection
                items={items}
                projects={projects.data?.items ?? []}
                history={history.data?.items ?? []}
                view={view}
              />
            )}
          </>
        )}
      </TabBar>
      <AddCluster open={adding} onOpenChange={setAdding} />
    </>
  );
}
