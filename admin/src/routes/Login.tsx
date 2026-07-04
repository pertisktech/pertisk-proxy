import { FormEvent, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Moon, Sun } from 'lucide-react';
import { api } from '@/api/client';
import { setToken, getToken, clearToken } from '@/auth';
import { useTheme } from '@/context/ThemeContext';
import styles from './Login.module.css';

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
    document.documentElement.classList.add('auth-route');
    return () => document.documentElement.classList.remove('auth-route');
  }, []);

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
    <div className={styles.page}>
      <header className={styles.toolbar}>
        <button
          type="button"
          onClick={toggleTheme}
          className={styles.themeToggle}
          title="Toggle theme"
        >
          {isDark ? <Sun size={16} /> : <Moon size={16} />}
          {isDark ? 'Light' : 'Dark'}
        </button>
      </header>

      <div className={styles.content}>
        <div className={styles.card}>
          <div className={styles.cardHeader}>
            <h1 className={styles.title}>Pertisk-Proxy</h1>
            <p className={styles.subtitle}>Management console</p>
            {version ? <p className={styles.version}>v{version}</p> : null}
          </div>

          {authRequired ? (
            <form onSubmit={onSubmit} className={styles.form}>
              <label className={styles.label}>
                Username
                <input
                  type="text"
                  value={username}
                  className={styles.input}
                  onChange={(e) => setUsername(e.target.value)}
                  autoComplete="username"
                  autoFocus
                />
              </label>
              <label className={styles.label}>
                Password
                <input
                  type="password"
                  value={password}
                  className={styles.input}
                  onChange={(e) => setPassword(e.target.value)}
                  autoComplete="current-password"
                />
              </label>
              {error ? <p className={styles.error}>{error}</p> : null}
              <button type="submit" disabled={loading} className={styles.button}>
                {loading ? 'Signing in…' : 'Sign in'}
              </button>
            </form>
          ) : (
            <p className={styles.hint}>Auth disabled — redirecting…</p>
          )}
        </div>
      </div>
    </div>
  );
}
