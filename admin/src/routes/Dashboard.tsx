import { useEffect, useState } from 'react';
import { api, type ManagementInfo, type ProxyConfig } from '@/api/client';
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

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5 sm:flex-row sm:items-baseline sm:justify-between sm:gap-4">
      <dt className="text-sm text-text-secondary">{label}</dt>
      <dd className="font-mono text-sm break-all text-right sm:max-w-[65%]">{value}</dd>
    </div>
  );
}

function UsageBar({ percent, tone }: { percent: number; tone: 'cpu' | 'memory' }) {
  const fill = tone === 'cpu' ? 'bg-primary' : 'bg-green-g1';
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
  const [error, setError] = useState('');

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        const [mgmt, cfg] = await Promise.all([api.management(), api.config()]);
        if (!cancelled) {
          setInfo(mgmt);
          setConfig(cfg);
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

  if (error) return <p className="text-red-r1">{error}</p>;
  if (!info) return <p className="text-text-secondary">Loading…</p>;

  const systemMemoryPercent =
    info.memory_used_bytes != null && info.memory_total_bytes
      ? clampPercent((info.memory_used_bytes / info.memory_total_bytes) * 100)
      : null;
  const processMemoryPercent =
    info.process_memory_bytes != null && info.memory_total_bytes
      ? clampPercent((info.process_memory_bytes / info.memory_total_bytes) * 100)
      : null;
  const processCpu = info.process_cpu_usage_percent ?? null;

  return (
    <div className="space-y-6">
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <Stat label="Version" value={info.version} />
        <Stat label="Uptime" value={formatUptime(info.uptime_secs)} />
        <Stat label="Sites" value={info.site_count} />
        <Stat label="Routes" value={info.route_count} />
      </div>

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
    </div>
  );
}
