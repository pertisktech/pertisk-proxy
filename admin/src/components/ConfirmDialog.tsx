import { useEffect } from 'react';
import { Loader2 } from 'lucide-react';
import { cn } from '@/utils';

export type ConfirmDialogProps = {
  open: boolean;
  title: string;
  message: string;
  primaryLabel?: string;
  cancelLabel?: string;
  variant?: 'danger' | 'default';
  loading?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
};

export function ConfirmDialog({
  open,
  title,
  message,
  primaryLabel = 'Confirm',
  cancelLabel = 'Cancel',
  variant = 'default',
  loading = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !loading) onCancel();
    };
    document.addEventListener('keydown', onKey);
    document.body.style.overflow = 'hidden';
    return () => {
      document.removeEventListener('keydown', onKey);
      document.body.style.overflow = '';
    };
  }, [open, loading, onCancel]);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      onClick={(e) => {
        if (!loading && e.target === e.currentTarget) onCancel();
      }}
    >
      <div className="w-full max-w-md rounded-lg border border-border bg-surface p-6 shadow-md" onClick={(e) => e.stopPropagation()}>
        <h2 className="text-lg font-semibold">{title}</h2>
        <p className="mt-3 text-sm text-text-secondary">{message}</p>
        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            disabled={loading}
            className="rounded-md border border-border px-4 py-2 text-sm hover:bg-hover disabled:opacity-50"
          >
            {cancelLabel}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={loading}
            className={cn(
              'inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium disabled:opacity-50',
              variant === 'danger' ? 'bg-red-r1 text-white' : 'bg-primary text-bg',
            )}
          >
            {loading ? <Loader2 size={16} className="animate-spin" /> : null}
            {primaryLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
