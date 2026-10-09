import { readPreferences } from './preferences';
import { ApiError } from '../api/errors';
const timeFormats = new Map<string, Intl.DateTimeFormat>();
/** Intl formatters are costly to build; result grids format many timestamps. */
function timeFormat(utc: boolean): Intl.DateTimeFormat {
  const key = utc ? 'utc' : 'local';
  let format = timeFormats.get(key);
  if (!format) {
    format = new Intl.DateTimeFormat(undefined, {
      timeZone: utc ? 'UTC' : undefined,
      timeZoneName: 'short',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
    timeFormats.set(key, format);
  }
  return format;
}
export const formatTime = (value: string | null) => {
  if (!value) return '—';
  const date = new Date(value);
  // Intl.DateTimeFormat#format throws on invalid dates, unlike toLocaleString.
  return Number.isNaN(date.getTime())
    ? 'Invalid Date'
    : timeFormat(readPreferences().timezone === 'utc').format(date);
};
export function message(error: unknown): string {
  if (
    error instanceof ApiError &&
    (error.code === 'busy' || error.code === 'rate_limited')
  )
    return `${error.message} Wait a moment before trying again.`;
  return error instanceof Error
    ? error.message
    : 'Something went wrong. Please retry.';
}
export function loginMessage(error: unknown): string {
  if (error instanceof ApiError && error.status === 401)
    return 'Invalid credentials or temporarily locked account. After repeated failed sign-ins, wait 15 minutes before trying again.';
  return message(error);
}
export function isCidr(value: string): boolean {
  const [address, prefix, extra] = value.trim().split('/');
  if (
    !address ||
    prefix === undefined ||
    extra !== undefined ||
    !/^\d+$/.test(prefix)
  )
    return false;
  if (address.includes(':')) {
    if (Number(prefix) > 128 || !/^[\da-f:.]+$/i.test(address)) return false;
    try {
      // The browser's URL parser validates IPv6 compression and group counts.
      return new URL(`http://[${address}]/`).hostname.startsWith('[');
    } catch {
      return false;
    }
  }
  const octets = address.split('.');
  return (
    Number(prefix) <= 32 &&
    octets.length === 4 &&
    octets.every((part) => /^\d{1,3}$/.test(part) && Number(part) <= 255)
  );
}
export function download(name: string, content: string, type: string) {
  const url = URL.createObjectURL(new Blob([content], { type }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
export const cellText = (value: unknown) =>
  value === null || value === undefined
    ? ''
    : typeof value === 'object'
      ? JSON.stringify(value)
      : String(value);
/** Quotes CSV cells and neutralizes formula prefixes, including leading whitespace. */
export function csvCell(value: unknown): string {
  const text = cellText(value);
  const safe =
    typeof value === 'string' &&
    (/^\s*[=+\-@]/.test(value) || /^[\t\r\n]/.test(value))
      ? `'${text}`
      : text;
  return `"${safe.replaceAll('"', '""')}"`;
}
