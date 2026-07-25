import { useCallback, useEffect, useState } from 'react';
import { BrowserRouter, Navigate, Outlet, Route, Routes } from 'react-router-dom';
import { Toaster } from 'sonner';
import { ThemeProvider, useTheme } from '@/context/ThemeContext';
import { ModeContext, type ApiMode } from '@/context/ModeContext';
import { ManagementContext } from '@/context/ManagementContext';
import { Layout } from '@/components/Layout';
import { Login } from '@/routes/Login';
import { Dashboard } from '@/routes/Dashboard';
import { Certificates } from '@/routes/Certificates';
import { Sites } from '@/routes/Sites';
import { K8sSites } from '@/routes/K8sSites';
import { Gateways } from '@/routes/Gateways';
import { DnsProviders } from '@/routes/DnsProviders';
import { Logs } from '@/routes/Logs';
import { Metrics } from '@/routes/Metrics';
import { RouteMap } from '@/routes/RouteMap';
import { AccessLists } from '@/routes/AccessLists';
import { WafPolicies } from '@/routes/WafPolicies';
import { Settings } from '@/routes/Settings';
import { Profile } from '@/routes/Profile';
import { Backup } from '@/routes/Backup';
import { api, type ManagementInfo } from '@/api/client';
import { clearToken, getToken, setUsername } from '@/auth';

function AuthGate({
  onAuthed,
  onLogout,
}: {
  onAuthed?: () => void;
  onLogout: () => void;
}) {
  const [ok, setOk] = useState<boolean | null>(null);

  useEffect(() => {
    api.authConfig().then(async (cfg) => {
      if (!cfg.auth_required) {
        setOk(true);
        return;
      }
      if (!getToken()) {
        setOk(false);
        return;
      }
      try {
        const check = await api.authCheck();
        if (check.username) setUsername(check.username);
        setOk(check.authenticated);
      } catch {
        setOk(false);
      }
    });
  }, []);

  useEffect(() => {
    if (ok) onAuthed?.();
  }, [ok, onAuthed]);

  if (ok === false) return <Navigate to="/login" replace />;
  if (ok === null) return <Layout onLogout={onLogout} loading />;
  return <Outlet />;
}

function ThemedToaster() {
  const { isDark } = useTheme();
  return <Toaster theme={isDark ? 'dark' : 'light'} richColors position="top-right" />;
}

export default function App() {
  const [mode, setMode] = useState<ApiMode | undefined>(undefined);
  const [managementInfo, setManagementInfo] = useState<ManagementInfo | null>(null);

  const refreshManagement = useCallback(() => {
    api.management().then((info) => {
      setManagementInfo(info);
      if (info.mode === 'proxy' || info.mode === 'ingress') setMode(info.mode);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    api.authConfig().then((cfg) => {
      if (!cfg.auth_required || getToken()) {
        refreshManagement();
      }
    }).catch(() => {});
  }, [refreshManagement]);

  function logout() {
    clearToken();
    window.location.href = '/login';
  }

  return (
    <ThemeProvider>
      <ModeContext.Provider value={mode}>
        <ManagementContext.Provider value={managementInfo}>
          <BrowserRouter
            future={{
              v7_startTransition: true,
              v7_relativeSplatPath: true,
            }}
          >
            <Routes>
              <Route path="/login" element={<Login />} />
              <Route element={<AuthGate onAuthed={refreshManagement} onLogout={logout} />}>
                <Route element={<Layout onLogout={logout} />}>
                  <Route index element={<Dashboard />} />
                  <Route path="sites" element={<Sites />} />
                  <Route path="sites/ingress" element={<K8sSites k8sPageKind="ingress" />} />
                  <Route path="sites/gateway" element={<Navigate to="/sites/gateway/gateways" replace />} />
                  <Route path="sites/gateway/gateways" element={<Gateways />} />
                  <Route path="sites/gateway/sites" element={<K8sSites k8sPageKind="gateway" />} />
                  <Route path="certificates" element={<Certificates />} />
                  <Route path="dns-providers" element={<DnsProviders />} />
                  <Route path="logs" element={<Logs />} />
                  <Route path="metrics" element={<Metrics />} />
                  <Route path="route-map" element={<RouteMap />} />
                  <Route path="access-lists" element={<AccessLists />} />
                  <Route path="waf" element={<WafPolicies />} />
                  <Route path="backup" element={<Backup />} />
                  <Route path="settings" element={<Settings />} />
                  <Route path="profile" element={<Profile />} />
                </Route>
              </Route>
              <Route path="*" element={<Navigate to="/" replace />} />
            </Routes>
          </BrowserRouter>
        </ManagementContext.Provider>
      </ModeContext.Provider>
      <ThemedToaster />
    </ThemeProvider>
  );
}
