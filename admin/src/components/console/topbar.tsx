import { useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { ChevronDown, Globe, KeyRound, LogOut, Menu, Plus, Search, User, X } from 'lucide-react';
import { api, type Site } from '@/api/client';
import type { CreateItem, NavSection } from '@/lib/nav';
import type { ApiMode } from '@/context/ModeContext';
import { Sidebar } from './sidebar';
import { StatusBadge } from './status-badge';
import { ThemeToggle } from './theme-toggle';

type SearchHit = {
  host: string;
  subtitle: string;
  href: string;
};

function siteSearchHref(mode: ApiMode, site: Site): string {
  const q = encodeURIComponent(site.host);
  if (mode === 'proxy') return `/sites?q=${q}`;
  if (site.k8s_resource_kind === 'HTTPRoute' || site.k8s_resource_kind === 'gateway') {
    return `/sites/gateway/sites?q=${q}`;
  }
  return `/sites/ingress?q=${q}`;
}

function matchSites(sites: Site[], query: string, mode: ApiMode): SearchHit[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  return sites
    .filter((site) => {
      const haystack = [
        site.host,
        site.backend,
        site.ingress_name,
        site.ingress_namespace,
        ...(site.routes ?? []).flatMap((r) => [r.path, r.upstream, r.rewrite]),
      ]
        .filter(Boolean)
        .join(' ')
        .toLowerCase();
      return haystack.includes(q);
    })
    .slice(0, 8)
    .map((site) => ({
      host: site.host,
      subtitle: [site.backend, site.ingress_namespace, site.ingress_name].filter(Boolean).join(' · ') || 'Site',
      href: siteSearchHref(mode, site),
    }));
}

export function Topbar({
  sections,
  version,
  hostname,
  mode,
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
  mode: ApiMode;
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
  const [query, setQuery] = useState('');
  const [sites, setSites] = useState<Site[]>([]);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchLoading, setSearchLoading] = useState(false);
  const createRef = useRef<HTMLDivElement>(null);
  const profileRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function onDocClick(e: MouseEvent) {
      if (!createRef.current?.contains(e.target as Node)) setCreateOpen(false);
      if (!profileRef.current?.contains(e.target as Node)) setProfileOpen(false);
      if (!searchRef.current?.contains(e.target as Node)) setSearchOpen(false);
    }
    if (createOpen || profileOpen || searchOpen) document.addEventListener('mousedown', onDocClick);
    return () => document.removeEventListener('mousedown', onDocClick);
  }, [createOpen, profileOpen, searchOpen]);

  useEffect(() => {
    setMobileOpen(false);
    setCreateOpen(false);
    setProfileOpen(false);
    setSearchOpen(false);
  }, [location.pathname, location.search]);

  useEffect(() => {
    const q = new URLSearchParams(location.search).get('q') ?? '';
    setQuery(q);
  }, [location.pathname, location.search]);

  useEffect(() => {
    let cancelled = false;
    const q = query.trim();
    if (q.length < 1) {
      setSites([]);
      setSearchLoading(false);
      return;
    }
    setSearchLoading(true);
    const timer = window.setTimeout(() => {
      api
        .config()
        .then((cfg) => {
          if (!cancelled) setSites(cfg.sites ?? []);
        })
        .catch(() => {
          if (!cancelled) setSites([]);
        })
        .finally(() => {
          if (!cancelled) setSearchLoading(false);
        });
    }, 180);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query]);

  const hits = useMemo(() => matchSites(sites, query, mode), [sites, query, mode]);
  const sitesPath = mode === 'ingress' ? '/sites/ingress' : '/sites';

  function goSearch(href?: string) {
    const target =
      href ??
      (query.trim()
        ? `${sitesPath}?q=${encodeURIComponent(query.trim())}`
        : sitesPath);
    setSearchOpen(false);
    navigate(target);
  }

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

        <div className="relative hidden max-w-sm flex-1 items-center md:flex" ref={searchRef}>
          <Search className="pointer-events-none absolute left-3 size-4 text-muted-foreground" />
          <input
            type="search"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setSearchOpen(true);
            }}
            onFocus={() => setSearchOpen(true)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                goSearch(hits[0]?.href);
              }
              if (e.key === 'Escape') {
                setSearchOpen(false);
                (e.target as HTMLInputElement).blur();
              }
            }}
            placeholder="Search sites, routes…"
            aria-label="Search sites"
            autoComplete="off"
            className="h-9 w-full rounded-md border border-border bg-card pl-9 pr-3 text-sm text-foreground placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
          {searchOpen && query.trim() ? (
            <div
              role="listbox"
              className="absolute left-0 right-0 top-full z-[60] mt-1.5 overflow-hidden rounded-lg border border-border bg-card shadow-md"
            >
              {searchLoading ? (
                <p className="px-3 py-2.5 text-sm text-muted-foreground">Searching…</p>
              ) : hits.length === 0 ? (
                <p className="px-3 py-2.5 text-sm text-muted-foreground">No sites match “{query.trim()}”</p>
              ) : (
                <ul className="max-h-72 overflow-auto py-1">
                  {hits.map((hit) => (
                    <li key={hit.href + hit.host}>
                      <button
                        type="button"
                        role="option"
                        className="flex w-full items-start gap-2.5 px-3 py-2 text-left text-sm hover:bg-accent hover:text-accent-foreground"
                        onClick={() => goSearch(hit.href)}
                      >
                        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-md border border-primary/25 bg-primary/10 text-primary">
                          <Globe className="size-3.5" />
                        </span>
                        <span className="min-w-0">
                          <span className="block truncate font-medium">{hit.host}</span>
                          <span className="block truncate text-xs text-muted-foreground">{hit.subtitle}</span>
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              <button
                type="button"
                className="flex w-full items-center border-t border-border px-3 py-2 text-left text-xs font-medium text-primary hover:bg-accent"
                onClick={() => goSearch()}
              >
                View all matches on Sites
              </button>
            </div>
          ) : null}
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
