import { BulkGrants } from './bulk-grants';
import { useState, useEffect, useMemo } from 'react';
import type { Cluster, Grant, Project, UserLookup } from '../../api/types';
import { useAction, useResource } from '../../lib/query';
import { useUser } from '../auth/session';
import {
  Button,
  Confirm,
  Empty,
  Field,
  Modal,
  Picker,
  ErrorPanel,
  Badge,
  RowMenu,
} from '../../components/ui';
import { formatTime } from '../../lib/utils';
export function GrantsTable({
  items,
  onAdd,
}: {
  items: Grant[];
  onAdd?: () => void;
}) {
  const [revoke, setRevoke] = useState<Grant | null>(null),
    [selected, setSelected] = useState<string[]>([]),
    [bulk, setBulk] = useState(false);
  const selectedIds = useMemo(() => new Set(selected), [selected]);
  const selection = useMemo(
    () => items.filter((item) => selectedIds.has(item.id)),
    [items, selectedIds],
  );
  const remove = useAction('Access revoked');
  return (
    <>
      <div className="section-toolbar">
        <span className="muted">
          {items.length} grants · Project grants apply to all clusters in that
          project
        </span>
        {selection.length > 0 && (
          <Button variant="danger" onClick={() => setBulk(true)}>
            Revoke selected ({selection.length})
          </Button>
        )}
        {onAdd && (
          <Button variant="primary" onClick={onAdd}>
            Add grant
          </Button>
        )}
      </div>
      {items.length === 0 ? (
        <Empty
          title="No explicit grants"
          description="Organization administrators have implicit access. Add a grant to give a colleague access."
          action={onAdd && <Button onClick={onAdd}>Add grant</Button>}
        />
      ) : (
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>
                  <input
                    type="checkbox"
                    aria-label="Select all loaded grants"
                    checked={
                      items.length > 0 &&
                      items.every((item) => selectedIds.has(item.id))
                    }
                    onChange={(event) =>
                      setSelected(
                        event.target.checked
                          ? items.map((item) => item.id)
                          : [],
                      )
                    }
                  />
                </th>
                <th>User</th>
                <th>Level</th>
                <th>Scope</th>
                <th>Expires</th>
                <th>Added by</th>
                <th>
                  <span className="sr-only">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {items.map((grant) => (
                <tr key={grant.id}>
                  <td>
                    <input
                      type="checkbox"
                      aria-label={`Select grant for ${grant.user.name} on ${grant.scope_name}`}
                      checked={selectedIds.has(grant.id)}
                      onChange={(event) =>
                        setSelected((ids) =>
                          event.target.checked
                            ? [...ids, grant.id]
                            : ids.filter((id) => id !== grant.id),
                        )
                      }
                    />
                  </td>
                  <td>
                    <strong>{grant.user.name}</strong>
                    <small>{grant.user.email}</small>
                  </td>
                  <td>
                    <Badge>{grant.level}</Badge>
                  </td>
                  <td>
                    {grant.scope_name}
                    <small>{grant.scope}</small>
                  </td>
                  <td>
                    {grant.expires_at ? formatTime(grant.expires_at) : 'Never'}
                  </td>
                  <td className="muted">—</td>
                  <td>
                    <RowMenu
                      label={`Actions for ${grant.user.name} on ${grant.scope_name}`}
                      items={[
                        { label: 'Revoke', onSelect: () => setRevoke(grant) },
                      ]}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <BulkGrants
        open={bulk}
        onOpenChange={setBulk}
        grants={selection}
        onRemoved={(ids) => {
          const removed = new Set(ids);
          setSelected((selected) => selected.filter((id) => !removed.has(id)));
        }}
      />
      <Confirm
        open={!!revoke}
        onOpenChange={(open) => {
          if (!open) setRevoke(null);
        }}
        title="Revoke this grant?"
        description={`${revoke?.user.name} will lose this ${revoke?.level} grant on ${revoke?.scope_name}. Other grants may still provide access.`}
        danger
        busy={remove.isPending}
        onConfirm={() =>
          remove.mutate(
            { path: `/grants/${revoke?.id}`, method: 'DELETE' },
            { onSuccess: () => setRevoke(null) },
          )
        }
      />
    </>
  );
}
export function AddGrant({
  open,
  onOpenChange,
  cluster,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  cluster?: Cluster;
}) {
  const user = useUser();
  const [search, setSearch] = useState(''),
    [debounced, setDebounced] = useState('');
  const [selectedUser, setSelectedUser] = useState<UserLookup | null>(null);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(search.trim()), 300);
    return () => clearTimeout(timer);
  }, [search]);
  const users = useResource<{ items: UserLookup[] }>(
    `/users/lookup?q=${encodeURIComponent(debounced)}`,
    open && [...debounced].length >= 2,
  );
  const projects = useResource<{ items: Project[] }>(
    '/projects',
    open && !cluster,
  );
  const clusters = useResource<{ items: Cluster[] }>(
    '/clusters',
    open && !cluster,
  );
  const [userId, setUserId] = useState(''),
    [level, setLevel] = useState('read'),
    [scope, setScope] = useState(cluster ? 'cluster' : 'project'),
    [scopeId, setScopeId] = useState(cluster?.id ?? ''),
    [expiry, setExpiry] = useState('8');
  const create = useAction('Access granted');
  const { reset: resetCreate } = create;
  const clusterId = cluster?.id;
  // Start every opening from a clean form instead of the previous grant's values.
  useEffect(() => {
    if (!open) return;
    setSearch('');
    setDebounced('');
    setSelectedUser(null);
    setUserId('');
    setLevel('read');
    setScope(clusterId ? 'cluster' : 'project');
    setScopeId(clusterId ?? '');
    setExpiry('8');
    resetCreate();
  }, [open, clusterId, resetCreate]);
  const scopeOptions =
    (scope === 'project' ? projects.data?.items : clusters.data?.items)?.map(
      (item) => ({ value: item.id, label: item.name }),
    ) ?? [];
  return (
    <Modal
      open={open}
      onOpenChange={onOpenChange}
      title="Grant database access"
      description="Prefer a short expiry and the least access needed for the task."
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (create.isPending || !userId || !scopeId) return;
          create.mutate(
            {
              path: '/grants',
              body: {
                user_id: userId,
                scope,
                scope_id: scopeId,
                level,
                expires_at:
                  expiry === 'never'
                    ? null
                    : new Date(
                        Date.now() + Number(expiry) * 3600000,
                      ).toISOString(),
              },
            },
            { onSuccess: () => onOpenChange(false) },
          );
        }}
      >
        <Field
          label="Find user"
          hint="Type at least 2 characters of a name or email. Up to 20 matches are shown."
        >
          <input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            autoComplete="off"
          />
        </Field>
        {users.error && <ErrorPanel error={users.error} />}
        {users.isFetching && <p className="muted">Searching users…</p>}
        {users.data?.items.length === 0 && (
          <p className="muted">No matching active users.</p>
        )}
        <Field label="User">
          <Picker
            value={userId}
            label="User"
            onChange={(id) => {
              setUserId(id);
              setSelectedUser(
                users.data?.items.find((user) => user.id === id) ?? null,
              );
            }}
            options={Array.from(
              new Map(
                [
                  ...(selectedUser ? [selectedUser] : []),
                  ...(users.data?.items ?? []),
                ].map((user) => [user.id, user]),
              ).values(),
            )
              .filter(
                (candidate) =>
                  user?.org_role === 'admin' || candidate.id !== user?.id,
              )
              .map((user) => ({
                value: user.id,
                label: `${user.name} (${user.email})`,
              }))}
          />
        </Field>
        {!cluster && (
          <>
            <Field label="Scope">
              <Picker
                value={scope}
                label="Scope"
                onChange={(value) => {
                  setScope(value);
                  setScopeId('');
                }}
                options={[
                  { value: 'project', label: 'Project' },
                  { value: 'cluster', label: 'Cluster' },
                ]}
              />
            </Field>
            <Field label={scope === 'project' ? 'Project' : 'Cluster'}>
              <Picker
                value={scopeId}
                label="Access scope"
                onChange={setScopeId}
                options={scopeOptions}
              />
            </Field>
          </>
        )}
        <Field label="Access level">
          <Picker
            value={level}
            label="Access level"
            onChange={setLevel}
            options={['read', 'write', 'admin'].map((value) => ({
              value,
              label: value,
            }))}
          />
        </Field>
        <Field label="Expires after">
          <Picker
            value={expiry}
            label="Expiry"
            onChange={setExpiry}
            options={[
              { value: '1', label: '1 hour' },
              { value: '8', label: '8 hours' },
              { value: '24', label: '24 hours' },
              { value: '168', label: '7 days' },
              { value: 'never', label: 'Never' },
            ]}
          />
        </Field>
        <div className="dialog-actions">
          <Button onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button
            variant="primary"
            type="submit"
            disabled={create.isPending || !userId || !scopeId}
          >
            Grant access
          </Button>
        </div>
      </form>
    </Modal>
  );
}
