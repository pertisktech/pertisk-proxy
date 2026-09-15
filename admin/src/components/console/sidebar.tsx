import { NavLink, useLocation } from 'react-router-dom';
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
}: {
  sections: NavSection[];
  version?: string | null;
  hostname?: string | null;
  footer?: React.ReactNode;
  onNavigate?: () => void;
}) {
  const { pathname } = useLocation();

  return (
    <div className="flex h-full flex-col bg-sidebar text-sidebar-foreground">
      <div className="flex h-16 items-center gap-2.5 border-b border-sidebar-border px-5">
        <div className="flex size-8 items-center justify-center rounded-md bg-primary/15 ring-1 ring-primary/30">
          <span className="font-mono text-sm font-bold text-primary">pk</span>
        </div>
        <div className="flex min-w-0 flex-col leading-none">
          <span className="truncate text-sm font-semibold tracking-tight">pertisk-proxy</span>
          {version ? (
            <span className="font-mono text-[10px] text-muted-foreground">v{version}</span>
          ) : null}
        </div>
      </div>

      <nav className="flex-1 overflow-y-auto px-3 py-4" aria-label="Main">
        {sections.map((section) => (
          <div key={section.id} className="mb-5 last:mb-0">
            <p className="px-2 pb-1.5 text-[10px] font-semibold uppercase tracking-widest text-muted-foreground/70">
              {section.title}
            </p>
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
                      className={cn(
                        'group relative flex items-center gap-2.5 rounded-md px-2.5 py-2 text-sm transition-colors',
                        active
                          ? 'bg-sidebar-accent text-foreground'
                          : 'text-muted-foreground hover:bg-sidebar-accent/60 hover:text-foreground',
                      )}
                    >
                      {active ? (
                        <span className="absolute inset-y-1.5 left-0 w-0.5 rounded-full bg-primary" />
                      ) : null}
                      <Icon
                        className={cn(
                          'size-4 shrink-0',
                          active ? 'text-primary' : 'text-muted-foreground group-hover:text-foreground',
                        )}
                      />
                      <span className="truncate">{item.label}</span>
                    </NavLink>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </nav>

      <div className="border-t border-sidebar-border px-5 py-3.5">
        {footer ? (
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
