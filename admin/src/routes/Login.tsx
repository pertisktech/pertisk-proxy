import { FormEvent, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Moon, Sun } from 'lucide-react';
import { api } from '@/api/client';
import { setToken, getToken, clearToken } from '@/auth';
import { useTheme } from '@/context/ThemeContext';

export function Login() {
  const navigate = useNavigate();
  const { toggleTheme, isDark } = useTheme();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [authRequired, setAuthRequired] = useState(true);
  const [version, setVersion] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    api.version().then((v) => setVersion(v.version));
    api.authConfig().then((c) => {
      setAuthRequired(c.auth_required);
      if (!c.auth_required) navigate('/');
    });
    if (getToken()) {
      api.authCheck().then((c) => {
        if (c.authenticated) {
          navigate('/');
        } else {
          clearToken();
        }
      }).catch(() => clearToken());
    }
  }, [navigate]);

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setLoading(true);
    setError('');
    try {
      const res = await api.login(password, username);
      if (res.token) setToken(res.token);
      navigate('/');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Login failed');
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="relative flex min-h-dvh items-center justify-center bg-bg p-4">
      <button
        type="button"
        onClick={toggleTheme}
        className="absolute right-4 top-4 inline-flex items-center gap-2 rounded-md border border-border px-3 py-1.5 text-sm text-text-secondary hover:bg-hover hover:text-text"
        title="Toggle theme"
      >
        {isDark ? <Sun size={16} /> : <Moon size={16} />}
        {isDark ? 'Light' : 'Dark'}
      </button>
      <div className="w-full max-w-md rounded-xl border border-border bg-surface p-8 shadow-md">
        <div className="mb-6 text-center">
          <h1 className="text-2xl font-semibold">Pertisk-Proxy</h1>
          <p className="mt-1 text-sm text-text-secondary">Management console</p>
          {version ? (
            <p className="mt-1 font-mono text-xs text-muted">v{version}</p>
          ) : null}
        </div>
        {authRequired ? (
          <form onSubmit={onSubmit} className="space-y-4">
            <div>
              <label className="mb-1 block text-sm text-text-secondary">Username</label>
              <input
                type="text"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                autoComplete="username"
                className="w-full rounded-md border border-border bg-bg px-3 py-2 outline-none focus:border-primary"
                autoFocus
              />
            </div>
            <div>
              <label className="mb-1 block text-sm text-text-secondary">Password</label>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete="current-password"
                className="w-full rounded-md border border-border bg-bg px-3 py-2 outline-none focus:border-primary"
              />
            </div>
            {error ? <p className="text-sm text-red-r1">{error}</p> : null}
            <button
              type="submit"
              disabled={loading}
              className="w-full rounded-md bg-primary px-4 py-2 font-medium text-white hover:opacity-90 disabled:opacity-50"
            >
              {loading ? 'Signing in…' : 'Sign in'}
            </button>
          </form>
        ) : (
          <p className="text-center text-sm text-text-secondary">Auth disabled — redirecting…</p>
        )}
      </div>
    </div>
  );
}
