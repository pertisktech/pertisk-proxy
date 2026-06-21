import { FormEvent, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { api } from '@/api/client';
import { setToken, getToken } from '@/auth';

export function Login() {
  const navigate = useNavigate();
  const [password, setPassword] = useState('');
  const [authRequired, setAuthRequired] = useState(true);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    api.authConfig().then((c) => {
      setAuthRequired(c.auth_required);
      if (!c.auth_required) navigate('/');
    });
    if (getToken()) {
      api.authCheck().then((c) => {
        if (c.authenticated) navigate('/');
      });
    }
  }, [navigate]);

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setLoading(true);
    setError('');
    try {
      const res = await api.login(password);
      if (res.token) setToken(res.token);
      navigate('/');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Login failed');
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-bg p-4">
      <div className="w-full max-w-md rounded-xl border border-border bg-surface p-8 shadow-md">
        <div className="mb-6 text-center">
          <div className="mx-auto mb-3 flex h-12 w-12 items-center justify-center rounded-lg bg-primary/20 text-xl font-bold text-primary">
            P
          </div>
          <h1 className="text-2xl font-semibold">Pertisk-Proxy</h1>
          <p className="mt-1 text-sm text-text-secondary">Management console</p>
        </div>
        {authRequired ? (
          <form onSubmit={onSubmit} className="space-y-4">
            <div>
              <label className="mb-1 block text-sm text-text-secondary">Password</label>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="w-full rounded-md border border-border bg-bg px-3 py-2 outline-none focus:border-primary"
                autoFocus
              />
            </div>
            {error ? <p className="text-sm text-red-r1">{error}</p> : null}
            <button
              type="submit"
              disabled={loading}
              className="w-full rounded-md bg-primary px-4 py-2 font-medium text-bg hover:opacity-90 disabled:opacity-50"
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
