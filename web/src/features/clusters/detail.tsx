import { lazy, Suspense, useState } from 'react';
import { useParams, useSearchParams } from 'react-router-dom';
import type { Cluster, Grant } from '../../api/types';
import { useResource } from '../../lib/query';
import { Button, ErrorPanel, Skeleton, TabBar } from '../../components/ui';
const ConsoleWorkspace = lazy(() =>
  import('../console/console').then((module) => ({
    default: module.ConsoleWorkspace,
  })),
);
import { SchemaExplorer } from '../console/schema-tree';
import { AddGrant, GrantsTable } from '../admin/grants';
import { HealthPage } from './health';
import { PolicyPage } from './policy';
import { ClusterSettings } from './settings';
import { ClusterHeading } from '../../components/ui/cluster-heading';
import { ClusterDriftBanner } from '../discovery/drift';
export function ClusterDetail() {
  const { id } = useParams();
  const query = useResource<Cluster>(`/clusters/${id}`);
  const [params, setParams] = useSearchParams();
  const paramSql = params.get('sql') ?? undefined;
  const [sql, setSql] = useState<string | undefined>(paramSql);
  // Follow `?sql=` changes while mounted; tab changes drop the param, which
  // must not discard SQL handed over from the schema explorer.
  const [seenParamSql, setSeenParamSql] = useState(paramSql);
  if (paramSql !== seenParamSql) {
    setSeenParamSql(paramSql);
    if (paramSql !== undefined) setSql(paramSql);
  }
  if (query.isPending || query.error)
    return (
      <>
        <div className="page-heading">
          <h1>Cluster</h1>
        </div>
        {query.isPending ? (
          <Skeleton />
        ) : (
          <ErrorPanel error={query.error!} retry={() => void query.refetch()} />
        )}
      </>
    );
  const cluster = query.data;
  const admin = cluster.my_access === 'admin';
  const tab = params.get('tab') ?? 'console';
  const changeTab = (value: string) => setParams({ tab: value });
  return (
    <>
      <ClusterHeading cluster={cluster}>
        <ClusterDriftBanner cluster={cluster} />
      </ClusterHeading>
      <TabBar
        className="cluster-tabs"
        value={tab}
        onChange={changeTab}
        items={[
          { value: 'console', label: 'Console' },
          { value: 'schema', label: 'Schema' },
          { value: 'health', label: 'Health' },
          ...(admin ? [{ value: 'access', label: 'Access' }] : []),
          { value: 'policy', label: 'Policy' },
          ...(admin ? [{ value: 'settings', label: 'Settings' }] : []),
        ]}
      >
        <div className="cluster-tab-content">
          {tab === 'console' && (
            <Suspense fallback={<Skeleton />}>
              <ConsoleWorkspace
                key={`${cluster.id}-${sql ?? ''}`}
                cluster={cluster}
                initialSql={sql}
              />
            </Suspense>
          )}
          {tab === 'schema' && (
            <div className="full-schema">
              <SchemaExplorer
                clusterId={cluster.id}
                engine={cluster.engine}
                onQuery={(sql) => {
                  setSql(sql);
                  changeTab('console');
                }}
              />
            </div>
          )}
          {tab === 'health' && <HealthPage cluster={cluster} />}
          {tab === 'access' && admin && <ClusterAccess cluster={cluster} />}
          {tab === 'policy' && <PolicyPage cluster={cluster} />}
          {tab === 'settings' && admin && (
            <ClusterSettings key={cluster.updated_at} cluster={cluster} />
          )}
          {![
            'console',
            'schema',
            'health',
            'access',
            'policy',
            'settings',
          ].includes(tab) && (
            <Button onClick={() => changeTab('console')}>Open console</Button>
          )}
        </div>
      </TabBar>
    </>
  );
}
function ClusterAccess({ cluster }: { cluster: Cluster }) {
  const [adding, setAdding] = useState(false);
  const query = useResource<{ items: Grant[] }>(
    `/clusters/${cluster.id}/grants`,
  );
  return (
    <>
      {query.isPending ? (
        <Skeleton />
      ) : query.error ? (
        <ErrorPanel error={query.error} retry={() => void query.refetch()} />
      ) : (
        <GrantsTable items={query.data.items} onAdd={() => setAdding(true)} />
      )}
      <AddGrant cluster={cluster} open={adding} onOpenChange={setAdding} />
    </>
  );
}
