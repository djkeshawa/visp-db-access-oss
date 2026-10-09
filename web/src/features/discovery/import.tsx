import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import type {
  Cluster,
  DiscoveredResource,
  DiscoverySource,
  Environment,
  Health,
  Project,
  TlsMode,
} from '../../api/types';
import { useAction, useResource } from '../../lib/query';
import {
  Badge,
  Button,
  EnvBadge,
  ErrorPanel,
  Field,
  HealthBadge,
  Modal,
  Picker,
} from '../../components/ui';

export function ImportResource({
  resource,
  source,
  onClose,
}: {
  resource: DiscoveredResource;
  source?: DiscoverySource;
  onClose: () => void;
}) {
  const projects = useResource<{ items: Project[] }>('/projects');
  const [draft, setDraft] = useState({
    project_id: source?.default_project_id ?? '',
    name: resource.identifier,
    environment: resource.suggested_environment,
    database: resource.database ?? '',
    username: '',
    password: '',
    tls_mode: 'verify_full' as TlsMode,
  });
  const [newProject, setNewProject] = useState<string | null>(null);
  const [createdProject, setCreatedProject] = useState<Project | null>(null);
  const projectItems =
    createdProject &&
    !projects.data?.items.some((project) => project.id === createdProject.id)
      ? [...(projects.data?.items ?? []), createdProject]
      : (projects.data?.items ?? []);
  const test = useAction<Health>(''),
    save = useAction<Cluster>('Database imported'),
    createProject = useAction<Project>('Project created');
  const navigate = useNavigate();
  const update = <K extends keyof typeof draft>(
    key: K,
    value: (typeof draft)[K],
  ) => {
    setDraft((draft) => ({ ...draft, [key]: value }));
    // Only credentials and TLS settings invalidate a passed connection test.
    if (['database', 'username', 'password', 'tls_mode'].includes(key))
      test.reset();
  };
  const busy = test.isPending || save.isPending || createProject.isPending;
  const connectionValid =
    !!draft.database.trim() && !!draft.username.trim() && !!draft.password;
  const connection = {
    engine: resource.engine,
    host: resource.host,
    port: resource.port,
    database: draft.database,
    username: draft.username,
    password: draft.password,
    tls_mode: draft.tls_mode,
  };
  const tested =
    test.data &&
    ['healthy', 'degraded'].includes(test.data.status) &&
    JSON.stringify(test.variables?.body) === JSON.stringify(connection);
  const valid =
    connectionValid &&
    draft.name.trim() &&
    projectItems.some((p) => p.id === draft.project_id);
  const createInlineProject = () => {
    if (!newProject?.trim() || busy) return;
    createProject.mutate(
      { path: '/projects', body: { name: newProject.trim(), description: '' } },
      {
        onSuccess: (project) => {
          setCreatedProject(project);
          update('project_id', project.id);
          setNewProject(null);
        },
      },
    );
  };
  return (
    <Modal
      open
      wide
      title="Import discovered database"
      description="Review the discovered endpoint and verify database credentials before creating a cluster."
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <section className="discovery-endpoint">
        <div className="inline">
          <strong>{resource.identifier}</strong>
          <Badge>{resource.kind === 'aurora_cluster' ? 'Aurora' : 'RDS'}</Badge>
        </div>
        <p className="mono discovery-break">
          {resource.host}:{resource.port}
        </p>
        <p className="muted">
          {resource.engine_detail} {resource.engine_version} ·{' '}
          {resource.account_id} / {resource.region}
        </p>
        {resource.replica_host && (
          <p className="mono discovery-break">
            Reader: {resource.replica_host}:{resource.replica_port}
          </p>
        )}
      </section>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (valid && tested && !busy)
            save.mutate(
              {
                path: `/discovery/resources/${resource.id}/import`,
                body: draft,
              },
              {
                onSuccess: (cluster) => {
                  onClose();
                  void navigate(`/clusters/${cluster.id}`);
                },
              },
            );
        }}
      >
        <div className="form-grid">
          <Field label="Project">
            <Picker
              label="Project"
              value={draft.project_id}
              onChange={(value) => {
                // The native Select bridge may emit an empty value as new options register.
                if (value) update('project_id', value);
              }}
              options={projectItems.map((project) => ({
                value: project.id,
                label: project.name,
              }))}
            />
          </Field>
          <Field label="Cluster name">
            <input
              required
              value={draft.name}
              onChange={(e) => update('name', e.target.value)}
            />
          </Field>
        </div>
        {projects.error && (
          <ErrorPanel
            error={projects.error}
            retry={() => void projects.refetch()}
          />
        )}
        {newProject === null ? (
          <Button onClick={() => setNewProject('')}>
            Create project inline
          </Button>
        ) : (
          <div className="discovery-project-create">
            <Field label="New project name">
              <input
                value={newProject}
                onChange={(e) => setNewProject(e.target.value)}
                onKeyDown={(e) => {
                  // Enter here creates the project rather than submitting the import form.
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    createInlineProject();
                  }
                }}
              />
            </Field>
            <Button
              disabled={!newProject.trim() || busy}
              onClick={createInlineProject}
            >
              Create project
            </Button>
            <Button disabled={busy} onClick={() => setNewProject(null)}>
              Cancel project
            </Button>
          </div>
        )}
        <div className="form-grid">
          <Field label="Environment">
            <Picker
              label="Environment"
              value={draft.environment}
              onChange={(value) => update('environment', value as Environment)}
              options={['production', 'staging', 'development'].map(
                (value) => ({ value, label: value }),
              )}
            />
          </Field>
          <Field label="TLS mode">
            <Picker
              label="TLS mode"
              value={draft.tls_mode}
              onChange={(value) => update('tls_mode', value as TlsMode)}
              options={['verify_full', 'require', 'prefer', 'disable'].map(
                (value) => ({ value, label: value.replaceAll('_', ' ') }),
              )}
            />
          </Field>
        </div>
        {draft.environment === 'production' && (
          <div className="callout warning">
            <EnvBadge environment="production" />
            <span>
              This database will use production safety defaults: read-only
              access, query limits, and approval for writes.
            </span>
          </div>
        )}
        <div className="form-grid">
          <Field label="Database">
            <input
              required
              value={draft.database}
              onChange={(e) => update('database', e.target.value)}
            />
          </Field>
          <Field label="Username">
            <input
              required
              value={draft.username}
              onChange={(e) => update('username', e.target.value)}
              autoComplete="off"
            />
          </Field>
        </div>
        <Field
          label="Password"
          hint="Credentials are encrypted by the gateway and never returned to users."
        >
          <input
            type="password"
            autoComplete="new-password"
            required
            value={draft.password}
            onChange={(e) => update('password', e.target.value)}
          />
        </Field>
        {test.data && (
          <div className="callout">
            <HealthBadge
              status={test.data.status}
              latency={test.data.latency_ms}
            />
            <span>
              {tested
                ? 'Connection verified. Ready to import.'
                : (test.data.error ?? 'Connection did not pass.')}
            </span>
          </div>
        )}
        {test.error && <ErrorPanel error={test.error} />}
        {save.error && <ErrorPanel error={save.error} />}
        {createProject.error && <ErrorPanel error={createProject.error} />}
        <div className="dialog-actions">
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            disabled={!connectionValid || busy}
            onClick={() =>
              test.mutate({
                path: '/clusters/test-connection',
                body: connection,
              })
            }
          >
            {test.isPending ? 'Testing…' : 'Test connection'}
          </Button>
          <Button
            type="submit"
            variant="primary"
            disabled={!valid || !tested || busy}
          >
            {save.isPending ? 'Importing…' : 'Import database'}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
