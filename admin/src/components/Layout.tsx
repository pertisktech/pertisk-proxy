import { NavLink, Outlet, useLocation, useNavigate } from 'react-router-dom';
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
  User,
  KeyRound,
  ChevronDown,
} from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { cn } from '@/utils';
import { Logo } from '@/components/Logo';
import { useTheme } from '@/context/ThemeContext';
import { useGatewayApiEnabled, useManagementInfo } from '@/context/ManagementContext';
import { useMode } from '@/context/ModeContext';
import { api } from '@/api/client';
import { getUsername, setUsername } from '@/auth';

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
  const navigate = useNavigate();
  const mode = useMode();
  const gatewayApiEnabled = useGatewayApiEnabled();
  const management = useManagementInfo();
  const [open, setOpen] = useState(false);
  const [collapsed, setCollapsed] = useState(getStoredSidebarCollapsed);
  const [profileOpen, setProfileOpen] = useState(false);
  const [username, setUser] = useState(getUsername() || 'admin');
  const [canChangePassword, setCanChangePassword] = useState(mode === 'proxy');
  const profileRef = useRef<HTMLDivElement>(null);

  const nav = mode === 'ingress' ? ingressNav(gatewayApiEnabled) : proxyNav();
  const title = nav.find((n) => (n.end ? location.pathname === n.to : location.pathname.startsWith(n.to)))?.label
    ?? (location.pathname.startsWith('/profile') ? 'Profile' : 'Admin');
  const modeLabel = mode === 'ingress' ? 'Ingress mode' : 'Proxy mode';

  useEffect(() => {
    if (mode === 'ingress' && location.pathname === '/sites') {
      window.location.replace('/sites/ingress');
    }
  }, [mode, location.pathname]);

  useEffect(() => {
    setOpen(false);
    setProfileOpen(false);
  }, [location.pathname]);

  useEffect(() => {
    localStorage.setItem(SIDEBAR_COLLAPSED_KEY, String(collapsed));
  }, [collapsed]);

  useEffect(() => {
    api
      .authCheck()
      .then((c) => {
        if (c.username) {
          setUser(c.username);
          setUsername(c.username);
        }
        if (typeof c.can_change_password === 'boolean') {
          setCanChangePassword(c.can_change_password);
        }
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    function onDocClick(e: MouseEvent) {
      if (!profileRef.current?.contains(e.target as Node)) setProfileOpen(false);
    }
    if (profileOpen) document.addEventListener('mousedown', onDocClick);
    return () => document.removeEventListener('mousedown', onDocClick);
  }, [profileOpen]);

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
              <div className="relative" ref={profileRef}>
                <button
                  type="button"
                  onClick={() => setProfileOpen((v) => !v)}
                  className="inline-flex h-8 max-w-[10rem] items-center gap-1.5 rounded-md border border-border px-2.5 text-sm hover:bg-hover"
                  aria-expanded={profileOpen}
                  aria-haspopup="menu"
                >
                  <User size={16} className="shrink-0" />
                  <span className="truncate">{username}</span>
                  <ChevronDown size={14} className="shrink-0 text-muted" />
                </button>
                {profileOpen ? (
                  <div
                    role="menu"
                    className="absolute right-0 z-50 mt-1 min-w-[11rem] rounded-md border border-border bg-surface py-1 shadow-lg"
                  >
                    <button
                      type="button"
                      role="menuitem"
                      className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-hover"
                      onClick={() => {
                        setProfileOpen(false);
                        navigate('/profile');
                      }}
                    >
                      <User size={14} /> Profile
                    </button>
                    {canChangePassword ? (
                      <button
                        type="button"
                        role="menuitem"
                        className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-hover"
                        onClick={() => {
                          setProfileOpen(false);
                          navigate('/profile');
                        }}
                      >
                        <KeyRound size={14} /> Change password
                      </button>
                    ) : null}
                    <div className="my-1 border-t border-border" />
                    <button
                      type="button"
                      role="menuitem"
                      className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-red-r1 hover:bg-hover"
                      onClick={() => {
                        setProfileOpen(false);
                        onLogout();
                      }}
                    >
                      <LogOut size={14} /> Logout
                    </button>
                  </div>
                ) : null}
              </div>
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
