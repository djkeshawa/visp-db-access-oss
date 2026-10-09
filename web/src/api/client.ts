import { ApiError, type Transport } from './errors';
let mock: Promise<Transport> | undefined;
/** Fetches JSON with session credentials and the contract's CSRF header. */
export const request: Transport = async <T>(
  path: string,
  init: RequestInit = {},
) => {
  if (import.meta.env.VITE_MOCK === '1') {
    mock ??= import('./mock').then((module) => module.createMockTransport());
    return (await mock)<T>(path, init);
  }
  const headers = new Headers(init.headers);
  headers.set('Content-Type', 'application/json');
  headers.set('X-Requested-With', 'vda');
  let response: Response;
  try {
    response = await fetch(`/api/v1${path}`, {
      ...init,
      credentials: 'same-origin',
      headers,
    });
  } catch (error) {
    if (
      !(error instanceof DOMException && error.name === 'AbortError') &&
      typeof window !== 'undefined'
    )
      window.dispatchEvent(new Event('vda:unreachable'));
    throw error;
  }
  if (response.status === 503 && typeof window !== 'undefined')
    window.dispatchEvent(new Event('vda:unreachable'));
  if (!response.ok) {
    const value = (await response.json().catch(() => null)) as {
      error?: { code: string; message: string; details: unknown };
    } | null;
    if (
      response.status === 401 &&
      typeof window !== 'undefined' &&
      !globalThis.location?.pathname.startsWith('/login')
    )
      window.dispatchEvent(new Event('vda:session-expired'));
    throw new ApiError(
      value?.error?.code ?? 'http_error',
      value?.error?.message ?? `Request failed (${response.status}).`,
      response.status,
      value?.error?.details,
    );
  }
  if (response.status === 204) return undefined as T;
  try {
    return (await response.json()) as T;
  } catch (error) {
    // An aborted body read must stay an abort; anything else is a bad payload.
    if (error instanceof DOMException && error.name === 'AbortError')
      throw error;
    throw new ApiError(
      'invalid_response',
      'The server returned an unreadable response.',
      response.status,
    );
  }
};
export const json = (method: string, body?: unknown): RequestInit => ({
  method,
  ...(body === undefined ? {} : { body: JSON.stringify(body) }),
});
export function params(
  values: Record<string, string | number | undefined | null>,
): string {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(values))
    if (value !== undefined && value !== null && value !== '')
      query.set(key, String(value));
  const text = query.toString();
  return text ? `?${text}` : '';
}
