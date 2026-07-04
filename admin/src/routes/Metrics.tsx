import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import {
  Area,
  AreaChart,
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';
import { api, type Metrics as ApiMetrics, type ManagementInfo } from '@/api/client';
import { Card } from '@/components/Card';
import { formatTimeOnly } from '@/utils/dateFormat';

const POLL_INTERVAL_MS = 5000;
const MAX_POINTS = 60;

const CHART = {
  primary: 'var(--color-primary-p3)',
  secondary: 'var(--color-blue-b1)',
  tertiary: 'var(--color-green-g1)',
  quaternary: 'var(--color-yellow-y1)',
  cpu: 'var(--color-usage-cpu)',
  memory: 'var(--color-usage-memory)',
  storage: 'var(--color-usage-storage)',
};

interface MetricPoint {
  t: number;
  timeLabel: string;
  mode: string;
  uptime_secs: number;
  log_entries: number;
  active_connections: number;
  http_requests_total: number;
  https_requests_total: number;
  grpc_requests_total: number;
  h2_requests_total: number;
  h3_requests_total: number;
  h3_vs_h2_ratio: number;
  bytes_sent_total: number;
  bytes_received_total: number;
  site_h2_requests_total: Record<string, number>;
  site_h3_requests_total: Record<string, number>;
  cpu_percent: number | null;
  memory_used_mb: number | null;
}

function formatTime(ms: number): string {
  return formatTimeOnly(new Date(ms).toISOString());
}

function formatMemoryMb(value: number): string {
  return `${value.toFixed(1)} MB`;
}

function formatThroughputKiB(value: number): string {
  return `${value.toFixed(2)} KiB/s`;
}

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  const kib = value / 1024;
  if (kib < 1024) return `${kib.toFixed(1)} KiB`;
  const mib = kib / 1024;
  if (mib < 1024) return `${mib.toFixed(1)} MiB`;
  return `${(mib / 1024).toFixed(2)} GiB`;
}

function deltaRate(curr: number, prev: number, seconds: number): number {
  if (seconds <= 0 || curr < prev) return 0;
  return (curr - prev) / seconds;
}

function formatRate(value: number): string {
  return `${value.toFixed(2)}/s`;
}

function selectBaselinePoint(history: MetricPoint[], windowMs: number): MetricPoint | null {
  if (history.length < 2) return null;
  const latest = history[history.length - 1];
  for (let i = history.length - 2; i >= 0; i -= 1) {
    if (latest.t - history[i].t >= windowMs) return history[i];
  }
  return history[history.length - 2] ?? null;
}

function SummaryCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-border bg-surface-elevated p-3">
      <div className="text-xs text-text-secondary">{label}</div>
      <div className="mt-1 font-mono text-sm font-semibold">{value}</div>
    </div>
  );
}

