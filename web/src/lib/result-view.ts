import type { QueryResult } from '../api/types';
import { cellText, csvCell } from './utils';
import { resultCsv, resultJson } from './result-export';
const collator = new Intl.Collator(undefined, { numeric: true });
export type RowSort = { column: number; direction: 'asc' | 'desc' } | null;
/** Filters and sorts only the fetched snapshot; never issues SQL or mutates rows. */
export function fetchedRows(
  rows: unknown[][],
  filter: string,
  sort: RowSort,
): unknown[][] {
  const query = filter.toLocaleLowerCase();
  const filtered = rows.filter(
    (row) =>
      !query ||
      row.some((cell) => cellText(cell).toLocaleLowerCase().includes(query)),
  );
  if (!sort) return filtered;
  const direction = sort.direction === 'asc' ? 1 : -1;
  // Text keys are computed once per row, not once per comparison.
  const keyed = filtered.map((row) => {
    const value = row[sort.column];
    return {
      row,
      value,
      text: value == null || typeof value === 'number' ? '' : cellText(value),
    };
  });
  keyed.sort((a, b) => {
    const left = a.value,
      right = b.value;
    if (left == null || right == null)
      return direction * (left == null ? (right == null ? 0 : -1) : 1);
    return (
      direction *
      (typeof left === 'number' && typeof right === 'number'
        ? left - right
        : collator.compare(
            typeof left === 'number' ? String(left) : a.text,
            typeof right === 'number' ? String(right) : b.text,
          ))
    );
  });
  return keyed.map((entry) => entry.row);
}
export type CopyFormat = 'csv' | 'tsv' | 'markdown' | 'json';
/** Exports the visible fetched snapshot, retaining duplicate names and masked values. */
export function resultText(
  result: Pick<QueryResult, 'columns' | 'rows'>,
  format: CopyFormat,
): string {
  if (format === 'csv') return resultCsv(result);
  if (format === 'json') return resultJson(result);
  const lines: unknown[][] = [
    result.columns.map((c) => c.name),
    ...result.rows,
  ];
  if (format === 'tsv')
    return lines.map((row) => row.map(csvCell).join('\t')).join('\r\n');
  const escape = (value: unknown) =>
    cellText(value)
      .replaceAll('\\', '\\\\')
      .replaceAll('|', '\\|')
      .replace(/\r?\n/g, '<br>');
  const markdown = lines.map((row) => `| ${row.map(escape).join(' | ')} |`);
  markdown.splice(1, 0, `| ${result.columns.map(() => '---').join(' | ')} |`);
  return markdown.join('\n');
}
