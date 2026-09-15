import { useEffect, useMemo, useRef, useState } from 'react';
import { Link } from 'react-router-dom';
import { ArrowUpRight, Cpu, HardDrive, MemoryStick } from 'lucide-react';
import { api, type K8sPodRow, type ManagementInfo, type Metrics, type ProxyConfig } from '@/api/client';
import {
  BandwidthChart,
  ChartLegend,
  RequestsChart,
  StatusMixChart,
  type BandwidthPoint,
  type RequestPoint,
  type StatusMixItem,
} from '@/components/console/charts';
import { PageHeader } from '@/components/console/page-header';
import { Panel } from '@/components/console/panel';
import { StatCard } from '@/components/console/stat-card';
import { StatusBadge } from '@/components/console/status-badge';
import { useLiveChannel } from '@/utils/useLiveChannel';

const MAX_POINTS = 24;

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

function formatBytesPerSec(bytesPerSec: number) {
  if (!Number.isFinite(bytesPerSec) || bytesPerSec <= 0) return '0 B/s';
  if (bytesPerSec < 1024) return `${bytesPerSec.toFixed(0)} B/s`;
  if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(1)} KB/s`;
  if (bytesPerSec < 1024 * 1024 * 1024) return `${(bytesPerSec / (1024 * 1024)).toFixed(1)} MB/s`;
  return `${(bytesPerSec / (1024 * 1024 * 1024)).toFixed(2)} GB/s`;
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

function tickLabel(ts: number) {
  return new Date(ts).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
}

function deltaRate(curr: number, prev: number, seconds: number): number {
  if (seconds <= 0 || curr < prev) return 0;
  return (curr - prev) / seconds;
}

export function Dashboard() {
  const [info, setInfo] = useState<ManagementInfo | null>(null);
  const [config, setConfig] = useState<ProxyConfig | null>(null);
  const [metrics, setMetrics] = useState<Metrics | null>(null);
  const [k8sPods, setK8sPods] = useState<K8sPodRow[]>([]);
  const [k8sLoading, setK8sLoading] = useState(false);
  const [error, setError] = useState('');
  const [requestSeries, setRequestSeries] = useState<RequestPoint[]>([]);
  const [bandwidthSeries, setBandwidthSeries] = useState<BandwidthPoint[]>([]);
  const [rates, setRates] = useState({ sent: 0, recv: 0 });
  const prevMetrics = useRef<{ m: Metrics; at: number } | null>(null);

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

  useEffect(() => {
    if (!metrics) return;
    const now = Date.now();
    const prev = prevMetrics.current;
    prevMetrics.current = { m: metrics, at: now };
    if (!prev) return;

    const secs = (now - prev.at) / 1000;
    const h2 = deltaRate(metrics.h2_requests_total, prev.m.h2_requests_total, secs);
    const h3 = deltaRate(metrics.h3_requests_total, prev.m.h3_requests_total, secs);
    const egress = deltaRate(metrics.bytes_sent_total, prev.m.bytes_sent_total, secs);
    const ingress = deltaRate(metrics.bytes_received_total, prev.m.bytes_received_total, secs);
    setRates({ sent: egress, recv: ingress });

    const label = tickLabel(now);
    setRequestSeries((series) => {
      const next = [...series, { t: label, http2: Math.round(h2 * 10) / 10, http3: Math.round(h3 * 10) / 10 }];
      return next.slice(-MAX_POINTS);
    });
    setBandwidthSeries((series) => {
      const next = [
        ...series,
        {
          t: label,
          egress: Math.round((egress / (1024 * 1024)) * 100) / 100,
          ingress: Math.round((ingress / (1024 * 1024)) * 100) / 100,
        },
      ];
      return next.slice(-MAX_POINTS);
    });
  }, [metrics]);

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

  const statusMix: StatusMixItem[] = useMemo(() => {
    const h2 = metrics?.h2_requests_total ?? 0;
    const h3 = metrics?.h3_requests_total ?? 0;
    const total = h2 + h3;
    const errors = metrics?.upstream_errors_total ?? 0;
    const blocked =
      (metrics?.waf_blocked_total ?? 0) +
      (metrics?.bot_blocked_total ?? 0) +
      (metrics?.geoip_blocked_total ?? 0);
    if (total <= 0) {
      return [
        { name: '2xx', value: 100, color: 'var(--color-success)' },
        { name: '3xx', value: 0, color: 'var(--color-info)' },
        { name: '4xx', value: 0, color: 'var(--color-warning)' },
        { name: '5xx', value: 0, color: 'var(--color-destructive)' },
      ];
    }
    const errPct = clampPercent((Math.min(errors, total) / total) * 100);
    const blockPct = clampPercent((Math.min(blocked, total) / total) * 100);
    const successPct = clampPercent(100 - errPct - blockPct);
    return [
      { name: '2xx', value: Math.round(successPct * 10) / 10, color: 'var(--color-success)' },
      { name: 'blocked', value: Math.round(blockPct * 10) / 10, color: 'var(--color-warning)' },
      { name: '5xx', value: Math.round(errPct * 10) / 10, color: 'var(--color-destructive)' },
      { name: 'other', value: Math.round(Math.max(0, 100 - successPct - blockPct - errPct) * 10) / 10, color: 'var(--color-info)' },
    ].filter((s) => s.value > 0 || s.name === '2xx');
  }, [metrics]);

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
  const h3Share = httpTotal > 0 ? (h3Total / httpTotal) * 100 : null;
  const successPct = statusMix.find((s) => s.name === '2xx')?.value ?? null;
  const sitesHref = isIngress ? '/sites/ingress' : '/sites';

  const stats = [
    { label: 'Sites', value: String(info.site_count), hint: `${info.route_count} routes` },
    { label: 'Routes', value: String(info.route_count), hint: `${info.tls_count} TLS entries` },
    {
      label: 'Active connections',
      value: activeConnections.toLocaleString(),
      hint: `${formatBytes(metrics?.bytes_sent_total ?? 0)} sent`,
    },
    {
      label: 'Requests',
      value: httpTotal.toLocaleString(),
      hint: `H2 ${h2Total.toLocaleString()} · H3 ${h3Total.toLocaleString()}`,
    },
    {
      label: 'HTTP/3 share',
      value: h3Share != null ? `${h3Share.toFixed(1)}%` : '—',
      hint: `${h3Total.toLocaleString()} of ${httpTotal.toLocaleString()} reqs`,
    },
    {
      label: 'Upstream errors',
      value: upstreamErrors.toLocaleString(),
      hint: config?.proxy_log === false ? 'proxy log off' : 'live counter',
    },
    {
      label: 'Bytes sent',
      value: formatBytesPerSec(rates.sent),
      hint: 'egress rate',
    },
    {
      label: 'Bytes received',
      value: formatBytesPerSec(rates.recv),
      hint: 'ingress rate',
    },
  ];

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
        {stats.map((stat) => (
          <StatCard key={stat.label} {...stat} />
        ))}
      </div>

      <div className="grid grid-cols-1 gap-6 xl:grid-cols-3">
        <Panel
          title="Request throughput"
          description="HTTP/2 vs HTTP/3 requests per sample"
          className="xl:col-span-2"
          actions={
            <ChartLegend
              items={[
                { label: 'HTTP/3', color: 'var(--color-chart-1)' },
                { label: 'HTTP/2', color: 'var(--color-chart-2)' },
              ]}
            />
          }
        >
          {requestSeries.length > 0 ? (
            <RequestsChart data={requestSeries} />
          ) : (
            <p className="py-16 text-center text-sm text-muted-foreground">Collecting live samples…</p>
          )}
        </Panel>

        <Panel title="Response status mix" description="Share derived from request and error counters">
          <div className="relative">
            <StatusMixChart data={statusMix} />
            <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center">
              <span className="font-mono text-2xl font-semibold">
                {successPct != null ? `${successPct.toFixed(2)}%` : '—'}
              </span>
              <span className="text-xs text-muted-foreground">success</span>
            </div>
          </div>
          <div className="mt-4 grid grid-cols-2 gap-2">
            {statusMix.map((s) => (
              <div
                key={s.name}
                className="flex items-center justify-between rounded-md border border-border px-2.5 py-1.5"
              >
                <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                  <span className="size-2 rounded-full" style={{ background: s.color }} />
                  {s.name}
                </span>
                <span className="font-mono text-xs">{s.value}%</span>
              </div>
            ))}
          </div>
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
            <dl className="mt-1 grid grid-cols-1 gap-1.5 border-t border-border pt-4 text-xs">
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
            <div className="mt-2 border-t border-border pt-3">
              <p className="mb-2 text-xs text-muted-foreground">
                {k8sLoading ? 'Loading pods…' : `${ingressPods.length} pod${ingressPods.length === 1 ? '' : 's'}`}
              </p>
              <ul className="divide-y divide-border rounded-md border border-border">
                {ingressPods.length === 0 ? (
                  <li className="px-3 py-4 text-center text-xs text-muted-foreground">
                    {k8sLoading ? 'Loading…' : 'No ingress pods found'}
                  </li>
                ) : (
                  ingressPods.slice(0, 4).map((pod) => (
                    <li key={`${pod.namespace}/${pod.name}`} className="flex items-center justify-between gap-2 px-3 py-2">
                      <span className="truncate font-mono text-xs" title={pod.name}>
                        {pod.name}
                      </span>
                      <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
                        {pod.ready}
                        {pod.cpu_usage_millicores != null ? ` · ${formatMillicores(pod.cpu_usage_millicores)}` : ''}
                        {pod.memory_usage_bytes != null ? ` · ${formatPodMemory(pod.memory_usage_bytes)}` : ''}
                      </span>
                    </li>
                  ))
                )}
              </ul>
            </div>
          </Panel>
        )}

        <Panel
          title="Bandwidth"
          description="Ingress vs egress rate (MB/s)"
          actions={
            <ChartLegend
              items={[
                { label: 'Egress', color: 'var(--color-chart-1)' },
                { label: 'Ingress', color: 'var(--color-chart-2)' },
              ]}
            />
          }
        >
          {bandwidthSeries.length > 0 ? (
            <BandwidthChart data={bandwidthSeries} unit="MB/s" />
          ) : (
            <p className="py-16 text-center text-sm text-muted-foreground">Collecting live samples…</p>
          )}
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
    </>
  );
}
