import { Outlet, useLocation } from 'react-router-dom';
import { useEffect, useMemo, useState } from 'react';
import { useGatewayApiEnabled, useManagementInfo } from '@/context/ManagementContext';
import { useMode } from '@/context/ModeContext';
import { api } from '@/api/client';
import { getUsername, setUsername } from '@/auth';
import {
  createMenuItems,
  ingressNavSections,
  proxyNavSections,
} from '@/lib/nav';
import { cn } from '@/lib/utils';
import { Sidebar } from '@/components/console/sidebar';
import { Topbar } from '@/components/console/topbar';

const SIDEBAR_COLLAPSED_KEY = 'pertisk_sidebar_collapsed';

function getStoredSidebarCollapsed(): boolean {
  return localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === '1';
}

export function Layout({ onLogout, loading = false }: { onLogout: () => void; loading?: boolean }) {
  const location = useLocation();
  const mode = useMode();
  const gatewayApiEnabled = useGatewayApiEnabled();
  const management = useManagementInfo();
  const [username, setUser] = useState(getUsername() || 'admin');
  const [canChangePassword, setCanChangePassword] = useState(mode === 'proxy');
  const [sidebarCollapsed, setSidebarCollapsed] = useState(getStoredSidebarCollapsed);

  const sections = useMemo(
    () => (mode === 'ingress' ? ingressNavSections(gatewayApiEnabled) : proxyNavSections()),
    [mode, gatewayApiEnabled],
  );
  const createItems = useMemo(() => createMenuItems(mode), [mode]);
  const modeLabel = mode === 'ingress' ? 'ingress mode' : 'proxy mode';

  useEffect(() => {
    if (mode === 'ingress' && location.pathname === '/sites') {
      window.location.replace('/sites/ingress');
    }
  }, [mode, location.pathname]);

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

  const sidebarFooter =
    mode === 'ingress' && management?.ingress_class ? (
      <p className="truncate text-xs text-muted-foreground">
        Class: {management.ingress_class}
        {gatewayApiEnabled && management.gateway_class ? ` · GW: ${management.gateway_class}` : null}
      </p>
    ) : undefined;

  function toggleSidebarCollapsed() {
    setSidebarCollapsed((prev) => {
      const next = !prev;
      localStorage.setItem(SIDEBAR_COLLAPSED_KEY, next ? '1' : '0');
      return next;
    });
  }

  return (
    <div className="flex min-h-screen bg-background text-foreground">
      <aside
        className={cn(
          'hidden shrink-0 border-r border-sidebar-border transition-[width] duration-200 ease-out lg:block',
          sidebarCollapsed ? 'w-16' : 'w-64',
        )}
      >
        <div className="sticky top-0 h-screen">
          <Sidebar
            sections={sections}
            version={management?.version}
            hostname={management?.hostname}
            footer={sidebarFooter}
            collapsed={sidebarCollapsed}
            onToggleCollapse={toggleSidebarCollapsed}
          />
        </div>
      </aside>
      <div className="flex min-w-0 flex-1 flex-col">
        <Topbar
          sections={sections}
          version={management?.version}
          hostname={management?.hostname}
          mode={mode ?? 'proxy'}
          modeLabel={modeLabel}
          username={username}
          createItems={createItems}
          canChangePassword={canChangePassword}
          sidebarFooter={sidebarFooter}
          onLogout={onLogout}
        />
        <main className="flex-1 px-4 py-6 sm:px-6 lg:px-8">
          <div className="flex w-full flex-col gap-6">
            {loading ? <p className="text-sm text-muted-foreground">Loading…</p> : <Outlet />}
          </div>
        </main>
      </div>
    </div>
  );
}
