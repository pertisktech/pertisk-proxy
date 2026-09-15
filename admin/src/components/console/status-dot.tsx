import { cn } from '@/lib/utils';

export function StatusDot({
  tone,
  pulse = false,
  className,
}: {
  tone: 'success' | 'warning' | 'danger' | 'info' | 'muted';
  pulse?: boolean;
  className?: string;
}) {
  const toneMap = {
    success: 'bg-success',
    warning: 'bg-warning',
    danger: 'bg-destructive',
    info: 'bg-info',
    muted: 'bg-muted-foreground',
  } as const;

  return (
    <span className={cn('relative flex size-2.5', className)}>
      {pulse ? (
        <span
          className={cn(
            'absolute inline-flex size-full animate-ping rounded-full opacity-60',
            toneMap[tone],
          )}
        />
      ) : null}
      <span className={cn('relative inline-flex size-2.5 rounded-full', toneMap[tone])} />
    </span>
  );
}
