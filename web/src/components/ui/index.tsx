import * as Dialog from '@radix-ui/react-dialog';
import * as Dropdown from '@radix-ui/react-dropdown-menu';
import { usePopoverLayer } from '../../lib/popover-layer';
import * as Select from '@radix-ui/react-select';
import * as Tooltip from '@radix-ui/react-tooltip';
import * as Tabs from '@radix-ui/react-tabs';
import {
  ChevronDown,
  Check,
  X,
  AlertTriangle,
  Inbox,
  MoreHorizontal,
} from 'lucide-react';
import {
  Component,
  useId,
  isValidElement,
  cloneElement,
  type ReactNode,
  type CSSProperties,
  type ButtonHTMLAttributes,
} from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { message } from '../../lib/utils';
/** Shared actions; icon-only actions use the same ghost and tooltip treatment. */
export function Button({
  variant = 'secondary',
  className = '',
  size = 'default',
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: 'primary' | 'secondary' | 'danger' | 'ghost';
  size?: 'default' | 'small';
}) {
  const icon = !!props['aria-label'] && isValidElement(props.children);
  const button = (
    <button
      type="button"
      {...props}
      className={`button ${icon ? 'ghost icon-button' : variant} ${size} ${className}`}
    />
  );
  return icon ? <Tip text={String(props['aria-label'])}>{button}</Tip> : button;
}
/** Boolean control with native keyboard behavior and explicit accessible state. */
export function Switch({
  checked,
  onChange,
  label,
  disabled = false,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      className="switch"
      onClick={() => onChange(!checked)}
    >
      <span className="switch-thumb" />
    </button>
  );
}
export function SwitchRow({
  label,
  description,
  ...props
}: {
  label: string;
  description?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="switch-row">
      <div>
        {label}
        {description && <small>{description}</small>}
      </div>
      <Switch label={label} {...props} />
    </div>
  );
}
/** View/filter selector; arrow keys move selection and the ink-free surface slides. */
export function SegmentedControl({
  value,
  onChange,
  items,
  label,
}: {
  value: string;
  onChange: (value: string) => void;
  items: { value: string; label: ReactNode; accessibleLabel?: string }[];
  label: string;
}) {
  const selected = Math.max(
    0,
    items.findIndex((item) => item.value === value),
  );
  return (
    <div
      className="segmented"
      role="group"
      aria-label={label}
      style={
        {
          '--segment-count': items.length,
          '--segment-index': selected,
        } as CSSProperties
      }
    >
      {items.map((item, index) => (
        <button
          type="button"
          key={item.value}
          aria-label={item.accessibleLabel}
          aria-pressed={value === item.value}
          tabIndex={index === selected ? 0 : -1}
          onClick={() => onChange(item.value)}
          onKeyDown={(event) => {
            if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key))
              return;
            event.preventDefault();
            const next =
              event.key === 'Home'
                ? 0
                : event.key === 'End'
                  ? items.length - 1
                  : (index +
                      (event.key === 'ArrowRight' ? 1 : -1) +
                      items.length) %
                    items.length;
            onChange(items[next]!.value);
            (
              event.currentTarget.parentElement?.children[next] as HTMLElement
            )?.focus();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}
/** Row commands share a quiet disclosure; destructive actions confirm in their caller. */
export function RowMenu({
  label,
  items,
}: {
  label: string;
  items: { label: string; onSelect: () => void; disabled?: boolean }[];
}) {
  const layer = usePopoverLayer();
  return (
    <Dropdown.Root modal={false}>
      <Dropdown.Trigger asChild>
        <Button variant="ghost" aria-label={label}>
          <MoreHorizontal size={16} />
        </Button>
      </Dropdown.Trigger>
      <Dropdown.Portal container={layer}>
        <Dropdown.Content className="dropdown" align="end">
          {items.map((item) => (
            <Dropdown.Item
              key={item.label}
              disabled={item.disabled}
              onSelect={item.onSelect}
            >
              {item.label}
            </Dropdown.Item>
          ))}
        </Dropdown.Content>
      </Dropdown.Portal>
    </Dropdown.Root>
  );
}
export function Modal({
  open,
  onOpenChange,
  title,
  description,
  children,
  wide = false,
  drawer = false,
  className = '',
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  children: ReactNode;
  wide?: boolean;
  drawer?: boolean;
  className?: string;
}) {
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="overlay" />
        <Dialog.Content
          className={`dialog ${wide ? 'wide' : ''} ${drawer ? 'drawer' : ''} ${className}`}
          onOpenAutoFocus={(event) => {
            // Focus the dialog itself: auto-focusing the close button opens its
            // tooltip, which then swallows the first Escape.
            event.preventDefault();
            (event.currentTarget as HTMLElement).focus();
          }}
        >
          <div className="dialog-heading">
            <div>
              <Dialog.Title>{title}</Dialog.Title>
              <Dialog.Description className={description ? 'muted' : 'sr-only'}>
                {description ?? title}
              </Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <Button variant="ghost" aria-label="Close dialog">
                <X size={18} />
              </Button>
            </Dialog.Close>
          </div>
          {children}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
export function Confirm({
  open,
  onOpenChange,
  title,
  description,
  onConfirm,
  busy = false,
  busyLabel = 'Working…',
  children,
  danger = false,
}: {
  open: boolean;
  onOpenChange: (value: boolean) => void;
  title: string;
  description: string;
  onConfirm: () => void;
  busy?: boolean;
  /** Shown on the confirm button while `busy`. */
  busyLabel?: string;
  children?: ReactNode;
  danger?: boolean;
}) {
  return (
    <Modal
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      description={description}
    >
      {children}
      <div className="dialog-actions">
        <Button onClick={() => onOpenChange(false)}>Cancel</Button>
        <Button
          disabled={busy}
          variant={danger ? 'danger' : 'primary'}
          onClick={onConfirm}
        >
          {busy ? busyLabel : 'Confirm'}
        </Button>
      </div>
    </Modal>
  );
}
export function Picker({
  value,
  onChange,
  options,
  label,
  placeholder = 'Choose…',
  disabled = false,
}: {
  value: string;
  onChange: (value: string) => void;
  options: { value: string; label: string }[];
  label: string;
  placeholder?: string;
  disabled?: boolean;
}) {
  return (
    <Select.Root value={value} onValueChange={onChange} disabled={disabled}>
      <Select.Trigger className="select" aria-label={label}>
        <Select.Value placeholder={placeholder} />
        <Select.Icon>
          <ChevronDown size={14} />
        </Select.Icon>
      </Select.Trigger>
      <Select.Portal>
        <Select.Content className="select-content" position="popper">
          <Select.Viewport>
            {options.map((option) => (
              <Select.Item
                className="select-item"
                key={option.value}
                value={option.value}
              >
                <Select.ItemText>{option.label}</Select.ItemText>
                <Select.ItemIndicator>
                  <Check size={14} />
                </Select.ItemIndicator>
              </Select.Item>
            ))}
          </Select.Viewport>
        </Select.Content>
      </Select.Portal>
    </Select.Root>
  );
}
export function Tip({ text, children }: { text: string; children: ReactNode }) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger asChild>{children}</Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content className="tooltip" sideOffset={5}>
          {text}
          <Tooltip.Arrow />
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
export function TabBar({
  children,
  value,
  onChange,
  items,
  className,
}: {
  value: string;
  onChange: (value: string) => void;
  items: { value: string; label: ReactNode }[];
  children: ReactNode;
  className?: string;
}) {
  return (
    <Tabs.Root className={className} value={value} onValueChange={onChange}>
      <Tabs.List
        className="tabs"
        aria-label="Views"
        style={
          {
            '--segment-count': items.length,
            '--segment-index': Math.max(
              0,
              items.findIndex((item) => item.value === value),
            ),
          } as CSSProperties
        }
      >
        {items.map((item) => (
          <Tabs.Trigger key={item.value} value={item.value} className="tab">
            {item.label}
          </Tabs.Trigger>
        ))}
      </Tabs.List>
      <Tabs.Content value={value} className="tab-content">
        {children}
      </Tabs.Content>
    </Tabs.Root>
  );
}
export function Badge({
  children,
  tone = 'neutral',
  className = '',
  variant = 'neutral',
}: {
  children: ReactNode;
  variant?: 'neutral' | 'env' | 'verdict' | 'status';
  tone?: string;
  className?: string;
}) {
  const label =
    typeof children === 'string'
      ? children.charAt(0).toUpperCase() + children.slice(1)
      : children;
  return (
    <span className={`badge ${tone} ${className}`} data-variant={variant}>
      {label}
    </span>
  );
}
export function EnvBadge({ environment }: { environment: string }) {
  return (
    <Badge variant="env" className="env-badge" tone={environment}>
      {environment.charAt(0).toUpperCase() + environment.slice(1)}
    </Badge>
  );
}
export function HealthBadge({
  status,
  latency,
}: {
  status: string;
  latency?: number | null;
}) {
  return (
    <span className={`health ${status}`}>
      <i />
      {status}
      {latency != null && (
        <span className="muted">{Math.round(latency)} ms</span>
      )}
    </span>
  );
}
/** Neutral engine monograms; raw discovery engine names retain MariaDB identity. */
export function EngineIcon({ engine }: { engine: string }) {
  const label = engine.includes('mariadb')
    ? 'MariaDB'
    : engine.includes('postgres')
      ? 'PostgreSQL'
      : 'MySQL';
  return (
    <span className="engine-icon" title={label} role="img" aria-label={label}>
      {label === 'MariaDB' ? 'MA' : label === 'PostgreSQL' ? 'PG' : 'MY'}
    </span>
  );
}
export function Empty({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      <Inbox size={16} strokeWidth={1.5} aria-hidden="true" />
      <h2>{title}</h2>
      <p className="sr-only">{description}</p>
      {action}
    </div>
  );
}
export function Skeleton() {
  return (
    <div
      role="status"
      aria-label="Loading"
      aria-busy="true"
      className="skeleton-group"
    >
      <div className="skeleton heading" />
      {[1, 2, 3, 4].map((i) => (
        <div className="skeleton" key={i} />
      ))}
    </div>
  );
}
export function ErrorPanel({
  error,
  retry,
}: {
  error: unknown;
  retry?: () => void;
}) {
  const cache = useQueryClient();
  return (
    <div className="callout danger" role="alert">
      <AlertTriangle size={18} />
      <div>
        <strong>Unable to complete this request</strong>
        <p>{message(error)}</p>
        <Button
          onClick={
            retry ?? (() => void cache.invalidateQueries({ queryKey: ['api'] }))
          }
        >
          Try again
        </Button>
      </div>
    </div>
  );
}
export class RouteBoundary extends Component<
  { children: ReactNode },
  { error: unknown }
> {
  state = { error: null as unknown };
  static getDerivedStateFromError(error: unknown) {
    return { error };
  }
  render() {
    return this.state.error ? (
      <ErrorPanel
        error={this.state.error}
        retry={() => this.setState({ error: null })}
      />
    ) : (
      this.props.children
    );
  }
}
export function Field({
  label,
  children,
  hint,
  group = false,
}: {
  label: string;
  children: ReactNode;
  hint?: string;
  /** Set for button groups (segmented controls): a <label> would name and
   * click-activate the first button instead of labelling the group. */
  group?: boolean;
}) {
  const id = useId();
  const control =
    hint && isValidElement<{ 'aria-describedby'?: string }>(children)
      ? cloneElement(children, {
          'aria-describedby': [children.props['aria-describedby'], id]
            .filter(Boolean)
            .join(' '),
        })
      : children;
  const Wrapper = group ? 'div' : 'label';
  return (
    <div className="field">
      <Wrapper className={group ? 'field-group' : undefined}>
        <span aria-hidden={group || undefined}>{label}</span>
        {control}
      </Wrapper>
      {hint && <small id={id}>{hint}</small>}
    </div>
  );
}
export function ChipInput({
  value,
  onChange,
  label,
  help,
  validate,
}: {
  value: string[];
  onChange: (value: string[]) => void;
  label: string;
  help?: string;
  validate?: (value: string) => boolean;
}) {
  return (
    <Field label={label} hint={help}>
      <div className="chips">
        {value.map((chip, i) => (
          <span
            className={`chip ${validate && !validate(chip) ? 'invalid' : ''}`}
            key={`${chip}-${i}`}
          >
            {chip}
            <button
              type="button"
              aria-label={`Remove ${chip}`}
              onClick={() => onChange(value.filter((_, index) => i !== index))}
            >
              <X size={12} />
            </button>
          </span>
        ))}
        <input
          aria-label={`Add ${label}`}
          placeholder="Type and press Enter"
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ',') {
              event.preventDefault();
              const text = event.currentTarget.value.trim().replace(/,$/, '');
              if (text && !value.includes(text)) onChange([...value, text]);
              event.currentTarget.value = '';
            }
          }}
          onBlur={(event) => {
            const text = event.currentTarget.value.trim();
            if (text && !value.includes(text)) onChange([...value, text]);
            event.currentTarget.value = '';
          }}
        />
      </div>
    </Field>
  );
}
