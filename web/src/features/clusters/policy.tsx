import { useMemo, useState, type ReactNode } from 'react';
import type { Cluster, Policy } from '../../api/types';
import { useAction, useResource } from '../../lib/query';
import {
  Button,
  ChipInput,
  Confirm,
  ErrorPanel,
  Skeleton,
  Switch,
} from '../../components/ui';
import { isCidr } from '../../lib/utils';
import {
  environmentPolicy,
  millisecondsFromSeconds,
  policySummary,
} from '../../lib/policy';
const same = (a: unknown, b: unknown) =>
  JSON.stringify(a) === JSON.stringify(b);
export function PolicyPage({ cluster }: { cluster: Cluster }) {
  const query = useResource<Policy>(`/clusters/${cluster.id}/policy`);
  return query.isPending ? (
    <Skeleton />
  ) : query.error ? (
    <ErrorPanel error={query.error} retry={() => void query.refetch()} />
  ) : (
    <PolicyForm cluster={cluster} original={query.data} />
  );
}
function PolicyForm({
  cluster,
  original,
}: {
  cluster: Cluster;
  original: Policy;
}) {
  const [draft, setDraft] = useState(original),
    [baseline, setBaseline] = useState(original),
    [confirm, setConfirm] = useState(false);
  // The server policy changed under us (save, or another administrator).
  // Adopt it silently unless that would discard unsaved edits.
  const serverChanged = !same(original, baseline);
  if (serverChanged && (same(draft, baseline) || same(draft, original))) {
    setBaseline(original);
    setDraft(original);
  }
  const stale =
    serverChanged && !same(draft, baseline) && !same(draft, original);
  const save = useAction('Safety policy saved'),
    admin = cluster.my_access === 'admin';
  const defaults = environmentPolicy(cluster.environment);
  const update = (values: Partial<Policy>) =>
    setDraft((draft) => ({ ...draft, ...values }));
  const diff = useMemo(
    () =>
      (Object.keys(baseline) as (keyof Policy)[]).filter(
        (key) => !same(baseline[key], draft[key]),
      ),
    [baseline, draft],
  );
  const customized = (
    key: keyof Policy,
    buttonText: string,
    ariaLabel?: string,
  ) =>
    !same(draft[key], defaults[key]) && (
      <div className="policy-reset">
        <span>Customized</span>
        {admin && (
          <Button
            variant="ghost"
            size="small"
            aria-label={ariaLabel}
            onClick={() => update({ [key]: defaults[key] })}
          >
            {buttonText}
          </Button>
        )}
      </div>
    );
  const row = (
    key: keyof Policy,
    label: string,
    help: string,
    control: ReactNode,
    labelFor?: string,
  ) => (
    <div key={key} className="policy-row">
      <div>
        {labelFor ? (
          <label htmlFor={labelFor}>{label}</label>
        ) : (
          <span>{label}</span>
        )}
        <small>{help}</small>
        {customized(key, 'Reset to default', `Reset ${label} to default`)}
      </div>
      <div className="policy-control">{control}</div>
    </div>
  );
  const numeric = (
    key: keyof Policy,
    label: string,
    help: string,
    unit: string,
    optional = false,
  ) => {
    const seconds = key.endsWith('_ms');
    return row(
      key,
      label,
      help,
      <>
        <input
          id={`policy-${key}`}
          type="number"
          min={optional ? 0.001 : seconds ? 0.1 : 1}
          max={
            seconds
              ? 600
              : key === 'max_rows' || key === 'max_affected_rows'
                ? 100000
                : key === 'max_concurrent_queries'
                  ? 64
                  : undefined
          }
          step={seconds ? 0.001 : optional ? 'any' : 1}
          disabled={!admin}
          value={
            draft[key] === null || !Number.isFinite(Number(draft[key]))
              ? ''
              : Number(draft[key]) / (seconds ? 1000 : 1)
          }
          onChange={(event) =>
            update({
              [key]:
                event.target.value === '' && optional
                  ? null
                  : seconds
                    ? millisecondsFromSeconds(event.target.value)
                    : event.target.value === ''
                      ? NaN
                      : Number(event.target.value),
            })
          }
        />
        <span>{unit}</span>
      </>,
      `policy-${key}`,
    );
  };
  const toggle = (key: keyof Policy, label: string, help: string) =>
    row(
      key,
      label,
      help,
      <Switch
        label={label}
        checked={draft[key] === true}
        disabled={!admin}
        onChange={(checked) => update({ [key]: checked })}
      />,
    );
  const patterns = (
    key: 'masked_columns' | 'blocked_tables' | 'allowed_cidrs',
    label: string,
    help: string,
  ) => (
    <div key={key} className="policy-patterns">
      <ChipInput
        label={label}
        value={draft[key]}
        onChange={(value) => update({ [key]: value })}
        validate={key === 'allowed_cidrs' ? isCidr : undefined}
        help={help}
      />
      {customized(key, `Reset ${label.toLowerCase()} to default`)}
    </div>
  );
  const invalid =
    [draft.max_rows, draft.max_affected_rows].some(
      (value) => !Number.isInteger(value) || value < 1 || value > 100000,
    ) ||
    [draft.statement_timeout_ms, draft.lock_timeout_ms].some(
      (value) => !Number.isInteger(value) || value < 100 || value > 600000,
    ) ||
    draft.lock_timeout_ms > draft.statement_timeout_ms ||
    !Number.isInteger(draft.max_concurrent_queries) ||
    draft.max_concurrent_queries < 1 ||
    draft.max_concurrent_queries > 64 ||
    (draft.max_cost !== null &&
      (!Number.isFinite(draft.max_cost) || draft.max_cost <= 0)) ||
    draft.allowed_cidrs.some((cidr) => !isCidr(cidr));
  return (
    <div className="policy-page">
      <div className="section-heading">
        <h2>Safety policy</h2>
        {admin && !diff.length && (
          <Button variant="ghost" size="small" onClick={() => setConfirm(true)}>
            Save reviewed policy
          </Button>
        )}
      </div>
      <p className="policy-summary">{policySummary(draft)}</p>
      {stale && (
        <div className="callout warning" role="status">
          <span>
            This policy was changed elsewhere. Saving will overwrite those
            changes.
          </span>
          <Button
            size="small"
            onClick={() => {
              setBaseline(original);
              setDraft(original);
            }}
          >
            Load latest policy
          </Button>
        </div>
      )}
      {!admin && (
        <p className="muted">You have read-only access to this policy.</p>
      )}
      <fieldset disabled={!admin}>
        <section className="settings-section">
          <h3>Read limits</h3>
          {numeric(
            'max_rows',
            'Maximum result rows',
            'The gateway caps each read result.',
            'rows',
          )}
          {numeric(
            'statement_timeout_ms',
            'Statement timeout',
            'Cancel a query that exceeds this time.',
            'seconds',
          )}
          {numeric(
            'lock_timeout_ms',
            'Lock timeout',
            'Stop waiting for a lock before the statement timeout.',
            'seconds',
          )}
          {numeric(
            'max_cost',
            'Maximum EXPLAIN cost',
            'Leave blank to disable the read cost gate.',
            'cost',
            true,
          )}
          {numeric(
            'max_concurrent_queries',
            'Concurrent queries per node',
            'Limit simultaneous upstream queries.',
            'queries',
          )}
        </section>
        <section className="settings-section">
          <h3>Writes</h3>
          {toggle(
            'allow_writes',
            'Allow writes',
            'Enable data changes on this cluster.',
          )}
          {toggle(
            'require_approval_for_writes',
            'Require approval for writes',
            'A second person must review each write.',
          )}
          {toggle(
            'allow_ddl',
            'Allow DDL',
            'Permit changes to database structure.',
          )}
          {numeric(
            'max_affected_rows',
            'Maximum affected rows',
            'Roll back writes that exceed this limit.',
            'rows',
          )}
          {cluster.environment === 'production' && draft.allow_writes && (
            <p className="callout warning">
              Writes are enabled on Production. Check grants, affected-row
              limits, and approval requirements before saving.
            </p>
          )}
        </section>
        <section className="settings-section">
          <h3>Routing</h3>
          {toggle(
            'route_reads_to_replica',
            'Route reads to a healthy replica',
            'Use an available read replica for read queries.',
          )}
        </section>
        <section className="settings-section">
          <h3>Data protection</h3>
          {patterns(
            'masked_columns',
            'Masked columns',
            'Patterns: email, users.ssn, *.password*',
          )}
          {patterns(
            'blocked_tables',
            'Blocked tables',
            'Patterns: secrets.*, audit_trail',
          )}
        </section>
        <section className="settings-section">
          <h3>Network</h3>
          {patterns(
            'allowed_cidrs',
            'Allowed CIDRs',
            'These CIDRs intersect with the organization allowlist. Empty uses the organization rule.',
          )}
        </section>
      </fieldset>
      {admin && diff.length > 0 && (
        <div className="save-bar">
          <span>
            {diff.length} unsaved changes
            {invalid ? ' · Check limits and CIDRs' : ''}
          </span>
          <div className="inline">
            <Button onClick={() => setDraft(baseline)}>Discard changes</Button>
            <Button
              variant="primary"
              disabled={invalid}
              onClick={() => setConfirm(true)}
            >
              Review changes
            </Button>
          </div>
        </div>
      )}
      <Confirm
        open={confirm}
        onOpenChange={setConfirm}
        title={diff.length ? 'Save policy changes?' : 'Save reviewed policy?'}
        description={
          diff.length
            ? `These ${diff.length} changes take effect for new queries on ${cluster.name}.`
            : `Record your review of the current safety policy on ${cluster.name}.`
        }
        busy={save.isPending}
        onConfirm={() =>
          save.mutate(
            {
              path: `/clusters/${cluster.id}/policy`,
              method: 'PUT',
              body: draft,
            },
            { onSuccess: () => setConfirm(false) },
          )
        }
      >
        <div className="policy-diff">
          {diff.map((key) => (
            <div key={key}>
              <strong>{key.replaceAll('_', ' ')}</strong>
              <del>{displayPolicyValue(key, baseline[key])}</del>
              <ins>{displayPolicyValue(key, draft[key])}</ins>
            </div>
          ))}
        </div>
      </Confirm>
    </div>
  );
}

function displayPolicyValue(
  key: keyof Policy,
  value: Policy[keyof Policy],
): string {
  if (key.endsWith('_ms') && typeof value === 'number')
    return `${value / 1000} seconds`;
  if (Array.isArray(value)) return value.join(', ') || 'None';
  return value === null
    ? 'Not set'
    : typeof value === 'boolean'
      ? value
        ? 'Yes'
        : 'No'
      : String(value);
}
