import { useEffect, useMemo, useState } from 'react';
import { TriangleAlert } from 'lucide-react';
import type { Cluster, DiscoveredResource } from '../../api/types';
import { params } from '../../api/client';
import { useResource, useAction } from '../../lib/query';
import { useUser } from '../auth/session';
import {
  Badge,
  Button,
  ErrorPanel,
  Field,
  Modal,
  Skeleton,
} from '../../components/ui';
import { driftDiff, driftLabels } from './helpers';
import { useDiscoveryPages } from './queries';

export function ReviewDrift({
  resource,
  onClose,
}: {
  resource: DiscoveredResource;
  onClose: () => void;
}) {
  const query = useResource<Cluster>(`/clusters/${resource.cluster_id}`);
  const [password, setPassword] = useState('');
  const apply = useAction<Cluster>('Discovered endpoint changes applied');
  const diff = query.data ? driftDiff(query.data, resource) : [];
  const endpointChanged = diff.some((d) => d.applied);
  const deleted =
    resource.status === 'gone' || resource.drift.includes('deleted');
  return (
    <Modal
      open
      wide
      title="Review discovered changes"
      description={`${resource.identifier} · ${resource.region}`}
      onOpenChange={(open) => {
        if (!open && !apply.isPending) onClose();
      }}
    >
      {query.isPending ? (
        <Skeleton />
      ) : query.error ? (
        <ErrorPanel error={query.error} retry={() => void query.refetch()} />
      ) : (
        <>
          <div className="discovery-chips">
            {resource.drift.map((drift) => (
              <Badge tone="warning" key={drift}>
                {driftLabels[drift]}
              </Badge>
            ))}
          </div>
          {deleted && (
            <div className="callout warning">
              This database is gone from AWS. Review the managed cluster
              manually; there is no endpoint to apply.
            </div>
          )}
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Field</th>
                  <th>Current cluster</th>
                  <th>Discovered in AWS</th>
                </tr>
              </thead>
              <tbody>
                {diff.map((d) => (
                  <tr key={d.field}>
                    <td>{d.field.replaceAll('_', ' ')}</td>
                    <td className="mono discovery-break">
                      {d.current ?? 'None'}
                    </td>
                    <td className="mono discovery-break">
                      {d.discovered ?? 'None'}{' '}
                      {!d.applied && (
                        <Badge tone="warning">Manual review</Badge>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {resource.drift.includes('engine_changed') && (
            <p className="muted">
              Engine changes require manual review. Apply updates host, port,
              and replica configuration only.
            </p>
          )}
          {endpointChanged && !deleted && (
            <Field
              label="Password"
              hint="Changing the connection endpoint requires re-entering the password"
            >
              <input
                required
                type="password"
                autoComplete="new-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </Field>
          )}
          {apply.error && <ErrorPanel error={apply.error} />}
          <div className="dialog-actions">
            <Button disabled={apply.isPending} onClick={onClose}>
              Cancel
            </Button>
            <Button
              variant="primary"
              disabled={
                apply.isPending || deleted || !endpointChanged || !password
              }
              onClick={() =>
                apply.mutate(
                  {
                    path: `/discovery/resources/${resource.id}/sync`,
                    body: { password },
                  },
                  { onSuccess: onClose },
                )
              }
            >
              {apply.isPending ? 'Applying…' : 'Apply'}
            </Button>
          </div>
        </>
      )}
    </Modal>
  );
}
export function ClusterDriftBanner({ cluster }: { cluster: Cluster }) {
  const user = useUser();
  const query = useDiscoveryPages<DiscoveredResource>(
    `/discovery/resources${params({ q: cluster.tags['discovery:arn'], limit: 100 })}`,
    user?.org_role === 'admin',
  );
  const [reviewing, setReviewing] = useState<DiscoveredResource | null>(null);
  const admin = user?.org_role === 'admin';
  const { data, hasNextPage, isFetching, isFetchNextPageError, fetchNextPage } =
    query;
  const resource = useMemo(
    () =>
      data?.pages
        .flatMap((page) => page.items)
        .find(
          (item) => item.cluster_id === cluster.id && item.drift.length > 0,
        ),
    [data, cluster.id],
  );
  // Older imports may not carry an ARN tag; continue through all matching pages.
  useEffect(() => {
    if (
      admin &&
      !resource &&
      hasNextPage &&
      !isFetching &&
      !isFetchNextPageError
    )
      void fetchNextPage();
  }, [
    admin,
    resource,
    hasNextPage,
    isFetching,
    isFetchNextPageError,
    fetchNextPage,
  ]);
  return admin ? (
    <>
      {query.error && (
        <span className="drift-notice">
          <TriangleAlert size={14} />
          Drift check unavailable{' '}
          <Button variant="ghost" onClick={() => void query.refetch()}>
            Retry
          </Button>
        </span>
      )}
      {resource && (
        <span className="drift-notice">
          <TriangleAlert size={14} />
          <Button
            variant="ghost"
            className="inline-link"
            aria-label="Review AWS changes"
            onClick={() => setReviewing(resource)}
          >
            {resource.drift.includes('endpoint_changed')
              ? 'Endpoint changed in AWS — review'
              : 'Changes found in AWS — review'}
          </Button>
        </span>
      )}
      {reviewing && (
        <ReviewDrift resource={reviewing} onClose={() => setReviewing(null)} />
      )}
    </>
  ) : null;
}
