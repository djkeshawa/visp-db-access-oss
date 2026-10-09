import { describe, expect, it } from 'vitest';
import { fuzzyScore } from '../src/lib/command';
import { fetchedRows, resultText } from '../src/lib/result-view';
import {
  loadDrafts,
  moveTab,
  closeTab,
  setupSteps,
} from '../src/lib/workspace';
const columns = [
  { name: 'name', type_name: 'text', masked: false },
  { name: 'n', type_name: 'int', masked: false },
];
describe('command search', () => {
  it('matches ordered fuzzy letters and favors direct matches', () => {
    expect(fuzzyScore('clusters', 'clst')).toBeGreaterThan(0);
    expect(fuzzyScore('clusters', 'clust')).toBeGreaterThan(
      fuzzyScore('clusters', 'clst'),
    );
    expect(fuzzyScore('clusters', 'z')).toBe(0);
  });
});
describe('fetched results', () => {
  it('sorts numbers numerically, searches JSON and preserves the input', () => {
    const rows = [
      ['x', 10],
      ['y', 2],
      ['z', null],
    ];
    expect(fetchedRows(rows, '', { column: 1, direction: 'asc' })).toEqual([
      ['z', null],
      ['y', 2],
      ['x', 10],
    ]);
    expect(rows[0]).toEqual(['x', 10]);
    expect(fetchedRows([[{ a: 'needle' }, 1]], 'needle', null)).toHaveLength(1);
  });
  it('escapes Markdown and spreadsheet formulas, preserving duplicate column names in JSON', () => {
    expect(resultText({ columns, rows: [['=SUM(A1)', 2]] }, 'tsv')).toContain(
      "'=SUM(A1)",
    );
    expect(
      resultText({ columns, rows: [['a|b\nc', 2]] }, 'markdown'),
    ).toContain('a\\|b<br>c');
    expect(
      resultText(
        { columns: [columns[0]!, columns[0]!], rows: [[1, 2]] },
        'json',
      ),
    ).toContain('2');
  });
});
describe('browser drafts', () => {
  it('rejects corrupt storage and migrates existing tabs', () => {
    expect(loadDrafts('bad')).toEqual([]);
    expect(
      loadDrafts('[{"id":"a","name":"One","sql":"SELECT 1"}]')[0]?.savedSql,
    ).toBe('SELECT 1');
  });
  it('reorders tabs and keeps the selected identity when closing another tab', () => {
    const tabs = loadDrafts(
      '[{"id":"a","name":"A","sql":"a"},{"id":"b","name":"B","sql":"b"},{"id":"c","name":"C","sql":"c"}]',
    );
    expect(moveTab(tabs, 0, 2).map((t) => t.id)).toEqual(['b', 'c', 'a']);
    expect(closeTab(tabs, 0, 'b').active).toBe('b');
    expect(closeTab(tabs, 1, 'b').active).toBe('a');
  });
  it('completes setup only from observed API state', () => {
    expect(
      setupSteps({ clusters: 0, policiesReviewed: 0, users: 1, queries: 0 }),
    ).toEqual([false, false, false, false]);
    expect(
      setupSteps({ clusters: 1, policiesReviewed: 1, users: 2, queries: 1 }),
    ).toEqual([true, true, true, true]);
  });
});

import { inDateRange } from '../src/components/ui/date-range';
import { expiryLabel, sqlDiff } from '../src/lib/sql-diff';
it('filters inclusive UTC calendar dates without accepting unsupported API filters', () => {
  expect(inDateRange('2026-10-02T23:59:59Z', '2026-10-02', '2026-10-02')).toBe(
    true,
  );
  expect(inDateRange('2026-10-01T23:59:59Z', '2026-10-02', '')).toBe(false);
});
it('shows live expiry and lossless SQL changes', () => {
  expect(
    expiryLabel('2026-10-02T01:15:00Z', Date.parse('2026-10-02T00:00:00Z')),
  ).toBe('Expires in 1h 15m');
  expect(
    expiryLabel('2026-10-01T00:00:00Z', Date.parse('2026-10-02T00:00:00Z')),
  ).toBe('Expired');
  const lines = sqlDiff(
    'SELECT id\nFROM users',
    'SELECT id\nFROM users\nLIMIT 1001',
  );
  expect(
    lines
      .filter((line) => line.kind !== 'added')
      .map((line) => line.text)
      .join('\n'),
  ).toBe('SELECT id\nFROM users');
  expect(
    lines
      .filter((line) => line.kind !== 'removed')
      .map((line) => line.text)
      .join('\n'),
  ).toBe('SELECT id\nFROM users\nLIMIT 1001');
});
describe('sort and time helpers', () => {
  it('sorts text naturally with nulls first and keeps mixed numbers stable', () => {
    const rows = [['b10'], ['b2'], [null], ['a']];
    expect(
      fetchedRows(rows, '', { column: 0, direction: 'asc' }).map((r) => r[0]),
    ).toEqual([null, 'a', 'b2', 'b10']);
    expect(
      fetchedRows(rows, '', { column: 0, direction: 'desc' }).map((r) => r[0]),
    ).toEqual(['b10', 'b2', 'a', null]);
  });
  it('formats timestamps and tolerates invalid dates', async () => {
    const { formatTime } = await import('../src/lib/utils');
    expect(formatTime(null)).toBe('—');
    expect(formatTime('not a date')).toBe('Invalid Date');
    expect(formatTime('2026-10-05T10:00:00Z')).toContain('Oct');
  });
});
