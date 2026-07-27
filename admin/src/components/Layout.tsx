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
  ShieldAlert,
  ListChecks,
  ChevronsLeft,
  ChevronsRight,
  Archive,
  User,
  KeyRound,
  ChevronDown,
  Plus,
  FileUp,
} from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { cn } from '@/utils';
import { Logo } from '@/components/Logo';
import { useTheme } from '@/context/ThemeContext';
import { useGatewayApiEnabled, useManagementInfo } from '@/context/ManagementContext';
import { useMode } from '@/context/ModeContext';
import { api } from '@/api/client';
import { getUsername, setUsername } from '@/auth';

const SIDEBAR_COLLAPSED_KEY = 'pertisk_sidebar_collapsed';

type NavItem = { to: string; label: string; icon: typeof Globe; end?: boolean };
type NavGroup = { id: string; label: string; items: NavItem[] };
type CreateItem = { to: string; label: string; icon: typeof Globe };

function getStoredSidebarCollapsed(): boolean {
  return localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === 'true';
}

function proxyNavGroups(): NavGroup[] {
  return [
    {
      id: 'overview',
      label: 'Overview',
      items: [{ to: '/', label: 'Dashboard', icon: LayoutDashboard, end: true }],
    },
    {
      id: 'routing',
      label: 'Routing',
      items: [{ to: '/sites', label: 'Sites', icon: Globe }],
    },
    {
      id: 'security',
      label: 'Security',
      items: [
        { to: '/access-lists', label: 'Access Control', icon: ListChecks },
        { to: '/waf', label: 'WAF', icon: ShieldAlert },
      ],
    },
    {
      id: 'tls',
      label: 'TLS & DNS',
      items: [
        { to: '/certificates', label: 'Certificates', icon: Shield },
        { to: '/dns-providers', label: 'DNS Providers', icon: Server },
      ],
    },
    {
      id: 'observe',
      label: 'Observe',
      items: [
        { to: '/logs', label: 'Logs', icon: ScrollText },
        { to: '/metrics', label: 'Metrics', icon: LineChart },
      ],
    },
    {
      id: 'system',
      label: 'System',
      items: [
        { to: '/backup', label: 'Backup', icon: Archive },
        { to: '/settings', label: 'Settings', icon: Settings },
      ],
    },
  ];
}

function ingressNavGroups(gatewayApiEnabled: boolean): NavGroup[] {
  const routing: NavItem[] = [{ to: '/sites/ingress', label: 'Ingress', icon: Network }];
  if (gatewayApiEnabled) {
    routing.push(
      { to: '/sites/gateway/gateways', label: 'Gateways', icon: DoorOpen },
      { to: '/sites/gateway/sites', label: 'HTTP Routes', icon: GitBranch },
    );
  }

  return [
    {
      id: 'overview',
      label: 'Overview',
      items: [{ to: '/', label: 'Dashboard', icon: LayoutDashboard, end: true }],
    },
    { id: 'routing', label: 'Routing', items: routing },
    {
      id: 'security',
      label: 'Security',
      items: [
        { to: '/access-lists', label: 'Access Control', icon: ListChecks },
        { to: '/waf', label: 'WAF', icon: ShieldAlert },
      ],
    },
    {
      id: 'tls',
      label: 'TLS',
      items: [{ to: '/certificates', label: 'Certificates', icon: Shield }],
    },
    {
      id: 'observe',
      label: 'Observe',
      items: [
        { to: '/logs', label: 'Logs', icon: ScrollText },
        { to: '/metrics', label: 'Metrics', icon: LineChart },
      ],
    },
    {
      id: 'system',
      label: 'System',
      items: [
        { to: '/backup', label: 'Backup', icon: Archive },
        { to: '/settings', label: 'Settings', icon: Settings },
      ],
    },
  ];
}

function resolveNavTitle(pathname: string, groups: NavGroup[]): string {
  const items = groups.flatMap((g) => g.items);
  // Prefer the longest matching path so /sites/gateway/sites wins over /sites.
  const match = items
    .filter((n) => (n.end ? pathname === n.to : pathname === n.to || pathname.startsWith(`${n.to}/`)))
    .sort((a, b) => b.to.length - a.to.length)[0];
  if (match) return match.label;
  if (pathname.startsWith('/profile')) return 'Profile';
  return 'Admin';
}

