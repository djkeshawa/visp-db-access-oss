import { afterEach, describe, it, expect, vi } from 'vitest';
import { request, json, params } from '../src/api/client';
afterEach(() => vi.unstubAllGlobals());
describe('HTTP contract', () => {
  it('uses session credentials and the CSRF header for reads and writes', async () => {
    const fetcher = vi
      .fn()
      .mockImplementation(
        async () => new Response(JSON.stringify({ ok: true }), { status: 200 }),
      );
    vi.stubGlobal('fetch', fetcher);
    await request('/auth/me');
    await request('/clusters/1/analyze', json('POST', { sql: 'SELECT 1' }));
    for (const [, init] of fetcher.mock.calls as [string, RequestInit][]) {
      expect(new Headers(init.headers).get('X-Requested-With')).toBe('vda');
    }
    expect(fetcher).toHaveBeenCalledWith(
      '/api/v1/auth/me',
      expect.objectContaining({
        credentials: 'same-origin',
      }),
    );
    expect(fetcher).toHaveBeenCalledWith(
      '/api/v1/clusters/1/analyze',
      expect.objectContaining({
        method: 'POST',
        body: '{"sql":"SELECT 1"}',
      }),
    );
  });
  it('preserves typed errors and signals an expired session without navigation', async () => {
    const assign = vi.fn(),
      dispatchEvent = vi.fn();
    vi.stubGlobal('window', { dispatchEvent });
    vi.stubGlobal('location', { pathname: '/console', assign });
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            error: {
              code: 'unauthenticated',
              message: 'Expired',
              details: { expired: true },
            },
          }),
          { status: 401 },
        ),
      ),
    );
    await expect(request('/auth/me')).rejects.toMatchObject({
      code: 'unauthenticated',
      message: 'Expired',
      status: 401,
      details: { expired: true },
    });
    expect(assign).not.toHaveBeenCalled();
    expect(dispatchEvent).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'vda:session-expired' }),
    );
  });
  it('handles 204 and preserves upstream DB error messages', async () => {
    vi.stubGlobal('location', { pathname: '/login', assign: vi.fn() });
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValueOnce(new Response(null, { status: 204 }))
        .mockResolvedValueOnce(
          new Response(
            JSON.stringify({
              error: {
                code: 'upstream',
                message: 'relation does not exist',
                details: null,
              },
            }),
            { status: 502 },
          ),
        ),
    );
    expect(await request('/auth/logout', json('POST'))).toBeUndefined();
    await expect(
      request('/clusters/1/query', json('POST', { sql: 'SELECT x' })),
    ).rejects.toMatchObject({
      code: 'upstream',
      message: 'relation does not exist',
      status: 502,
    });
  });
  it('escapes filters and omits empty values', () =>
    expect(params({ q: 'a & b', status: '', cursor: null, limit: 50 })).toBe(
      '?q=a+%26+b&limit=50',
    ));
});
it('preserves Headers inputs while enforcing the CSRF value', async () => {
  const fetcher = vi
    .fn()
    .mockResolvedValue(new Response('{}', { status: 200 }));
  vi.stubGlobal('fetch', fetcher);
  await request('/auth/me', {
    headers: new Headers({ 'X-Test': 'kept', 'X-Requested-With': 'wrong' }),
  });
  const init = fetcher.mock.calls[0]?.[1] as RequestInit;
  const headers = new Headers(init.headers);
  expect(headers.get('X-Test')).toBe('kept');
  expect(headers.get('X-Requested-With')).toBe('vda');
});

it('signals network failure and service unavailability without treating DB errors as offline', async () => {
  const dispatchEvent = vi.fn();
  vi.stubGlobal('window', { dispatchEvent });
  vi.stubGlobal('location', { pathname: '/console' });
  vi.stubGlobal(
    'fetch',
    vi
      .fn()
      .mockRejectedValueOnce(new TypeError('Failed to fetch'))
      .mockResolvedValueOnce(new Response('{}', { status: 503 }))
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            error: { code: 'upstream', message: 'relation missing' },
          }),
          { status: 502 },
        ),
      ),
  );
  await expect(request('/overview')).rejects.toThrow('Failed to fetch');
  await expect(request('/overview')).rejects.toMatchObject({ status: 503 });
  await expect(request('/clusters/1/query')).rejects.toMatchObject({
    code: 'upstream',
    message: 'relation missing',
  });
  expect(dispatchEvent).toHaveBeenCalledTimes(2);
  expect(dispatchEvent).toHaveBeenCalledWith(
    expect.objectContaining({ type: 'vda:unreachable' }),
  );
});
it('does not show an offline banner for an aborted read', async () => {
  const dispatchEvent = vi.fn();
  vi.stubGlobal('window', { dispatchEvent });
  vi.stubGlobal(
    'fetch',
    vi.fn().mockRejectedValue(new DOMException('Aborted', 'AbortError')),
  );
  await expect(request('/overview')).rejects.toMatchObject({
    name: 'AbortError',
  });
  expect(dispatchEvent).not.toHaveBeenCalled();
});
it('reports an unreadable success body as a typed error', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue(new Response('<html>', { status: 200 })),
  );
  await expect(request('/overview')).rejects.toMatchObject({
    name: 'ApiError',
    code: 'invalid_response',
    status: 200,
  });
});
it('survives a 401 outside a browser without a location', async () => {
  vi.stubGlobal('location', undefined);
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue(new Response('{}', { status: 401 })),
  );
  await expect(request('/auth/me')).rejects.toMatchObject({ status: 401 });
});
