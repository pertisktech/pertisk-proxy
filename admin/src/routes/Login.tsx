import { FormEvent, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Eye, EyeOff, Lock, Moon, Sun, User } from 'lucide-react';
import { api } from '@/api/client';
import { Checkbox } from '@/components/Checkbox';
import {
  clearToken,
  getRememberedCredentials,
  getToken,
  isRememberEnabled,
  setRememberCredentials,
  setToken,
} from '@/auth';
import { useTheme } from '@/context/ThemeContext';
import { Logo } from '@/components/Logo';
import styles from './Login.module.css';

export function Login() {
  const navigate = useNavigate();
  const { toggleTheme, isDark } = useTheme();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [remember, setRemember] = useState(isRememberEnabled());
  const [showPassword, setShowPassword] = useState(false);
  const [authRequired, setAuthRequired] = useState(true);
  const [version, setVersion] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    document.documentElement.classList.add('auth-route');
    return () => document.documentElement.classList.remove('auth-route');
  }, []);

  useEffect(() => {
    const saved = getRememberedCredentials();
    if (saved) {
      setUsername(saved.username);
      setPassword(saved.password);
      setRemember(true);
    }
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
      setRememberCredentials(username, password, remember);
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
          <div className={styles.brand}>
            <Logo className={styles.brandLogo} alt="" />
            <div>
              <h1 className={styles.title}>Pertisk-Proxy</h1>
              <p className={styles.subtitle}>Management console</p>
              {version ? <p className={styles.version}>v{version}</p> : null}
            </div>
          </div>

          {authRequired ? (
            <form onSubmit={onSubmit} className={styles.form}>
              <div className={styles.field}>
                <label className={styles.fieldLabel} htmlFor="login-username">
                  Username
                </label>
                <div className={styles.inputWrap}>
                  <User size={16} className={styles.inputIcon} aria-hidden />
                  <input
                    id="login-username"
                    type="text"
                    value={username}
                    className={styles.input}
                    onChange={(e) => setUsername(e.target.value)}
                    autoComplete="username"
                    autoFocus
                  />
                </div>
              </div>

              <div className={styles.field}>
                <label className={styles.fieldLabel} htmlFor="login-password">
                  Password
                </label>
                <div className={styles.inputWrap}>
                  <Lock size={16} className={styles.inputIcon} aria-hidden />
                  <input
                    id="login-password"
                    type={showPassword ? 'text' : 'password'}
                    value={password}
                    className={`${styles.input} ${styles.inputWithToggle}`}
                    onChange={(e) => setPassword(e.target.value)}
                    autoComplete="current-password"
                  />
                  <button
                    type="button"
                    className={styles.togglePassword}
                    onClick={() => setShowPassword((v) => !v)}
                    aria-label={showPassword ? 'Hide password' : 'Show password'}
                  >
                    {showPassword ? <EyeOff size={16} /> : <Eye size={16} />}
                  </button>
                </div>
              </div>

              <div className={styles.formRow}>
                <Checkbox
                  checked={remember}
                  onChange={setRemember}
                  label="Remember password"
                />
              </div>

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
