import type { Cluster, Engine, Health, Project } from '../../api/types';
import { useEffect, useState } from 'react';
import { useAction, useResource } from '../../lib/query';
import {
  Button,
  EnvBadge,
  Field,
  HealthBadge,
  Modal,
  Picker,
  ErrorPanel,
} from '../../components/ui';
import { defaultPolicy } from '../../api/fixtures';
export type ClusterDraft = Omit<
  Cluster,
  'id' | 'health' | 'my_access' | 'created_at' | 'updated_at'
> & { password: string };
/** Copies only the connection fields allowed by the HTTP contract. */
export function connectionDraft(cluster: Cluster): ClusterDraft {
  const {
    project_id,
    name,
    engine,
    provider,
    region,
    environment,
    host,
    port,
    database,
    username,
    tls_mode,
    replica_host,
    replica_port,
    tags,
  } = cluster;
  return {
    project_id,
    name,
    engine,
    provider,
    region,
    environment,
    host,
    port,
    database,
    username,
    tls_mode,
    replica_host,
    replica_port,
    tags,
    password: '',
  };
}
const initialDraft: ClusterDraft = {
  project_id: '',
  name: '',
  engine: 'postgres',
  provider: 'aws',
  region: 'us-east-1',
  environment: 'production',
  host: '',
  port: 5432,
  database: '',
  username: '',
  password: '',
  tls_mode: 'verify_full',
  replica_host: null,
  replica_port: null,
  tags: {},
};
/** Parses `key=value, key2=value2`; values may themselves contain `=`. */
function parseTags(text: string): Record<string, string> {
  const tags: Record<string, string> = {};
  for (const part of text.split(',')) {
    const tag = part.trim();
    const at = tag.indexOf('=');
    if (at > 0) tags[tag.slice(0, at).trim()] = tag.slice(at + 1).trim();
  }
  return tags;
}
const options = (values: string[]) =>
  values.map((value) => ({ value, label: value.replaceAll('_', ' ') }));
