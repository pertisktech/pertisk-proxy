import { useEffect, useState } from 'react';
import { BrowserRouter, Navigate, Route, Routes } from 'react-router-dom';
import { Toaster } from 'sonner';
import { ThemeProvider } from '@/context/ThemeContext';
import { Layout } from '@/components/Layout';
import { Login } from '@/routes/Login';
import { Dashboard } from '@/routes/Dashboard';
import { RoutesPage } from '@/routes/RoutesPage';
import { Certificates } from '@/routes/Certificates';
import { Settings } from '@/routes/Settings';
import { api } from '@/api/client';
import { clearToken, getToken } from '@/auth';

function Protected({ children }: { children: React.ReactNode }) {
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
        setOk(check.authenticated);
      } catch {
        setOk(false);
      }
    });
  }, []);

  if (ok === null) return <div className="flex min-h-screen items-center justify-center text-text-secondary">Loading…</div>;
  if (!ok) return <Navigate to="/login" replace />;
  return <>{children}</>;
}

export default function App() {
  function logout() {
    clearToken();
    window.location.href = '/login';
  }

  return (
    <ThemeProvider>
      <BrowserRouter
        future={{
          v7_startTransition: true,
          v7_relativeSplatPath: true,
        }}
      >
        <Routes>
          <Route path="/login" element={<Login />} />
          <Route
            element={
              <Protected>
                <Layout onLogout={logout} />
              </Protected>
            }
          >
            <Route index element={<Dashboard />} />
            <Route path="routes" element={<RoutesPage />} />
            <Route path="certificates" element={<Certificates />} />
            <Route path="settings" element={<Settings />} />
          </Route>
          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </BrowserRouter>
      <Toaster theme="dark" richColors position="top-right" />
    </ThemeProvider>
  );
}
