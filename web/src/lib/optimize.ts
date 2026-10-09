import type { Analysis, Engine, SchemaTree, Suggestion } from '../api/types';

const SIMPLE = /^[a-z_][a-z0-9_]*$/;
const RESERVED = new Set(
  'all and as asc by case check column current_date current_time date default desc distinct else end false from group having in index is join key limit not null offset on or order primary range rank references row rows select table then time timestamp to true union user values when where window'.split(
    ' ',
  ),
);

export function quoteIdent(name: string, engine: Engine): string {
  if (SIMPLE.test(name) && !RESERVED.has(name)) return name;
  return engine === 'mysql'
    ? `\`${name.replaceAll('`', '``')}\``
    : `"${name.replaceAll('"', '""')}"`;
}

function columnsFor(table: string, schema: SchemaTree): string[] | undefined {
  const [schemaName, tableName] = table.includes('.')
    ? table.split('.', 2)
    : [undefined, table];
  const matches = schema.schemas
    .filter((s) => !schemaName || s.name.toLowerCase() === schemaName)
    .flatMap((s) => s.tables.filter((t) => t.name.toLowerCase() === tableName));
  // An unqualified name present in several schemas is ambiguous.
  return matches.length === 1 && matches[0]?.columns.length
    ? matches[0].columns.map((c) => c.name)
    : undefined;
}

/**
 * `SELECT * FROM t` with a single known table becomes an explicit column list.
 * Only the leading star is touched, so the rest of the text is kept verbatim.
 */
export function expandStar(
  sql: string,
  analysis: Analysis,
  schema: SchemaTree | undefined,
  engine: Engine,
): string | undefined {
  const [statement] = analysis.statements;
  if (!schema || analysis.statements.length !== 1 || !statement) return;
  if (statement.kind !== 'select' || statement.tables.length !== 1) return;
  const columns = columnsFor(statement.tables[0] ?? '', schema);
  const match = /^(\s*select\s+)\*(\s+from\s)/i.exec(sql);
  if (!columns || !match) return;
  const prefix = match[1] ?? '';
  const list = columns.map((c) => quoteIdent(c, engine)).join(', ');
  return prefix + list + sql.slice(prefix.length + 1);
}

/** Server suggestions plus fixes only the client can build (it has the schema). */
export function withClientFixes(
  sql: string,
  analysis: Analysis | undefined,
  schema: SchemaTree | undefined,
  engine: Engine,
): Suggestion[] {
  if (!analysis || analysis.verdict === 'deny') return [];
  return (analysis.suggestions ?? []).map((suggestion) => {
    if (suggestion.code !== 'select_star' || suggestion.fix) return suggestion;
    const expanded = expandStar(sql, analysis, schema, engine);
    return expanded
      ? {
          ...suggestion,
          fix: { label: 'List columns', sql: expanded, action: 'replace' },
        }
      : suggestion;
  });
}
