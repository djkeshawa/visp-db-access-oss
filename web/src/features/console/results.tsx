import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import * as Dropdown from '@radix-ui/react-dropdown-menu';
import { Copy, Download, LockKeyhole, MoreHorizontal } from 'lucide-react';
import type { QueryResult } from '../../api/types';
import { useMedia } from '../../lib/media';
import { Button, Badge, Modal, Tip } from '../../components/ui';
import { presentCell, sampleWidths } from '../../lib/result-presentation';
import { cellText, download, message } from '../../lib/utils';
import { useToast } from '../../components/ui/toast';
import { usePopoverLayer } from '../../lib/popover-layer';
import {
  fetchedRows,
  resultText,
  type CopyFormat,
  type RowSort,
} from '../../lib/result-view';
type Column = QueryResult['columns'][number];
type VisibleColumn = { column: Column; index: number };
const MIN_WIDTH = 90,
  MAX_WIDTH = 1000,
  ROW_NUMBER_WIDTH = 46;
const clampWidth = (value: number) =>
  Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, value));
const cursorOffsets: Record<string, [number, number]> = {
  ArrowRight: [0, 1],
  ArrowLeft: [0, -1],
  ArrowDown: [1, 0],
  ArrowUp: [-1, 0],
};
/** Header cell; memoized so filtering and cell focus do not rebuild every menu. */
const ColumnHeader = memo(function ColumnHeader({
  column,
  index,
  position,
  width,
  left,
  direction,
  onSort,
  onCopyColumn,
  onHide,
  onTogglePin,
  onResize,
}: {
  column: Column;
  index: number;
  position: number;
  width: number;
  /** Sticky offset when the column is pinned. */
  left: number | undefined;
  direction: 'asc' | 'desc' | null;
  onSort: (index: number, direction: 'asc' | 'desc') => void;
  onCopyColumn: (index: number) => void;
  onHide: (index: number) => void;
  onTogglePin: (index: number) => void;
  onResize: (index: number, width: number) => void;
}) {
  const popoverLayer = usePopoverLayer();
  const pinned = left !== undefined;
  const endResize = useRef<(() => void) | null>(null);
  // A drag must not leak listeners if the header unmounts mid-drag.
  useEffect(() => () => endResize.current?.(), []);
  return (
    <div
      role="columnheader"
      aria-label={column.name}
      aria-colindex={position + 2}
      aria-sort={
        direction ? (direction === 'asc' ? 'ascending' : 'descending') : 'none'
      }
      className={`result-column ${pinned ? 'pinned-column' : ''}`}
      style={pinned ? { left } : undefined}
    >
      <span>
        {column.masked && <LockKeyhole size={12} />} {column.name}
        {direction && (
          <span aria-hidden="true">{direction === 'asc' ? '↑' : '↓'}</span>
        )}
        <Dropdown.Root modal={false}>
          <Dropdown.Trigger asChild>
            <button
              className="column-menu"
              aria-label={`Column options for ${column.name}`}
            >
              <MoreHorizontal size={14} />
            </button>
          </Dropdown.Trigger>
          <Dropdown.Portal container={popoverLayer}>
            <Dropdown.Content className="dropdown">
              <Dropdown.Item onSelect={() => onSort(index, 'asc')}>
                Sort ascending
              </Dropdown.Item>
              <Dropdown.Item onSelect={() => onSort(index, 'desc')}>
                Sort descending
              </Dropdown.Item>
              <Dropdown.Item onSelect={() => onCopyColumn(index)}>
                Copy column
              </Dropdown.Item>
              <Dropdown.Item onSelect={() => onHide(index)}>
                Hide column
              </Dropdown.Item>
              <Dropdown.Item onSelect={() => onTogglePin(index)}>
                {pinned ? 'Unpin' : 'Pin'} column
              </Dropdown.Item>
            </Dropdown.Content>
          </Dropdown.Portal>
        </Dropdown.Root>
      </span>
      <small>
        {column.type_name}
        {column.masked ? ' · masked' : ''}
      </small>
      <div
        className="resize-handle"
        role="separator"
        aria-label={`Resize ${column.name}`}
        aria-orientation="vertical"
        aria-valuenow={width}
        aria-valuemin={MIN_WIDTH}
        aria-valuemax={MAX_WIDTH}
        tabIndex={0}
        onKeyDown={(event) => {
          if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
            event.preventDefault();
            onResize(
              index,
              clampWidth(width + (event.key === 'ArrowRight' ? 20 : -20)),
            );
          }
        }}
        onPointerDown={(event) => {
          endResize.current?.();
          const element = event.currentTarget;
          try {
            element.setPointerCapture(event.pointerId);
          } catch {
            // Capture is best effort; dragging still works while over the handle.
          }
          const start = event.clientX,
            initial = width;
          const move = (event: PointerEvent) =>
            onResize(index, clampWidth(initial + event.clientX - start));
          const end = () => {
            element.removeEventListener('pointermove', move);
            element.removeEventListener('pointerup', end);
            element.removeEventListener('pointercancel', end);
            element.removeEventListener('lostpointercapture', end);
            endResize.current = null;
          };
          element.addEventListener('pointermove', move);
          element.addEventListener('pointerup', end);
          element.addEventListener('pointercancel', end);
          element.addEventListener('lostpointercapture', end);
          endResize.current = end;
        }}
      />
    </div>
  );
});
/** One virtualized row; memoized so cell focus changes touch only two rows. */
const ResultRow = memo(function ResultRow({
  rowIndex,
  start,
  size,
  row,
  lastRow,
  visible,
  lefts,
  template,
  totalWidth,
  focusColumn,
  onCopyRow,
  onCopyCell,
  onInspect,
  onFocusCell,
  onMove,
}: {
  rowIndex: number;
  start: number;
  size: number;
  row: unknown[] | undefined;
  lastRow: number;
  visible: VisibleColumn[];
  lefts: Record<number, number | undefined>;
  template: string;
  totalWidth: number;
  /** Position of the roving-tabindex cell in this row, or -1. */
  focusColumn: number;
  onCopyRow: (row: number) => void;
  onCopyCell: (value: unknown) => void;
  onInspect: (row: number, column: number) => void;
  onFocusCell: (row: number, column: number) => void;
  onMove: (row: number, column: number) => void;
}) {
  return (
    <div
      className="result-row"
      role="row"
      aria-rowindex={rowIndex + 2}
      style={{
        position: 'absolute',
        top: start,
        height: size,
        gridTemplateColumns: template,
        width: totalWidth,
      }}
    >
      <Tip text="Copy row as TSV">
        <button
          role="gridcell"
          className="row-number pinned-column"
          style={{ left: 0 }}
          aria-colindex={1}
          aria-label={`Copy row ${rowIndex + 1}`}
          tabIndex={-1}
          onClick={() => onCopyRow(rowIndex)}
        >
          {rowIndex + 1}
        </button>
      </Tip>
      {visible.map(({ column, index }, position) => {
        const value = row?.[index];
        const cell = presentCell(value, column);
        const left = lefts[index];
        return (
          <div
            role="gridcell"
            key={index}
            aria-colindex={position + 2}
            className={`result-cell cell-${cell.kind} ${value === null ? 'null' : ''} ${left !== undefined ? 'pinned-column' : ''}`}
            style={left !== undefined ? { left } : undefined}
          >
            <button
              className="cell-value"
              data-cell={`${rowIndex}-${position}`}
              tabIndex={focusColumn === position ? 0 : -1}
              title={cell.raw}
              onFocus={() => onFocusCell(rowIndex, position)}
              onClick={() => onInspect(rowIndex, position)}
              onKeyDown={(event) => {
                const offset = cursorOffsets[event.key];
                if (offset) {
                  event.preventDefault();
                  onMove(rowIndex + offset[0], position + offset[1]);
                } else if (event.key === 'Home' || event.key === 'End') {
                  event.preventDefault();
                  onMove(
                    event.ctrlKey
                      ? event.key === 'Home'
                        ? 0
                        : lastRow
                      : rowIndex,
                    event.key === 'Home' ? 0 : visible.length - 1,
                  );
                } else if (
                  (event.ctrlKey || event.metaKey) &&
                  event.key.toLowerCase() === 'c' &&
                  !window.getSelection()?.toString()
                ) {
                  // The per-cell copy button is pointer-only, so mirror it here.
                  onCopyCell(value);
                }
              }}
            >
              <span className={cell.kind === 'json' ? 'json-chip' : undefined}>
                {cell.text}
              </span>
            </button>
            <button
              className="cell-copy"
              tabIndex={-1}
              aria-label={`Copy ${column.name} row ${rowIndex + 1}`}
              onClick={() => onCopyCell(value)}
            >
              <Copy size={12} />
            </button>
          </div>
        );
      })}
    </div>
  );
});
/** Virtualized, keyboard-accessible interaction with the fetched result snapshot. */
export const Results = memo(function Results({
  result,
}: {
  result: QueryResult;
}) {
  const scroll = useRef<HTMLDivElement>(null),
    toast = useToast();
  const popoverLayer = usePopoverLayer();
  const [filter, setFilter] = useState(''),
    [sort, setSort] = useState<RowSort>(null),
    [hidden, setHidden] = useState<number[]>([]),
    [pinned, setPinned] = useState<number[]>([]),
    [widths, setWidths] = useState<Record<number, number>>({}),
    [expanded, setExpanded] = useState<{
      value: unknown;
      name: string;
      row: number;
    } | null>(null),
    [focus, setFocus] = useState({ row: 0, column: 0 });
  const rows = useMemo(
    () => fetchedRows(result.rows, filter, sort),
    [result.rows, filter, sort],
  );
  const rowsRef = useRef(rows);
  useEffect(() => {
    rowsRef.current = rows;
  });
  const visible = useMemo(
    () =>
      result.columns
        .map((column, index) => ({ column, index }))
        .filter(({ index }) => !hidden.includes(index))
        .sort(
          (a, b) =>
            Number(pinned.includes(b.index)) - Number(pinned.includes(a.index)),
        ),
    [result.columns, hidden, pinned],
  );
  const sampled = useMemo(() => sampleWidths(result), [result]);
  const width = (index: number) => widths[index] ?? sampled[index] ?? 104;
  const layout = useMemo(() => {
    const sizes = visible.map(
      ({ index }) => widths[index] ?? sampled[index] ?? 104,
    );
    const lefts: Record<number, number | undefined> = {};
    let total = ROW_NUMBER_WIDTH;
    visible.forEach(({ index }, position) => {
      if (pinned.includes(index)) lefts[index] = total;
      total += sizes[position] ?? 0;
    });
    return {
      template: `${ROW_NUMBER_WIDTH}px ${sizes.map((size) => `${size}px`).join(' ')}`,
      totalWidth: total,
      lefts,
    };
  }, [visible, widths, sampled, pinned]);
  const rowHeight = useMedia('(pointer: coarse), (max-width: 600px)') ? 44 : 40;
  const virtual = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroll.current,
    estimateSize: () => rowHeight,
    overscan: 12,
  });
  useEffect(() => {
    virtual.measure();
  }, [rowHeight, virtual]);
  const copy = useCallback(
    async (text: string) => {
      try {
        await navigator.clipboard.writeText(text);
        toast('Copied to clipboard');
      } catch (error) {
        toast(message(error), 'error');
      }
    },
    [toast],
  );
  // Only exports and copies need every row, so build the snapshot on demand.
  const snapshot = () => ({
    columns: visible.map((c) => c.column),
    rows: rows.map((row) => visible.map((c) => row[c.index])),
  });
  const exportResult = (format: 'csv' | 'json') => {
    download(
      `query-result.${format}`,
      resultText(snapshot(), format),
      format === 'json' ? 'application/json' : 'text/csv;charset=utf-8',
    );
    toast('Exported fetched rows');
  };
  const onCopyRow = useCallback(
    (index: number) => {
      const row = rows[index];
      void copy(
        resultText(
          {
            columns: visible.map((c) => c.column),
            rows: [row ? visible.map((c) => row[c.index]) : []],
          },
          'tsv',
        ),
      );
    },
    [copy, rows, visible],
  );
  const onCopyCell = useCallback(
    (value: unknown) => void copy(cellText(value)),
    [copy],
  );
  const onInspect = useCallback(
    (row: number, column: number) => {
      const col = visible[column];
      if (col)
        setExpanded({
          value: rows[row]?.[col.index],
          name: col.column.name,
          row: row + 1,
        });
    },
    [rows, visible],
  );
  const onFocusCell = useCallback(
    (row: number, column: number) =>
      setFocus((value) =>
        value.row === row && value.column === column ? value : { row, column },
      ),
    [],
  );
  const rowCount = rows.length,
    columnCount = visible.length;
  const onMove = useCallback(
    (row: number, column: number) => {
      const next = {
        row: Math.max(0, Math.min(rowCount - 1, row)),
        column: Math.max(0, Math.min(columnCount - 1, column)),
      };
      setFocus(next);
      virtual.scrollToIndex(next.row, { align: 'auto' });
      requestAnimationFrame(() =>
        requestAnimationFrame(() =>
          scroll.current
            ?.querySelector<HTMLElement>(
              `[data-cell="${next.row}-${next.column}"]`,
            )
            ?.focus(),
        ),
      );
    },
    [rowCount, columnCount, virtual],
  );
  const onSort = useCallback(
    (column: number, direction: 'asc' | 'desc') =>
      setSort({ column, direction }),
    [],
  );
  const onCopyColumn = useCallback(
    (index: number) => {
      const column = result.columns[index];
      if (column)
        void copy(
          resultText(
            {
              columns: [column],
              rows: rowsRef.current.map((row) => [row[index]]),
            },
            'tsv',
          ),
        );
    },
    [copy, result.columns],
  );
  const onHide = useCallback(
    (index: number) => setHidden((values) => [...values, index]),
    [],
  );
  const onTogglePin = useCallback(
    (index: number) =>
      setPinned((values) =>
        values.includes(index)
          ? values.filter((i) => i !== index)
          : [...values, index],
      ),
    [],
  );
  const onResize = useCallback(
    (index: number, size: number) =>
      setWidths((values) => ({ ...values, [index]: size })),
    [],
  );
  const expandedObject =
    expanded?.value !== null && typeof expanded?.value === 'object';
  const expandedText = expandedObject
    ? JSON.stringify(expanded?.value, null, 2)
    : cellText(expanded?.value);
  return (
    <section className="results" aria-label="Result snapshot">
      <div className="section-toolbar">
        <div className="inline wrap">
          <strong>Results</strong>
          <Badge variant="status">Completed</Badge>
        </div>
        <div className="inline wrap">
          <Dropdown.Root modal={false}>
            <Dropdown.Trigger asChild>
              <Button>
                <Download size={14} />
                Export
              </Button>
            </Dropdown.Trigger>
            <Dropdown.Portal container={popoverLayer}>
              <Dropdown.Content className="dropdown">
                <Dropdown.Item onSelect={() => exportResult('csv')}>
                  Download CSV
                </Dropdown.Item>
                <Dropdown.Item onSelect={() => exportResult('json')}>
                  Download JSON
                </Dropdown.Item>
                <Dropdown.Separator />
                {(['csv', 'tsv', 'markdown', 'json'] as CopyFormat[]).map(
                  (format) => (
                    <Dropdown.Item
                      key={format}
                      onSelect={() => void copy(resultText(snapshot(), format))}
                    >
                      Copy as{' '}
                      {format === 'markdown'
                        ? 'Markdown'
                        : format.toUpperCase()}
                    </Dropdown.Item>
                  ),
                )}
              </Dropdown.Content>
            </Dropdown.Portal>
          </Dropdown.Root>
        </div>
      </div>
      {result.affected_rows !== null && (
        <div className="callout success">
          {result.affected_rows.toLocaleString()} row
          {result.affected_rows === 1 ? '' : 's'} affected.
        </div>
      )}
      {result.columns.length > 0 && (
        <>
          <div className="results-controls">
            <input
              aria-label="Filter fetched rows"
              placeholder="Filter fetched rows…"
              value={filter}
              onChange={(event) => {
                setFilter(event.target.value);
                setFocus({ row: 0, column: 0 });
              }}
            />
            <small className="muted">
              Search and sort apply to fetched rows only.
            </small>
            {hidden.length > 0 && (
              <Button onClick={() => setHidden([])}>
                Show all columns ({hidden.length} hidden)
              </Button>
            )}
            {sort && <Button onClick={() => setSort(null)}>Clear sort</Button>}
          </div>
          <div
            ref={scroll}
            className="result-scroll"
            role="grid"
            aria-label="Query results"
            aria-rowcount={rows.length + 1}
            aria-colcount={visible.length + 1}
            tabIndex={rows.length && visible.length ? undefined : 0}
          >
            <div
              className="result-header"
              role="row"
              aria-rowindex={1}
              style={{
                gridTemplateColumns: layout.template,
                width: layout.totalWidth,
              }}
            >
              <div
                role="columnheader"
                className="pinned-column"
                style={{ left: 0 }}
              >
                #
              </div>
              {visible.map(({ column, index }, position) => (
                <ColumnHeader
                  key={index}
                  column={column}
                  index={index}
                  position={position}
                  width={width(index)}
                  left={layout.lefts[index]}
                  direction={sort?.column === index ? sort.direction : null}
                  onSort={onSort}
                  onCopyColumn={onCopyColumn}
                  onHide={onHide}
                  onTogglePin={onTogglePin}
                  onResize={onResize}
                />
              ))}
            </div>
            <div
              role="rowgroup"
              style={{
                height: virtual.getTotalSize(),
                width: layout.totalWidth,
                position: 'relative',
              }}
            >
              {virtual.getVirtualItems().map((item) => (
                <ResultRow
                  key={item.key}
                  rowIndex={item.index}
                  start={item.start}
                  size={item.size}
                  row={rows[item.index]}
                  lastRow={rows.length - 1}
                  visible={visible}
                  lefts={layout.lefts}
                  template={layout.template}
                  totalWidth={layout.totalWidth}
                  focusColumn={
                    focus.row === item.index
                      ? Math.min(focus.column, visible.length - 1)
                      : -1
                  }
                  onCopyRow={onCopyRow}
                  onCopyCell={onCopyCell}
                  onInspect={onInspect}
                  onFocusCell={onFocusCell}
                  onMove={onMove}
                />
              ))}
            </div>
          </div>
          {rows.length === 0 && (
            <div className="result-empty">
              {result.rows.length === 0
                ? 'No rows returned. Check the WHERE condition or try a different table.'
                : 'No fetched rows match this filter.'}
              {filter && (
                <Button onClick={() => setFilter('')}>Clear filter</Button>
              )}
            </div>
          )}
        </>
      )}
      <div className="result-status" role="status">
        <span>
          {result.row_count.toLocaleString()} rows fetched
          {filter ? ` · ${rows.length} matching` : ''} · {result.elapsed_ms} ms
        </span>
        <span>
          Routed to <strong>{result.routed_to}</strong>
        </span>
        {result.columns.some((column) => column.masked) && (
          <span>
            <LockKeyhole size={12} />{' '}
            <span className="status-long">
              Protected columns stay masked in copies and exports.
            </span>
            <span className="status-short">Masked columns stay masked</span>
          </span>
        )}
        {result.truncated && (
          <span className="warning-text">
            <span className="status-long">
              Showing first {result.row_count} rows — limit set by policy.
              Narrow the query to see other rows.
            </span>
            <span className="status-short">
              First {result.row_count.toLocaleString()} rows (policy limit)
            </span>
          </span>
        )}
      </div>
      <Modal
        open={!!expanded}
        onOpenChange={(open) => {
          if (!open) setExpanded(null);
        }}
        drawer
        title={expandedObject ? 'JSON value' : 'Cell value'}
        description={
          expanded
            ? `${expanded.name} · fetched row ${expanded.row}`
            : undefined
        }
      >
        <pre className="json-preview" tabIndex={0}>
          {expanded?.value === null ? 'NULL' : expandedText}
        </pre>
        <Button onClick={() => void copy(expandedText)}>
          <Copy size={14} />
          {expandedObject ? 'Copy JSON' : 'Copy value'}
        </Button>
      </Modal>
    </section>
  );
});
