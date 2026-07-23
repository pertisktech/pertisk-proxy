import type { ReactNode } from 'react';
import { cn } from '@/utils';

export function ResourceCardGrid({ children }: { children: ReactNode }) {
  return <div className="resource-card-grid">{children}</div>;
}

export function ResourceCard({
  icon,
  title,
  titleExtra,
  badge,
  subtitle,
  tags,
  meta,
  children,
  actions,
  className,
}: {
  icon?: ReactNode;
  title: ReactNode;
  titleExtra?: ReactNode;
  badge?: ReactNode;
  subtitle?: ReactNode;
  tags?: ReactNode;
  meta?: Array<{ label: string; value: ReactNode }>;
  children?: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  return (
    <article className={cn('resource-card', className)}>
      <div className="resource-card-head">
        <div className="resource-card-title-row">
          {icon ? <div className="resource-card-icon">{icon}</div> : null}
          <div className="resource-card-title-block min-w-0 flex-1">
            <div className="flex items-start justify-between gap-2">
              <h3 className="resource-card-title">{title}</h3>
              {badge}
            </div>
            {titleExtra}
            {subtitle ? <div className="resource-card-subtitle">{subtitle}</div> : null}
          </div>
        </div>
      </div>

      {tags ? <div className="resource-card-tags">{tags}</div> : null}

      {meta && meta.length > 0 ? (
        <dl className="resource-card-meta">
          {meta.map((row) => (
            <div key={row.label} className="resource-card-meta-row">
              <dt>{row.label}</dt>
              <dd>{row.value}</dd>
            </div>
          ))}
        </dl>
      ) : null}

      {children ? <div className="resource-card-body">{children}</div> : null}

      {actions ? <div className="resource-card-actions icon-actions">{actions}</div> : null}
    </article>
  );
}

export function ResourceBadge({
  children,
  tone = 'neutral',
}: {
  children: ReactNode;
  tone?: 'neutral' | 'success' | 'warning' | 'danger' | 'info';
}) {
  return <span className={cn('resource-badge', `tone-${tone}`)}>{children}</span>;
}

export function ResourceTag({ children }: { children: ReactNode }) {
  return <span className="resource-tag">{children}</span>;
}
