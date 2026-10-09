import { useState } from 'react';
import { Cloud, Plus } from 'lucide-react';
import { Link } from 'react-router-dom';
import type { DiscoverySource, DiscoveryTest, ScanRun } from '../../api/types';
import { useAction, useResource } from '../../lib/query';
import {
  Badge,
  Button,
  Confirm,
  Empty,
  ErrorPanel,
  Modal,
  Skeleton,
  Switch,
} from '../../components/ui';
import { SourceForm, TestResults } from './source-form';
import { useDiscoveryPages } from './queries';
import { formatTime } from '../../lib/utils';

export function AwsLogo() {
  return (
    <svg className="aws-logo" viewBox="0 0 48 32" role="img" aria-label="AWS">
      <text
        x="2"
        y="20"
        fill="currentColor"
        fontSize="21"
        fontFamily="var(--font-ui)"
        fontWeight="600"
      >
        aws
      </text>
      <path
        d="M6 25 Q25 34 43 23 M38 23 L43 23 L42 28"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
    </svg>
  );
}
function ScanSummary({ run }: { run: ScanRun | null }) {
  if (!run) return <span className="muted">Never scanned</span>;
  return (
    <div className="discovery-stack">
      <Badge
        tone={
          run.status === 'succeeded'
            ? 'success'
            : run.status === 'failed'
              ? 'danger'
              : 'warning'
        }
      >
        {run.status}
      </Badge>
      <span className="muted">
        {formatTime(run.finished_at ?? run.started_at)}
      </span>
      <small>
        {run.found} found · {run.new} new · {run.gone} gone · {run.changed}{' '}
        changed
      </small>
    </div>
  );
}
function RunHistory({
  source,
  onClose,
}: {
  source: DiscoverySource;
  onClose: () => void;
}) {
  const query = useDiscoveryPages<ScanRun>(
    `/discovery/sources/${source.id}/runs?limit=20`,
  );
  return (
    <Modal
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={`Scan history · ${source.name}`}
      drawer
    >
      {query.isPending ? (
        <Skeleton />
      ) : query.error ? (
        <ErrorPanel error={query.error} retry={() => void query.refetch()} />
      ) : !query.data.pages[0]?.items.length ? (
        <Empty
          title="No scans yet"
          description="Scan this source to find RDS and Aurora databases."
        />
      ) : (
        <div className="discovery-stack">
          {query.data.pages
            .flatMap((page) => page.items)
            .map((run) => (
              <section className="settings-section" key={run.id}>
                <ScanSummary run={run} />
                <p className="muted">Started {formatTime(run.started_at)}</p>
                {run.errors.map((error, i) => (
                  <div className="callout warning" key={i}>
                    <span>
                      {error.region ?? 'Account'}: {error.message}
                    </span>
                  </div>
                ))}
              </section>
            ))}
        </div>
      )}
      {query.hasNextPage && (
        <Button
          disabled={query.isFetchingNextPage}
          onClick={() => void query.fetchNextPage()}
        >
          {query.isFetchingNextPage ? 'Loading…' : 'Load more scans'}
        </Button>
      )}
    </Modal>
  );
}
export function DiscoverySources() {
  const query = useResource<{ items: DiscoverySource[] }>('/discovery/sources');
  const [editing, setEditing] = useState<DiscoverySource | 'new' | null>(null),
    [deleting, setDeleting] = useState<DiscoverySource | null>(null),
    [history, setHistory] = useState<DiscoverySource | null>(null);
  const [testing, setTesting] = useState<DiscoverySource | null>(null);
  const action = useAction('Source updated'),
    remove = useAction('Source deleted'),
    scan = useAction<ScanRun>('Discovery scan completed'),
    test = useAction<DiscoveryTest>('');
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Cloud discovery</h1>
          <p>Find AWS databases and review them before importing.</p>
        </div>
        <Button variant="primary" onClick={() => setEditing('new')}>
          <Plus size={16} />
          Add source
        </Button>
      </div>
      <div className="overview-strip">
        <Cloud size={18} />
        <span>RDS & Aurora</span>
        <span className="muted">
          Read-only AWS APIs. Import requires database credentials.
        </span>
        <Link to="/clusters?tab=discovered">View discovered databases →</Link>
      </div>
      {query.isPending ? (
        <Skeleton />
      ) : query.error ? (
        <ErrorPanel error={query.error} retry={() => void query.refetch()} />
      ) : !query.data.items.length ? (
        <Empty
          title="Connect your first AWS account"
          description="Add a source using the gateway’s identity or a cross-account role, then scan for RDS and Aurora databases."
          action={<Button onClick={() => setEditing('new')}>Add source</Button>}
        />
      ) : (
        <div className="table-wrap">
          <table className="discovery-sources">
            <thead>
              <tr>
                <th>Source</th>
                <th>Account</th>
                <th>Regions</th>
                <th>Interval</th>
                <th>Enabled</th>
                <th>Last scan</th>
                <th>Actions</th>
              </tr>
            </thead>
            <tbody>
              {query.data.items.map((source) => (
                <tr key={source.id}>
                  <td>
                    <div className="inline strong">
                      <AwsLogo />
                      <span>{source.name}</span>
                    </div>
                    <small className="muted">
                      {source.role_arn
                        ? 'Cross-account role'
                        : 'Gateway identity'}
                    </small>
                  </td>
                  <td className="mono">
                    {source.last_test?.account_id ?? 'Not tested'}
                    <br />
                    <small className="muted">
                      {source.last_test
                        ? `${source.last_test.ok ? 'Verified' : 'Needs attention'} · ${formatTime(source.last_test.tested_at)}`
                        : ''}
                    </small>
                  </td>
                  <td>
                    <div className="discovery-chips">
                      {source.regions.map((region) => (
                        <Badge key={region}>{region}</Badge>
                      ))}
                    </div>
                  </td>
                  <td>{source.scan_interval_minutes} min</td>
                  <td>
                    <Switch
                      label={`Enable ${source.name}`}
                      checked={source.enabled}
                      disabled={action.isPending}
                      onChange={(enabled) =>
                        action.mutate({
                          path: `/discovery/sources/${source.id}`,
                          method: 'PATCH',
                          body: { enabled },
                        })
                      }
                    />
                  </td>
                  <td>
                    <ScanSummary run={source.last_scan} />
                  </td>
                  <td>
                    <div className="discovery-actions">
                      <Button
                        disabled={
                          scan.isPending ||
                          source.last_scan?.status === 'running'
                        }
                        onClick={() =>
                          scan.mutate({
                            path: `/discovery/sources/${source.id}/scan`,
                          })
                        }
                      >
                        {scan.isPending &&
                        scan.variables?.path.includes(source.id)
                          ? 'Scanning…'
                          : 'Scan now'}
                      </Button>
                      <Button
                        disabled={test.isPending}
                        onClick={() => {
                          setTesting(source);
                          test.reset();
                          test.mutate({
                            path: `/discovery/sources/${source.id}/test`,
                          });
                        }}
                      >
                        Test
                      </Button>
                      <Button onClick={() => setEditing(source)}>Edit</Button>
                      <Button onClick={() => setHistory(source)}>
                        History
                      </Button>
                      <Button
                        variant="ghost"
                        onClick={() => setDeleting(source)}
                      >
                        Delete
                      </Button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {scan.error && <ErrorPanel error={scan.error} />}
      {editing && (
        <SourceForm
          key={editing === 'new' ? 'new' : editing.id}
          source={editing === 'new' ? undefined : editing}
          onClose={() => setEditing(null)}
        />
      )}
      {history && (
        <RunHistory source={history} onClose={() => setHistory(null)} />
      )}
      <Modal
        open={!!testing}
        onOpenChange={(open) => {
          if (!open) setTesting(null);
        }}
        title={`Connection test · ${testing?.name ?? ''}`}
      >
        {test.isPending ? (
          <Skeleton />
        ) : test.error ? (
          <ErrorPanel error={test.error} />
        ) : test.data ? (
          <TestResults result={test.data} />
        ) : null}
      </Modal>
      <Confirm
        open={!!deleting}
        onOpenChange={(open) => {
          if (!open && !remove.isPending) setDeleting(null);
        }}
        danger
        title="Delete discovery source?"
        description={`Delete ${deleting?.name ?? 'this source'} and its discovery inventory? Imported clusters will remain available.`}
        busy={remove.isPending}
        onConfirm={() =>
          remove.mutate(
            { path: `/discovery/sources/${deleting?.id}`, method: 'DELETE' },
            { onSuccess: () => setDeleting(null) },
          )
        }
      />
    </>
  );
}
