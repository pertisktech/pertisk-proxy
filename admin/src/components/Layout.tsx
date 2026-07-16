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
  LineChart,
  ChevronsLeft,
  ChevronsRight,
  Archive,
} from 'lucide-react';
import { useEffect, useState } from 'react';
import { cn } from '@/utils';
import { Logo } from '@/components/Logo';
import { useTheme } from '@/context/ThemeContext';
import { useGatewayApiEnabled, useManagementInfo } from '@/context/ManagementContext';
import { useMode } from '@/context/ModeContext';

const SIDEBAR_COLLAPSED_KEY = 'pertisk_sidebar_collapsed';

type NavItem = { to: string; label: string; icon: typeof Globe; end?: boolean };

function getStoredSidebarCollapsed(): boolean {
  return localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === 'true';
}

function proxyNav(): NavItem[] {
  return [
    { to: '/', label: 'Dashboard', icon: LayoutDashboard, end: true },
    { to: '/sites', label: 'Sites', icon: Globe },
    { to: '/certificates', label: 'Certificates', icon: Shield },
    { to: '/dns-providers', label: 'DNS Providers', icon: Server },
    { to: '/logs', label: 'Logs', icon: ScrollText },
    { to: '/metrics', label: 'Metrics', icon: LineChart },
    { to: '/backup', label: 'Backup', icon: Archive },
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
    { to: '/metrics', label: 'Metrics', icon: LineChart },
    { to: '/backup', label: 'Backup', icon: Archive },
    { to: '/settings', label: 'Settings', icon: Settings },
  );
  return items;
}

export function Layout({ onLogout, loading = false }: { onLogout: () => void; loading?: boolean }) {
  const { toggleTheme, isDark } = useTheme();
  const location = useLocation();
  const mode = useMode();
  const gatewayApiEnabled = useGatewayApiEnabled();
  const management = useManagementInfo();
  const [open, setOpen] = useState(false);
  const [collapsed, setCollapsed] = useState(getStoredSidebarCollapsed);

  const nav = mode === 'ingress' ? ingressNav(gatewayApiEnabled) : proxyNav();
  const title = nav.find((n) => (n.end ? location.pathname === n.to : location.pathname.startsWith(n.to)))?.label ?? 'Admin';
  const modeLabel = mode === 'ingress' ? 'Ingress mode' : 'Proxy mode';

  useEffect(() => {
    if (mode === 'ingress' && location.pathname === '/sites') {
      window.location.replace('/sites/ingress');
    }
  }, [mode, location.pathname]);

  useEffect(() => {
    setOpen(false);
  }, [location.pathname]);

  useEffect(() => {
    localStorage.setItem(SIDEBAR_COLLAPSED_KEY, String(collapsed));
  }, [collapsed]);

  return (
    <div className="app-shell text-text">
      <div
        className={open ? 'app-sidebar-backdrop open' : 'app-sidebar-backdrop'}
        aria-hidden={!open}
        onClick={() => setOpen(false)}
      />

      <aside
        id="app-sidebar"
        className={cn('app-sidebar', open && 'open', collapsed && 'collapsed')}
      >
        <div className="app-sidebar-header">
          <div className="app-sidebar-brand">
            <Logo className="app-sidebar-logo" alt="" />
            <span className="app-sidebar-brand-text">Pertisk-Proxy</span>
          </div>
          <button
            type="button"
            onClick={() => setCollapsed((v) => !v)}
            className={cn('app-sidebar-collapse-btn', !collapsed && 'anchor-right')}
            title={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
            aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          >
            {collapsed ? <ChevronsRight size={16} strokeWidth={2.25} /> : <ChevronsLeft size={16} strokeWidth={2.25} />}
          </button>
        </div>
        <nav className="app-sidebar-nav">
          {nav.map(({ to, label, icon: Icon, end }) => (
            <NavLink
              key={to}
              to={to}
              end={end}
              title={collapsed ? label : undefined}
              onClick={() => setOpen(false)}
              className={({ isActive }) => cn('app-sidebar-link', isActive && 'active')}
            >
              <Icon size={18} className="shrink-0" />
              <span className={cn('truncate app-sidebar-link-label')}>{label}</span>
            </NavLink>
          ))}
        </nav>
        {mode === 'ingress' && management?.ingress_class ? (
          <div className="app-sidebar-footer">
            Class: {management.ingress_class}
            {gatewayApiEnabled && management.gateway_class ? ` · GW: ${management.gateway_class}` : null}
          </div>
        ) : null}
      </aside>

      <div className={cn('app-content', open && 'sidebar-open')}>
        <header className="app-content-top">
          <div className="app-content-top-inner">
            <div className="flex items-center gap-3">
              <button
                type="button"
                className="inline-flex items-center justify-center lg:hidden"
                aria-controls="app-sidebar"
                aria-expanded={open}
                aria-label={open ? 'Close menu' : 'Open menu'}
                onClick={() => setOpen((v) => !v)}
              >
                {open ? <X size={20} /> : <Menu size={20} />}
              </button>
              <h1 className="text-base font-semibold leading-none">{title}</h1>
            </div>
            <div className="flex items-center gap-2">
              <span className="hidden rounded border border-border px-2 py-0.5 text-xs text-text-secondary sm:inline">
                {modeLabel}
              </span>
              <button
                type="button"
                onClick={toggleTheme}
                className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-border hover:bg-hover"
                title="Toggle theme"
              >
                {isDark ? <Sun size={16} /> : <Moon size={16} />}
              </button>
              <button
                type="button"
                onClick={onLogout}
                className="inline-flex h-8 items-center gap-2 rounded-md border border-border px-2.5 text-sm hover:bg-hover"
              >
                <LogOut size={16} />
                Logout
              </button>
            </div>
          </div>
        </header>
        <main className="app-main">
          {loading ? <p className="app-main-status">Loading…</p> : <Outlet />}
        </main>
      </div>
    </div>
  );
}