export function Metrics() {
  const [history, setHistory] = useState<MetricPoint[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;

    function tick() {
      Promise.all([api.metrics(), api.management()])
        .then(([m, mgmt]: [ApiMetrics, ManagementInfo]) => {
          if (cancelled) return;
          const t = Date.now();
          const point: MetricPoint = {
            t,
            timeLabel: formatTime(t),
            mode: mgmt.mode,
            uptime_secs: m.uptime_secs,
            log_entries: m.log_entries,
            active_connections: m.active_connections ?? 0,
            http_requests_total: m.http_requests_total ?? 0,
            https_requests_total: m.https_requests_total ?? 0,
            grpc_requests_total: m.grpc_requests_total ?? 0,
            h2_requests_total: m.h2_requests_total ?? 0,
            h3_requests_total: m.h3_requests_total ?? 0,
            h3_vs_h2_ratio: m.h3_vs_h2_ratio ?? 0,
            bytes_sent_total: m.bytes_sent_total ?? 0,
            bytes_received_total: m.bytes_received_total ?? 0,
            site_h2_requests_total: m.site_h2_requests_total ?? {},
            site_h3_requests_total: m.site_h3_requests_total ?? {},
            cpu_percent: mgmt.process_cpu_usage_percent ?? null,
            memory_used_mb:
              mgmt.process_memory_bytes != null
                ? Math.round((mgmt.process_memory_bytes / (1024 * 1024)) * 10) / 10
                : null,
          };
          setHistory((prev) => [...prev, point].slice(-MAX_POINTS));
          setError(null);
        })
        .catch((e) => {
          if (!cancelled) setError(e instanceof Error ? e.message : 'Failed to load metrics');
        })
        .finally(() => {
          if (!cancelled) setLoading(false);
        });
    }

    tick();
    const id = setInterval(tick, POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  const hasCpu = useMemo(() => history.some((p) => p.cpu_percent != null), [history]);
  const hasMemory = useMemo(() => history.some((p) => p.memory_used_mb != null), [history]);
  const mode = history[history.length - 1]?.mode;

  const requestsTrend = useMemo(() => {
    if (history.length < 2) return [];
    const out: Array<{ timeLabel: string; http_rps: number; https_rps: number; h3_rps: number; grpc_rps: number }> = [];
    for (let i = 1; i < history.length; i += 1) {
      const prev = history[i - 1];
      const curr = history[i];
      const seconds = (curr.t - prev.t) / 1000;
      out.push({
        timeLabel: curr.timeLabel,
        http_rps: deltaRate(curr.http_requests_total, prev.http_requests_total, seconds),
        https_rps: deltaRate(curr.https_requests_total, prev.https_requests_total, seconds),
        h3_rps: deltaRate(curr.h3_requests_total, prev.h3_requests_total, seconds),
        grpc_rps: deltaRate(curr.grpc_requests_total, prev.grpc_requests_total, seconds),
      });
    }
    return out;
  }, [history]);

  const throughputTrend = useMemo(() => {
    if (history.length < 2) return [];
    const out: Array<{ timeLabel: string; sent_kibps: number; recv_kibps: number }> = [];
    for (let i = 1; i < history.length; i += 1) {
      const prev = history[i - 1];
      const curr = history[i];
      const seconds = (curr.t - prev.t) / 1000;
      out.push({
        timeLabel: curr.timeLabel,
        sent_kibps: deltaRate(curr.bytes_sent_total, prev.bytes_sent_total, seconds) / 1024,
        recv_kibps: deltaRate(curr.bytes_received_total, prev.bytes_received_total, seconds) / 1024,
      });
    }
    return out;
  }, [history]);

  const hostProtocolRows = useMemo(() => {
    if (history.length === 0) return [];
    const latest = history[history.length - 1];
    const hosts = new Set([
      ...Object.keys(latest.site_h2_requests_total ?? {}),
      ...Object.keys(latest.site_h3_requests_total ?? {}),
    ]);
    return Array.from(hosts)
      .map((host) => ({
        host,
        h2: latest.site_h2_requests_total?.[host] ?? 0,
        h3: latest.site_h3_requests_total?.[host] ?? 0,
        ratio: (latest.site_h3_requests_total?.[host] ?? 0) > 0 && (latest.site_h2_requests_total?.[host] ?? 0) > 0
          ? (latest.site_h3_requests_total?.[host] ?? 0) / (latest.site_h2_requests_total?.[host] ?? 1)
          : 0,
      }))
      .sort((a, b) => b.h2 + b.h3 - (a.h2 + a.h3));
  }, [history]);

  const currentSnapshot = useMemo(() => {
    if (history.length === 0) return null;
    const latest = history[history.length - 1];
    const baseline = selectBaselinePoint(history, 30_000);
    if (!baseline) {
      return { latest, req_total_rps: null, h2_rps: null, h3_rps: null, sent_kibps: null, recv_kibps: null, sampleWindowSec: null };
    }
    const seconds = (latest.t - baseline.t) / 1000;
    return {
      latest,
      req_total_rps: deltaRate(
        latest.http_requests_total + latest.https_requests_total + latest.grpc_requests_total,
        baseline.http_requests_total + baseline.https_requests_total + baseline.grpc_requests_total,
        seconds,
      ),
      h2_rps: deltaRate(latest.h2_requests_total, baseline.h2_requests_total, seconds),
      h3_rps: deltaRate(latest.h3_requests_total, baseline.h3_requests_total, seconds),
      sent_kibps: deltaRate(latest.bytes_sent_total, baseline.bytes_sent_total, seconds) / 1024,
      recv_kibps: deltaRate(latest.bytes_received_total, baseline.bytes_received_total, seconds) / 1024,
      sampleWindowSec: seconds,
    };
  }, [history]);

  if (error && history.length === 0) {
    return <p className="text-red-r1">{error}</p>;
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center gap-2">
        {mode ? (
          <span className="badge status-gray">{mode === 'ingress' ? 'Ingress mode' : 'Proxy mode'}</span>
        ) : null}
        {!loading && history.length > 0 ? (
          <span className="badge status-green">Live · 5s refresh</span>
        ) : null}
        <Link to="/" className="ml-auto text-sm text-primary hover:underline">
          Back to dashboard
        </Link>
      </div>

      {currentSnapshot ? (
        <Card>
          <h2 className="mb-1 text-lg font-semibold">Current snapshot</h2>
          <p className="mb-4 text-sm text-text-secondary">
            {currentSnapshot.sampleWindowSec == null
              ? 'Collecting traffic sample window…'
              : `Rates averaged over the last ${Math.max(1, Math.round(currentSnapshot.sampleWindowSec))}s`}
          </p>
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4 xl:grid-cols-5">
            <SummaryCard label="Uptime" value={`${currentSnapshot.latest.uptime_secs.toLocaleString()} s`} />
            <SummaryCard label="Active connections" value={currentSnapshot.latest.active_connections.toLocaleString()} />
            <SummaryCard label="HTTP total" value={currentSnapshot.latest.http_requests_total.toLocaleString()} />
            <SummaryCard label="HTTPS total" value={currentSnapshot.latest.https_requests_total.toLocaleString()} />
            <SummaryCard label="gRPC total" value={currentSnapshot.latest.grpc_requests_total.toLocaleString()} />
            <SummaryCard
              label="Request rate"
              value={currentSnapshot.req_total_rps == null ? 'Collecting…' : formatRate(currentSnapshot.req_total_rps)}
            />
            <SummaryCard label="H2 total" value={currentSnapshot.latest.h2_requests_total.toLocaleString()} />
            <SummaryCard label="H3 total" value={currentSnapshot.latest.h3_requests_total.toLocaleString()} />
            <SummaryCard
              label="H2 rate"
              value={currentSnapshot.h2_rps == null ? 'Collecting…' : formatRate(currentSnapshot.h2_rps)}
            />
            <SummaryCard
              label="H3 rate"
              value={currentSnapshot.h3_rps == null ? 'Collecting…' : formatRate(currentSnapshot.h3_rps)}
            />
            <SummaryCard label="Sent total" value={formatBytes(currentSnapshot.latest.bytes_sent_total)} />
            <SummaryCard label="Received total" value={formatBytes(currentSnapshot.latest.bytes_received_total)} />
            <SummaryCard
              label="Sent throughput"
              value={currentSnapshot.sent_kibps == null ? 'Collecting…' : formatThroughputKiB(currentSnapshot.sent_kibps)}
            />
            <SummaryCard
              label="Received throughput"
              value={currentSnapshot.recv_kibps == null ? 'Collecting…' : formatThroughputKiB(currentSnapshot.recv_kibps)}
            />
            {currentSnapshot.latest.cpu_percent != null ? (
              <SummaryCard label="CPU" value={`${currentSnapshot.latest.cpu_percent.toFixed(1)}%`} />
            ) : null}
            {currentSnapshot.latest.memory_used_mb != null ? (
              <SummaryCard label="Memory" value={formatMemoryMb(currentSnapshot.latest.memory_used_mb)} />
            ) : null}
          </div>
        </Card>
      ) : null}

      <Card>
        <h2 className="mb-1 text-lg font-semibold">Protocol request rate</h2>
        <p className="mb-4 text-sm text-text-secondary">HTTP/1.x, HTTPS, HTTP/3, and gRPC requests per second</p>
        <div className="h-[220px]">
          {requestsTrend.length === 0 ? (
            <div className="flex h-full items-center justify-center text-sm text-text-secondary">Collecting data…</div>
          ) : (
            <ResponsiveContainer width="100%" height="100%">
              <LineChart data={requestsTrend} margin={{ top: 8, right: 16, left: 8, bottom: 8 }}>
                <CartesianGrid strokeDasharray="3 3" stroke="var(--color-border)" />
                <XAxis dataKey="timeLabel" tick={{ fontSize: 11, fill: 'var(--color-text-secondary)' }} interval="preserveStartEnd" />
                <YAxis tick={{ fontSize: 11, fill: 'var(--color-text-secondary)' }} />
                <Tooltip formatter={(value: number, name: string) => [formatRate(value), name]} />
                <Legend />
                <Line type="monotone" dataKey="http_rps" name="HTTP/1.x" stroke={CHART.quaternary} strokeWidth={2} dot={false} isAnimationActive={false} />
                <Line type="monotone" dataKey="https_rps" name="HTTPS" stroke={CHART.secondary} strokeWidth={2} dot={false} isAnimationActive={false} />
                <Line type="monotone" dataKey="h3_rps" name="HTTP/3" stroke={CHART.tertiary} strokeWidth={2} dot={false} isAnimationActive={false} />
                <Line type="monotone" dataKey="grpc_rps" name="gRPC" stroke={CHART.primary} strokeWidth={2} dot={false} isAnimationActive={false} />
              </LineChart>
            </ResponsiveContainer>
          )}
        </div>
      </Card>

      <Card>
        <h2 className="mb-1 text-lg font-semibold">Network throughput</h2>
        <p className="mb-4 text-sm text-text-secondary">Bytes sent and received per second (KiB/s)</p>
        <div className="h-[220px]">
          {throughputTrend.length === 0 ? (
            <div className="flex h-full items-center justify-center text-sm text-text-secondary">Collecting data…</div>
          ) : (
            <ResponsiveContainer width="100%" height="100%">
              <LineChart data={throughputTrend} margin={{ top: 8, right: 16, left: 8, bottom: 8 }}>
                <CartesianGrid strokeDasharray="3 3" stroke="var(--color-border)" />
                <XAxis dataKey="timeLabel" tick={{ fontSize: 11, fill: 'var(--color-text-secondary)' }} interval="preserveStartEnd" />
                <YAxis tick={{ fontSize: 11, fill: 'var(--color-text-secondary)' }} />
                <Tooltip formatter={(value: number, name: string) => [`${value.toFixed(2)} KiB/s`, name]} />
                <Legend />
                <Line type="monotone" dataKey="sent_kibps" name="Sent" stroke={CHART.tertiary} strokeWidth={2} dot={false} isAnimationActive={false} />
                <Line type="monotone" dataKey="recv_kibps" name="Received" stroke={CHART.quaternary} strokeWidth={2} dot={false} isAnimationActive={false} />
              </LineChart>
            </ResponsiveContainer>
          )}
        </div>
      </Card>

      <Card>
        <h2 className="mb-1 text-lg font-semibold">Runtime state</h2>
        <p className="mb-4 text-sm text-text-secondary">Connections, log buffer, and process resource usage</p>
        <div className="h-[220px]">
          {history.length === 0 ? (
            <div className="flex h-full items-center justify-center text-sm text-text-secondary">Collecting data…</div>
          ) : (
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={history} margin={{ top: 8, right: 16, left: 8, bottom: 8 }}>
                <defs>
                  <linearGradient id="connFill" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor={CHART.primary} stopOpacity={0.35} />
                    <stop offset="95%" stopColor={CHART.primary} stopOpacity={0.02} />
                  </linearGradient>
                  <linearGradient id="logFill" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor={CHART.secondary} stopOpacity={0.3} />
                    <stop offset="95%" stopColor={CHART.secondary} stopOpacity={0.02} />
                  </linearGradient>
                </defs>
                <CartesianGrid strokeDasharray="3 3" stroke="var(--color-border)" />
                <XAxis dataKey="timeLabel" tick={{ fontSize: 11, fill: 'var(--color-text-secondary)' }} interval="preserveStartEnd" />
                <YAxis yAxisId="left" tick={{ fontSize: 11, fill: 'var(--color-text-secondary)' }} />
                <YAxis yAxisId="right" orientation="right" tick={{ fontSize: 11, fill: 'var(--color-text-secondary)' }} />
                <Tooltip />
                <Legend />
                <Area yAxisId="left" type="monotone" dataKey="active_connections" name="Active connections" stroke={CHART.primary} fill="url(#connFill)" strokeWidth={2} isAnimationActive={false} />
                <Area yAxisId="right" type="monotone" dataKey="log_entries" name="Log entries" stroke={CHART.secondary} fill="url(#logFill)" strokeWidth={2} isAnimationActive={false} />
                {hasCpu ? (
                  <Line yAxisId="left" type="monotone" dataKey="cpu_percent" name="CPU %" stroke={CHART.cpu} strokeWidth={2} dot={false} isAnimationActive={false} />
                ) : null}
                {hasMemory ? (
                  <Line yAxisId="right" type="monotone" dataKey="memory_used_mb" name="Memory (MB)" stroke={CHART.memory} strokeWidth={2} dot={false} isAnimationActive={false} />
                ) : null}
              </AreaChart>
            </ResponsiveContainer>
          )}
        </div>
      </Card>

      {hostProtocolRows.length > 0 ? (
        <Card>
          <h2 className="mb-1 text-lg font-semibold">Host protocol summary</h2>
          <p className="mb-4 text-sm text-text-secondary">HTTP/2 and HTTP/3 request counters by host</p>
          <div className="overflow-x-auto rounded-lg border border-border">
            <table className="min-w-full text-left text-sm">
              <thead className="border-b border-border bg-surface-elevated text-text-secondary">
                <tr>
                  <th className="px-4 py-3 font-medium">Host</th>
                  <th className="px-4 py-3 font-medium">H2</th>
                  <th className="px-4 py-3 font-medium">H3</th>
                  <th className="px-4 py-3 font-medium">H3/H2 ratio</th>
                </tr>
              </thead>
              <tbody>
                {hostProtocolRows.map((row) => (
                  <tr key={row.host} className="border-t border-border hover:bg-hover/50">
                    <td className="px-4 py-3 font-medium">{row.host}</td>
                    <td className="px-4 py-3">{row.h2.toLocaleString()}</td>
                    <td className="px-4 py-3">{row.h3.toLocaleString()}</td>
                    <td className="px-4 py-3">{row.ratio.toFixed(3)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      ) : null}
    </div>
  );
}
