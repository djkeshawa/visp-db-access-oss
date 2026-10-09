import { useEffect, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { useQueryClient } from '@tanstack/react-query';
import { request, json } from '../../api/client';
import { loginMessage } from '../../lib/utils';
import { BrandMark } from '../../components/ui/brand';
import { Button, Field } from '../../components/ui';
/** Only same-origin absolute paths are valid post-login targets (no `//host` or `/\\host`). */
const safeRedirect = (from: string | undefined) =>
  from && from.startsWith('/') && !/^\/[/\\]/.test(from) ? from : '/overview';
export function Login() {
  useEffect(() => {
    document.title = 'Sign in — visp';
  }, []);
  const navigate = useNavigate(),
    location = useLocation(),
    cache = useQueryClient();
  const [error, setError] = useState(''),
    [busy, setBusy] = useState(false);
  return (
    <main className="login">
      <div className="login-brand">
        <BrandMark size={32} />
        <strong>
          visp <span>· db access</span>
        </strong>
      </div>
      <p className="login-subtitle">Guarded database access</p>
      <form
        className="login-card"
        onSubmit={async (event) => {
          event.preventDefault();
          if (busy) return;
          setBusy(true);
          setError('');
          const data = new FormData(event.currentTarget);
          try {
            const value = await request(
              '/auth/login',
              json('POST', Object.fromEntries(data)),
            );
            cache.clear();
            cache.setQueryData(['api', '/auth/me'], value);
            const from = (location.state as { from?: string } | null)?.from;
            navigate(safeRedirect(from), { replace: true });
          } catch (error) {
            setError(loginMessage(error));
          } finally {
            setBusy(false);
          }
        }}
      >
        <h1>Sign in to your workspace</h1>

        <Field label="Email">
          <input
            name="email"
            type="email"
            autoComplete="username"
            required
            defaultValue={
              import.meta.env.VITE_MOCK === '1' ? 'admin@visp.dev' : ''
            }
          />
        </Field>
        <Field label="Password">
          <input
            name="password"
            type="password"
            autoComplete="current-password"
            required
            defaultValue={
              import.meta.env.VITE_MOCK === '1' ? 'demo-password' : ''
            }
          />
        </Field>
        {error && (
          <p className="error-text" role="alert">
            {error}
          </p>
        )}
        <Button type="submit" variant="primary" disabled={busy}>
          {busy ? 'Signing in…' : 'Sign in'}
        </Button>
        {import.meta.env.VITE_MOCK === '1' && (
          <div className="demo-note">
            Demo accounts: <code>admin@visp.dev</code>,{' '}
            <code>reviewer@visp.dev</code>, <code>member@visp.dev</code>
            <br />
            Password: <code>demo-password</code>
          </div>
        )}
      </form>
    </main>
  );
}
