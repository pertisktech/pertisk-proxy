import { useEffect, useMemo, useState } from 'react';
import { toast } from 'sonner';
import { ExternalLink, RefreshCw } from 'lucide-react';
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';
import { api, type ManagementInfo, type Metrics as ApiMetrics } from '@/api/client';
import { Card, Stat } from '@/components/Card';
import { Checkbox } from '@/components/Checkbox';
import { formatTimeOnly } from '@/utils/dateFormat';

const POLL_INTERVAL_MS = 3000;
const MAX_POINTS = 60;

type MetricPoint = {
  t: number;
  time: string;
  http: number;
  https: number;
  h2: number;
  h3: number;
  grpc: number;
  sent: number;
  recv: number;
  connections: number;
  errors: number;
  cpu: number | null;
  memory_mb: number | null;
};

function formatBytes(value: number | undefined): string {
  if (value == null || !Number.isFinite(value)) return '—';
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  if (value < 1024 * 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
}

function formatNum(value: number | undefined | null): string {
  if (value == null || !Number.isFinite(value)) return '—';
  return Number.isInteger(value) ? value.toLocaleString() : value.toFixed(2);
}

function formatRate(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) return '—';
  return `${value.toFixed(2)}/s`;
}

function deltaRate(curr: number, prev: number, seconds: number): number {
  if (seconds <= 0 || curr < prev) return 0;
  return (curr - prev) / seconds;
}

function prometheusMetricsURL(metricsAddr: string | undefined, hostname: string | null | undefined): string {
  const addr = metricsAddr?.trim() || '0.0.0.0:9090';
  const fallbackHost = typeof window !== 'undefined' ? window.location.hostname : 'localhost';
  const displayHost = hostname?.trim() || fallbackHost;
  const lastColon = addr.lastIndexOf(':');
  const bindHost = lastColon > 0 ? addr.slice(0, lastColon) : '0.0.0.0';
  const port = lastColon > 0 ? addr.slice(lastColon + 1) : '9090';
  const host =
    bindHost === '0.0.0.0' || bindHost === '[::]' || bindHost === '::' ? displayHost : bindHost.replace(/^\[|\]$/g, '');
  return `http://${host.includes(':') ? `[${host}]` : host}:${port}/metrics`;
}

const chartTooltipStyle = {
  backgroundColor: 'var(--color-surface-elevated)',
  border: '1px solid var(--color-border)',
  borderRadius: 8,
  color: 'var(--color-text)',
  fontSize: 12,
};

function EmptyChart({ message }: { message: string }) {
  return (
    <div className="flex h-56 items-center justify-center text-sm text-text-secondary">{message}</div>
  );
}

