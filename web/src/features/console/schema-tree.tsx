import { memo, useMemo, useState, type ReactNode } from 'react';
import * as Dropdown from '@radix-ui/react-dropdown-menu';
import {
  Table2,
  KeyRound,
  RefreshCw,
  TextCursorInput,
  Play,
  X,
} from 'lucide-react';
import type { SchemaTree } from '../../api/types';
type Namespace = SchemaTree['schemas'][number];
type Table = Namespace['tables'][number];
import { useResource } from '../../lib/query';
import { Button, ErrorPanel, Skeleton, Empty, Tip } from '../../components/ui';
import { request } from '../../api/client';
import { useQueryClient } from '@tanstack/react-query';
import { useToast } from '../../components/ui/toast';
import { message } from '../../lib/utils';
import { usePopoverLayer } from '../../lib/popover-layer';
export const identifier = (name: string, engine = 'postgres') =>
  engine === 'mysql'
    ? `\`${name.replaceAll('`', '``')}\``
    : `"${name.replaceAll('"', '""')}"`;
const selectTop = (qualifiedName: string) =>
  `SELECT * FROM ${qualifiedName} LIMIT 100;`;
/** One table row; columns mount only while expanded to keep large schemas light. */
const SchemaTable = memo(function SchemaTable({
  namespace,
  table,
  engine,
  menuOpen,
  onMenuTable,
  onInsert,
  onQuery,
}: {
  namespace: string;
  table: Table;
  engine: string;
  menuOpen: boolean;
  onMenuTable: (name: string | null) => void;
  onInsert?: (name: string) => void;
  onQuery?: (sql: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const name = `${identifier(namespace, engine)}.${identifier(table.name, engine)}`;
  return (
    <div
      className="schema-table-group"
      onContextMenu={(event) => {
        if (onInsert || onQuery) {
          event.preventDefault();
          onMenuTable(name);
        }
      }}
    >
      <details
        className="schema-table"
        open={open}
        onToggle={(event) => setOpen(event.currentTarget.open)}
      >
        <summary
          onKeyDown={(event) => {
            if (
              (event.shiftKey && event.key === 'F10') ||
              event.key === 'ContextMenu'
            ) {
              event.preventDefault();
              onMenuTable(name);
            }
          }}
        >
          <Table2 size={14} />
          <span>{table.name}</span>
          <small>{table.row_estimate?.toLocaleString() ?? 'view'}</small>
        </summary>
        {open &&
          table.columns.map((column) => (
            <button
              key={column.name}
              className="schema-column"
              title={`${column.data_type}${column.nullable ? ' · nullable' : ' · not null'}`}
              onClick={() =>
                onInsert
                  ? onInsert(identifier(column.name, engine))
                  : onQuery?.(
                      `SELECT ${identifier(column.name, engine)} FROM ${name} LIMIT 100;`,
                    )
              }
            >
              {column.is_primary_key ? (
                <KeyRound size={12} />
              ) : (
                <span className="column-dot" />
              )}
              <span>{column.name}</span>
              <small>
                {column.data_type}
                {column.nullable ? ' ?' : ''}
              </small>
            </button>
          ))}
      </details>
      <span className="schema-row-actions">
        {onInsert && (
          <Tip text="Insert table name">
            <Button
              variant="ghost"
              className="icon"
              aria-label={`Insert ${table.name}`}
              onClick={() => onInsert(name)}
            >
              <TextCursorInput size={13} />
            </Button>
          </Tip>
        )}
        {onQuery && (
          <Tip text="SELECT top 100">
            <Button
              variant="ghost"
              className="icon"
              aria-label={`Query ${table.name} top 100`}
              onClick={() => onQuery(selectTop(name))}
            >
              <Play size={13} />
            </Button>
          </Tip>
        )}
        {(onInsert || onQuery) && (
          <TableMenu
            table={table.name}
            open={menuOpen}
            onOpenChange={(next) => onMenuTable(next ? name : null)}
          >
            {onInsert && (
              <Dropdown.Item onSelect={() => onInsert(name)}>
                Insert name
              </Dropdown.Item>
            )}
            {onQuery && (
              <Dropdown.Item onSelect={() => onQuery(selectTop(name))}>
                SELECT top 100
              </Dropdown.Item>
            )}
          </TableMenu>
        )}
      </span>
    </div>
  );
});
function TableMenu({
  table,
  open,
  onOpenChange,
  children,
}: {
  table: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  children: ReactNode;
}) {
  const popoverLayer = usePopoverLayer();
  return (
    <Dropdown.Root modal={false} open={open} onOpenChange={onOpenChange}>
      <Dropdown.Trigger asChild>
        <Button
          variant="ghost"
          className="icon schema-menu-trigger"
          aria-label={`Actions for ${table}`}
        >
          ⋯
        </Button>
      </Dropdown.Trigger>
      <Dropdown.Portal container={popoverLayer}>
        <Dropdown.Content className="dropdown">{children}</Dropdown.Content>
      </Dropdown.Portal>
    </Dropdown.Root>
  );
}
export const SchemaExplorer = memo(function SchemaExplorer({
  clusterId,
  engine = 'postgres',
  onInsert,
  onQuery,
  onClose,
}: {
  clusterId: string;
  engine?: string;
  onInsert?: (name: string) => void;
  onQuery?: (sql: string) => void;
  onClose?: () => void;
}) {
  const query = useResource<SchemaTree>(`/clusters/${clusterId}/schema`);
  const [menuTable, setMenuTable] = useState<string | null>(null);
  const [search, setSearch] = useState(''),
    [refreshing, setRefreshing] = useState(false);
  const cache = useQueryClient(),
    toast = useToast();
  // Search text is lowercased once per schema load, not per keystroke per table.
  const haystacks = useMemo(
    () =>
      query.data?.schemas.map((namespace) => ({
        namespace,
        tables: namespace.tables.map((table) => ({
          table,
          text: `${namespace.name} ${table.name} ${table.columns.map((c) => c.name).join(' ')}`.toLowerCase(),
        })),
      })) ?? [],
    [query.data],
  );
  const matches = useMemo(() => {
    const needle = search.toLowerCase();
    return haystacks.map(({ namespace, tables }) => ({
      namespace,
      tables: tables
        .filter(({ text }) => text.includes(needle))
        .map(({ table }) => table),
    }));
  }, [haystacks, search]);
  const noMatches =
    !!search && matches.every(({ tables }) => tables.length === 0);
  return (
    <div className="schema-explorer">
      <div className="section-toolbar">
        <strong>Schema</strong>
        {onClose && (
          <Button
            variant="ghost"
            className="icon"
            aria-label="Close schema panel"
            onClick={onClose}
          >
            <X size={14} />
          </Button>
        )}
        <Button
          variant="ghost"
          aria-label="Refresh schema"
          disabled={refreshing}
          onClick={async () => {
            setRefreshing(true);
            try {
              const data = await request<SchemaTree>(
                `/clusters/${clusterId}/schema?refresh=true`,
              );
              cache.setQueryData(
                ['api', `/clusters/${clusterId}/schema`],
                data,
              );
            } catch (error) {
              toast(message(error), 'error');
            } finally {
              setRefreshing(false);
            }
          }}
        >
          <RefreshCw size={14} />
        </Button>
      </div>
      <input
        aria-label="Search schema"
        placeholder="Find a table or column…"
        value={search}
        onChange={(event) => setSearch(event.target.value)}
      />
      {noMatches && (
        <Empty
          title="No matching tables or columns"
          description="Try a shorter name or clear the search."
          action={<Button onClick={() => setSearch('')}>Clear search</Button>}
        />
      )}
      {query.isPending ? (
        <Skeleton />
      ) : query.error ? (
        <ErrorPanel error={query.error} />
      ) : query.data.schemas.length === 0 ? (
        <Empty
          title="No visible tables"
          description="Check your access or refresh the schema."
        />
      ) : (
        matches.map(({ namespace, tables }) => (
          <details open key={namespace.name}>
            <summary className="schema-name">{namespace.name}</summary>
            {tables.map((table) => (
              <SchemaTable
                key={table.name}
                namespace={namespace.name}
                table={table}
                engine={engine}
                menuOpen={
                  menuTable ===
                  `${identifier(namespace.name, engine)}.${identifier(table.name, engine)}`
                }
                onMenuTable={setMenuTable}
                onInsert={onInsert}
                onQuery={onQuery}
              />
            ))}
          </details>
        ))
      )}
    </div>
  );
});
