import { useEffect, useMemo, useState, FormEvent } from 'react';
import { Link, useLocation, useNavigate } from 'react-router-dom';
import { Info, Lock, Mail, Shield, Zap } from 'lucide-react';
import { toast } from 'sonner';
import { api, type ManagementInfo, type ProxyConfig } from '@/api/client';
import { Card } from '@/components/Card';
import { SmtpSettingsPanel } from '@/components/SmtpSettingsPanel';
import { useManagementInfo } from '@/context/ManagementContext';
import { useMode } from '@/context/ModeContext';
import { cn } from '@/utils';

type SettingsTab = 'general' | 'certificates' | 'notifications' | 'performance';

function formatBytes(bytes: number | null | undefined) {
  if (bytes == null) return 'Unavailable';
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(bytes % (1024 * 1024) === 0 ? 0 : 1)} MiB`;
}

function SettingRow({
  label,
  value,
  recommended,
}: {
  label: string;
  value: string | number;
  recommended?: boolean;
}) {
  return (
    <div className="flex flex-col gap-1 border-b border-border py-3 last:border-b-0 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
      <dt className="text-sm text-text-secondary">{label}</dt>
      <dd className="flex items-center gap-2 text-right font-mono text-sm">
        <span>{value}</span>
        {recommended != null ? (
          <span
            className={`rounded-full px-2 py-0.5 font-sans text-xs ${
              recommended ? 'bg-green-g1/15 text-green-g1' : 'bg-yellow-y1/15 text-yellow-y1'
            }`}
          >
            {recommended ? 'ready' : 'review'}
          </span>
        ) : null}
      </dd>
    </div>
  );
}

function EnvVar({ name, description }: { name: string; description: string }) {
  return (
    <li className="rounded-lg border border-border bg-surface-elevated p-3">
      <code className="text-sm text-primary">{name}</code>
      <p className="mt-1 text-xs text-text-secondary">{description}</p>
    </li>
  );
}

function ProxyDeploymentGuide() {
  return (
    <Card>
      <h2 className="mb-3 text-lg font-semibold">Deployment guide</h2>
      <ol className="list-decimal space-y-3 pl-5 text-sm text-text-secondary">
        <li>
          Install the RPM/DEB; it installs and applies{' '}
          <code>/etc/sysctl.d/99-pertisk-proxy.conf</code>.
        </li>
        <li>
          Set <code>PERTISK_PROXY_MODE=performance</code> in the service environment file.
        </li>
        <li>
          For dedicated hosts, copy the packaged CPU-affinity example into a systemd service drop-in
          and choose valid CPU IDs.
        </li>
        <li>
          Run <code>systemctl daemon-reload &amp;&amp; systemctl restart pertisk-proxy</code>.
        </li>
        <li>
          Return here and confirm all effective values. Benchmark from the same network region and
          watch p95/p99, CPU, RSS, and errors.
        </li>
      </ol>
      <pre className="mt-4 overflow-x-auto rounded-lg border border-border bg-bg p-4 text-xs text-text-secondary">
{`mkdir -p /etc/systemd/system/pertisk-proxy.service.d
cp /usr/share/pertisk-proxy/cpu-affinity.conf.example \\
  /etc/systemd/system/pertisk-proxy.service.d/cpu-affinity.conf
