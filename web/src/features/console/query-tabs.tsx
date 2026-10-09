import * as Dropdown from '@radix-ui/react-dropdown-menu';
import { MoreHorizontal, Plus, X } from 'lucide-react';
import { Button, Tip } from '../../components/ui';
import type { QueryTab } from '../../lib/workspace';
import { usePopoverLayer } from '../../lib/popover-layer';
/** The tablist contains only tabs; operations remain separate keyboard controls. */
export function QueryTabs({
  tabs,
  active,
  busy,
  onSelect,
  onNew,
  onClose,
  onRename,
  onMove,
  onCloseOthers,
}: {
  tabs: QueryTab[];
  active: string;
  busy: boolean;
  onSelect: (id: string) => void;
  onNew: () => void;
  onClose: () => void;
  onRename: () => void;
  onMove: (direction: number) => void;
  onCloseOthers: () => void;
}) {
  const popoverLayer = usePopoverLayer();
  const index = tabs.findIndex((tab) => tab.id === active),
    current = tabs[index];
  return (
    <div className="query-tabbar">
      <div className="query-tabs" role="tablist" aria-label="Queries">
        {tabs.map((tab, i) => (
          <button
            key={tab.id}
            id={`query-tab-${tab.id}`}
            role="tab"
            className={tab.id === active ? 'active' : ''}
            tabIndex={tab.id === active ? 0 : -1}
            aria-selected={tab.id === active}
            aria-controls="active-query-panel"
            disabled={busy}
            draggable={!busy}
            onDragStart={(event) =>
              event.dataTransfer.setData('text/plain', String(i))
            }
            onDragOver={(event) => event.preventDefault()}
            onDrop={(event) => {
              event.preventDefault();
              const from = Number(event.dataTransfer.getData('text/plain'));
              window.dispatchEvent(
                new CustomEvent('vda:reorder-tab', { detail: { from, to: i } }),
              );
            }}
            onClick={() => onSelect(tab.id)}
            onKeyDown={(event) => {
              if (
                !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)
              )
                return;
              event.preventDefault();
              const next =
                event.key === 'Home'
                  ? 0
                  : event.key === 'End'
                    ? tabs.length - 1
                    : (index +
                        (event.key === 'ArrowRight' ? 1 : -1) +
                        tabs.length) %
                      tabs.length;
              const tab = tabs[next];
              if (tab) {
                onSelect(tab.id);
                document.getElementById(`query-tab-${tab.id}`)?.focus();
              }
            }}
          >
            {tab.name}
            {tab.sql !== tab.savedSql && (
              <span
                className="draft-dot"
                title="Changed since last run or favorite"
                aria-label="Unsaved changes"
              >
                ●
              </span>
            )}
          </button>
        ))}
      </div>
      <Dropdown.Root modal={false}>
        <Dropdown.Trigger asChild>
          <Button
            variant="ghost"
            aria-label={`Manage ${current?.name ?? 'query'}`}
            disabled={busy}
          >
            <MoreHorizontal size={16} />
          </Button>
        </Dropdown.Trigger>
        <Dropdown.Portal container={popoverLayer}>
          <Dropdown.Content className="dropdown" align="end">
            <Dropdown.Item onSelect={onRename}>Rename tab</Dropdown.Item>
            <Dropdown.Item disabled={index === 0} onSelect={() => onMove(-1)}>
              Move left
            </Dropdown.Item>
            <Dropdown.Item
              disabled={index === tabs.length - 1}
              onSelect={() => onMove(1)}
            >
              Move right
            </Dropdown.Item>
            <Dropdown.Item disabled={tabs.length < 2} onSelect={onCloseOthers}>
              Close other tabs
            </Dropdown.Item>
            <Dropdown.Item onSelect={onClose}>Close tab</Dropdown.Item>
          </Dropdown.Content>
        </Dropdown.Portal>
      </Dropdown.Root>
      <Tip text="New query tab · ⌘ / Ctrl Alt T">
        <Button
          variant="ghost"
          aria-label="New query tab"
          disabled={busy}
          onClick={onNew}
        >
          <Plus size={16} />
        </Button>
      </Tip>
      <Button
        variant="ghost"
        aria-label={`Close ${current?.name ?? 'query'}`}
        disabled={busy || tabs.length < 2}
        onClick={onClose}
      >
        <X size={14} />
      </Button>
    </div>
  );
}
