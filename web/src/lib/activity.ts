/** Tokenization is presentation only; SQL execution and analysis remain server-owned. */
export function sqlParts(sql: string): { text: string; kind: string }[] {
  return (
    sql.match(
      /--[^\n]*|\/\*[\s\S]*?\*\/|'(?:''|[^'])*'|"(?:""|[^"])*"|`(?:``|[^`])*`|\$([\w]*)\$[\s\S]*?\$\1\$|\b[A-Za-z_][\w$]*\b|[^A-Za-z_'"`$-]+|./g,
    ) ?? []
  ).map((text) => ({
    text,
    kind: /^--|^\/\*/.test(text)
      ? 'comment'
      : /^'|^\$/.test(text)
        ? 'string'
        : /^(select|from|where|join|left|right|inner|outer|on|update|delete|insert|into|values|set|limit|offset|order|by|group|having|with|as|create|alter|drop|returning|and|or|not|null|true|false|begin|commit)$/i.test(
              text,
            )
          ? 'keyword'
          : 'plain',
  }));
}
export function statementSummary(sql: string): string {
  const source = sqlParts(sql)
    .map((part) =>
      ['string', 'comment'].includes(part.kind) ? ' ' : part.text,
    )
    .join('');
  const verb =
    source
      .trim()
      .match(/^([a-z]+)/i)?.[1]
      ?.toUpperCase() ?? 'SQL';
  const identifier =
    '(?:"(?:""|[^"])+"|`(?:``|[^`])+`|[a-z_][\\w$]*)(?:\\s*\\.\\s*(?:"(?:""|[^"])+"|`(?:``|[^`])+`|[a-z_][\\w$]*))*';
  const tables = [
    ...source.matchAll(
      new RegExp(`\\b(?:from|join|update|into|table)\\s+(${identifier})`, 'gi'),
    ),
  ].map((match) => match[1]!.replace(/\s*\.\s*/g, '.'));
  return `${verb}${tables.length ? ` · ${[...new Set(tables)].join(', ')}` : ''}`;
}
const dayFormat = new Intl.DateTimeFormat('en', {
  month: 'short',
  day: 'numeric',
  year: 'numeric',
  timeZone: 'UTC',
});
/** UTC calendar grouping matches inDateRange rather than the browser's timezone. */
export function dayLabel(value: string, now = new Date()): string {
  const day = value.slice(0, 10),
    today = now.toISOString().slice(0, 10);
  const yesterday = new Date(now);
  yesterday.setUTCDate(yesterday.getUTCDate() - 1);
  return day === today
    ? 'Today'
    : day === yesterday.toISOString().slice(0, 10)
      ? 'Yesterday'
      : dayFormat.format(new Date(`${day}T12:00:00Z`));
}
export function datePreset(
  days: number,
  now = new Date(),
): { from: string; to: string } {
  const start = new Date(now);
  start.setUTCDate(start.getUTCDate() - days + 1);
  return {
    from: start.toISOString().slice(0, 10),
    to: now.toISOString().slice(0, 10),
  };
}
