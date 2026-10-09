import { useMemo, useState } from 'react';
import type {
  DiscoverySource,
  DiscoveryTest,
  Project,
  SourceDraft,
} from '../../api/types';
import { useAction, useResource } from '../../lib/query';
import {
  Badge,
  Button,
  ChipInput,
  ErrorPanel,
  Field,
  Modal,
  Picker,
  SwitchRow,
} from '../../components/ui';
import { groupRegions, minimalPolicy } from './helpers';
import { useToast } from '../../components/ui/toast';
import { message } from '../../lib/utils';

export function TestResults({ result }: { result: DiscoveryTest }) {
  return (
    <div className="discovery-test" role="status">
      <div className="inline">
        <Badge tone={result.ok ? 'success' : 'warning'}>
          {result.ok ? 'Connection verified' : 'Connection needs attention'}
        </Badge>
        <span>Account {result.account_id ?? 'unavailable'}</span>
      </div>
      <p className="mono discovery-break">
        {result.identity_arn ?? 'Identity unavailable'}
      </p>
      {result.regions.map((region) => (
        <div className="inline" key={region.region}>
          <Badge tone={region.ok ? 'success' : 'danger'}>
            {region.ok ? 'OK' : 'Error'}
          </Badge>
          <span className="mono">{region.region}</span>
          <span>{region.error}</span>
        </div>
      ))}
    </div>
  );
}
function IamPermissions() {
  const toast = useToast();
  return (
    <details className="discovery-permissions">
      <summary>Required IAM permissions</summary>
      <p className="muted">
        Grant these read permissions to the gateway identity or the assumed
        role.
      </p>
      <pre className="json-preview">{minimalPolicy}</pre>
      <Button
        onClick={() =>
          void navigator.clipboard
            .writeText(minimalPolicy)
            .then(() => toast('IAM policy copied'))
            .catch((error) => toast(message(error), 'error'))
        }
      >
        Copy policy
      </Button>
      <p className="muted">
        For cross-account discovery, the gateway identity also needs
        sts:AssumeRole on the target role. The target role must trust the
        gateway identity and require your external ID when configured.
      </p>
    </details>
  );
}
function RegionSelect({
  value,
  onChange,
}: {
  value: string[];
  onChange: (value: string[]) => void;
}) {
  const [search, setSearch] = useState('');
  const groups = useMemo(() => groupRegions(search), [search]);
  return (
    <fieldset className="region-select">
      <legend>
        Regions <span className="muted">{value.length}/20 selected</span>
      </legend>
      <input
        aria-label="Search AWS regions"
        placeholder="Search region or location…"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        onKeyDown={(e) => {
          // Enter must not submit the surrounding source form.
          if (e.key === 'Enter') e.preventDefault();
        }}
      />
      <div className="discovery-chips">
        {value.map((region) => (
          <Button
            key={region}
            variant="ghost"
            aria-label={`Remove region ${region}`}
            onClick={() => onChange(value.filter((r) => r !== region))}
          >
            {region} ×
          </Button>
        ))}
      </div>
      <div className="region-options">
        {groups.map((group) => (
          <div key={group.geography}>
            <h3>{group.geography}</h3>
            {group.regions.map((region) => (
              <label className="checkbox" key={region.code}>
                <input
                  type="checkbox"
                  checked={value.includes(region.code)}
                  disabled={!value.includes(region.code) && value.length >= 20}
                  onChange={(e) =>
                    onChange(
                      e.target.checked
                        ? [...value, region.code]
                        : value.filter((r) => r !== region.code),
                    )
                  }
                />
                <span>
                  {region.name}{' '}
                  <span className="muted mono">{region.code}</span>
                </span>
              </label>
            ))}
          </div>
        ))}
        {!groups.length && <p className="muted">No matching regions.</p>}
      </div>
    </fieldset>
  );
}
export function SourceForm({
  source,
  onClose,
}: {
  source?: DiscoverySource;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState<SourceDraft>({
    provider: 'aws',
    name: source?.name ?? '',
    role_arn: source?.role_arn ?? null,
    regions: source?.regions ?? ['us-east-1'],
    default_project_id: source?.default_project_id ?? null,
    environment_tag_keys: source?.environment_tag_keys ?? [
      'environment',
      'env',
      'stage',
    ],
    scan_interval_minutes: source?.scan_interval_minutes ?? 60,
    enabled: source?.enabled ?? true,
  });
  const projects = useResource<{ items: Project[] }>('/projects');
  const save = useAction<DiscoverySource>('Discovery source saved');
  const test = useAction<DiscoveryTest>('');
  const update = <K extends keyof SourceDraft>(
    field: K,
    value: SourceDraft[K],
  ) => {
    setDraft((draft) => ({ ...draft, [field]: value }));
    // A passed test only goes stale when connection settings change.
    if (['role_arn', 'external_id', 'regions'].includes(field)) test.reset();
  };
  const connectionChanged =
    !source ||
    draft.role_arn !== source.role_arn ||
    draft.external_id !== undefined ||
    JSON.stringify(draft.regions) !== JSON.stringify(source.regions);
  const needsExternalId =
    connectionChanged &&
    source?.external_id_set &&
    draft.external_id === undefined;
  const valid =
    draft.name.trim() &&
    draft.regions.length > 0 &&
    draft.regions.length <= 20 &&
    draft.scan_interval_minutes >= 5 &&
    draft.scan_interval_minutes <= 1440 &&
    Number.isInteger(draft.scan_interval_minutes) &&
    (!draft.role_arn ||
      /^arn:aws:iam::\d{12}:role\/[\w+=,.@/-]+$/.test(draft.role_arn));
  return (
    <Modal
      open
      onOpenChange={(open) => {
        if (!open && !save.isPending && !test.isPending) onClose();
      }}
      wide
      title={source ? 'Edit discovery source' : 'Add discovery source'}
      description="Discover RDS and Aurora databases using the gateway’s AWS identity or an assumed role."
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (valid && !save.isPending && !test.isPending)
            save.mutate(
              {
                path: source
                  ? `/discovery/sources/${source.id}`
                  : '/discovery/sources',
                method: source ? 'PATCH' : 'POST',
                body: draft,
              },
              {
                onSuccess: onClose,
              },
            );
        }}
      >
        <div className="form-grid">
          <Field label="Source name">
            <input
              required
              value={draft.name}
              onChange={(e) => update('name', e.target.value)}
              autoFocus
            />
          </Field>
          <Field
            label="Scan interval (minutes)"
            hint="Between 5 and 1440 minutes."
          >
            <input
              type="number"
              min={5}
              max={1440}
              step={1}
              required
              value={draft.scan_interval_minutes}
              onChange={(e) =>
                update('scan_interval_minutes', Number(e.target.value))
              }
            />
          </Field>
        </div>
        <Field
          label="Role ARN (optional)"
          hint="Leave empty to use the gateway's own AWS identity"
        >
          <input
            placeholder="arn:aws:iam::123456789012:role/VispDiscovery"
            value={draft.role_arn ?? ''}
            onChange={(e) => update('role_arn', e.target.value || null)}
          />
        </Field>
        <Field
          label="External ID"
          hint="Write-only. Leave blank to keep the existing value."
        >
          <input
            type="password"
            autoComplete="new-password"
            value={draft.external_id ?? ''}
            placeholder={
              source?.external_id_set && draft.external_id !== null
                ? 'External ID is set'
                : 'Optional external ID'
            }
            onChange={(e) => update('external_id', e.target.value || undefined)}
          />
        </Field>
        <div className="inline">
          <Badge>
            {draft.external_id === null
              ? 'Cleared on save'
              : draft.external_id || source?.external_id_set
                ? 'External ID set'
                : 'External ID not set'}
          </Badge>
          <Button onClick={() => update('external_id', null)}>Clear</Button>
        </div>
        <RegionSelect
          value={draft.regions}
          onChange={(value) => update('regions', value)}
        />
        <Field label="Default project">
          <Picker
            value={draft.default_project_id ?? 'none'}
            label="Default project"
            onChange={(value) =>
              update('default_project_id', value === 'none' ? null : value)
            }
            options={[
              { value: 'none', label: 'Choose when importing' },
              ...(projects.data?.items ?? []).map((project) => ({
                value: project.id,
                label: project.name,
              })),
            ]}
          />
        </Field>
        {projects.error && (
          <ErrorPanel
            error={projects.error}
            retry={() => void projects.refetch()}
          />
        )}
        <ChipInput
          label="Environment tag keys"
          value={draft.environment_tag_keys}
          onChange={(value) => update('environment_tag_keys', value)}
          help="Checked in order. Defaults: environment, env, stage."
        />
        <SwitchRow
          label="Enable scheduled discovery"
          description="Scan this source at its configured interval."
          checked={draft.enabled}
          onChange={(enabled) => update('enabled', enabled)}
        />
        <IamPermissions />
        {test.data && <TestResults result={test.data} />}
        {test.error && <ErrorPanel error={test.error} />}
        {save.error && <ErrorPanel error={save.error} />}
        <p className="muted">
          {needsExternalId
            ? 'Re-enter the existing external ID to test changed connection settings, or clear it.'
            : 'Test this configuration before saving. No source is created by testing.'}
        </p>
        <div className="dialog-actions">
          <Button onClick={onClose} disabled={save.isPending || test.isPending}>
            Cancel
          </Button>
          <Button
            disabled={
              !valid || needsExternalId || test.isPending || save.isPending
            }
            onClick={() =>
              test.mutate(
                source && !connectionChanged
                  ? { path: `/discovery/sources/${source.id}/test` }
                  : { path: '/discovery/sources/test', body: draft },
              )
            }
          >
            {test.isPending ? 'Testing…' : 'Test connection'}
          </Button>
          <Button
            type="submit"
            variant="primary"
            disabled={!valid || save.isPending || test.isPending}
          >
            {save.isPending ? 'Saving…' : 'Save source'}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
