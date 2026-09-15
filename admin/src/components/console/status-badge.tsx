import { cn } from '@/lib/utils';
import { StatusDot } from './status-dot';

type Tone = 'success' | 'warning' | 'danger' | 'info' | 'muted';

const toneStyles: Record<Tone, string> = {
  success: 'border-success/25 bg-success/10 text-success',
  warning: 'border-warning/25 bg-warning/10 text-warning',
  danger: 'border-destructive/25 bg-destructive/10 text-destructive',
  info: 'border-info/25 bg-info/10 text-info',
  muted: 'border-border bg-muted text-muted-foreground',
};

export function StatusBadge({
  tone,
  children,
  dot = true,
  pulse = false,
  className,
}: {
  tone: Tone;
  children: React.ReactNode;
  dot?: boolean;
  pulse?: boolean;
  className?: string;
}) {
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 font-mono text-xs font-medium tracking-tight',
        toneStyles[tone],
        className,
      )}
    >
      {dot ? <StatusDot tone={tone} pulse={pulse} /> : null}
      {children}
    </span>
  );
}
