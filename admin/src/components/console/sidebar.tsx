import { NavLink, useLocation } from 'react-router-dom';
import { ChevronsLeft, ChevronsRight } from 'lucide-react';
import { Logo } from '@/components/Logo';
import { cn } from '@/lib/utils';
import type { NavSection } from '@/lib/nav';
import { StatusDot } from './status-dot';

function isNavActive(pathname: string, to: string, end?: boolean) {
  if (end) return pathname === to;
  return pathname === to || pathname.startsWith(`${to}/`);
}

export function Sidebar({
  sections,
  version,
  hostname,
  footer,
  onNavigate,
  collapsed = false,
  onToggleCollapse,
}: {
  sections: NavSection[];
  version?: string | null;
  hostname?: string | null;
  footer?: React.ReactNode;
  onNavigate?: () => void;
  collapsed?: boolean;
  onToggleCollapse?: () => void;
}) {
  const { pathname } = useLocation();

  return (
    <div className="flex h-full flex-col bg-sidebar text-sidebar-foreground">
      <div
        className={cn(
          'flex h-16 shrink-0 items-center border-b border-sidebar-border',
          collapsed ? 'justify-center px-2' : 'gap-2.5 pr-3 pl-5',
        )}
      >
        {!collapsed ? (
          <>
            <Logo className="size-8 shrink-0" alt="" />
            <div className="flex min-w-0 flex-1 flex-col leading-none">
              <span className="truncate text-sm font-semibold tracking-tight">Pertisk Proxy</span>
              {version ? (
                <span className="font-mono text-[10px] text-muted-foreground">v{version}</span>
              ) : null}
            </div>
          </>
        ) : null}
        {onToggleCollapse ? (
          <button
            type="button"
            onClick={onToggleCollapse}
            className="inline-flex size-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-foreground"
            aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
            title={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          >
            {collapsed ? <ChevronsRight className="size-4" /> : <ChevronsLeft className="size-4" />}
          </button>
        ) : (
          <Logo className="size-8 shrink-0" alt="" />
        )}
      </div>

      <nav
        className={cn('min-h-0 flex-1 overflow-y-auto py-4', collapsed ? 'px-2' : 'px-3')}
        aria-label="Main"
      >
        {sections.map((section) => (
          <div key={section.id} className="mb-5 last:mb-0">
            {!collapsed ? (
              <p className="px-2 pb-1.5 text-[10px] font-semibold uppercase tracking-widest text-muted-foreground/70">
                {section.title}
              </p>
            ) : (
              <div className="mx-auto mb-1.5 h-px w-6 bg-sidebar-border" aria-hidden />
            )}
            <ul className="flex flex-col gap-0.5">
              {section.items.map((item) => {
                const Icon = item.icon;
                const active = isNavActive(pathname, item.to, item.end);
                return (
                  <li key={item.to}>
                    <NavLink
                      to={item.to}
                      end={item.end}
                      onClick={onNavigate}
                      title={collapsed ? item.label : undefined}
                      aria-label={collapsed ? item.label : undefined}
                      className={cn(
                        'group relative flex items-center rounded-md text-sm transition-colors',
                        collapsed ? 'justify-center px-0 py-2.5' : 'gap-2.5 px-2.5 py-2',
                        active
                          ? 'bg-sidebar-accent text-foreground'
                          : 'text-muted-foreground hover:bg-sidebar-accent/60 hover:text-foreground',
                      )}
                    >
                      {active ? (
                        <span
                          className={cn(
                            'absolute inset-y-1.5 w-0.5 rounded-full bg-primary',
                            collapsed ? 'left-0.5' : 'left-0',
                          )}
                        />
                      ) : null}
                      <Icon
                        className={cn(
                          'size-4 shrink-0',
                          active ? 'text-primary' : 'text-muted-foreground group-hover:text-foreground',
                        )}
                      />
                      {!collapsed ? <span className="truncate">{item.label}</span> : null}
                    </NavLink>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </nav>

      <div className={cn('shrink-0 border-t border-sidebar-border', collapsed ? 'px-2 py-3' : 'px-5 py-3.5')}>
        {collapsed ? (
          <div className="flex justify-center" title={hostname ? `Healthy · ${hostname}` : 'Healthy'}>
            <StatusDot tone="success" pulse />
          </div>
        ) : footer ? (
          footer
        ) : (
          <div className="flex items-center gap-2">
            <StatusDot tone="success" pulse />
            <span className="min-w-0 truncate text-xs text-muted-foreground">
              Healthy
              {hostname ? (
                <>
                  {' · '}
                  <span className="font-mono">{hostname}</span>
                </>
              ) : null}
            </span>
          </div>
        )}
      </div>
    </div>
  );
}
