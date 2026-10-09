import { describe, it, expect } from 'vitest';
import { expandStar, quoteIdent, withClientFixes } from '../src/lib/optimize';
import { analyzeDemo } from '../src/api/mock-guard';
import { defaultPolicy } from '../src/api/fixtures';
import type { Analysis, SchemaTree } from '../src/api/types';

const schema: SchemaTree = {
  schemas: ['public', 'audit'].map((name) => ({
    name,
    tables: [
      {
        name: name === 'public' ? 'users' : 'events',
        kind: 'table' as const,
        row_estimate: null,
        columns: ['id', 'email', 'order', 'Created At'].map((column) => ({
          name: column,
          data_type: 'text',
          nullable: false,
          is_primary_key: column === 'id',
        })),
      },
      {
        name: 'shared',
        kind: 'table' as const,
        row_estimate: null,
        columns: [
          {
            name: 'id',
            data_type: 'int',
            nullable: false,
            is_primary_key: true,
          },
        ],
      },
    ],
  })),
} as SchemaTree;

const analysis = (table: string, sql = 'SELECT * FROM t'): Analysis => ({
  verdict: 'allow',
  risk: 'medium',
  rewritten_sql: sql,
  issues: [],
  suggestions: [{ code: 'select_star', message: 'List columns' }],
  statements: [
    {
      kind: 'select',
      sql,
      tables: [table],
      functions: [],
      risk: 'medium',
      issues: [],
      has_where: false,
      has_limit: false,
    },
  ],
});

describe('query optimization fixes', () => {
  it('quotes only identifiers that need it', () => {
    expect(quoteIdent('email', 'postgres')).toBe('email');
    expect(quoteIdent('order', 'postgres')).toBe('"order"');
    expect(quoteIdent('Created At', 'mysql')).toBe('`Created At`');
    expect(quoteIdent('a"b', 'postgres')).toBe('"a""b"');
  });
  it('expands SELECT * for a single known table and keeps the rest verbatim', () => {
    const sql = 'select *\nFROM public.users -- recent\nWHERE id > 5';
    expect(
      expandStar(sql, analysis('public.users', sql), schema, 'postgres'),
    ).toBe(
      'select id, email, "order", "Created At"\nFROM public.users -- recent\nWHERE id > 5',
    );
    expect(
      expandStar('SELECT * FROM users', analysis('users'), schema, 'mysql'),
    ).toBe('SELECT id, email, `order`, `Created At` FROM users');
  });
  it('declines when the table is unknown or ambiguous, or the star is not leading', () => {
    expect(
      expandStar('SELECT * FROM x', analysis('x'), schema, 'postgres'),
    ).toBe(undefined);
    expect(
      expandStar(
        'SELECT * FROM shared',
        analysis('shared'),
        schema,
        'postgres',
      ),
    ).toBe(undefined);
    expect(
      expandStar(
        'SELECT id, * FROM users',
        analysis('users'),
        schema,
        'postgres',
      ),
    ).toBe(undefined);
    expect(
      expandStar(
        'SELECT * FROM users',
        analysis('users'),
        undefined,
        'postgres',
      ),
    ).toBe(undefined);
  });
  it('attaches the client fix and hides suggestions for denied SQL', () => {
    const [hint] = withClientFixes(
      'SELECT * FROM users',
      analysis('users'),
      schema,
      'postgres',
    );
    expect(hint?.fix).toMatchObject({
      action: 'replace',
      label: 'List columns',
    });
    expect(
      withClientFixes(
        'SELECT * FROM users',
        { ...analysis('users'), verdict: 'deny' },
        schema,
        'postgres',
      ),
    ).toEqual([]);
    expect(withClientFixes('', undefined, schema, 'postgres')).toEqual([]);
  });
  it('demo guard suggests a sample limit only for unfiltered reads', () => {
    const policy = defaultPolicy('staging');
    const hints = analyzeDemo('SELECT * FROM users;', policy).suggestions ?? [];
    expect(hints.map((h) => h.code)).toEqual(['select_star', 'add_limit']);
    expect(hints[1]?.fix?.sql).toBe('SELECT * FROM users\nLIMIT 100');
    expect(
      analyzeDemo('SELECT count(*) FROM users', policy).suggestions,
    ).toEqual([]);
    expect(
      analyzeDemo(
        "SELECT id FROM users WHERE email LIKE '%@x.io'",
        policy,
      ).suggestions?.map((h) => h.code),
    ).toEqual(['leading_wildcard']);
  });
});
