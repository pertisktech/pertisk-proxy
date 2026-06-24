import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { api, type K8sPodRow, type ManagementInfo, type Metrics, type ProxyConfig } from '@/api/client';
import { Card, Stat } from '@/components/Card';

function formatUptime(secs: number) {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  return `${h}h ${m}m ${s}s`;
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function clampPercent(value: number) {
  if (!Number.isFinite(value)) return 0;
  return Math.min(100, Math.max(0, value));
}

function formatMillicores(millicores: number): string {
  return `${millicores}m`;
}

function formatPodMemory(bytes: number): string {
  if (bytes <= 0) return '0Mi';
  const mib = bytes / (1024 * 1024);
  if (mib < 1) return `${Math.max(1, Math.round(bytes / 1024))}Ki`;
  if (mib < 1024) return `${Math.round(mib)}Mi`;
  return `${(mib / 1024).toFixed(1)}Gi`;
}

function deploymentPrefixFromHostname(hostname: string | null | undefined): string | null {
  const host = hostname?.trim();
  if (!host) return null;
  const parts = host.split('-');
  if (parts.length >= 4) {
    return parts.slice(0, -2).join('-');
  }
  return host;
}

function filterIngressPods(pods: K8sPodRow[], info: ManagementInfo): K8sPodRow[] {
  const prefix = deploymentPrefixFromHostname(info.hostname) ?? 'pertisk-proxy-ingress';
  const namespace = info.leader_election?.namespace?.trim();
  return pods.filter((pod) => {
    if (!pod.name.startsWith(`${prefix}-`)) return false;
    if (namespace && pod.namespace !== namespace) return false;
    return true;
  });
}

function prometheusUrls(metricsAddr: string | undefined, hostname: string | null | undefined) {
  const addr = metricsAddr?.trim() || '0.0.0.0:9090';
  const fallbackHost = typeof window !== 'undefined' ? window.location.hostname : 'localhost';
  const displayHost = hostname?.trim() || fallbackHost;
  const lastColon = addr.lastIndexOf(':');
  const bindHost = lastColon > 0 ? addr.slice(0, lastColon) : '0.0.0.0';
  const port = lastColon > 0 ? addr.slice(lastColon + 1) : '9090';
  const host =
    bindHost === '0.0.0.0' || bindHost === '[::]' || bindHost === '::' ? displayHost : bindHost.replace(/^\[|\]$/g, '');
  const base = `http://${host.includes(':') ? `[${host}]` : host}:${port}`;
  return { metrics: `${base}/metrics`, health: `${base}/health` };
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5 sm:flex-row sm:items-baseline sm:justify-between sm:gap-4">
      <dt className="text-sm text-text-secondary">{label}</dt>
      <dd className="font-mono text-sm break-all text-right sm:max-w-[65%]">{value}</dd>
    </div>
  );
}

function UsageBar({ percent, tone }: { percent: number; tone: 'cpu' | 'memory' | 'storage' }) {
  const fill =
    tone === 'cpu' ? 'bg-primary' : tone === 'storage' ? 'bg-yellow-y1' : 'bg-green-g1';
  const width = clampPercent(percent).toFixed(1);
  return (
    <div
      className="mt-1.5 h-2 overflow-hidden rounded-full bg-border"
      role="progressbar"
      aria-valuenow={Number(width)}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <div className={`h-full rounded-full transition-all duration-500 ${fill}`} style={{ width: `${width}%` }} />
    </div>
  );
}

