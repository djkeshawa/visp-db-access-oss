import { useState } from 'react';
import { useUser } from '../auth/session';
import { Link } from 'react-router-dom';
import type {
  AuditEvent,
  Cluster,
  HistoryEntry,
  Page,
  User,
} from '../../api/types';
import { useResource } from '../../lib/query';
import { setupSteps } from '../../lib/workspace';
import { Button, ErrorPanel, Skeleton } from '../../components/ui';
/** All completion evidence comes from API data; no locally checked-off setup claims. */
export function SetupChecklist() {
  const user = useUser();
  const dismissalKey = `vda.setup.dismissed.${user?.id}`;
  const [dismissed, setDismissed] = useState(() => {
    try {
      return localStorage.getItem(dismissalKey) === '1';
    } catch {
      return false;
    }
  });
  const clusters = useResource<{ items: Cluster[] }>('/clusters'),
    users = useResource<{ items: User[] }>('/users'),
    history = useResource<Page<HistoryEntry>>('/history?status=ok&limit=1'),
    audit = useResource<Page<AuditEvent>>(
      '/audit?action=policy.update&limit=50',
    );
  if (
    clusters.isPending ||
    users.isPending ||
    history.isPending ||
    audit.isPending
  )
    return <Skeleton />;
  const error = clusters.error ?? users.error ?? history.error ?? audit.error;
  if (error)
    return (
      <ErrorPanel
        error={error}
        retry={() => {
          void clusters.refetch();
          void users.refetch();
          void history.refetch();
          void audit.refetch();
        }}
      />
    );
  const clusterIds = new Set(clusters.data?.items.map((c) => c.id));
  const complete = setupSteps({
    clusters: clusters.data?.items.length ?? 0,
    policiesReviewed: new Set(
      audit.data?.items
        .map((event) => event.target_id)
        .filter((id): id is string => id !== null && clusterIds.has(id)),
    ).size,
    users: users.data?.items.filter((member) => !member.disabled).length ?? 0,
    queries: history.data?.items.length ?? 0,
  });
  if (dismissed || complete.every(Boolean)) return null;
  const dismiss = () => {
    try {
      localStorage.setItem(dismissalKey, '1');
    } catch {
      /* Session-only dismissal remains available. */
    }
    setDismissed(true);
  };
  const cluster = clusters.data?.items[0];
  const steps = [
    {
      title: 'Add a cluster',
      detail: 'Connect a database to the gateway.',
      to: '/clusters?add=true',
    },
    {
      title: 'Review and save its safety policy',
      detail: 'Check row limits, masking and write approvals.',
      to: cluster ? `/clusters/${cluster.id}?tab=policy` : '/clusters?add=true',
    },
    {
      title: 'Invite a teammate',
      detail: 'Create an account, then grant database access.',
      to: '/users?add=true',
    },
    {
      title: 'Run a first query',
      detail: 'Start with a limited SELECT.',
      to: cluster ? `/console?cluster_id=${cluster.id}` : '/clusters?add=true',
    },
  ];
  return (
    <section className="setup-checklist" aria-label="Workspace setup">
      <div className="section-heading">
        <h2>Set up your workspace</h2>
        <span className="muted">
          {complete.filter(Boolean).length} of 4 complete
        </span>
        <Button variant="ghost" size="small" onClick={dismiss}>
          Dismiss setup checklist
        </Button>
      </div>
      <ol>
        {steps.map((step, index) => (
          <li key={step.title} className={complete[index] ? 'complete' : ''}>
            <span
              className="setup-number"
              aria-label={complete[index] ? 'Completed' : `Step ${index + 1}`}
            >
              {complete[index] ? '✓' : index + 1}
            </span>
            <div>
              <Link to={step.to}>{step.title}</Link>
              <p>{step.detail}</p>
            </div>
          </li>
        ))}
      </ol>
    </section>
  );
}