export function Metrics() {
  const [history, setHistory] = useState<MetricPoint[]>([]);
  const [latest, setLatest] = useState<ApiMetrics | null>(null);
  const [mgmt, setMgmt] = useState<ManagementInfo | null>(null);
  const [live, setLive] = useState(true);
  const [loading, setLoading] = useState(true);
  const [showRaw, setShowRaw] = useState(false);

  function appendPoint(m: ApiMetrics, management: ManagementInfo) {
    const t = Date.now();
    const point: MetricPoint = {
      t,
      time: formatTimeOnly(new Date(t).toISOString()),
      http: m.http_requests_total ?? 0,
      https: m.https_requests_total ?? 0,
      h2: m.h2_requests_total ?? 0,
      h3: m.h3_requests_total ?? 0,
      grpc: m.grpc_requests_total ?? 0,
      sent: m.bytes_sent_total ?? 0,
      recv: m.bytes_received_total ?? 0,
      connections: m.active_connections ?? 0,
      errors: m.upstream_errors_total ?? 0,
      cpu: management.process_cpu_usage_percent ?? null,
      memory_mb:
        management.process_memory_bytes != null
          ? Math.round((management.process_memory_bytes / (1024 * 1024)) * 10) / 10
          : null,
    };
    setLatest(m);
    setMgmt(management);
    setHistory((prev) => [...prev, point].slice(-MAX_POINTS));
  }

  function fetchMetrics() {
    setLoading(true);
    Promise.all([api.metrics(), api.management()])
      .then(([m, management]) => appendPoint(m, management))
      .catch((e) => toast.error(e instanceof Error ? e.message : 'Failed to load metrics'))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    fetchMetrics();
  }, []);

  useEffect(() => {
    if (!live) return;
    const id = setInterval(() => {
      Promise.all([api.metrics(), api.management()])
        .then(([m, management]) => appendPoint(m, management))
        .catch(() => {
          /* keep last snapshot; toast only on manual refresh */
        });
    }, POLL_INTERVAL_MS);
    return () => clearInterval(id);
  }, [live]);

  const rates = useMemo(() => {
    if (history.length < 2) return [];
    const out: Array<{
      time: string;
      requests: number;
      http: number;
      https: number;
      h3: number;
      grpc: number;
      sent_kibps: number;
      recv_kibps: number;
    }> = [];
    for (let i = 1; i < history.length; i += 1) {
      const prev = history[i - 1];
      const curr = history[i];
      const seconds = (curr.t - prev.t) / 1000;
      const http = deltaRate(curr.http, prev.http, seconds);
      const https = deltaRate(curr.https, prev.https, seconds);
      const h3 = deltaRate(curr.h3, prev.h3, seconds);
      const grpc = deltaRate(curr.grpc, prev.grpc, seconds);
      out.push({
        time: curr.time,
        requests: http + https + grpc,
        http,
        https,
        h3,
        grpc,
        sent_kibps: deltaRate(curr.sent, prev.sent, seconds) / 1024,
        recv_kibps: deltaRate(curr.recv, prev.recv, seconds) / 1024,
      });
    }
    return out;
  }, [history]);

  const snapshotRates = useMemo(() => {
    if (history.length < 2) {
      return { rps: null as number | null, sent: null as number | null, recv: null as number | null };
    }
    const last = rates[rates.length - 1];
    return {
      rps: last?.requests ?? null,
      sent: last?.sent_kibps ?? null,
      recv: last?.recv_kibps ?? null,
    };
  }, [history.length, rates]);

  const protocolBars = useMemo(() => {
    if (!latest) return [];
    return [
      { name: 'HTTP', count: latest.http_requests_total ?? 0 },
      { name: 'HTTPS', count: latest.https_requests_total ?? 0 },
      { name: 'H2', count: latest.h2_requests_total ?? 0 },
      { name: 'H3', count: latest.h3_requests_total ?? 0 },
      { name: 'gRPC', count: latest.grpc_requests_total ?? 0 },
    ].filter((row) => row.count > 0);
  }, [latest]);

  const siteRows = useMemo(() => {
    if (!latest) return [];
    const hosts = new Set([
      ...Object.keys(latest.site_h2_requests_total ?? {}),
      ...Object.keys(latest.site_h3_requests_total ?? {}),
    ]);
    return Array.from(hosts)
      .map((domain) => {
        const h2 = latest.site_h2_requests_total?.[domain] ?? 0;
        const h3 = latest.site_h3_requests_total?.[domain] ?? 0;
        return {
          domain,
          h2,
          h3,
          total: h2 + h3,
          ratio: h2 > 0 ? h3 / h2 : h3 > 0 ? Infinity : 0,
        };
      })
      .sort((a, b) => b.total - a.total);
  }, [latest]);

  const current = history[history.length - 1];
  const totalRequests =
    (latest?.http_requests_total ?? 0) + (latest?.https_requests_total ?? 0) + (latest?.grpc_requests_total ?? 0);
  const totalBytes = (latest?.bytes_sent_total ?? 0) + (latest?.bytes_received_total ?? 0);
  const promUrl = prometheusMetricsURL(latest?.metrics_addr, mgmt?.hostname ?? null);

  const rawSnapshot = useMemo(
    () => ({
      metrics: latest,
      management: mgmt
        ? {
            mode: mgmt.mode,
            hostname: mgmt.hostname,
            process_cpu_usage_percent: mgmt.process_cpu_usage_percent,
            process_memory_bytes: mgmt.process_memory_bytes,
          }
        : null,
      derived: {
        request_rate: snapshotRates.rps,
        sent_kibps: snapshotRates.sent,
        recv_kibps: snapshotRates.recv,
      },
    }),
    [latest, mgmt, snapshotRates],
  );

  return (
    <div className="space-y-4 overflow-y-auto pb-8">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-3">
          <Checkbox checked={live} onChange={setLive} label="Live refresh" />
          <button
            type="button"
            onClick={fetchMetrics}
            disabled={loading}
            className="inline-flex items-center gap-2 rounded-md border border-border px-3 py-2 text-sm hover:bg-hover disabled:opacity-50"
          >
            <RefreshCw size={14} className={loading ? 'animate-spin' : undefined} /> Refresh
          </button>
          <Checkbox checked={showRaw} onChange={setShowRaw} label="Show JSON" />
        </div>
        <a
          href={promUrl}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-2 rounded-md border border-border px-3 py-2 text-sm hover:bg-hover"
        >
          Prometheus <ExternalLink size={14} />
        </a>
      </div>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-5">
        <Stat label="Total requests" value={formatNum(totalRequests)} />
        <Stat label="Active connections" value={formatNum(latest?.active_connections)} />
        <Stat label="Requests / s" value={formatRate(snapshotRates.rps)} />
        <Stat label="Total bytes" value={formatBytes(totalBytes)} />
        <Stat label="Upstream errors" value={formatNum(latest?.upstream_errors_total)} />
      </div>

      {(current?.cpu != null || current?.memory_mb != null) && (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <Stat label="Uptime" value={`${formatNum(latest?.uptime_secs)} s`} />
          <Stat label="H2 requests" value={formatNum(latest?.h2_requests_total)} />
          <Stat label="H3 requests" value={formatNum(latest?.h3_requests_total)} />
          <Stat
            label="Process"
            value={
              current.cpu != null && current.memory_mb != null
                ? `${current.cpu.toFixed(1)}% · ${current.memory_mb} MiB`
                : current.cpu != null
                  ? `${current.cpu.toFixed(1)}% CPU`
                  : `${current.memory_mb} MiB`
            }
          />
        </div>
      )}

      <Card>
        <h2 className="mb-1 text-lg font-semibold">Requests / second</h2>
        <p className="mb-3 text-xs text-text-secondary">
          Derived from successive snapshots{live ? ' (live every 3s)' : ''}. First point appears after the second sample.
        </p>
        {rates.length === 0 ? (
          <EmptyChart message={loading ? 'Loading…' : 'No time-series data yet — wait for the next sample.'} />
        ) : (
          <div className="h-64 w-full">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={rates} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
                <defs>
                  <linearGradient id="rpsFill" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor="var(--color-primary)" stopOpacity={0.35} />
                    <stop offset="100%" stopColor="var(--color-primary)" stopOpacity={0} />
                  </linearGradient>
                </defs>
                <CartesianGrid stroke="var(--color-border)" strokeDasharray="3 3" vertical={false} />
                <XAxis dataKey="time" tick={{ fill: 'var(--color-text-secondary)', fontSize: 11 }} />
                <YAxis allowDecimals tick={{ fill: 'var(--color-text-secondary)', fontSize: 11 }} width={48} />
                <Tooltip
                  contentStyle={chartTooltipStyle}
                  labelStyle={{ color: 'var(--color-text-secondary)' }}
                  formatter={(value: number) => [formatRate(value), 'Requests']}
                />
                <Area
                  type="monotone"
                  dataKey="requests"
                  stroke="var(--color-primary)"
                  fill="url(#rpsFill)"
                  strokeWidth={2}
                  isAnimationActive={false}
                />
              </AreaChart>
            </ResponsiveContainer>
          </div>
        )}
      </Card>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <h2 className="mb-3 text-lg font-semibold">Protocol totals</h2>
          {protocolBars.length === 0 ? (
            <EmptyChart message={loading ? 'Loading…' : 'No protocol counters yet.'} />
          ) : (
            <div className="h-56 w-full">
              <ResponsiveContainer width="100%" height="100%">
                <BarChart data={protocolBars} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
                  <CartesianGrid stroke="var(--color-border)" strokeDasharray="3 3" vertical={false} />
                  <XAxis dataKey="name" tick={{ fill: 'var(--color-text-secondary)', fontSize: 11 }} />
                  <YAxis allowDecimals={false} tick={{ fill: 'var(--color-text-secondary)', fontSize: 11 }} width={48} />
                  <Tooltip contentStyle={chartTooltipStyle} formatter={(value: number) => [value.toLocaleString(), 'Count']} />
                  <Bar dataKey="count" fill="var(--color-blue-b1)" radius={[4, 4, 0, 0]} isAnimationActive={false} />
                </BarChart>
              </ResponsiveContainer>
            </div>
          )}
        </Card>

        <Card>
          <h2 className="mb-3 text-lg font-semibold">Network throughput</h2>
          {rates.length === 0 ? (
            <EmptyChart message={loading ? 'Loading…' : 'No throughput samples yet.'} />
          ) : (
            <div className="h-56 w-full">
              <ResponsiveContainer width="100%" height="100%">
                <AreaChart data={rates} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
                  <defs>
                    <linearGradient id="sentFill" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="0%" stopColor="var(--color-green-g1)" stopOpacity={0.35} />
                      <stop offset="100%" stopColor="var(--color-green-g1)" stopOpacity={0} />
                    </linearGradient>
                    <linearGradient id="recvFill" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="0%" stopColor="var(--color-yellow-y1)" stopOpacity={0.3} />
                      <stop offset="100%" stopColor="var(--color-yellow-y1)" stopOpacity={0} />
                    </linearGradient>
                  </defs>
                  <CartesianGrid stroke="var(--color-border)" strokeDasharray="3 3" vertical={false} />
                  <XAxis dataKey="time" tick={{ fill: 'var(--color-text-secondary)', fontSize: 11 }} />
                  <YAxis tick={{ fill: 'var(--color-text-secondary)', fontSize: 11 }} width={48} />
                  <Tooltip
                    contentStyle={chartTooltipStyle}
                    formatter={(value: number, name: string) => [`${value.toFixed(2)} KiB/s`, name === 'sent_kibps' ? 'Sent' : 'Received']}
                  />
                  <Area type="monotone" dataKey="sent_kibps" stroke="var(--color-green-g1)" fill="url(#sentFill)" strokeWidth={2} isAnimationActive={false} />
                  <Area type="monotone" dataKey="recv_kibps" stroke="var(--color-yellow-y1)" fill="url(#recvFill)" strokeWidth={2} isAnimationActive={false} />
                </AreaChart>
              </ResponsiveContainer>
            </div>
          )}
        </Card>
      </div>

      <Card>
        <h2 className="mb-3 text-lg font-semibold">Per-site metrics</h2>
        {siteRows.length === 0 ? (
          <p className="text-sm text-text-secondary">{loading ? 'Loading…' : 'No site metrics yet.'}</p>
        ) : (
          <div className="overflow-x-auto rounded-lg border border-border">
            <table className="min-w-full text-sm">
              <thead className="border-b border-border bg-surface-elevated text-left text-text-secondary">
                <tr>
                  <th className="px-3 py-2.5 font-medium">Domain</th>
                  <th className="px-3 py-2.5 font-medium">H2</th>
                  <th className="px-3 py-2.5 font-medium">H3</th>
                  <th className="px-3 py-2.5 font-medium">Total</th>
                  <th className="px-3 py-2.5 font-medium">H3/H2</th>
                </tr>
              </thead>
              <tbody>
                {siteRows.map((row) => (
                  <tr key={row.domain} className="border-t border-border hover:bg-hover/40">
                    <td className="whitespace-nowrap px-3 py-2 font-medium">{row.domain}</td>
                    <td className="px-3 py-2">{formatNum(row.h2)}</td>
                    <td className="px-3 py-2">{formatNum(row.h3)}</td>
                    <td className="px-3 py-2">{formatNum(row.total)}</td>
                    <td className="px-3 py-2">
                      {!Number.isFinite(row.ratio) ? '∞' : row.ratio === 0 ? '—' : row.ratio.toFixed(3)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>

      {showRaw && latest ? (
        <Card>
          <h2 className="mb-3 text-lg font-semibold">Raw snapshot</h2>
          <pre className="overflow-x-auto rounded-md border border-border bg-bg p-3 font-mono text-xs">
            {JSON.stringify(rawSnapshot, null, 2)}
          </pre>
        </Card>
      ) : null}
    </div>
  );
}
