import { NavLink, Outlet, useLocation } from 'react-router-dom';
import {
  LayoutDashboard,
  Route,
  Shield,
  Settings,
  Sun,
  Moon,
  LogOut,
  Menu,
  X,
} from 'lucide-react';
import { useState } from 'react';
import { cn } from '@/utils';
import { useTheme } from '@/context/ThemeContext';

const NAV = [
  { to: '/', label: 'Dashboard', icon: LayoutDashboard },
  { to: '/routes', label: 'Routes', icon: Route },
  { to: '/certificates', label: 'Certificates', icon: Shield },
  { to: '/settings', label: 'Settings', icon: Settings },
];

export function Layout({ onLogout }: { onLogout: () => void }) {
  const { toggleTheme, isDark } = useTheme();
  const location = useLocation();
  const [open, setOpen] = useState(false);
  const title = NAV.find((n) => n.to === location.pathname)?.label ?? 'Admin';

  return (
    <div className="flex min-h-screen bg-bg text-text">
      <aside
        className={cn(
          'fixed inset-y-0 left-0 z-40 flex w-64 flex-col border-r border-border bg-sidebar transition-transform lg:static lg:translate-x-0',
          open ? 'translate-x-0' : '-translate-x-full lg:translate-x-0',
        )}
      >
        <div className="flex h-16 items-center gap-2 border-b border-border px-4">
          <div className="flex h-9 w-9 items-center justify-center rounded-md bg-primary/20 text-primary font-bold">P</div>
          <div>
            <div className="font-semibold">pertisk-proxy</div>
            <div className="text-xs text-text-secondary">Management</div>
          </div>
        </div>
        <nav className="flex-1 space-y-1 p-3">
          {NAV.map(({ to, label, icon: Icon }) => (
            <NavLink
              key={to}
              to={to}
              end={to === '/'}
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
