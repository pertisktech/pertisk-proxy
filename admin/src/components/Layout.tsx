import { NavLink, Outlet, useLocation } from 'react-router-dom';
import {
  LayoutDashboard,
  Shield,
  Settings,
  Sun,
  Moon,
  LogOut,
  Menu,
  X,
  Globe,
  Server,
  ScrollText,
  Network,
  GitBranch,
  DoorOpen,
} from 'lucide-react';
import { useEffect, useState } from 'react';
import { cn } from '@/utils';
import { useTheme } from '@/context/ThemeContext';
import { useGatewayApiEnabled, useManagementInfo } from '@/context/ManagementContext';
import { useMode } from '@/context/ModeContext';

type NavItem = { to: string; label: string; icon: typeof Globe; end?: boolean };

function proxyNav(): NavItem[] {
  return [
    { to: '/', label: 'Dashboard', icon: LayoutDashboard, end: true },
    { to: '/sites', label: 'Sites', icon: Globe },
    { to: '/certificates', label: 'Certificates', icon: Shield },
    { to: '/dns-providers', label: 'DNS Providers', icon: Server },
    { to: '/logs', label: 'Logs', icon: ScrollText },
    { to: '/settings', label: 'Settings', icon: Settings },
  ];
}

function ingressNav(gatewayApiEnabled: boolean): NavItem[] {
  const items: NavItem[] = [
    { to: '/', label: 'Dashboard', icon: LayoutDashboard, end: true },
    { to: '/sites/ingress', label: 'Ingress', icon: Network },
  ];
  if (gatewayApiEnabled) {
    items.push({ to: '/sites/gateway/gateways', label: 'Gateways', icon: DoorOpen });
    items.push({ to: '/sites/gateway/sites', label: 'HTTP Routes', icon: GitBranch });
  }
  items.push(
    { to: '/certificates', label: 'Certificates', icon: Shield },
    { to: '/logs', label: 'Logs', icon: ScrollText },
    { to: '/settings', label: 'Settings', icon: Settings },
  );
  return items;
}

export function Layout({ onLogout }: { onLogout: () => void }) {
  const { toggleTheme, isDark } = useTheme();
  const location = useLocation();
  const mode = useMode();
  const gatewayApiEnabled = useGatewayApiEnabled();
  const management = useManagementInfo();
  const [open, setOpen] = useState(false);

  const nav = mode === 'ingress' ? ingressNav(gatewayApiEnabled) : proxyNav();
  const title = nav.find((n) => (n.end ? location.pathname === n.to : location.pathname.startsWith(n.to)))?.label ?? 'Admin';
  const modeLabel = mode === 'ingress' ? 'Ingress mode' : 'Proxy mode';

  useEffect(() => {
    if (mode === 'ingress' && location.pathname === '/sites') {
      window.location.replace('/sites/ingress');
    }
  }, [mode, location.pathname]);

  return (
    <div className="flex min-h-screen bg-bg text-text">
      <aside
        className={cn(
          'fixed inset-y-0 left-0 z-40 flex w-64 flex-col border-r border-border bg-sidebar transition-transform lg:static lg:translate-x-0',
          open ? 'translate-x-0' : '-translate-x-full lg:translate-x-0',
        )}
      >
        <div className="flex h-16 items-center gap-2 border-b border-border px-4">
          <div>
            <div className="font-semibold">Pertisk-Proxy</div>
            <div className="text-xs text-text-secondary">{modeLabel}</div>
          </div>
        </div>
        <nav className="flex-1 space-y-1 p-3">
          {nav.map(({ to, label, icon: Icon, end }) => (
            <NavLink
              key={to}
              to={to}
              end={end}
              onClick={() => setOpen(false)}
              className={({ isActive }) =>
                cn(
                  'flex items-center gap-3 rounded-md px-3 py-2 text-sm transition-colors',
                  isActive
                    ? 'bg-hover font-semibold text-primary'
                    : 'text-text-secondary hover:bg-hover hover:text-text',
                )
              }
            >
              <Icon size={18} />
              {label}
            </NavLink>
          ))}
        </nav>
        {mode === 'ingress' && management?.ingress_class && (
          <div className="border-t border-border px-4 py-3 text-xs text-text-secondary">
            Class: {management.ingress_class}
            {gatewayApiEnabled && management.gateway_class ? ` · GW: ${management.gateway_class}` : null}
          </div>
        )}
      </aside>

      <div className="flex min-h-screen flex-1 flex-col">
        <header className="sticky top-0 z-30 flex h-16 items-center justify-between border-b border-border bg-surface px-4">
          <div className="flex items-center gap-3">
            <button type="button" className="lg:hidden" onClick={() => setOpen((v) => !v)}>
              {open ? <X size={20} /> : <Menu size={20} />}
            </button>
            <h1 className="text-lg font-semibold">{title}</h1>
          </div>
          <div className="flex items-center gap-2">
            <span className="hidden rounded border border-border px-2 py-1 text-xs text-text-secondary sm:inline">
              {modeLabel}
            </span>
            <button
              type="button"
              onClick={toggleTheme}
              className="rounded-md border border-border p-2 hover:bg-hover"
              title="Toggle theme"
            >
              {isDark ? <Sun size={18} /> : <Moon size={18} />}
            </button>
            <button
              type="button"
              onClick={onLogout}
              className="flex items-center gap-2 rounded-md border border-border px-3 py-2 text-sm hover:bg-hover"
            >
              <LogOut size={16} />
              Logout
            </button>
          </div>
        </header>
        <main className="flex-1 overflow-auto p-4">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
