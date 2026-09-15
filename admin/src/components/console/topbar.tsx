import { useEffect, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { ChevronDown, KeyRound, LogOut, Menu, Plus, Search, User, X } from 'lucide-react';
import type { CreateItem, NavSection } from '@/lib/nav';
import { Sidebar } from './sidebar';
import { StatusBadge } from './status-badge';
import { ThemeToggle } from './theme-toggle';

export function Topbar({
  sections,
  version,
  hostname,
  modeLabel,
  username,
  createItems,
  canChangePassword,
  sidebarFooter,
  onLogout,
}: {
  sections: NavSection[];
  version?: string | null;
  hostname?: string | null;
  modeLabel: string;
  username: string;
  createItems: CreateItem[];
  canChangePassword: boolean;
  sidebarFooter?: React.ReactNode;
  onLogout: () => void;
}) {
  const navigate = useNavigate();
  const location = useLocation();
  const [mobileOpen, setMobileOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [profileOpen, setProfileOpen] = useState(false);
  const createRef = useRef<HTMLDivElement>(null);
  const profileRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function onDocClick(e: MouseEvent) {
      if (!createRef.current?.contains(e.target as Node)) setCreateOpen(false);
      if (!profileRef.current?.contains(e.target as Node)) setProfileOpen(false);
    }
    if (createOpen || profileOpen) document.addEventListener('mousedown', onDocClick);
    return () => document.removeEventListener('mousedown', onDocClick);
  }, [createOpen, profileOpen]);

  useEffect(() => {
    setMobileOpen(false);
    setCreateOpen(false);
    setProfileOpen(false);
  }, [location.pathname, location.search]);

  const initial = (username.trim()[0] || 'a').toLowerCase();

  return (
    <>
      <header className="sticky top-0 z-30 flex h-16 items-center gap-3 border-b border-border bg-background/80 px-4 backdrop-blur-md sm:px-6">
        <button
          type="button"
          onClick={() => setMobileOpen(true)}
          className="flex size-9 items-center justify-center rounded-md border border-border text-muted-foreground hover:text-foreground lg:hidden"
          aria-label="Open navigation"
        >
          <Menu className="size-4" />
        </button>

        <div className="relative hidden max-w-sm flex-1 items-center md:flex">
          <Search className="pointer-events-none absolute left-3 size-4 text-muted-foreground" />
          <input
            type="search"
            placeholder="Search sites, routes, rules…"
            disabled
            aria-disabled="true"
            title="Search coming soon"
            className="h-9 w-full cursor-not-allowed rounded-md border border-border bg-card pl-9 pr-3 text-sm text-muted-foreground opacity-70 placeholder:text-muted-foreground"
          />
        </div>

        <div className="ml-auto flex items-center gap-2 sm:gap-3">
          <StatusBadge tone="success" pulse className="hidden sm:inline-flex">
            {modeLabel}
          </StatusBadge>

          <ThemeToggle />

          <div className="relative" ref={createRef}>
            <button
              type="button"
              onClick={() => {
                setCreateOpen((v) => !v);
                setProfileOpen(false);
              }}
              className="inline-flex h-9 items-center gap-1.5 rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90"
              aria-expanded={createOpen}
              aria-haspopup="menu"
            >
              <Plus className="size-4" />
              <span className="hidden sm:inline">Create</span>
              <ChevronDown className="size-3.5 opacity-70" />
            </button>
            {createOpen ? (
              <div
                role="menu"
                className="absolute right-0 top-full z-[60] mt-1.5 min-w-[15rem] rounded-lg border border-border bg-card p-1 shadow-md"
              >
                {createItems.map(({ to, label, icon: Icon }) => (
                  <button
                    key={to}
                    type="button"
                    role="menuitem"
                    className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-sm hover:bg-accent hover:text-accent-foreground"
                    onClick={() => {
                      setCreateOpen(false);
                      navigate(to);
                    }}
                  >
                    <span className="flex size-7 items-center justify-center rounded-md border border-primary/25 bg-primary/10 text-primary">
                      <Icon className="size-3.5" />
                    </span>
                    {label}
                  </button>
                ))}
              </div>
            ) : null}
          </div>

          <div className="relative" ref={profileRef}>
            <button
              type="button"
              onClick={() => {
                setProfileOpen((v) => !v);
                setCreateOpen(false);
              }}
              className="flex items-center gap-2 rounded-md border border-border px-2 py-1.5 text-sm hover:bg-accent"
              aria-expanded={profileOpen}
              aria-haspopup="menu"
            >
              <span className="flex size-6 items-center justify-center rounded-full bg-primary/15 font-mono text-xs font-semibold text-primary">
                {initial}
              </span>
              <span className="hidden max-w-[8rem] truncate font-medium sm:inline">{username}</span>
              <ChevronDown className="hidden size-3.5 text-muted-foreground sm:inline" />
            </button>
            {profileOpen ? (
              <div
                role="menu"
                className="absolute right-0 top-full z-[60] mt-1.5 min-w-[12rem] rounded-lg border border-border bg-card py-1 shadow-md"
              >
                <button
                  type="button"
                  role="menuitem"
                  className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-accent"
                  onClick={() => {
                    setProfileOpen(false);
                    navigate('/profile');
                  }}
                >
                  <User className="size-3.5" /> Profile
                </button>
                {canChangePassword ? (
                  <button
                    type="button"
                    role="menuitem"
                    className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-accent"
                    onClick={() => {
                      setProfileOpen(false);
                      navigate('/profile');
                    }}
                  >
                    <KeyRound className="size-3.5" /> Change password
                  </button>
                ) : null}
                <div className="my-1 border-t border-border" />
                <button
                  type="button"
                  role="menuitem"
                  className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-destructive hover:bg-accent"
                  onClick={() => {
                    setProfileOpen(false);
                    onLogout();
                  }}
                >
                  <LogOut className="size-3.5" /> Logout
                </button>
              </div>
            ) : null}
          </div>
        </div>
      </header>

      {mobileOpen ? (
        <div className="fixed inset-0 z-50 lg:hidden">
          <div
            className="absolute inset-0 bg-black/60 backdrop-blur-sm"
            onClick={() => setMobileOpen(false)}
          />
          <div className="absolute inset-y-0 left-0 w-72 border-r border-sidebar-border shadow-xl">
            <button
              type="button"
              onClick={() => setMobileOpen(false)}
              className="absolute -right-11 top-3 flex size-9 items-center justify-center rounded-md border border-border bg-background text-muted-foreground"
              aria-label="Close navigation"
            >
              <X className="size-4" />
            </button>
            <Sidebar
              sections={sections}
              version={version}
              hostname={hostname}
              footer={sidebarFooter}
              onNavigate={() => setMobileOpen(false)}
            />
          </div>
        </div>
      ) : null}
    </>
  );
}
