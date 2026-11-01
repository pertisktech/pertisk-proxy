import { useEffect, useMemo, useState, FormEvent } from 'react';
import { Link, useLocation, useNavigate } from 'react-router-dom';
import { Info, Lock, Mail, Cloud, Shield, Zap, Gauge, Terminal } from 'lucide-react';
import { toast } from 'sonner';
import { api, type ManagementInfo, type ProxyConfig } from '@/api/client';
import { Card } from '@/components/Card';
import { SmtpSettingsPanel } from '@/components/SmtpSettingsPanel';
import { S3SettingsPanel } from '@/components/S3SettingsPanel';
import { EnvironmentPanel } from '@/components/settings/EnvironmentPanel';
import { PerformancePanel } from '@/components/settings/PerformancePanel';
import { useManagementInfo } from '@/context/ManagementContext';
import { useMode } from '@/context/ModeContext';
import { cn } from '@/utils';

type SettingsTab =
  | 'general'
  | 'certificates'
  | 'edge'
  | 'notifications'
  | 'storage'
  | 'environment'
  | 'performance';

function tabFromHash(hash: string, proxyOnly: boolean): SettingsTab {
  const id = hash.replace(/^#/, '').toLowerCase();
  if (id === 'performance' || id === 'performance-tuning') return 'performance';
  if (
    id === 'environment' ||
    id === 'env' ||
    id === 'configuration' ||
    id === 'configuration-reference'
  ) {
    return 'environment';
  }
  if (proxyOnly && (id === 'certificates' || id === 'tls' || id === 'acme')) return 'certificates';
  if (proxyOnly && id === 'edge') return 'edge';
  if (proxyOnly && (id === 'notifications' || id === 'email' || id === 'smtp')) {
    return 'notifications';
  }
  if (proxyOnly && (id === 'storage' || id === 's3' || id === 'backup')) return 'storage';
  return 'general';
}

export function Settings() {
  const mode = useMode();
  const sharedInfo = useManagementInfo();
  const location = useLocation();
  const navigate = useNavigate();
  const [version, setVersion] = useState('');
  const [authRequired, setAuthRequired] = useState(false);
  const [management, setManagement] = useState<ManagementInfo | null>(null);
  const [error, setError] = useState('');
  const [config, setConfig] = useState<ProxyConfig | null>(null);
  const [acmeEmail, setAcmeEmail] = useState('');
  const [acmeSaving, setAcmeSaving] = useState(false);
  const [configError, setConfigError] = useState('');
  const [cacheMb, setCacheMb] = useState('0');
  const [cacheTtl, setCacheTtl] = useState('60');
  const [tlsGrade, setTlsGrade] = useState<'high' | 'medium' | 'unsafe'>('medium');
  const [consulAddress, setConsulAddress] = useState('');
  const [consulToken, setConsulToken] = useState('');
  const [consulDc, setConsulDc] = useState('');
  const [jwtSecret, setJwtSecret] = useState('');
  const [edgeSaving, setEdgeSaving] = useState(false);

  useEffect(() => {
    Promise.all([api.version(), api.authConfig(), api.management()])
      .then(([v, auth, info]) => {
        setVersion(v.version);
        setAuthRequired(auth.auth_required);
        setManagement(info);
      })
      .catch((e) => setError(e instanceof Error ? e.message : 'Failed to load settings'));
  }, []);

  useEffect(() => {
    if (mode === 'ingress') return;
    let cancelled = false;
    setConfigError('');
    api
      .config()
      .then((cfg) => {
        if (cancelled) return;
        setConfig(cfg);
        setAcmeEmail(cfg.acme_email?.trim() ?? '');
        setCacheMb(String(cfg.cache_size_mb ?? 0));
        setCacheTtl(String(cfg.cache_ttl_secs ?? 60));
        setTlsGrade(cfg.tls_grade ?? 'medium');
        setConsulAddress(cfg.consul?.address ?? '');
        setConsulToken(cfg.consul?.token ?? '');
        setConsulDc(cfg.consul?.datacenter ?? '');
        setJwtSecret(cfg.jwt_secret ?? '');
      })
      .catch((e) => {
        if (cancelled) return;
        setConfigError(e instanceof Error ? e.message : 'Failed to load config');
      });
    return () => {
      cancelled = true;
    };
  }, [mode]);

  async function saveAcmeEmail(e: FormEvent) {
    e.preventDefault();
    const email = acmeEmail.trim();
    if (email && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
      toast.error('Enter a valid email address');
      return;
    }
    setAcmeSaving(true);
    setConfigError('');
    try {
      const latest = await api.config();
      const next = { ...latest, acme_email: email || null };
      await api.saveConfig(next);
      setConfig(next);
      toast.success(email ? 'Contact email saved' : 'Contact email cleared');
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to save';
      setConfigError(msg);
      toast.error(msg);
    } finally {
      setAcmeSaving(false);
    }
  }

  async function saveEdge(e: FormEvent) {
    e.preventDefault();
    setEdgeSaving(true);
    setConfigError('');
    try {
      const latest = await api.config();
      const address = consulAddress.trim();
      const next = {
        ...latest,
        cache_size_mb: Math.max(0, Number(cacheMb) || 0),
        cache_ttl_secs: Math.max(1, Number(cacheTtl) || 60),
        tls_grade: tlsGrade,
        consul: address
          ? {
              address,
              token: consulToken.trim() || undefined,
              datacenter: consulDc.trim() || undefined,
            }
          : null,
        jwt_secret: jwtSecret.trim() || null,
      };
      await api.saveConfig(next);
      setConfig(next);
      toast.success('Edge settings saved. TLS grade applies on the next process start.');
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to save';
      setConfigError(msg);
      toast.error(msg);
    } finally {
      setEdgeSaving(false);
    }
  }

  const info = management ?? sharedInfo;
  const isIngress = (mode ?? info?.mode) === 'ingress';
  const showProxyConfig = !isIngress;

  const tab = useMemo(
    () => tabFromHash(location.hash, showProxyConfig),
    [location.hash, showProxyConfig],
  );

  function setTab(next: SettingsTab) {
    const hash =
      next === 'general'
        ? ''
        : next === 'performance'
          ? '#performance-tuning'
          : next === 'environment'
            ? '#environment'
            : `#${next}`;
    navigate({ pathname: '/settings', hash }, { replace: true });
  }

  const tabs = [
    { id: 'general' as const, label: 'General', icon: Info },
    ...(showProxyConfig
      ? [
          { id: 'certificates' as const, label: 'Certificates', icon: Lock },
          { id: 'edge' as const, label: 'Edge', icon: Gauge },
          { id: 'notifications' as const, label: 'Notifications', icon: Mail },
          { id: 'storage' as const, label: 'Storage', icon: Cloud },
        ]
      : []),
    { id: 'environment' as const, label: 'Environment', icon: Terminal },
    { id: 'performance' as const, label: 'Performance', icon: Zap },
  ];

  return (
    <div className="max-w-5xl space-y-6">
      {error ? <p className="text-red-r1">{error}</p> : null}

      <div className="tab-bar w-full overflow-x-auto sm:w-auto" role="tablist" aria-label="Settings sections">
        {tabs.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={tab === id}
            onClick={() => setTab(id)}
            className={cn('tab-item', tab === id && 'active')}
          >
            <Icon size={15} />
            {label}
          </button>
        ))}
      </div>

      {tab === 'general' ? (
        <div className="space-y-6">
          <Card>
            <h2 className="mb-3 text-lg font-semibold">About</h2>
            <p className="text-text-secondary">
              Pertisk-Proxy management UI{isIngress ? ' (ingress mode)' : ' (proxy mode)'}
            </p>
            <p className="mt-2 font-mono text-sm">Version {version || info?.version || '…'}</p>
          </Card>
          <Card>
            <div className="mb-3 flex items-center gap-2">
              <Shield size={18} className="text-text-secondary" />
              <h2 className="text-lg font-semibold">Security</h2>
            </div>
            <p className="text-sm text-text-secondary">
              Management API auth: {authRequired ? 'enabled (username/password)' : 'disabled'}
            </p>
            {isIngress ? (
              <p className="mt-2 text-sm text-text-secondary">
                Credentials come from the Helm auth Secret (
                <code className="rounded bg-bg px-1 text-text">PERTISK_ADMIN</code> /{' '}
                <code className="rounded bg-bg px-1 text-text">PERTISK_PASSWORD</code>).
              </p>
            ) : (
              <p className="mt-2 text-sm text-text-secondary">
                Default credentials on first start:{' '}
                <code className="rounded bg-bg px-1 text-text">admin</code> /{' '}
                <code className="rounded bg-bg px-1 text-text">admin</code> (stored in SQLite).
                Change the password after first login from{' '}
                <Link to="/profile" className="text-primary underline-offset-2 hover:underline">
                  Profile
                </Link>
                .
              </p>
            )}
          </Card>
        </div>
      ) : null}

      {tab === 'certificates' && showProxyConfig ? (
        <Card>
          <h2 className="mb-1 text-lg font-semibold">Let&apos;s Encrypt</h2>
          <p className="mb-4 text-sm text-text-secondary">
            Default contact email for ACME certificate issuance. Used when adding a site with
            Generate SSL; each site can override it.
          </p>
          {configError ? <p className="mb-3 text-sm text-red-r1">{configError}</p> : null}
          <form onSubmit={saveAcmeEmail} className="flex flex-col gap-3 sm:flex-row sm:items-end">
            <label className="block min-w-0 flex-1 text-sm text-text-secondary">
              Contact email (Let&apos;s Encrypt)
              <input
                type="email"
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={acmeEmail}
                onChange={(e) => setAcmeEmail(e.target.value)}
                placeholder="you@yourdomain.com"
                autoComplete="email"
              />
            </label>
            <button
              type="submit"
              disabled={acmeSaving}
              className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-bg hover:opacity-90 disabled:opacity-50"
            >
              {acmeSaving ? 'Saving…' : 'Save'}
            </button>
          </form>
          {config?.acme_email?.trim() ? (
            <p className="mt-3 text-xs text-text-secondary">
              Saved default:{' '}
              <span className="font-mono text-text">{config.acme_email.trim()}</span>
            </p>
          ) : (
            <p className="mt-3 text-xs text-text-secondary">No default saved yet.</p>
          )}
        </Card>
      ) : null}

      {tab === 'edge' && showProxyConfig ? (
        <Card>
          <h2 className="mb-1 text-lg font-semibold">Edge</h2>
          <p className="mb-4 text-sm text-text-secondary">
            Shared cache, TLS cipher grade, Consul discovery, and the secret used by{' '}
            <code className="font-mono">POST /api/jwt</code>. Per-site auth, rate limits, file and
            auto-TLS upstreams are set on each site.
          </p>
          {configError ? <p className="mb-3 text-sm text-red-r1">{configError}</p> : null}
          <form onSubmit={saveEdge} className="grid gap-4 sm:grid-cols-2">
            <label className="text-sm text-text-secondary">
              Cache size (MB)
              <input
                type="number"
                min={0}
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={cacheMb}
                onChange={(e) => setCacheMb(e.target.value)}
              />
              <span className="mt-1 block text-xs">0 disables the response cache.</span>
            </label>
            <label className="text-sm text-text-secondary">
              Cache TTL (seconds)
              <input
                type="number"
                min={1}
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={cacheTtl}
                onChange={(e) => setCacheTtl(e.target.value)}
              />
            </label>
            <label className="text-sm text-text-secondary">
              TLS grade
              <select
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={tlsGrade}
                onChange={(e) => setTlsGrade(e.target.value as 'high' | 'medium' | 'unsafe')}
              >
                <option value="high">High</option>
                <option value="medium">Medium</option>
                <option value="unsafe">Unsafe</option>
              </select>
              <span className="mt-1 block text-xs">Takes effect after pertisk-proxy restarts.</span>
            </label>
            <label className="text-sm text-text-secondary">
              JWT secret
              <input
                type="password"
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={jwtSecret}
                onChange={(e) => setJwtSecret(e.target.value)}
                autoComplete="new-password"
                placeholder="optional master secret"
              />
            </label>
            <label className="text-sm text-text-secondary sm:col-span-2">
              Consul address
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={consulAddress}
                onChange={(e) => setConsulAddress(e.target.value)}
                placeholder="http://127.0.0.1:8500"
              />
            </label>
            <label className="text-sm text-text-secondary">
              Consul token
              <input
                type="password"
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={consulToken}
                onChange={(e) => setConsulToken(e.target.value)}
                autoComplete="new-password"
              />
            </label>
            <label className="text-sm text-text-secondary">
              Consul datacenter
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={consulDc}
                onChange={(e) => setConsulDc(e.target.value)}
                placeholder="optional"
              />
            </label>
            <div className="sm:col-span-2">
              <button
                type="submit"
                disabled={edgeSaving}
                className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-bg hover:opacity-90 disabled:opacity-50"
              >
                {edgeSaving ? 'Saving…' : 'Save edge settings'}
              </button>
            </div>
          </form>
        </Card>
      ) : null}

      {tab === 'notifications' && showProxyConfig ? <SmtpSettingsPanel /> : null}

      {tab === 'storage' && showProxyConfig ? <S3SettingsPanel /> : null}

      {tab === 'environment' ? <EnvironmentPanel isIngress={isIngress} /> : null}

      {tab === 'performance' ? <PerformancePanel info={info} isIngress={isIngress} /> : null}
    </div>
  );
}
