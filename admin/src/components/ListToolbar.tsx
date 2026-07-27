import type { ReactNode } from 'react';
import { LayoutGrid, List, Plus } from 'lucide-react';
import { cn } from '@/utils';

export type ViewMode = 'card' | 'list';

type ListToolbarProps = {
  viewMode: ViewMode;
  onViewModeChange: (mode: ViewMode) => void;
  addLabel: string;
  onAdd: () => void;
  /** Filled primary (default) or outline secondary-style action. */
  actionVariant?: 'primary' | 'outline';
  /** Defaults to Plus. Pass e.g. FileUp for import actions. */
  actionIcon?: ReactNode;
  /** Optional actions rendered between the primary button and the view toggle. */
  children?: ReactNode;
};

export function ListToolbar({
  viewMode,
  onViewModeChange,
  addLabel,
  onAdd,
  actionVariant = 'primary',
  actionIcon,
  children,
}: ListToolbarProps) {
  return (
    <div className="list-toolbar">
      <div className="list-toolbar-actions">
        <button
          type="button"
          onClick={onAdd}
          className={cn(
            'list-toolbar-btn',
            actionVariant === 'outline' ? 'list-toolbar-btn--outline' : 'list-toolbar-btn--primary',
          )}
        >
          {actionIcon ?? <Plus size={16} strokeWidth={2.25} />}
          {addLabel}
        </button>
        {children}
      </div>
      <div className="list-toolbar-view" role="group" aria-label="View mode">
        <button
          type="button"
          onClick={() => onViewModeChange('card')}
          className={cn('list-toolbar-view-btn', viewMode === 'card' && 'active')}
          aria-pressed={viewMode === 'card'}
          title="Cards"
        >
          <LayoutGrid size={14} />
          <span className="list-toolbar-view-label">Cards</span>
        </button>
        <button
          type="button"
          onClick={() => onViewModeChange('list')}
          className={cn('list-toolbar-view-btn', viewMode === 'list' && 'active')}
          aria-pressed={viewMode === 'list'}
          title="List"
        >
          <List size={14} />
          <span className="list-toolbar-view-label">List</span>
        </button>
      </div>
    </div>
  );
}
