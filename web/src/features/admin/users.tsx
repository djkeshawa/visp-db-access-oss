import { useSearchParams } from 'react-router-dom';
import { useMemo, useState } from 'react';
import type { User } from '../../api/types';
import { useAction, useResource } from '../../lib/query';
import {
  Badge,
  Button,
  Confirm,
  Empty,
  ErrorPanel,
  Field,
  Modal,
  Picker,
  Skeleton,
  RowMenu,
  SwitchRow,
} from '../../components/ui';
import { formatTime } from '../../lib/utils';
import { useUser } from '../auth/session';
export function UsersPage() {
  const query = useResource<{ items: User[] }>('/users'),
    current = useUser();
  const [params] = useSearchParams();
  const [editing, setEditing] = useState<User | 'new' | null>(
      params.get('add') === 'true' ? 'new' : null,
    ),
    [disable, setDisable] = useState<User | null>(null),
    [search, setSearch] = useState('');
  const filtered = useMemo(() => {
    const needle = search.toLowerCase();
    return (
      query.data?.items.filter((user) =>
        `${user.name} ${user.email}`.toLowerCase().includes(needle),
      ) ?? []
    );
  }, [query.data, search]);
  const remove = useAction('User disabled and sessions revoked');
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Users</h1>
          <p>Manage workspace identities and organization roles.</p>
        </div>
        <Button variant="primary" onClick={() => setEditing('new')}>
          Create user
        </Button>
      </div>
      <div className="filterbar">
        <input
          aria-label="Search users"
          placeholder="Search by name or email…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>
      {query.isPending ? (
        <Skeleton />
      ) : query.error ? (
        <ErrorPanel error={query.error} />
      ) : filtered.length === 0 ? (
        <Empty
          title="No matching users"
          description="Clear your search, or create an identity to grant database access."
          action={
            <Button
              onClick={() => (search ? setSearch('') : setEditing('new'))}
            >
              {search ? 'Clear search' : 'Create user'}
            </Button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>User</th>
                <th>Role</th>
                <th>Status</th>
                <th>Last login</th>
                <th>Created</th>
                <th>
                  <span className="sr-only">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((user) => (
                <tr key={user.id}>
                  <td>
                    <strong>{user.name}</strong>
                    <small>{user.email}</small>
                  </td>
                  <td>
                    <Badge>{user.org_role}</Badge>
                  </td>
                  <td>
                    <Badge tone={user.disabled ? 'neutral' : 'success'}>
                      {user.disabled ? 'Disabled' : 'Active'}
                    </Badge>
                  </td>
                  <td>{formatTime(user.last_login_at)}</td>
                  <td>{formatTime(user.created_at)}</td>
                  <td>
                    <RowMenu
                      label={`Actions for ${user.name}`}
                      items={[
                        { label: 'Edit', onSelect: () => setEditing(user) },
                        ...(!user.disabled && user.id !== current?.id
                          ? [
                              {
                                label: 'Disable',
                                onSelect: () => setDisable(user),
                              },
                            ]
                          : []),
                      ]}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <Modal
        open={!!editing}
        onOpenChange={(open) => {
          if (!open) setEditing(null);
        }}
        title={editing === 'new' ? 'Create user' : 'Edit user'}
      >
        {editing && (
          <UserForm
            key={editing === 'new' ? 'new' : editing.id}
            user={editing === 'new' ? undefined : editing}
            close={() => setEditing(null)}
          />
        )}
      </Modal>
      <Confirm
        open={!!disable}
        onOpenChange={(open) => {
          if (!open) setDisable(null);
        }}
        title="Disable this user?"
        description={`${disable?.name} will lose access and all active sessions will be revoked.`}
        busy={remove.isPending}
        danger
        onConfirm={() =>
          remove.mutate(
            { path: `/users/${disable?.id}`, method: 'DELETE' },
            { onSuccess: () => setDisable(null) },
          )
        }
      />
    </>
  );
}
function UserForm({ user, close }: { user?: User; close: () => void }) {
  const [role, setRole] = useState(user?.org_role ?? 'member'),
    [disabled, setDisabled] = useState(user?.disabled ?? false);
  const save = useAction(user ? 'User updated' : 'User created');
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        if (save.isPending) return;
        const values = Object.fromEntries(new FormData(event.currentTarget));
        const { password, ...fields } = values;
        save.mutate(
          {
            path: user ? `/users/${user.id}` : '/users',
            method: user ? 'PATCH' : 'POST',
            body: {
              ...fields,
              org_role: role,
              ...(user ? { disabled } : {}),
              ...(password ? { password } : {}),
            },
          },
          { onSuccess: close },
        );
      }}
    >
      <Field label="Name">
        <input name="name" required defaultValue={user?.name} />
      </Field>
      {!user && (
        <Field label="Email">
          <input name="email" type="email" required />
        </Field>
      )}
      <Field label="Organization role">
        <Picker
          value={role}
          onChange={(value) => setRole(value as User['org_role'])}
          label="Organization role"
          options={[
            { value: 'member', label: 'Member' },
            { value: 'admin', label: 'Administrator' },
          ]}
        />
      </Field>
      {role === 'admin' && (
        <div className="callout warning">
          Organization administrators have full access to every cluster.
        </div>
      )}
      <Field
        label={user ? 'Reset password (optional)' : 'Initial password'}
        hint="At least 12 characters."
      >
        <input
          name="password"
          type="password"
          minLength={12}
          required={!user}
          autoComplete="new-password"
        />
      </Field>
      {user && (
        <SwitchRow label="Disabled" checked={disabled} onChange={setDisabled} />
      )}
      <div className="dialog-actions">
        <Button onClick={close}>Cancel</Button>
        <Button type="submit" variant="primary" disabled={save.isPending}>
          {user ? 'Save user' : 'Create user'}
        </Button>
      </div>
    </form>
  );
}
