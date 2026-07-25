import { LayoutGrid, List, Plus } from 'lucide-react';
import { cn } from '@/utils';

export type ViewMode = 'card' | 'list';

type ListToolbarProps = {
  viewMode: ViewMode;
  onViewModeChange: (mode: ViewMode) => void;
  addLabel: string;
  onAdd: () => void;
};

export function ListToolbar({ viewMode, onViewModeChange, addLabel, onAdd }: ListToolbarProps) {
  return (
    <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
      <button
        type="button"
        onClick={onAdd}
        className="inline-flex items-center gap-2 rounded-md bg-primary px-4 py-2 text-sm font-medium text-bg hover:opacity-90"
      >
        <Plus size={16} /> {addLabel}
      </button>
      <div className="inline-flex rounded-md border border-border p-0.5">
        <button
          type="button"
          onClick={() => onViewModeChange('card')}
          className={cn(
            'inline-flex items-center gap-1.5 rounded px-3 py-1.5 text-sm',
            viewMode === 'card' ? 'bg-hover font-medium text-primary' : 'text-text-secondary hover:text-text',
          )}
        >
          <LayoutGrid size={14} /> Cards
        </button>
        <button
          type="button"
          onClick={() => onViewModeChange('list')}
          className={cn(
            'inline-flex items-center gap-1.5 rounded px-3 py-1.5 text-sm',
            viewMode === 'list' ? 'bg-hover font-medium text-primary' : 'text-text-secondary hover:text-text',
          )}
        >
          <List size={14} /> List
        </button>
      </div>
    </div>
  );
}
