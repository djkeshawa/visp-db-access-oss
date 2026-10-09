import type {
  Analysis,
  Issue,
  Policy,
  StatementKind,
  Suggestion,
} from './types';
/** Deliberately conservative demo classifier; the backend AST guard is authoritative. */
export function analyzeDemo(sql: string, policy: Policy): Analysis {
  const clean = sql
    .replace(/'(?:''|\\.|[^'\\])*'|--[^\n]*|\/\*[\s\S]*?\*\//g, (token) =>
      token.startsWith("'") ? token : ' ',
    )
    .trim();
  const code = clean.replace(
    /'(?:''|\\.|[^'\\])*'|"(?:""|[^"])*"|`(?:``|[^`])*`/g,
    (token) => ' '.repeat(token.length),
  );
  const leading = code.split(/\s+/)[0]?.toLowerCase() ?? 'other';
  const kinds: StatementKind[] = [
    'select',
    'explain',
    'show',
    'insert',
    'update',
    'delete',
    'merge',
  ];
  const kind: StatementKind = kinds.includes(leading as StatementKind)
    ? (leading as StatementKind)
    : /^(create|alter|drop|truncate)/i.test(code)
      ? 'ddl'
      : leading === 'with'
        ? 'select'
        : 'other';
  const write =
    /\b(insert|update|delete|merge|create|alter|drop|truncate)\b/i.test(code);
  const has_where = /\bwhere\b/i.test(code),
    has_limit = /\blimit\s+\d+/i.test(code);
  const limit = /\blimit\s+(\d+)/i.exec(code);
  const issues: Issue[] = [];
  const tables = Array.from(
    clean.matchAll(/\b(?:from|join|update|into)\s+([\w."`]+)/gi),
    (match) => match[1] ?? '',
  );
  if (!clean)
    issues.push({
      severity: 'block',
      code: 'empty_sql',
      message: 'Enter a SQL statement to analyze.',
    });
  if (/\b(pg_sleep|sleep|benchmark)\s*\(/i.test(code))
    issues.push({
      severity: 'block',
      code: 'blocked_function',
      message: 'Long-running sleep functions are blocked.',
    });
  if (/\b(update|delete)\b/i.test(code) && !has_where)
    issues.push({
      severity: 'block',
      code: 'missing_where',
      message: 'UPDATE and DELETE require a WHERE clause.',
    });
  if (
    code.replace(/;\s*$/, '').includes(';') ||
    (/^with\b/i.test(code) && write)
  )
    issues.push({
      severity: 'block',
      code: 'complex_demo_sql',
      message:
        'The demo blocks multi-statement and writable CTE queries. The server uses a full SQL parser.',
    });
  if (
    policy.blocked_tables.some((pattern) =>
      tables.some((table) =>
        table.replaceAll('"', '').startsWith(pattern.replace('*', '')),
      ),
    )
  )
    issues.push({
      severity: 'block',
      code: 'blocked_table',
      message: 'A referenced table is blocked by policy.',
    });
  if (kind === 'other')
    issues.push({
      severity: 'block',
      code: 'unsupported_demo_sql',
      message: 'This statement is not supported by the demo classifier.',
    });
  if (kind === 'ddl' && !policy.allow_ddl)
    issues.push({
      severity: 'block',
      code: 'ddl_disabled',
      message: 'DDL is disabled by this policy.',
    });
  if (write)
    issues.push({
      severity: 'warning',
      code: 'write_approval',
      message: 'A second person must review this write before execution.',
    });
  let rewritten = clean.replace(/;\s*$/, '');
  if (!write && kind === 'select') {
    if (limit) {
      rewritten =
        rewritten.slice(0, limit.index) +
        `LIMIT ${Math.min(Number(limit[1]), policy.max_rows + 1)}` +
        rewritten.slice(limit.index + limit[0].length);
    } else {
      rewritten += `\nLIMIT ${policy.max_rows + 1}`;
      issues.push({
        severity: 'info',
        code: 'limit_added',
        message: `Policy caps results at ${policy.max_rows.toLocaleString()} rows.`,
      });
    }
  }
  const verdict = issues.some((issue) => issue.severity === 'block')
    ? 'deny'
    : write
      ? 'requires_approval'
      : 'allow';
  const risk = verdict === 'deny' ? 'critical' : write ? 'high' : 'low';
  const suggestions: Suggestion[] = [];
  if (verdict !== 'deny') {
    if (/^select\s+\*/i.test(code))
      suggestions.push({
        code: 'select_star',
        message:
          'SELECT * reads every column, including wide ones you may not need. List only the columns you use.',
      });
    if (/\blike\s+'%/i.test(clean))
      suggestions.push({
        code: 'leading_wildcard',
        message:
          'A LIKE pattern that starts with % cannot use a normal index and scans every row.',
      });
    if (
      kind === 'select' &&
      !has_where &&
      !has_limit &&
      !/\b(count|sum|avg|min|max)\s*\(/i.test(code) &&
      policy.max_rows > 100
    )
      suggestions.push({
        code: 'add_limit',
        message: `This reads the whole table without a filter. The gateway stops at ${policy.max_rows} rows, but a small LIMIT returns a sample faster.`,
        fix: {
          label: 'Add LIMIT 100',
          sql: `${clean.replace(/;\s*$/, '')}\nLIMIT 100`,
          action: 'replace',
        },
      });
  }
  return {
    suggestions,
    verdict,
    risk,
    rewritten_sql: verdict === 'deny' ? null : rewritten + ';',
    issues,
    statements: [
      { kind, sql, tables, functions: [], risk, issues, has_where, has_limit },
    ],
  };
}
