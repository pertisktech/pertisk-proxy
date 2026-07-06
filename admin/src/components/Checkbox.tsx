import { Check } from 'lucide-react';
import type { ReactNode } from 'react';
import { cn } from '@/utils';

type CheckboxProps = {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label?: ReactNode;
  id?: string;
  disabled?: boolean;
  className?: string;
};

export function Checkbox({ checked, onChange, label, id, disabled, className }: CheckboxProps) {
  return (
    <label className={cn('checkbox', disabled && 'disabled', className)}>
      <input
        type="checkbox"
        id={id}
        className="checkbox-input"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span className="checkbox-box" aria-hidden="true">
        <Check size={12} strokeWidth={3} className="checkbox-check" />
      </span>
      {label != null ? <span className="checkbox-label">{label}</span> : null}
    </label>
  );
}