systemctl daemon-reload
systemctl restart pertisk-proxy`}
      </pre>
    </Card>
  );
}

function IngressDeploymentGuide() {
  return (
    <Card>
      <h2 className="mb-3 text-lg font-semibold">Deployment guide</h2>
      <ol className="list-decimal space-y-3 pl-5 text-sm text-text-secondary">
        <li>
          Set Helm values for ingress tuning in your overlay (for example{' '}
          <code>deploy/helm/pertisk-ingress/285/values.yaml</code>).
        </li>
        <li>
          Ensure <code>runtime.mode=performance</code> and give the pod enough CPU (at least{' '}
          <code>2</code> cores) so auto worker sizing is not stuck at 1.
        </li>
        <li>
          Configure <code>runtime.listenerTasks</code>, <code>runtime.tcpListenBacklog</code>,{' '}
          <code>upstream.*</code>, and <code>h3Upstream.*</code> / <code>http3.*</code> as needed.
        </li>
        <li>
          Redeploy with Helm (for 285h: <code>VERSION=x.y.z ./deploy/285h.sh</code>).
        </li>
        <li>
          Confirm effective values here. Benchmark from the same cluster/region and watch p95/p99,
          CPU, RSS, and errors.
        </li>
      </ol>
      <pre className="mt-4 overflow-x-auto rounded-lg border border-border bg-bg p-4 text-xs text-text-secondary">
{`# Example: pin one replica for HTTP/3 benchmarks
REPLICA_COUNT=1 VERSION=0.1.46 ./deploy/285h.sh

