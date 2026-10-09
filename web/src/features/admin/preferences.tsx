import { AboutConsole } from './about';
import { useState } from 'react';
import { usePreferences } from '../../lib/preferences';
import { useTheme } from '../../lib/theme';
import { useUser } from '../auth/session';
import { useAction } from '../../lib/query';
import { Button, Field, SegmentedControl } from '../../components/ui';
export function PersonalSettings() {
  const user = useUser(),
    { preferences, setPreferences } = usePreferences(),
    { theme, setTheme } = useTheme();
  const profile = useAction('Profile updated'),
    password = useAction('Password changed'),
    [name, setName] = useState(user?.name ?? '');
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Your settings</h1>
          <p>Profile, security and preferences for this browser.</p>
        </div>
      </div>
      <section className="settings-section">
        <h2>Profile</h2>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (user?.org_role === 'admin' && !profile.isPending)
              profile.mutate({
                path: `/users/${user.id}`,
                method: 'PATCH',
                body: { name: name.trim() },
              });
          }}
        >
          <Field label="Name">
            <input
              value={name}
              onChange={(event) => setName(event.target.value)}
              readOnly={user?.org_role !== 'admin'}
              required
            />
          </Field>
          <Field label="Email">
            <input value={user?.email ?? ''} readOnly />
          </Field>
          {user?.org_role === 'admin' ? (
            <Button
              type="submit"
              variant="secondary"
              disabled={
                profile.isPending || !name.trim() || name.trim() === user.name
              }
            >
              {profile.isPending ? 'Saving…' : 'Save name'}
            </Button>
          ) : (
            <p className="muted">
              Ask an organization administrator to update your name.
            </p>
          )}
        </form>
      </section>
      <section className="settings-section">
        <h2>Password</h2>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (password.isPending) return;
            const form = event.currentTarget;
            password.mutate(
              {
                path: '/auth/change-password',
                body: Object.fromEntries(new FormData(form)),
              },
              { onSuccess: () => form.reset() },
            );
          }}
        >
          <Field label="Current password">
            <input
              name="current_password"
              type="password"
              required
              autoComplete="current-password"
            />
          </Field>
          <Field label="New password" hint="At least 12 characters.">
            <input
              name="new_password"
              type="password"
              required
              minLength={12}
              autoComplete="new-password"
            />
          </Field>
          <Button
            type="submit"
            variant="secondary"
            disabled={password.isPending}
          >
            {password.isPending ? 'Changing…' : 'Change password'}
          </Button>
        </form>
      </section>
      <section className="settings-section">
        <h2>Preferences</h2>
        <p className="muted">Saved in this browser only.</p>
        <Field label="Theme" group>
          <SegmentedControl
            label="Theme"
            value={theme}
            onChange={(value) => setTheme(value as typeof theme)}
            items={['light', 'dark', 'system'].map((value) => ({
              value,
              label: value[0]?.toUpperCase() + value.slice(1),
            }))}
          />
        </Field>
        <div>
          <Field label="Density" group>
            <SegmentedControl
              label="Density"
              value={preferences.density}
              onChange={(density) =>
                setPreferences({
                  ...preferences,
                  density: density as typeof preferences.density,
                })
              }
              items={[
                { value: 'comfortable', label: 'Comfortable' },
                { value: 'compact', label: 'Compact' },
              ]}
            />
          </Field>
          <Field label="Timezone display" group>
            <SegmentedControl
              label="Timezone display"
              value={preferences.timezone}
              onChange={(timezone) =>
                setPreferences({
                  ...preferences,
                  timezone: timezone as typeof preferences.timezone,
                })
              }
              items={[
                { value: 'local', label: 'Local timezone' },
                { value: 'utc', label: 'UTC' },
              ]}
            />
          </Field>
          <Field label="Editor font size" group>
            <SegmentedControl
              label="Editor font size"
              value={String(preferences.editorFontSize)}
              onChange={(value) =>
                setPreferences({
                  ...preferences,
                  editorFontSize: Number(value),
                })
              }
              items={[12, 13, 14, 16, 20].map((value) => ({
                value: String(value),
                label: `${value} px`,
              }))}
            />
          </Field>
        </div>
      </section>
      <AboutConsole />
    </>
  );
}
