import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { ArrowUpRight, Cpu, HardDrive, MemoryStick } from 'lucide-react';
import { api, type K8sPodRow, type ManagementInfo, type Metrics, type ProxyConfig } from '@/api/client';
import { PageHeader } from '@/components/console/page-header';
import { Panel } from '@/components/console/panel';
import { StatCard } from '@/components/console/stat-card';
import { StatusBadge } from '@/components/console/status-badge';
import { useLiveChannel } from '@/utils/useLiveChannel';

function formatUptime(secs: number) {
  const total = Math.max(0, Math.floor(Number.isFinite(secs) ? secs : 0));
  const d = Math.floor(total / 86400);
  const h = Math.floor((total % 86400) / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (d > 0) return h > 0 ? `${d}d ${h}h` : `${d}d`;
  if (h > 0) return m > 0 ? `${h}h ${m}m` : `${h}h`;
  if (m > 0) return s > 0 && m < 10 ? `${m}m ${s}s` : `${m}m`;
  return `${s}s`;
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
  const addr = metricsAddr?.trim() || '0.0.0.0:9990';
  const fallbackHost = typeof window !== 'undefined' ? window.location.hostname : 'localhost';
  const displayHost = hostname?.trim() || fallbackHost;
  const lastColon = addr.lastIndexOf(':');
  const bindHost = lastColon > 0 ? addr.slice(0, lastColon) : '0.0.0.0';
  const port = lastColon > 0 ? addr.slice(lastColon + 1) : '9990';
  const host =
    bindHost === '0.0.0.0' || bindHost === '[::]' || bindHost === '::' ? displayHost : bindHost.replace(/^\[|\]$/g, '');
  const base = `http://${host.includes(':') ? `[${host}]` : host}:${port}`;
  return { metrics: `${base}/metrics`, health: `${base}/health` };
}

function Meter({
  label,
  detail,
  pct,
  icon: Icon,
}: {
  label: string;
  detail: string;
  pct: number | null;
  icon: typeof Cpu;
}) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-3 text-sm">
        <span className="flex items-center gap-2 text-muted-foreground">
          <Icon className="size-4 text-primary" />
          {label}
        </span>
        <span className="font-mono text-xs text-foreground">{detail}</span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-muted">
        <div
          className="h-full rounded-full bg-primary transition-[width] duration-500"
          style={{ width: `${clampPercent(pct ?? 0)}%` }}
        />
      </div>
    </div>
  );
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-4 text-xs">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="truncate font-mono text-foreground">{value}</dd>
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
    return () => {
      cancelled = true;
    };
  }, []);

  useLiveChannel<ManagementInfo>('management', {
    onData: (mgmt) => {
      setInfo(mgmt);
      setError('');
    },
  });
  useLiveChannel<ProxyConfig>('config', {
    onData: (cfg) => setConfig(cfg),
  });
  useLiveChannel<Metrics>('metrics', {
    onData: (m) => setMetrics(m),
  });

  const podsLive = info?.mode === 'ingress';
  useEffect(() => {
    if (!podsLive) {
      setK8sPods([]);
      setK8sLoading(false);
      return;
    }
    let cancelled = false;
    setK8sLoading(true);
    api.kubernetes
      .pods()
      .then((pods) => {
        if (!cancelled) setK8sPods(pods);
      })
      .catch(() => {
        if (!cancelled) setK8sPods([]);
      })
      .finally(() => {
        if (!cancelled) setK8sLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [podsLive]);

  useLiveChannel<K8sPodRow[]>('pods', {
    enabled: !!podsLive,
    onData: (pods) => {
      setK8sPods(pods);
      setK8sLoading(false);
    },
  });

  const ingressPods = useMemo(
    () => (info ? filterIngressPods(k8sPods, info) : []),
    [k8sPods, info],
  );

  if (error) return <p className="text-destructive">{error}</p>;
  if (!info) return <p className="text-sm text-muted-foreground">Loading…</p>;

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
  const busiestSites = Array.from(allSites)
    .map((host) => ({
      host,
      count: (siteH2Totals[host] ?? 0) + (siteH3Totals[host] ?? 0),
    }))
    .filter((s) => s.count > 0)
    .sort((a, b) => b.count - a.count)
    .slice(0, 5);

  const h2Total = metrics?.h2_requests_total ?? 0;
  const h3Total = metrics?.h3_requests_total ?? 0;
  const httpTotal = h2Total + h3Total;
  const upstreamErrors = metrics?.upstream_errors_total ?? 0;
  const successPct =
    httpTotal > 0 ? ((httpTotal - Math.min(upstreamErrors, httpTotal)) / httpTotal) * 100 : null;

  const promUrls = prometheusUrls(metrics?.metrics_addr, info.hostname);
  const sitesHref = isIngress ? '/sites/ingress' : '/sites';

  return (
    <>
      <PageHeader
        title="Dashboard"
        description="Live overview of the pertisk-proxy runtime, traffic, and host health."
        actions={
          <>
            <StatusBadge tone="success" pulse>
              uptime {formatUptime(info.uptime_secs)}
            </StatusBadge>
            <StatusBadge tone="muted" dot={false}>
              {info.mode} mode
            </StatusBadge>
          </>
        }
      />

      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <StatCard label="Version" value={info.version} hint={info.hostname ?? undefined} />
        <StatCard label="Sites" value={String(info.site_count)} hint={`${info.route_count} routes`} />
        <StatCard
          label="Active connections"
          value={activeConnections.toLocaleString()}
          hint={`${(metrics?.bytes_sent_total ?? 0) > 0 ? formatBytes(metrics?.bytes_sent_total ?? 0) : '0 B'} sent`}
        />
        <StatCard
          label="Requests"
          value={(httpTotal || 0).toLocaleString()}
          hint={`H2 ${(metrics?.h2_requests_total ?? 0).toLocaleString()} · H3 ${(metrics?.h3_requests_total ?? 0).toLocaleString()}`}
        />
      </div>

      <div className="grid grid-cols-1 gap-6 xl:grid-cols-3">
        <Panel
          title="Traffic"
          description="Cumulative request and byte counters"
          className="xl:col-span-2"
        >
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
            <div className="rounded-md border border-border px-3 py-2.5">
              <p className="text-xs text-muted-foreground">HTTP/2</p>
              <p className="mt-1 font-mono text-lg font-semibold tabular-nums">
                {(metrics?.h2_requests_total ?? 0).toLocaleString()}
              </p>
            </div>
            <div className="rounded-md border border-border px-3 py-2.5">
              <p className="text-xs text-muted-foreground">HTTP/3</p>
              <p className="mt-1 font-mono text-lg font-semibold tabular-nums">
                {(metrics?.h3_requests_total ?? 0).toLocaleString()}
              </p>
            </div>
            <div className="rounded-md border border-border px-3 py-2.5">
              <p className="text-xs text-muted-foreground">H3/H2 ratio</p>
              <p className="mt-1 font-mono text-lg font-semibold tabular-nums">
                {metrics?.h3_vs_h2_ratio != null && Number.isFinite(metrics.h3_vs_h2_ratio)
                  ? metrics.h3_vs_h2_ratio.toFixed(2)
                  : '—'}
              </p>
            </div>
            <div className="rounded-md border border-border px-3 py-2.5">
              <p className="text-xs text-muted-foreground">Bytes sent</p>
              <p className="mt-1 font-mono text-lg font-semibold tabular-nums">
                {formatBytes(metrics?.bytes_sent_total ?? 0)}
              </p>
            </div>
            <div className="rounded-md border border-border px-3 py-2.5">
              <p className="text-xs text-muted-foreground">Bytes received</p>
              <p className="mt-1 font-mono text-lg font-semibold tabular-nums">
                {formatBytes(metrics?.bytes_received_total ?? 0)}
              </p>
            </div>
            <div className="rounded-md border border-border px-3 py-2.5">
              <p className="text-xs text-muted-foreground">Upstream errors</p>
              <p className="mt-1 font-mono text-lg font-semibold tabular-nums">
                {upstreamErrors.toLocaleString()}
              </p>
            </div>
          </div>
          {successPct != null ? (
            <p className="mt-4 text-xs text-muted-foreground">
              Approx. success rate from request totals vs upstream errors:{' '}
              <span className="font-mono text-foreground">{successPct.toFixed(2)}%</span>
            </p>
          ) : null}
        </Panel>

        <Panel
          title="Busiest sites"
          description="By request volume"
          actions={
            <Link
              to={sitesHref}
              className="inline-flex items-center gap-1 text-xs text-primary hover:underline"
            >
              All sites <ArrowUpRight className="size-3" />
            </Link>
          }
          bodyClassName="p-0"
        >
          {busiestSites.length === 0 ? (
            <p className="px-5 py-8 text-center text-sm text-muted-foreground">No traffic yet</p>
          ) : (
            <ul className="divide-y divide-border">
              {busiestSites.map((site) => (
                <li key={site.host} className="flex items-center justify-between gap-3 px-5 py-3">
                  <div className="flex min-w-0 items-center gap-2.5">
                    <StatusBadge tone="success" dot>
                      {''}
                    </StatusBadge>
                    <span className="truncate font-mono text-sm">{site.host}</span>
                  </div>
                  <span className="shrink-0 font-mono text-xs text-muted-foreground">
                    {site.count.toLocaleString()}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Panel>
      </div>

      <div className="grid grid-cols-1 gap-6 xl:grid-cols-3">
        {!isIngress ? (
          <Panel
            title="Host system"
            description={`${info.hostname ?? 'host'} · ${info.os ?? 'unknown OS'}`}
            bodyClassName="flex flex-col gap-4"
          >
            <Meter
              label="CPU"
              detail={
                info.cpu_count != null
                  ? `${info.cpu_count} cores${
                      info.cpu_usage_percent != null ? ` · ${info.cpu_usage_percent.toFixed(1)}%` : ''
                    }`
                  : '—'
              }
              pct={info.cpu_usage_percent ?? null}
              icon={Cpu}
            />
            <Meter
              label="Memory"
              detail={
                info.memory_used_bytes != null && info.memory_total_bytes
                  ? `${formatBytes(info.memory_used_bytes)} / ${formatBytes(info.memory_total_bytes)}`
                  : '—'
              }
              pct={systemMemoryPercent}
              icon={MemoryStick}
            />
            <Meter
              label="Storage"
              detail={
                info.disk_used_bytes != null && info.disk_total_bytes
                  ? `${formatBytes(info.disk_used_bytes)} / ${formatBytes(info.disk_total_bytes)}`
                  : '—'
              }
              pct={diskPercent}
              icon={HardDrive}
            />
            <dl className="mt-1 grid grid-cols-1 gap-1.5 border-t border-border pt-4">
              <InfoRow
                label="IPv4"
                value={info.ipv4_addrs?.length ? info.ipv4_addrs.join(', ') : '—'}
              />
              <InfoRow
                label="IPv6"
                value={info.ipv6_addrs?.length ? info.ipv6_addrs.join(', ') : '—'}
              />
              <InfoRow label="Database" value={info.db_path || '—'} />
            </dl>
          </Panel>
        ) : (
          <Panel title="Controller" description="Ingress runtime" bodyClassName="flex flex-col gap-3">
            <InfoRow label="Hostname" value={info.hostname ?? '—'} />
            <InfoRow label="PID" value={String(info.process_pid)} />
            <InfoRow label="Runtime" value={info.runtime_mode} />
            <InfoRow
              label="Leader"
              value={
                info.leader_election?.enabled
                  ? info.leader_election.is_leader
                    ? 'this pod'
                    : 'standby'
                  : 'n/a'
              }
            />
          </Panel>
        )}

        {!isIngress ? (
          <Panel title="App usage" description="pertisk-proxy process" bodyClassName="flex flex-col gap-4">
            <Meter
              label="CPU"
              detail={processCpu != null ? `${processCpu.toFixed(1)}%` : '—'}
              pct={processCpu}
              icon={Cpu}
            />
            <Meter
              label="Memory"
              detail={
                info.process_memory_bytes != null
                  ? `${formatBytes(info.process_memory_bytes)}${
                      processMemoryPercent != null ? ` · ${processMemoryPercent.toFixed(1)}% host` : ''
                    }`
                  : '—'
              }
              pct={processMemoryPercent}
              icon={MemoryStick}
            />
          </Panel>
        ) : (
          <Panel
            title="Ingress pods"
            description={k8sLoading ? 'Loading…' : `${ingressPods.length} pod${ingressPods.length === 1 ? '' : 's'}`}
            bodyClassName="p-0"
          >
            <div className="overflow-x-auto">
              <table className="min-w-full text-left text-sm">
                <thead className="border-b border-border bg-muted/50 text-muted-foreground">
                  <tr>
                    <th className="px-5 py-2.5 font-medium">Name</th>
                    <th className="px-5 py-2.5 font-medium">Ready</th>
                    <th className="px-5 py-2.5 font-medium">CPU</th>
                    <th className="px-5 py-2.5 font-medium">Mem</th>
                  </tr>
                </thead>
                <tbody>
                  {ingressPods.length === 0 ? (
                    <tr>
                      <td colSpan={4} className="px-5 py-8 text-center text-muted-foreground">
                        {k8sLoading ? 'Loading pods…' : 'No ingress controller pods found'}
                      </td>
                    </tr>
                  ) : (
                    ingressPods.map((pod) => (
                      <tr key={`${pod.namespace}/${pod.name}`} className="border-t border-border">
                        <td className="max-w-[160px] truncate px-5 py-2.5 font-mono text-xs" title={pod.name}>
                          {pod.name}
                        </td>
                        <td className="px-5 py-2.5">{pod.ready}</td>
                        <td className="px-5 py-2.5 font-mono text-xs">
                          {pod.cpu_usage_millicores != null ? formatMillicores(pod.cpu_usage_millicores) : 'n/a'}
                        </td>
                        <td className="px-5 py-2.5 font-mono text-xs">
                          {pod.memory_usage_bytes != null ? formatPodMemory(pod.memory_usage_bytes) : 'n/a'}
                        </td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
          </Panel>
        )}

        <Panel
          title="Configuration"
          description="Listeners and loaded config"
          actions={
            <Link to="/settings" className="inline-flex items-center gap-1 text-xs text-primary hover:underline">
              Settings <ArrowUpRight className="size-3" />
            </Link>
          }
          bodyClassName="flex flex-col gap-1.5"
        >
          <InfoRow label="HTTP" value={info.listeners.http} />
          <InfoRow label="HTTPS" value={info.listeners.https} />
          <InfoRow label="HTTP/3 UDP" value={info.listeners.h3_udp} />
          <InfoRow label="Management" value={info.management_addr} />
          <InfoRow label="HTTP/3" value={info.enable_h3 ? 'enabled' : 'disabled'} />
          <InfoRow label="Auto HTTPS" value={info.auto_https ? 'enabled' : 'disabled'} />
          <InfoRow label="Proxy log" value={config?.proxy_log === false ? 'disabled' : 'enabled'} />
          <InfoRow label="TLS entries" value={String(info.tls_count)} />
        </Panel>
      </div>

      <div className="grid grid-cols-1 gap-6 xl:grid-cols-2">
        <Panel
          title="Performance tuning"
          description="Effective process and Linux network settings"
          actions={
            <Link
              to="/settings#performance-tuning"
              className="inline-flex items-center gap-1 text-xs text-primary hover:underline"
            >
              Guide <ArrowUpRight className="size-3" />
            </Link>
          }
        >
          <div className="grid grid-cols-2 gap-3">
            <div className="rounded-md border border-border px-3 py-2.5">
              <p className="text-xs text-muted-foreground">Runtime mode</p>
              <p className="mt-1 text-sm font-semibold capitalize">{info.tuning.resolved_mode}</p>
              <p className="mt-0.5 text-xs text-muted-foreground">Requested: {info.tuning.requested_mode}</p>
            </div>
            <div className="rounded-md border border-border px-3 py-2.5">
              <p className="text-xs text-muted-foreground">Workers</p>
              <p className="mt-1 text-sm font-semibold">{info.tuning.pingora_service_threads} Pingora</p>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {info.tuning.h3_worker_threads} H3 · {info.tuning.tokio_worker_threads} Tokio
              </p>
            </div>
            <div className="rounded-md border border-border px-3 py-2.5">
              <p className="text-xs text-muted-foreground">HTTP/3 data plane</p>
              <p className="mt-1 text-sm font-semibold">{info.tuning.h3_stack}</p>
              <p className="mt-0.5 text-xs text-muted-foreground">{info.tuning.udp_offload}</p>
            </div>
            <div className="rounded-md border border-border px-3 py-2.5">
              <p className="text-xs text-muted-foreground">Linux TCP</p>
              <p className="mt-1 text-sm font-semibold uppercase">
                {info.tuning.kernel.tcp_congestion_control ?? 'n/a'}
              </p>
              <p className="mt-0.5 text-xs text-muted-foreground">
                qdisc {info.tuning.kernel.default_qdisc ?? 'n/a'} · backlog {info.tuning.tcp_listen_backlog}
              </p>
            </div>
          </div>
        </Panel>

        <Panel
          title="Prometheus metrics"
          description="Scrape endpoints"
          actions={
            <Link to="/metrics" className="inline-flex items-center gap-1 text-xs text-primary hover:underline">
              Open metrics <ArrowUpRight className="size-3" />
            </Link>
          }
          bodyClassName="flex flex-col gap-3"
        >
          <InfoRow label="Metrics" value={promUrls.metrics} />
          <InfoRow label="Health" value={promUrls.health} />
          <div className="flex flex-wrap gap-2 pt-1">
            {[
              'pertisk_h2_requests_total',
              'pertisk_h3_requests_total',
              'pertisk_active_connections',
              'pertisk_bytes_sent_total',
            ].map((name) => (
              <span
                key={name}
                className="rounded border border-border bg-muted/40 px-2 py-1 font-mono text-[10px] text-muted-foreground"
              >
                {name}
              </span>
            ))}
          </div>
        </Panel>
      </div>
    </>
  );
}