export function Dashboard() {
  const [info, setInfo] = useState<ManagementInfo | null>(null);
  const [config, setConfig] = useState<ProxyConfig | null>(null);
  const [metrics, setMetrics] = useState<Metrics | null>(null);
  const [k8sPods, setK8sPods] = useState<K8sPodRow[]>([]);
  const [k8sLoading, setK8sLoading] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        const [mgmt, cfg, m] = await Promise.all([api.management(), api.config(), api.metrics()]);
        if (!cancelled) {
          setInfo(mgmt);
          setConfig(cfg);
          setMetrics(m);
          setError('');
        }
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : 'Failed to load dashboard');
      }
    }

    load();
    const timer = setInterval(load, 5000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    if (info?.mode !== 'ingress') {
      setK8sPods([]);
      return;
    }
    let cancelled = false;

    async function loadPods() {
      setK8sLoading(true);
      try {
        const pods = await api.kubernetes.pods();
        if (!cancelled) setK8sPods(pods);
      } catch {
        if (!cancelled) setK8sPods([]);
      } finally {
        if (!cancelled) setK8sLoading(false);
      }
    }

    loadPods();
    const timer = setInterval(loadPods, 10000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [info?.mode]);

  const ingressPods = useMemo(
    () => (info ? filterIngressPods(k8sPods, info) : []),
    [k8sPods, info],
  );

  if (error) return <p className="text-red-r1">{error}</p>;
  if (!info) return <p className="text-text-secondary">Loading…</p>;

  const isIngress = info.mode === 'ingress';
  const systemMemoryPercent =
    info.memory_used_bytes != null && info.memory_total_bytes
      ? clampPercent((info.memory_used_bytes / info.memory_total_bytes) * 100)
      : null;
  const diskPercent =
    info.disk_used_bytes != null && info.disk_total_bytes
      ? clampPercent((info.disk_used_bytes / info.disk_total_bytes) * 100)
      : null;
  const processMemoryPercent =
    info.process_memory_bytes != null && info.memory_total_bytes
      ? clampPercent((info.process_memory_bytes / info.memory_total_bytes) * 100)
      : null;
  const processCpu = info.process_cpu_usage_percent ?? null;

  const activeConnections = metrics?.active_connections ?? 0;
  const siteH2Totals = metrics?.site_h2_requests_total ?? {};
  const siteH3Totals = metrics?.site_h3_requests_total ?? {};
  const allSites = new Set([...Object.keys(siteH2Totals), ...Object.keys(siteH3Totals)]);
  const busiestSite = Array.from(allSites)
    .map((host) => [host, (siteH2Totals[host] ?? 0) + (siteH3Totals[host] ?? 0)] as const)
    .sort((a, b) => b[1] - a[1])
    .find(([, count]) => count > 0);
  const promUrls = prometheusUrls(metrics?.metrics_addr, info.hostname);

  return (
    <div className="space-y-6">
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <Stat label="Version" value={info.version} />
        <Stat label="Uptime" value={formatUptime(info.uptime_secs)} />
        <Stat label="Sites" value={info.site_count} />
        <Stat label="Routes" value={info.route_count} />
      </div>

      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <Stat label="Active connections" value={activeConnections} />
        <Stat label="HTTP/2 requests" value={(metrics?.h2_requests_total ?? 0).toLocaleString()} />
        <Stat label="HTTP/3 requests" value={(metrics?.h3_requests_total ?? 0).toLocaleString()} />
        <Stat
          label="H3/H2 ratio"
          value={
            metrics?.h3_vs_h2_ratio != null && Number.isFinite(metrics.h3_vs_h2_ratio)
              ? metrics.h3_vs_h2_ratio.toFixed(2)
              : '—'
          }
        />
      </div>

      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <Stat label="Bytes sent" value={formatBytes(metrics?.bytes_sent_total ?? 0)} />
        <Stat label="Bytes received" value={formatBytes(metrics?.bytes_received_total ?? 0)} />
        <Stat label="Upstream errors" value={(metrics?.upstream_errors_total ?? 0).toLocaleString()} />
        <Stat
          label="Busiest site"
          value={busiestSite ? busiestSite[0] : 'No traffic yet'}
        />
      </div>

      {!isIngress ? (
        <div className="grid gap-6 xl:grid-cols-2">
          <Card>
            <h2 className="mb-1 text-lg font-semibold">Hardware</h2>
            <p className="mb-4 text-sm text-text-secondary">Host system information</p>
            <dl className="space-y-3">
              <InfoRow label="Hostname" value={info.hostname ?? '—'} />
              <InfoRow label="OS" value={info.os ?? '—'} />
              <InfoRow
                label="CPU"
                value={
                  info.cpu_count != null
                    ? `${info.cpu_count} cores${
                        info.cpu_usage_percent != null ? ` · ${info.cpu_usage_percent.toFixed(1)}% system` : ''
                      }`
                    : '—'
                }
              />
              {info.cpu_usage_percent != null ? (
                <UsageBar percent={info.cpu_usage_percent} tone="cpu" />
              ) : null}
              <div>
                <InfoRow
                  label="Memory"
                  value={
                    info.memory_used_bytes != null && info.memory_total_bytes
                      ? `${formatBytes(info.memory_used_bytes)} / ${formatBytes(info.memory_total_bytes)}${
                          systemMemoryPercent != null ? ` (${systemMemoryPercent.toFixed(1)}%)` : ''
                        }`
                      : '—'
                  }
                />
                {systemMemoryPercent != null ? (
                  <UsageBar percent={systemMemoryPercent} tone="memory" />
                ) : null}
              </div>
              <div>
                <InfoRow
                  label="Storage"
                  value={
                    info.disk_used_bytes != null && info.disk_total_bytes
                      ? `${formatBytes(info.disk_used_bytes)} / ${formatBytes(info.disk_total_bytes)}${
                          diskPercent != null ? ` (${diskPercent.toFixed(1)}%)` : ''
                        }${info.disk_mount_point ? ` · ${info.disk_mount_point}` : ''}`
                      : '—'
                  }
                />
                {diskPercent != null ? <UsageBar percent={diskPercent} tone="storage" /> : null}
              </div>
              <InfoRow label="IPv4" value={info.ipv4_addrs?.length ? info.ipv4_addrs.join(', ') : '—'} />
              <InfoRow label="IPv6" value={info.ipv6_addrs?.length ? info.ipv6_addrs.join(', ') : '—'} />
            </dl>
          </Card>

          <Card>
            <h2 className="mb-1 text-lg font-semibold">App usage</h2>
            <p className="mb-4 text-sm text-text-secondary">pertisk-proxy process CPU and memory</p>
            <div className="space-y-5">
              <div>
                <div className="flex items-baseline justify-between gap-3">
                  <span className="text-sm text-text-secondary">CPU</span>
                  <span className="font-mono text-sm">
                    {processCpu != null ? `${processCpu.toFixed(1)}%` : '—'}
                  </span>
                </div>
                {processCpu != null ? <UsageBar percent={processCpu} tone="cpu" /> : null}
              </div>
              <div>
                <div className="flex items-baseline justify-between gap-3">
                  <span className="text-sm text-text-secondary">Memory</span>
                  <span className="font-mono text-sm">
                    {info.process_memory_bytes != null
                      ? `${formatBytes(info.process_memory_bytes)}${
                          processMemoryPercent != null ? ` · ${processMemoryPercent.toFixed(1)}% of host` : ''
                        }`
                      : '—'}
                  </span>
                </div>
                {processMemoryPercent != null ? (
                  <UsageBar percent={processMemoryPercent} tone="memory" />
                ) : null}
              </div>
            </div>
          </Card>
        </div>
      ) : null}

      <Card>
        <h2 className="mb-1 text-lg font-semibold">Configuration</h2>
        <p className="mb-4 text-sm text-text-secondary">Proxy runtime and loaded config</p>
        <div className="grid gap-6 lg:grid-cols-2">
          <dl className="space-y-3">
            <InfoRow label="Mode" value={info.mode} />
            <InfoRow label="Runtime" value={info.runtime_mode} />
            <InfoRow label="HTTP" value={info.listeners.http} />
            <InfoRow label="HTTPS" value={info.listeners.https} />
            <InfoRow label="HTTP/3 UDP" value={info.listeners.h3_udp} />
            <InfoRow label="Management API" value={info.management_addr} />
          </dl>
          <dl className="space-y-3">
            <InfoRow label="Database" value={info.db_path} />
            <InfoRow label="Sites" value={String(info.site_count)} />
            <InfoRow label="Backends" value={String(info.backend_count)} />
            <InfoRow label="TLS entries" value={String(info.tls_count)} />
            <InfoRow label="TLS hosts loaded" value={String(info.tls_host_count)} />
            <InfoRow label="HTTP/3" value={info.enable_h3 ? 'enabled' : 'disabled'} />
            <InfoRow label="Auto HTTPS" value={info.auto_https ? 'enabled' : 'disabled'} />
            <InfoRow label="Proxy log" value={config?.proxy_log === false ? 'disabled' : 'enabled'} />
          </dl>
        </div>
      </Card>

      {isIngress ? (
        <Card>
          <div className="mb-4 flex flex-wrap items-baseline justify-between gap-2">
            <div>
              <h2 className="text-lg font-semibold">Ingress pods</h2>
              <p className="text-sm text-text-secondary">
                Pods for this controller deployment only (refreshes every 10s)
              </p>
            </div>
            <span className="text-sm text-text-secondary">
              {k8sLoading ? 'Loading…' : `${ingressPods.length} pod${ingressPods.length === 1 ? '' : 's'}`}
            </span>
          </div>
          <div className="overflow-x-auto rounded-lg border border-border">
            <table className="min-w-full text-left text-sm">
              <thead className="border-b border-border bg-surface-elevated text-text-secondary">
                <tr>
                  <th className="px-4 py-3 font-medium">Name</th>
                  <th className="px-4 py-3 font-medium">Namespace</th>
                  <th className="px-4 py-3 font-medium">Phase</th>
                  <th className="px-4 py-3 font-medium">Ready</th>
                  <th className="px-4 py-3 font-medium">Restarts</th>
                  <th className="px-4 py-3 font-medium">CPU</th>
                  <th className="px-4 py-3 font-medium">Memory</th>
                  <th className="px-4 py-3 font-medium">Pod IP</th>
                  <th className="px-4 py-3 font-medium">Node</th>
                </tr>
              </thead>
              <tbody>
                {ingressPods.length === 0 ? (
                  <tr>
                    <td colSpan={9} className="px-4 py-8 text-center text-text-secondary">
                      {k8sLoading ? 'Loading pods…' : 'No ingress controller pods found'}
                    </td>
                  </tr>
                ) : (
                  ingressPods.map((pod) => (
                    <tr key={`${pod.namespace}/${pod.name}`} className="border-t border-border hover:bg-hover/50">
                      <td className="max-w-[220px] truncate px-4 py-3 font-medium" title={pod.name}>
                        {pod.name}
                      </td>
                      <td className="px-4 py-3">{pod.namespace}</td>
                      <td className="px-4 py-3">{pod.phase}</td>
                      <td className="px-4 py-3">{pod.ready}</td>
                      <td className="px-4 py-3">{pod.restarts ?? 0}</td>
                      <td className="px-4 py-3">
                        {pod.cpu_usage_millicores != null ? formatMillicores(pod.cpu_usage_millicores) : 'n/a'}
                      </td>
                      <td className="px-4 py-3">
                        {pod.memory_usage_bytes != null ? formatPodMemory(pod.memory_usage_bytes) : 'n/a'}
                      </td>
                      <td className="px-4 py-3 font-mono text-xs">{pod.pod_ip ?? 'n/a'}</td>
                      <td className="px-4 py-3">{pod.node_name ?? pod.node ?? 'n/a'}</td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </div>
          {info.leader_election?.enabled ? (
            <p className="mt-3 text-xs text-text-secondary">
              Leader election: {info.leader_election.is_leader ? 'this pod is leader' : 'standby'} ·{' '}
              {info.leader_election.namespace}/{info.leader_election.lease_name}
            </p>
          ) : null}
        </Card>
      ) : null}

      <Card>
        <div className="mb-4 flex flex-wrap items-baseline justify-between gap-2">
          <div>
            <h2 className="text-lg font-semibold">Prometheus metrics</h2>
            <p className="text-sm text-text-secondary">Scrape endpoints and key series for monitoring</p>
          </div>
          <Link to="/metrics" className="text-sm text-primary hover:underline">
            Open metrics dashboard →
          </Link>
        </div>
        <div className="grid gap-4 lg:grid-cols-2">
          <div className="rounded-lg border border-border bg-surface-elevated p-4">
            <div className="mb-3 text-sm font-medium text-text-secondary">Endpoints</div>
            <dl className="space-y-2 text-sm">
              <InfoRow label="Metrics" value={promUrls.metrics} />
              <InfoRow label="Health" value={promUrls.health} />
            </dl>
            <p className="mt-3 text-xs text-text-secondary">
              Set <code className="font-mono">PERTISK_METRICS_ADDR</code> to change the listen address (default port 9090).
            </p>
          </div>
          <div className="rounded-lg border border-border bg-surface-elevated p-4">
            <div className="mb-3 text-sm font-medium text-text-secondary">Key series</div>
            <div className="flex flex-wrap gap-2">
              {[
                'pertisk_http_requests_total',
                'pertisk_https_requests_total',
                'pertisk_h2_requests_total',
                'pertisk_h3_requests_total',
                'pertisk_grpc_requests_total',
                'pertisk_upstream_errors_total',
                'pertisk_active_connections',
                'pertisk_bytes_sent_total',
                'pertisk_bytes_received_total',
              ].map((name) => (
                <span key={name} className="rounded border border-border bg-bg px-2 py-1 font-mono text-xs">
                  {name}
                </span>
              ))}
            </div>
            <p className="mt-3 text-xs text-text-secondary">
              Scrape <code className="font-mono">/metrics</code> from your Prometheus job or ServiceMonitor.
            </p>
          </div>
        </div>
      </Card>
    </div>
  );
}
