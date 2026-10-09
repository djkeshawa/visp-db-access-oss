import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { Cloud, Database, Layers, Search, AlertTriangle } from 'lucide-react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { json, params, request } from '../../api/client';
import type {
  DiscoveredResource,
  DiscoverySource,
  ResourcePage,
  ResourceStatus,
} from '../../api/types';
import { useAction, useResource } from '../../lib/query';
import {
  Badge,
  Button,
  Empty,
  EnvBadge,
  EngineIcon,
  ErrorPanel,
  Picker,
  Skeleton,
} from '../../components/ui';
import { useToast } from '../../components/ui/toast';
import { message } from '../../lib/utils';
import { driftLabels, groupRegions } from './helpers';
import { useDiscoveryPages } from './queries';
import { ImportResource } from './import';
import { ReviewDrift } from './drift';

const allRegions = groupRegions().flatMap((group) => group.regions);
/** Lags a fast-changing value so search typing does not query on every keystroke. */
function useDebounced<T>(value: T, delay = 300) {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);
  return debounced;
}
function ResourceFlags({ resource }: { resource: DiscoveredResource }) {
  return (
    <div className="discovery-chips">
      {resource.publicly_accessible && (
        <Badge tone="warning">
          <AlertTriangle size={12} /> Publicly accessible
        </Badge>
      )}
      {!resource.encrypted && (
        <Badge tone="warning">
          <AlertTriangle size={12} /> Unencrypted
        </Badge>
      )}
      {resource.iam_auth_enabled && <Badge>IAM auth</Badge>}
    </div>
  );
}
function ResourceStatusBadge({ resource }: { resource: DiscoveredResource }) {
  const badge = (
    <Badge
      variant="status"
      tone={resource.status === 'gone' ? 'warning' : 'neutral'}
    >
      {resource.status}
    </Badge>
  );
  return resource.cluster_id ? (
    <Link to={`/clusters/${resource.cluster_id}`}>{badge} ↗</Link>
  ) : (
    badge
  );
}
export function DiscoveredResources() {
  const sources = useResource<{ items: DiscoverySource[] }>(
    '/discovery/sources',
  );
  const [filters, setFilters] = useState({
    status: 'all',
    engine: 'all',
    region: 'all',
    source_id: 'all',
    q: '',
  });
  const set = (key: keyof typeof filters, value: string) =>
    setFilters((filters) => ({ ...filters, [key]: value }));
  const debouncedQ = useDebounced(filters.q.trim());
  const q = filters.q.trim() ? debouncedQ : '';
  const path = `/discovery/resources${params({ ...Object.fromEntries(Object.entries({ ...filters, q }).map(([key, value]) => [key, value === 'all' ? '' : value])), limit: 20 })}`;
  const query = useDiscoveryPages<DiscoveredResource, ResourcePage>(path);
  const counts = query.data?.pages[0]?.counts;
  return (
    <>
      <div className="overview-strip">
        <Cloud size={18} />
        <span>AWS RDS & Aurora</span>
        <span className="muted">
          Review discovered databases, then import with credentials.
        </span>
        <Link to="/settings?tab=discovery">Manage sources →</Link>
      </div>
      <div
        className="tabs discovery-status"
        role="group"
        aria-label="Resource status"
      >
        {['all', 'new', 'imported', 'ignored', 'gone'].map((status) => (
          <button
            className="tab"
            type="button"
            aria-pressed={filters.status === status}
            key={status}
            onClick={() => set('status', status)}
          >
            {status === 'all'
              ? 'All'
              : status.charAt(0).toUpperCase() + status.slice(1)}{' '}
            <Badge>
              {counts
                ? status === 'all'
                  ? Object.values(counts).reduce((sum, n) => sum + n, 0)
                  : counts[status as ResourceStatus]
                : '—'}
            </Badge>
          </button>
        ))}
      </div>
      <div className="filterbar">
        <div className="search-input">
          <Search size={16} />
          <input
            aria-label="Search discovered databases"
            placeholder="Search identifier, endpoint, or ARN…"
            value={filters.q}
            onChange={(e) => set('q', e.target.value)}
          />
        </div>
        <Picker
          label="Discovery engine"
          value={filters.engine}
          onChange={(value) => set('engine', value)}
          options={[
            { value: 'all', label: 'All engines' },
            { value: 'postgres', label: 'PostgreSQL' },
            { value: 'mysql', label: 'MySQL / MariaDB' },
          ]}
        />
        <Picker
          label="Discovery region"
          value={filters.region}
          onChange={(value) => set('region', value)}
          options={[
            { value: 'all', label: 'All regions' },
            ...allRegions.map((region) => ({
              value: region.code,
              label: region.code,
            })),
          ]}
        />
        <Picker
          label="Discovery source"
          value={filters.source_id}
          onChange={(value) => set('source_id', value)}
          options={[
            { value: 'all', label: 'All sources' },
            ...(sources.data?.items ?? []).map((source) => ({
              value: source.id,
              label: source.name,
            })),
          ]}
        />
        <Button
          onClick={() =>
            setFilters({
              status: 'all',
              engine: 'all',
              region: 'all',
              source_id: 'all',
              q: '',
            })
          }
        >
          Clear filters
        </Button>
      </div>
      {sources.error && (
        <ErrorPanel
          error={sources.error}
          retry={() => void sources.refetch()}
        />
      )}
      {query.isPending ? (
        <Skeleton />
      ) : query.error ? (
        <ErrorPanel error={query.error} retry={() => void query.refetch()} />
      ) : (
        <ResourceResults
          key={path}
          items={query.data.pages.flatMap((page) => page.items)}
          sources={sources.data?.items ?? []}
          hasNext={query.hasNextPage}
          loading={query.isFetchingNextPage}
          loadMore={() => void query.fetchNextPage()}
        />
      )}
    </>
  );
}
function ResourceResults({
  items,
  sources,
  hasNext,
  loading,
  loadMore,
}: {
  items: DiscoveredResource[];
  sources: DiscoverySource[];
  hasNext: boolean;
  loading: boolean;
  loadMore: () => void;
}) {
  const [selected, setSelected] = useState<string[]>([]),
    [importing, setImporting] = useState<DiscoveredResource | null>(null),
    [reviewing, setReviewing] = useState<DiscoveredResource | null>(null);
  const action = useAction('Resource updated');
  const cache = useQueryClient(),
    toast = useToast();
  const selectable = useMemo(
    () =>
      items
        .filter((resource) => resource.status === 'new')
        .map((resource) => resource.id),
    [items],
  );
  // Rows that refetched into another status can no longer be bulk-ignored.
  const chosen = useMemo(() => {
    const available = new Set(selectable);
    return selected.filter((id) => available.has(id));
  }, [selectable, selected]);
  const chosenSet = useMemo(() => new Set(chosen), [chosen]);
  const bulk = useMutation({
    mutationFn: async () => {
      const results = await Promise.allSettled(
        chosen.map((id) =>
          request(`/discovery/resources/${id}/ignore`, json('POST')),
        ),
      );
      const failed = chosen.filter(
        (_, index) => results[index]?.status === 'rejected',
      );
      const firstError = results.find((result) => result.status === 'rejected');
      return { failed, firstError, succeeded: results.length - failed.length };
    },
    onSuccess: ({ failed, firstError, succeeded }) => {
      setSelected(failed);
      void cache.invalidateQueries({
        predicate: ({ queryKey }) =>
          typeof queryKey[1] === 'string' &&
          queryKey[1].startsWith('/discovery'),
      });
      if (succeeded) toast(`${succeeded} resources ignored`);
      if (firstError?.status === 'rejected')
        toast(message(firstError.reason), 'error');
    },
    onError: (error) => toast(message(error), 'error'),
  });
  const toggle = (id: string) =>
    setSelected((ids) =>
      ids.includes(id) ? ids.filter((value) => value !== id) : [...ids, id],
    );
  if (!items.length)
    return (
      <Empty
        title="No discovered databases"
        description="Add an AWS source in Settings → Cloud discovery and run a scan. If a source is already configured, try clearing these filters."
        action={
          <Link className="button primary" to="/settings?tab=discovery">
            Add a discovery source
          </Link>
        }
      />
    );
  return (
    <>
      {chosen.length > 0 && (
        <div className="discovery-bulk">
          <span role="status">{chosen.length} selected</span>
          <Button
            disabled={bulk.isPending || action.isPending}
            onClick={() => bulk.mutate()}
          >
            {bulk.isPending ? 'Ignoring…' : 'Ignore selected'}
          </Button>
          <Button disabled={bulk.isPending} onClick={() => setSelected([])}>
            Clear selection
          </Button>
        </div>
      )}
      <div className="table-wrap">
        <table className="discovered-table">
          <thead>
            <tr>
              <th>
                <input
                  type="checkbox"
                  aria-label="Select all new resources on this page"
                  disabled={!selectable.length || bulk.isPending}
                  checked={
                    !!selectable.length &&
                    selectable.every((id) => chosenSet.has(id))
                  }
                  onChange={(e) =>
                    setSelected(e.target.checked ? selectable : [])
                  }
                />
              </th>
              <th>Database</th>
              <th>Account / region</th>
              <th>Endpoint</th>
              <th>Environment / flags</th>
              <th>Status / drift</th>
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            {items.map((resource) => (
              <tr key={resource.id}>
                <td>
                  {resource.status === 'new' && (
                    <input
                      type="checkbox"
                      aria-label={`Select ${resource.identifier}`}
                      checked={chosenSet.has(resource.id)}
                      disabled={bulk.isPending}
                      onChange={() => toggle(resource.id)}
                    />
                  )}
                </td>
                <td>
                  <div className="inline strong">
                    {resource.kind === 'aurora_cluster' ? (
                      <Layers size={18} aria-label="Aurora" />
                    ) : (
                      <Database size={18} aria-label="RDS" />
                    )}
                    <EngineIcon engine={resource.engine_detail} />
                    <span>{resource.identifier}</span>
                  </div>
                  <small className="muted">
                    {resource.kind === 'aurora_cluster' ? 'Aurora' : 'RDS'} ·{' '}
                    {resource.engine_detail} {resource.engine_version}
                  </small>
                </td>
                <td>
                  <div className="discovery-stack">
                    <span className="mono">{resource.account_id}</span>
                    <span>{resource.region}</span>
                    <small className="muted">{resource.source_name}</small>
                  </div>
                </td>
                <td>
                  <div className="discovery-stack">
                    <span
                      className="mono discovery-break"
                      title={`${resource.host}:${resource.port}`}
                    >
                      {resource.host}:{resource.port}
                    </span>
                    {resource.replica_host && <Badge>Read replica</Badge>}
                    {resource.tags.replica_of && (
                      <Badge>Replica of {resource.tags.replica_of}</Badge>
                    )}
                  </div>
                </td>
                <td>
                  <div className="discovery-stack">
                    <EnvBadge environment={resource.suggested_environment} />
                    <ResourceFlags resource={resource} />
                  </div>
                </td>
                <td>
                  <div className="discovery-stack">
                    <ResourceStatusBadge resource={resource} />
                    <div className="discovery-chips">
                      {resource.drift.map((drift) => (
                        <Badge key={drift} tone="warning">
                          {driftLabels[drift]}
                        </Badge>
                      ))}
                    </div>
                  </div>
                </td>
                <td>
                  <div className="discovery-actions">
                    {resource.status === 'new' && (
                      <>
                        <Button onClick={() => setImporting(resource)}>
                          Import
                        </Button>
                        <Button
                          disabled={action.isPending || bulk.isPending}
                          onClick={() =>
                            action.mutate(
                              {
                                path: `/discovery/resources/${resource.id}/ignore`,
                              },
                              {
                                onSuccess: () =>
                                  setSelected((ids) =>
                                    ids.filter((id) => id !== resource.id),
                                  ),
                              },
                            )
                          }
                        >
                          Ignore
                        </Button>
                      </>
                    )}
                    {resource.status === 'ignored' && (
                      <Button
                        disabled={action.isPending}
                        onClick={() =>
                          action.mutate({
                            path: `/discovery/resources/${resource.id}/unignore`,
                          })
                        }
                      >
                        Unignore
                      </Button>
                    )}
                    {resource.cluster_id && !!resource.drift.length && (
                      <Button onClick={() => setReviewing(resource)}>
                        Review changes
                      </Button>
                    )}
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {action.error && <ErrorPanel error={action.error} />}
      {hasNext && (
        <div className="dialog-actions">
          <Button disabled={loading} onClick={loadMore}>
            {loading ? 'Loading…' : 'Load more resources'}
          </Button>
        </div>
      )}
      {importing && (
        <ImportResource
          resource={importing}
          source={sources.find((source) => source.id === importing.source_id)}
          onClose={() => setImporting(null)}
        />
      )}
      {reviewing && (
        <ReviewDrift resource={reviewing} onClose={() => setReviewing(null)} />
      )}
    </>
  );
}