function createMenuItems(mode: 'proxy' | 'ingress' | undefined): CreateItem[] {
  const siteTo = mode === 'ingress' ? '/sites/ingress?new=1' : '/sites?new=1';
  const items: CreateItem[] = [
    { to: siteTo, label: 'Site', icon: Globe },
  ];
  if (mode !== 'ingress') {
    items.push({ to: '/dns-providers?new=1', label: 'DNS provider', icon: Server });
  }
  items.push(
    { to: '/certificates?import=1', label: 'Import certificate', icon: FileUp },
    { to: '/access-lists?new=1', label: 'Access Control', icon: ListChecks },
    { to: '/waf?new=1', label: 'WAF', icon: ShieldAlert },
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
  const [createOpen, setCreateOpen] = useState(false);
  const [username, setUser] = useState(getUsername() || 'admin');
  const [canChangePassword, setCanChangePassword] = useState(mode === 'proxy');
  const profileRef = useRef<HTMLDivElement>(null);
  const createRef = useRef<HTMLDivElement>(null);

  const navGroups = useMemo(
    () => (mode === 'ingress' ? ingressNavGroups(gatewayApiEnabled) : proxyNavGroups()),
    [mode, gatewayApiEnabled],
  );
  const createItems = useMemo(() => createMenuItems(mode), [mode]);
  const title = resolveNavTitle(location.pathname, navGroups);
  const modeLabel = mode === 'ingress' ? 'Ingress mode' : 'Proxy mode';

  useEffect(() => {
    if (mode === 'ingress' && location.pathname === '/sites') {
      window.location.replace('/sites/ingress');
    }
  }, [mode, location.pathname]);

  useEffect(() => {
    setOpen(false);
    setProfileOpen(false);
    setCreateOpen(false);
  }, [location.pathname, location.search]);

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
      if (!createRef.current?.contains(e.target as Node)) setCreateOpen(false);
    }
    if (profileOpen || createOpen) document.addEventListener('mousedown', onDocClick);
    return () => document.removeEventListener('mousedown', onDocClick);
  }, [profileOpen, createOpen]);

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
        <nav className="app-sidebar-nav" aria-label="Main">
          {navGroups.map((group) => (
            <div key={group.id} className="app-sidebar-group">
              <div className="app-sidebar-group-label" title={collapsed ? group.label : undefined}>
                <span className="app-sidebar-group-label-text">{group.label}</span>
              </div>
              <div className="app-sidebar-group-items" role="group" aria-label={group.label}>
                {group.items.map(({ to, label, icon: Icon, end }) => (
                  <NavLink
                    key={to}
                    to={to}
                    end={end}
                    title={collapsed ? label : undefined}
                    onClick={() => setOpen(false)}
                    className={({ isActive }) => cn('app-sidebar-link', isActive && 'active')}
                  >
                    <Icon size={18} className="shrink-0" />
                    <span className="truncate app-sidebar-link-label">{label}</span>
                  </NavLink>
                ))}
              </div>
            </div>
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
              <div className="relative" ref={createRef}>
                <button
                  type="button"
                  onClick={() => {
                    setCreateOpen((v) => !v);
                    setProfileOpen(false);
                  }}
                  className="topbar-create-btn"
                  aria-expanded={createOpen}
                  aria-haspopup="menu"
                >
                  <Plus size={15} strokeWidth={2.5} className="shrink-0" />
                  <span className="hidden sm:inline">Create</span>
                  <ChevronDown size={14} className="shrink-0 opacity-80" />
                </button>
                {createOpen ? (
                  <div role="menu" className="topbar-create-menu">
                    {createItems.map(({ to, label, icon: Icon }) => (
                      <button
                        key={to}
                        type="button"
                        role="menuitem"
                        className="topbar-create-item"
                        onClick={() => {
                          setCreateOpen(false);
                          navigate(to);
                        }}
                      >
                        <span className="topbar-create-item-icon">
                          <Icon size={14} />
                        </span>
                        {label}
                      </button>
                    ))}
                  </div>
                ) : null}
              </div>
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
                  onClick={() => {
                    setProfileOpen((v) => !v);
                    setCreateOpen(false);
                  }}
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
                    className="absolute right-0 top-full z-[60] mt-1 min-w-[12rem] rounded-md border border-border bg-surface py-1 shadow-lg"
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
