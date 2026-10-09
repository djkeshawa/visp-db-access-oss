import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import type { Cluster } from '../../api/types';
import { Button, Confirm, Field } from '../../components/ui';
import { useAction } from '../../lib/query';
import { endpointChanged } from '../../lib/connection';
import { useUser } from '../auth/session';
import {
  ConnectionFields,
  ConnectionTest,
  connectionDraft,
  type ClusterDraft,
} from './forms';
export function ClusterSettings({ cluster }: { cluster: Cluster }) {
  const [draft, setDraft] = useState<ClusterDraft>(() =>
      connectionDraft(cluster),
    ),
    [deleting, setDeleting] = useState(false),
    [confirmation, setConfirmation] = useState('');
  const save = useAction('Cluster connection updated'),
    remove = useAction('Cluster deleted'),
    user = useUser(),
    navigate = useNavigate();
  const requirePassword = endpointChanged(cluster, draft);
  return (
    <div className="settings-page">
      <h2>Connection settings</h2>
      <p className="muted">
        Update the connection or rotate its encrypted password.
      </p>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (requirePassword && !draft.password) return;
          const { password, ...fields } = draft;
          save.mutate(
            {
              path: `/clusters/${cluster.id}`,
              method: 'PATCH',
              body: { ...fields, ...(password ? { password } : {}) },
            },
            {
              onSuccess: () =>
                setDraft((draft) => ({ ...draft, password: '' })),
            },
          );
        }}
      >
        <Field label="Cluster name">
          <input
            value={draft.name}
            onChange={(e) =>
              setDraft((draft) => ({ ...draft, name: e.target.value }))
            }
            required
          />
        </Field>
        <ConnectionFields
          draft={draft}
          requirePassword={requirePassword}
          update={(values) => setDraft((draft) => ({ ...draft, ...values }))}
        />
        {user?.org_role === 'admin' && (
          <ConnectionTest
            draft={draft}
            clusterId={cluster.id}
            requirePassword={requirePassword}
          />
        )}
        {requirePassword && !draft.password && (
          <p className="muted" role="status">
            Re-enter the password to save a changed connection endpoint.
          </p>
        )}
        <div className="dialog-actions">
          <Button
            type="submit"
            variant="primary"
            disabled={save.isPending || (requirePassword && !draft.password)}
          >
            Save connection
          </Button>
        </div>
      </form>
      {user?.org_role === 'admin' && (
        <section className="danger-zone">
          <h3>Delete cluster</h3>
          <p>
            The gateway will remove this connection. This does not delete the
            target database.
          </p>
          <Button variant="danger" onClick={() => setDeleting(true)}>
            Delete cluster
          </Button>
        </section>
      )}
      <Confirm
        open={deleting}
        onOpenChange={(open) => {
          setDeleting(open);
          if (!open) setConfirmation('');
        }}
        title="Delete cluster?"
        description={`Type ${cluster.name} to confirm removing this connection.`}
        danger
        busy={remove.isPending || confirmation !== cluster.name}
        onConfirm={() => {
          if (confirmation === cluster.name)
            remove.mutate(
              { path: `/clusters/${cluster.id}`, method: 'DELETE' },
              { onSuccess: () => navigate('/clusters') },
            );
        }}
      >
        <Field label="Cluster name to confirm">
          <input
            value={confirmation}
            autoComplete="off"
            spellCheck={false}
            onChange={(e) => setConfirmation(e.target.value)}
          />
        </Field>
      </Confirm>
    </div>
  );
}
