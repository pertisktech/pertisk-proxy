import { cn } from '@/utils';

const inputCls =
  'mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text placeholder:text-muted/40 placeholder:italic';
const labelCls = 'block text-sm text-text-secondary';

/** Stable (module-level) controlled field — do not nest inside route components. */
export function ConfigTextField({
  label,
  example,
  value,
  onChange,
  disabled,
}: {
  label: string;
  example: string;
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  const hasValue = value.trim().length > 0;
  return (
    <label className={labelCls}>
      <span className="flex items-center justify-between gap-2">
        <span>{label}</span>
        <span
          className={cn(
            'rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide',
            hasValue ? 'bg-green-g1/15 text-green-g1' : 'bg-surface-elevated text-muted',
          )}
        >
          {hasValue ? 'set' : 'empty'}
        </span>
      </span>
      <span className="mt-0.5 block text-[11px] font-normal italic text-muted">
        Example · {example}
      </span>
      <input
        className={cn(inputCls, hasValue && 'border-primary/50')}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled}
        autoComplete="off"
      />
    </label>
  );
}