# Or scale Helm values directly
helm upgrade --install pertisk-proxy-ingress ./deploy/helm/pertisk-ingress \\
  -n pertisk-proxy -f ./deploy/helm/pertisk-ingress/285/values.yaml \\
  --set image.tag=0.1.46 --set runtime.mode=performance`}
      </pre>
    </Card>
  );
}

function tabFromHash(hash: string, proxyOnly: boolean): SettingsTab {
  const id = hash.replace(/^#/, '').toLowerCase();
  if (id === 'performance' || id === 'performance-tuning') return 'performance';
  if (proxyOnly && (id === 'certificates' || id === 'tls' || id === 'acme')) return 'certificates';
  if (proxyOnly && (id === 'notifications' || id === 'email' || id === 'smtp')) {
    return 'notifications';
  }
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

  const info = management ?? sharedInfo;
  const isIngress = (mode ?? info?.mode) === 'ingress';
  const showProxyConfig = !isIngress;
  const tuning = info?.tuning;
  const kernel = tuning?.kernel;
  const quic = tuning?.effective_quic;
  const hasConfiguredHttp3 =
    info != null &&
    (info.http3.max_data != null ||
      info.http3.max_stream_data != null ||
      info.http3.max_streams_bidi != null ||
      info.http3.congestion_control != null);

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
          : `#${next}`;
    navigate({ pathname: '/settings', hash }, { replace: true });
  }

  const tabs = [
    { id: 'general' as const, label: 'General', icon: Info },
    ...(showProxyConfig
      ? [
          { id: 'certificates' as const, label: 'Certificates', icon: Lock },
          { id: 'notifications' as const, label: 'Notifications', icon: Mail },
        ]
      : []),
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
              <p className="mt-2 text-sm text-muted">
                Credentials come from the Helm auth Secret (
                <code className="rounded bg-bg px-1">PERTISK_ADMIN</code> /{' '}
                <code className="rounded bg-bg px-1">PERTISK_PASSWORD</code>).
              </p>
            ) : (
              <p className="mt-2 text-sm text-muted">
                Default credentials on first start: <code className="rounded bg-bg px-1">admin</code> /{' '}
                <code className="rounded bg-bg px-1">admin</code> (stored in SQLite). Change the
                password after first login from{' '}
                <Link to="/profile" className="text-primary underline-offset-2 hover:underline">
                  Profile
                </Link>
                .
              </p>
            )}
          </Card>
          <Card>
            <h2 className="mb-3 text-lg font-semibold">General environment</h2>
            <ul className="space-y-1 text-sm font-mono text-text-secondary">
              {isIngress ? (
                <>
                  <li>PERTISK_INGRESS_MODE (from Helm runtime.mode)</li>
                  <li>PERTISK_MANAGEMENT_ADDR (from Helm managementAddr)</li>
                  <li>PERTISK_INGRESS_CLASS / PERTISK_GATEWAY_CLASS</li>
                  <li>PERTISK_CPU_LIMIT_MILLICORES (from pod CPU limit)</li>
                </>
              ) : (
                <>
                  <li>PERTISK_DB_PATH (default ./data/proxy.sqlite)</li>
                  <li>PERTISK_MANAGEMENT_ADDR (default 127.0.0.1:9080)</li>
                  <li>ROUTES_CONFIG (optional one-time migration from legacy yaml)</li>
                  <li>PERTISK_ADMIN_UI_DEV_ORIGIN (Vite dev redirect)</li>
                </>
              )}
            </ul>
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
            <p className="mt-3 text-xs text-muted">No default saved yet.</p>
          )}
        </Card>
      ) : null}

      {tab === 'notifications' && showProxyConfig ? <SmtpSettingsPanel /> : null}

      {tab === 'performance' ? (
        <div id="performance-tuning" className="scroll-mt-6 space-y-6">
          <Card>
            <h2 className="mb-1 text-lg font-semibold">Performance tuning</h2>
            <p className="mb-4 text-sm text-text-secondary">
              Effective values reported by the running process.
              {isIngress
                ? ' Change Helm values and redeploy to update them.'
                : ' Environment and systemd changes require a restart.'}
            </p>
            {tuning ? (
              <div className="grid gap-6 lg:grid-cols-2">
                <div>
                  <h3 className="mb-2 font-medium">Runtime and I/O</h3>
                  <dl>
                    <SettingRow label="Requested mode" value={tuning.requested_mode} />
                    <SettingRow
                      label="Resolved mode"
                      value={tuning.resolved_mode}
                      recommended={tuning.resolved_mode === 'performance'}
                    />
                    <SettingRow label="Tokio workers" value={tuning.tokio_worker_threads} />
                    <SettingRow label="Blocking workers limit" value={tuning.max_blocking_threads} />
                    <SettingRow
                      label="Pingora service threads"
                      value={tuning.pingora_service_threads}
                    />
                    <SettingRow
                      label="Accept tasks per listener"
                      value={tuning.pingora_listener_tasks_per_fd}
                    />
                    <SettingRow label="HTTP/3 workers" value={tuning.h3_worker_threads} />
                    <SettingRow label="TCP listen backlog" value={tuning.tcp_listen_backlog} />
                  </dl>
                </div>
                <div>
                  <h3 className="mb-2 font-medium">Connections and HTTP/3</h3>
                  <dl>
                    <SettingRow label="HTTP/3 stack" value={tuning.h3_stack} />
                    <SettingRow label="UDP offload" value={tuning.udp_offload} />
                    <SettingRow
                      label="Pingora upstream keepalive pool"
                      value={tuning.pingora_upstream_keepalive_pool_size}
                    />
                    <SettingRow
                      label="H3 upstream idle / host"
                      value={tuning.h3_upstream_pool.max_idle_per_host}
                    />
                    <SettingRow
                      label="H3 pool idle timeout"
                      value={`${tuning.h3_upstream_pool.idle_timeout_secs}s`}
                    />
                    <SettingRow
                      label="H3 TCP keepalive"
                      value={`${tuning.h3_upstream_pool.tcp_keepalive_secs}s`}
                    />
                    <SettingRow
                      label="CPU affinity"
                      value={kernel?.cpu_affinity ?? 'Unavailable'}
                    />
                    <SettingRow
                      label="Open files limit"
                      value={kernel?.open_files_limit?.toLocaleString() ?? 'Unavailable'}
                      recommended={(kernel?.open_files_limit ?? 0) >= 1_048_576}
                    />
                  </dl>
                </div>
              </div>
            ) : (
              <p className="app-main-status">Loading effective tuning…</p>
            )}
          </Card>

          <Card>
            <h2 className="mb-1 text-lg font-semibold">Effective HTTP/3 / QUIC</h2>
            <p className="mb-4 text-sm text-text-secondary">
              Values the active <code className="font-mono">{tuning?.h3_stack ?? 'HTTP/3'}</code>{' '}
              stack actually uses. Default builds use Quinn and read these from environment
              variables, not route-level <code className="font-mono">http3</code> config.
            </p>
            {quic ? (
              <div className="grid gap-x-8 lg:grid-cols-2">
                <dl>
                  <SettingRow label="Source" value={quic.source} />
                  <SettingRow
                    label="Connection receive window"
                    value={formatBytes(quic.conn_receive_window)}
                  />
                  <SettingRow
                    label="Stream receive window"
                    value={formatBytes(quic.stream_receive_window)}
                  />
                  <SettingRow label="Bidirectional streams" value={quic.max_streams_bidi} />
                </dl>
                <dl>
                  <SettingRow label="Idle timeout" value={`${quic.idle_timeout_secs}s`} />
                  <SettingRow
                    label="Keepalive"
                    value={quic.keepalive_secs != null ? `${quic.keepalive_secs}s` : 'n/a'}
                  />
                  <SettingRow
                    label="UDP socket buffers"
                    value={formatBytes(quic.udp_buffer_bytes)}
                  />
                  <SettingRow
                    label="Congestion control"
                    value={quic.congestion_control ?? 'Quinn default'}
                  />
                </dl>
              </div>
            ) : (
              <p className="text-sm text-text-secondary">
                Effective QUIC details are unavailable for this build. Route-level{' '}
                <code className="font-mono">http3</code> options apply only when the tokio-quiche
                backend is compiled in.
              </p>
            )}
            {info ? (
              <p className="mt-4 text-xs text-muted">
                Stored route/config <code className="font-mono">http3</code> overrides
                {hasConfiguredHttp3 ? ' are present' : ' are unset'}; with Quinn they do not change
                the effective transport above.
              </p>
            ) : null}
          </Card>

          <Card>
            <h2 className="mb-1 text-lg font-semibold">Linux network tuning</h2>
            <p className="mb-4 text-sm text-text-secondary">
              Live kernel values read from <code className="font-mono">/proc/sys</code>.
              {isIngress
                ? ' In Kubernetes these usually come from the node image or a DaemonSet, not the container itself.'
                : ' “Review” means the value is below the packaged recommendation.'}
            </p>
            {kernel ? (
              <div className="grid gap-x-8 lg:grid-cols-2">
                <dl>
                  <SettingRow
                    label="Receive buffer ceiling"
                    value={formatBytes(kernel.rmem_max)}
                    recommended={(kernel.rmem_max ?? 0) >= 16 * 1024 * 1024}
                  />
                  <SettingRow
                    label="Send buffer ceiling"
                    value={formatBytes(kernel.wmem_max)}
                    recommended={(kernel.wmem_max ?? 0) >= 16 * 1024 * 1024}
                  />
                  <SettingRow
                    label="Socket accept backlog"
                    value={kernel.somaxconn ?? 'Unavailable'}
                    recommended={(kernel.somaxconn ?? 0) >= 8192}
                  />
                  <SettingRow
                    label="NIC receive backlog"
                    value={kernel.netdev_max_backlog ?? 'Unavailable'}
                    recommended={(kernel.netdev_max_backlog ?? 0) >= 16384}
                  />
                  <SettingRow
                    label="TCP SYN backlog"
                    value={kernel.tcp_max_syn_backlog ?? 'Unavailable'}
                    recommended={(kernel.tcp_max_syn_backlog ?? 0) >= 8192}
                  />
                </dl>
                <dl>
                  <SettingRow
                    label="TCP congestion control"
                    value={kernel.tcp_congestion_control ?? 'Unavailable'}
                    recommended={kernel.tcp_congestion_control === 'bbr'}
                  />
                  <SettingRow
                    label="Default qdisc"
                    value={kernel.default_qdisc ?? 'Unavailable'}
                    recommended={kernel.default_qdisc === 'fq'}
                  />
                  <SettingRow
                    label="Ephemeral port range"
                    value={kernel.ip_local_port_range ?? 'Unavailable'}
                  />
                  <SettingRow
                    label="TCP TIME_WAIT reuse"
                    value={kernel.tcp_tw_reuse ?? 'Unavailable'}
                  />
                </dl>
              </div>
            ) : (
              <p className="app-main-status">
                Kernel tuning is only available on supported Linux hosts.
              </p>
            )}
          </Card>

          <Card>
            <h2 className="mb-1 text-lg font-semibold">Configuration reference</h2>
            <p className="mb-4 text-sm text-text-secondary">
              {isIngress
                ? 'Set these in your Helm overlay (for example deploy/helm/pertisk-ingress/285/values.yaml), then redeploy.'
                : 'Put these values in /etc/pertisk-proxy/pertisk-proxy.conf, then restart the service.'}
            </p>
            <ul className="grid gap-3 md:grid-cols-2">
              {isIngress ? (
                <>
                  <EnvVar
                    name="runtime.mode=performance"
                    description="Helm value mapped to PERTISK_INGRESS_MODE."
                  />
                  <EnvVar
                    name="runtime.listenerTasks"
                    description="Mapped to PERTISK_PINGORA_LISTENER_TASKS."
                  />
                  <EnvVar
                    name="runtime.tcpListenBacklog"
                    description="Mapped to PERTISK_TCP_LISTEN_BACKLOG."
                  />
                  <EnvVar
                    name="runtime.pingoraThreads"
                    description="Optional override for PERTISK_PINGORA_THREADS."
                  />
                  <EnvVar
                    name="upstream.poolMaxIdlePerHost"
                    description="Mapped to PERTISK_UPSTREAM_POOL_MAX_IDLE_PER_HOST."
                  />
                  <EnvVar
                    name="h3Upstream.poolMaxIdlePerHost"
                    description="Mapped to PERTISK_H3_UPSTREAM_POOL_MAX_IDLE."
                  />
                  <EnvVar
                    name="http3.maxStreams"
                    description="Mapped to PERTISK_HTTP3_MAX_STREAMS."
                  />
                  <EnvVar
                    name="resources.limits.cpu"
                    description="Drives auto worker sizing via PERTISK_CPU_LIMIT_MILLICORES."
                  />
                </>
              ) : (
                <>
                  <EnvVar
                    name="PERTISK_PROXY_MODE=performance"
                    description="Enable CPU-scaled worker, listener, pool, and QUIC defaults."
                  />
                  <EnvVar
                    name="PERTISK_WORKER_THREADS"
                    description="Override Tokio worker count; normally leave unset to use available CPUs."
                  />
                  <EnvVar
                    name="PERTISK_PINGORA_THREADS"
                    description="Override HTTP/1 and HTTP/2 Pingora service threads."
                  />
                  <EnvVar
                    name="PERTISK_PINGORA_LISTENER_TASKS"
                    description="Parallel accept tasks per listener; performance default is 4."
                  />
                  <EnvVar
                    name="PERTISK_TCP_LISTEN_BACKLOG"
                    description="Application listen backlog; performance default is 8192."
                  />
                  <EnvVar
                    name="PERTISK_H3_UPSTREAM_POOL_MAX_IDLE"
                    description="Idle H3-to-upstream connections per host; performance default is 256."
                  />
                  <EnvVar
                    name="PERTISK_H3_UPSTREAM_POOL_IDLE_TIMEOUT_SECS"
                    description="How long idle upstream connections remain pooled."
                  />
                  <EnvVar
                    name="PERTISK_H3_UPSTREAM_TCP_KEEPALIVE_SECS"
                    description="TCP keepalive interval for H3-to-upstream connections."
                  />
                  <EnvVar
                    name="PERTISK_HTTP3_MAX_STREAMS"
                    description="Maximum concurrent QUIC streams."
                  />
                  <EnvVar
                    name="PERTISK_HTTP3_STREAM_RECEIVE_WINDOW"
                    description="QUIC per-stream receive window in bytes."
                  />
                  <EnvVar
                    name="PERTISK_HTTP3_CONN_RECEIVE_WINDOW"
                    description="QUIC connection receive window in bytes."
                  />
                  <EnvVar
                    name="PERTISK_HTTP3_CC_ALGORITHM"
                    description="QUIC congestion control for tokio-quiche: bbr, cubic, or reno."
                  />
                </>
              )}
            </ul>
          </Card>

          {isIngress ? <IngressDeploymentGuide /> : <ProxyDeploymentGuide />}
        </div>
      ) : null}
    </div>
  );
}