export function ConnectionFields({
  draft,
  update,
  requirePassword = false,
}: {
  draft: ClusterDraft;
  update: (values: Partial<ClusterDraft>) => void;
  requirePassword?: boolean;
}) {
  return (
    <div className="form-grid">
      <Field label="Database engine">
        <Picker
          value={draft.engine}
          label="Database engine"
          options={[
            { value: 'postgres', label: 'PostgreSQL' },
            { value: 'mysql', label: 'MySQL' },
          ]}
          onChange={(engine) =>
            update({
              engine: engine as Engine,
              port: engine === 'postgres' ? 5432 : 3306,
            })
          }
        />
      </Field>
      <Field label="TLS mode">
        <Picker
          value={draft.tls_mode}
          label="TLS mode"
          options={options(['disable', 'prefer', 'require', 'verify_full'])}
          onChange={(value) =>
            update({ tls_mode: value as ClusterDraft['tls_mode'] })
          }
        />
      </Field>
      <Field label="Host">
        <input
          required
          value={draft.host}
          onChange={(e) => update({ host: e.target.value })}
          placeholder="database.internal.example.com"
        />
      </Field>
      <Field label="Port">
        <input
          type="number"
          required
          min={1}
          max={65535}
          value={draft.port}
          onChange={(e) => update({ port: Number(e.target.value) })}
        />
      </Field>
      <Field label="Database">
        <input
          required
          value={draft.database}
          onChange={(e) => update({ database: e.target.value })}
        />
      </Field>
      <Field label="Username">
        <input
          required
          value={draft.username}
          onChange={(e) => update({ username: e.target.value })}
          autoComplete="off"
        />
      </Field>
      <Field
        label={requirePassword ? 'Password' : 'Rotate password (optional)'}
        hint={
          requirePassword
            ? 'Changing the connection endpoint requires re-entering the password'
            : 'Write-only. Stored encrypted by the gateway.'
        }
      >
        <input
          type="password"
          required={requirePassword}
          value={draft.password}
          onChange={(e) => update({ password: e.target.value })}
          autoComplete="new-password"
          placeholder={
            requirePassword ? '' : 'Leave blank to keep current password'
          }
        />
      </Field>
      <Field label="Read replica host (optional)">
        <input
          value={draft.replica_host ?? ''}
          onChange={(e) => update({ replica_host: e.target.value || null })}
        />
      </Field>
      {draft.replica_host && (
        <Field label="Read replica port">
          <input
            type="number"
            min={1}
            max={65535}
            value={draft.replica_port ?? draft.port}
            onChange={(e) => update({ replica_port: Number(e.target.value) })}
          />
        </Field>
      )}
    </div>
  );
}
export function ConnectionTest({
  draft,
  clusterId,
  requirePassword = !clusterId,
}: {
  draft: ClusterDraft;
  clusterId?: string;
  requirePassword?: boolean;
}) {
  const test = useAction<Health>('');
  const { reset } = test;
  // A result only describes the endpoint that was tested; drop it on edits.
  useEffect(() => {
    reset();
  }, [
    reset,
    draft.engine,
    draft.host,
    draft.port,
    draft.database,
    draft.username,
    draft.tls_mode,
    draft.password,
  ]);
  return (
    <div className="connection-test">
      <Button
        disabled={
          test.isPending ||
          !draft.host ||
          !draft.database ||
          !draft.username ||
          (requirePassword && !draft.password)
        }
        onClick={() =>
          test.mutate({
            path: '/clusters/test-connection',
            body: {
              engine: draft.engine,
              host: draft.host,
              port: draft.port,
              database: draft.database,
              username: draft.username,
              tls_mode: draft.tls_mode,
              ...(draft.password
                ? { password: draft.password }
                : { cluster_id: clusterId }),
            },
          })
        }
      >
        {test.isPending ? 'Testing…' : 'Test connection'}
      </Button>
      {test.error && <ErrorPanel error={test.error} />}
      {test.data && (
        <div className="callout success">
          <HealthBadge
            status={test.data.status}
            latency={test.data.latency_ms}
          />
          <span>
            {test.data.server_version} ·{' '}
            {test.data.is_replica ? 'read replica' : 'primary'}
          </span>
        </div>
      )}
    </div>
  );
}
export function AddCluster({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [draft, setDraft] = useState<ClusterDraft>(initialDraft),
    [step, setStep] = useState(0),
    [createProject, setCreateProject] = useState(false),
    [tagsText, setTagsText] = useState(''),
    [newProject, setNewProject] = useState('');
  const projects = useResource<{ items: Project[] }>('/projects'),
    addProject = useAction<Project>('Project created'),
    create = useAction<Cluster>('Cluster created');
  const update = (values: Partial<ClusterDraft>) =>
    setDraft((draft) => ({ ...draft, ...values }));
  const policy = defaultPolicy(draft.environment);
  const submitProject = () => {
    const name = newProject.trim();
    if (!name || addProject.isPending) return;
    addProject.mutate(
      { path: '/projects', body: { name, description: '' } },
      {
        onSuccess: (project) => {
          update({ project_id: project.id });
          setCreateProject(false);
          setNewProject('');
        },
      },
    );
  };
  return (
    <Modal
      open={open}
      onOpenChange={onOpenChange}
      title="Add a cluster"
      description="Credentials are encrypted and never returned to your browser."
      wide
    >
      <ol className="wizard-steps">
        {['Basics', 'Connection', 'Review'].map((name, i) => (
          <li
            className={step === i ? 'active' : step > i ? 'complete' : ''}
            key={name}
          >
            <span>{i + 1}</span>
            {name}
          </li>
        ))}
      </ol>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (create.isPending) return;
          if (step < 2) {
            if (step === 0 && !draft.project_id) return;
            setStep(step + 1);
          } else
            create.mutate(
              {
                path: '/clusters',
                body: {
                  ...draft,
                  replica_port: draft.replica_host
                    ? (draft.replica_port ?? draft.port)
                    : null,
                },
              },
              {
                onSuccess: () => {
                  onOpenChange(false);
                  setDraft(initialDraft);
                  setTagsText('');
                  setStep(0);
                },
              },
            );
        }}
      >
        {step === 0 && (
          <>
            <div className="form-grid">
              <Field label="Project">
                <Picker
                  value={draft.project_id}
                  label="Project"
                  onChange={(value) => update({ project_id: value })}
                  options={
                    projects.data?.items.map((p) => ({
                      value: p.id,
                      label: p.name,
                    })) ?? []
                  }
                />
              </Field>
              <Button
                aria-expanded={createProject}
                onClick={() => setCreateProject((value) => !value)}
              >
                + Create project
              </Button>
              <Field label="Cluster name">
                <input
                  value={draft.name}
                  onChange={(e) => update({ name: e.target.value })}
                  required
                  placeholder="commerce-primary"
                />
              </Field>
              <Field label="Environment">
                <Picker
                  value={draft.environment}
                  label="Environment"
                  options={options(['production', 'staging', 'development'])}
                  onChange={(value) =>
                    update({
                      environment: value as ClusterDraft['environment'],
                    })
                  }
                />
              </Field>
              <Field label="Provider">
                <Picker
                  value={draft.provider}
                  label="Provider"
                  options={options(['aws', 'gcp', 'azure', 'onprem', 'other'])}
                  onChange={(value) =>
                    update({ provider: value as ClusterDraft['provider'] })
                  }
                />
              </Field>
              <Field label="Region">
                <input
                  required
                  value={draft.region}
                  onChange={(e) => update({ region: e.target.value })}
                />
              </Field>
              <Field
                label="Tags"
                hint="Comma-separated key=value pairs, e.g. team=commerce, owner=platform"
              >
                <input
                  value={tagsText}
                  onChange={(e) => {
                    setTagsText(e.target.value);
                    update({ tags: parseTags(e.target.value) });
                  }}
                />
              </Field>
            </div>
            {createProject && (
              <div className="inline-project">
                <Field label="New project name">
                  <input
                    value={newProject}
                    onChange={(e) => setNewProject(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key !== 'Enter') return;
                      e.preventDefault();
                      submitProject();
                    }}
                  />
                </Field>
                <Button
                  disabled={addProject.isPending || !newProject.trim()}
                  onClick={submitProject}
                >
                  Create project
                </Button>
              </div>
            )}
          </>
        )}
        {step === 1 && (
          <>
            <ConnectionFields draft={draft} update={update} requirePassword />
            <ConnectionTest draft={draft} />
          </>
        )}
        {step === 2 && (
          <>
            <div className="cluster-review">
              <h3>
                {draft.name} <EnvBadge environment={draft.environment} />
              </h3>
              <p className="mono">
                {draft.host}:{draft.port} / {draft.database}
              </p>
              <p>
                {draft.engine} · {draft.provider} · {draft.region}
              </p>
            </div>
            <h3>Default safety policy</h3>
            <dl className="review-policy">
              <dt>Maximum rows</dt>
              <dd>{policy.max_rows.toLocaleString()}</dd>
              <dt>Statement timeout</dt>
              <dd>{policy.statement_timeout_ms / 1000} seconds</dd>
              <dt>Writes</dt>
              <dd>
                {policy.allow_writes ? 'Allowed with approval' : 'Disabled'}
              </dd>
              <dt>DDL</dt>
              <dd>Disabled</dd>
              <dt>Read routing</dt>
              <dd>Healthy replica preferred</dd>
            </dl>
            <div className="callout warning">
              Review the connection and environment carefully before saving. You
              can adjust the policy after creation.
            </div>
          </>
        )}
        <div className="dialog-actions">
          {step > 0 && <Button onClick={() => setStep(step - 1)}>Back</Button>}
          <Button onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button
            type="submit"
            variant="primary"
            disabled={create.isPending || (step === 0 && !draft.project_id)}
          >
            {create.isPending
              ? 'Creating…'
              : step === 2
                ? 'Create cluster'
                : 'Continue'}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
