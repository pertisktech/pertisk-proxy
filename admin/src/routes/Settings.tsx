import { useEffect, useState } from 'react';
import { api, type ManagementInfo } from '@/api/client';
import { Card } from '@/components/Card';

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

export function Settings() {
  const [version, setVersion] = useState('');
  const [authRequired, setAuthRequired] = useState(false);
  const [management, setManagement] = useState<ManagementInfo | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    Promise.all([api.version(), api.authConfig(), api.management()])
      .then(([v, auth, info]) => {
        setVersion(v.version);
        setAuthRequired(auth.auth_required);
        setManagement(info);
      })
      .catch((e) => setError(e instanceof Error ? e.message : 'Failed to load settings'));
  }, []);

  const tuning = management?.tuning;
  const kernel = tuning?.kernel;

  return (
    <div className="max-w-5xl space-y-6">
      {error ? <p className="text-red-r1">{error}</p> : null}
      <Card>
        <h2 className="mb-3 text-lg font-semibold">About</h2>
        <p className="text-text-secondary">Pertisk-Proxy management UI</p>
        <p className="mt-2 font-mono text-sm">Version {version || '…'}</p>
      </Card>
      <Card>
        <h2 className="mb-3 text-lg font-semibold">Security</h2>
        <p className="text-sm text-text-secondary">
          Management API auth: {authRequired ? 'enabled (username/password)' : 'disabled'}
        </p>
        <p className="mt-2 text-sm text-muted">
          Default credentials on first start: <code className="rounded bg-bg px-1">admin</code> /{' '}
          <code className="rounded bg-bg px-1">admin</code> (stored in SQLite). Change the password
          after first login.
        </p>
      </Card>

      <div id="performance-tuning" className="scroll-mt-6 space-y-6">
        <Card>
          <h2 className="mb-1 text-lg font-semibold">Performance tuning</h2>
          <p className="mb-4 text-sm text-text-secondary">
            Effective values reported by the running process. Environment and systemd changes require a restart.
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
                  <SettingRow label="Pingora service threads" value={tuning.pingora_service_threads} />
                  <SettingRow label="Accept tasks per listener" value={tuning.pingora_listener_tasks_per_fd} />
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
                  <SettingRow label="CPU affinity" value={kernel?.cpu_affinity ?? 'Unavailable'} />
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
            Values the active <code className="font-mono">{tuning?.h3_stack ?? 'HTTP/3'}</code> stack
            actually uses. Default builds use Quinn and read these from environment variables, not
            route-level <code className="font-mono">http3</code> config.
          </p>
          {tuning?.effective_quic ? (
            <div className="grid gap-x-8 lg:grid-cols-2">
              <dl>
                <SettingRow label="Source" value={tuning.effective_quic.source} />
                <SettingRow
                  label="Connection receive window"
                  value={formatBytes(tuning.effective_quic.conn_receive_window)}
                />
                <SettingRow
                  label="Stream receive window"
                  value={formatBytes(tuning.effective_quic.stream_receive_window)}
                />
                <SettingRow
                  label="Bidirectional streams"
                  value={tuning.effective_quic.max_streams_bidi}
                />
              </dl>
              <dl>
                <SettingRow
                  label="Idle timeout"
                  value={`${tuning.effective_quic.idle_timeout_secs}s`}
                />
                <SettingRow
                  label="Keepalive"
                  value={
                    tuning.effective_quic.keepalive_secs != null
                      ? `${tuning.effective_quic.keepalive_secs}s`
                      : 'n/a'
                  }
                />
                <SettingRow
                  label="UDP socket buffers"
                  value={formatBytes(tuning.effective_quic.udp_buffer_bytes)}
                />
                <SettingRow
                  label="Congestion control"
                  value={tuning.effective_quic.congestion_control ?? 'Quinn default'}
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
          {management ? (
            <p className="mt-4 text-xs text-muted">
              Stored route/config <code className="font-mono">http3</code> overrides
              {management.http3.max_data != null ||
              management.http3.max_stream_data != null ||
              management.http3.max_streams_bidi != null ||
              management.http3.congestion_control != null
                ? ' are present'
                : ' are unset'}
              ; with Quinn they do not change the effective transport above.
            </p>
          ) : null}
        </Card>

        <Card>
          <h2 className="mb-1 text-lg font-semibold">Linux network tuning</h2>
          <p className="mb-4 text-sm text-text-secondary">
            Live kernel values read from <code className="font-mono">/proc/sys</code>. “Review” means the value is below the packaged recommendation.
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
                <SettingRow label="Ephemeral port range" value={kernel.ip_local_port_range ?? 'Unavailable'} />
                <SettingRow label="TCP TIME_WAIT reuse" value={kernel.tcp_tw_reuse ?? 'Unavailable'} />
              </dl>
            </div>
          ) : (
            <p className="app-main-status">Kernel tuning is only available on supported Linux hosts.</p>
          )}
        </Card>

        <Card>
          <h2 className="mb-1 text-lg font-semibold">Configuration reference</h2>
          <p className="mb-4 text-sm text-text-secondary">
            Put these values in <code>/etc/pertisk-proxy/pertisk-proxy.conf</code>, then restart the service.
          </p>
          <ul className="grid gap-3 md:grid-cols-2">
            <EnvVar name="PERTISK_PROXY_MODE=performance" description="Enable CPU-scaled worker, listener, pool, and QUIC defaults." />
            <EnvVar name="PERTISK_WORKER_THREADS" description="Override Tokio worker count; normally leave unset to use available CPUs." />
            <EnvVar name="PERTISK_PINGORA_THREADS" description="Override HTTP/1 and HTTP/2 Pingora service threads." />
            <EnvVar name="PERTISK_PINGORA_LISTENER_TASKS" description="Parallel accept tasks per listener; performance default is 4." />
            <EnvVar name="PERTISK_TCP_LISTEN_BACKLOG" description="Application listen backlog; performance default is 8192." />
            <EnvVar name="PERTISK_H3_UPSTREAM_POOL_MAX_IDLE" description="Idle H3-to-upstream connections per host; performance default is 256." />
            <EnvVar name="PERTISK_H3_UPSTREAM_POOL_IDLE_TIMEOUT_SECS" description="How long idle upstream connections remain pooled." />
            <EnvVar name="PERTISK_H3_UPSTREAM_TCP_KEEPALIVE_SECS" description="TCP keepalive interval for H3-to-upstream connections." />
            <EnvVar name="PERTISK_HTTP3_MAX_STREAMS" description="Maximum concurrent QUIC streams." />
            <EnvVar name="PERTISK_HTTP3_STREAM_RECEIVE_WINDOW" description="QUIC per-stream receive window in bytes." />
            <EnvVar name="PERTISK_HTTP3_CONN_RECEIVE_WINDOW" description="QUIC connection receive window in bytes." />
            <EnvVar name="PERTISK_HTTP3_CC_ALGORITHM" description="QUIC congestion control for tokio-quiche: bbr, cubic, or reno." />
          </ul>
        </Card>

        <Card>
          <h2 className="mb-3 text-lg font-semibold">Deployment guide</h2>
          <ol className="list-decimal space-y-3 pl-5 text-sm text-text-secondary">
            <li>Install the RPM/DEB; it installs and applies <code>/etc/sysctl.d/99-pertisk-proxy.conf</code>.</li>
            <li>Set <code>PERTISK_PROXY_MODE=performance</code> in the service environment file.</li>
            <li>For dedicated hosts, copy the packaged CPU-affinity example into a systemd service drop-in and choose valid CPU IDs.</li>
            <li>Run <code>systemctl daemon-reload &amp;&amp; systemctl restart pertisk-proxy</code>.</li>
            <li>Return here and confirm all effective values. Benchmark from the same network region and watch p95/p99, CPU, RSS, and errors.</li>
          </ol>
          <pre className="mt-4 overflow-x-auto rounded-lg border border-border bg-bg p-4 text-xs text-text-secondary">
{`mkdir -p /etc/systemd/system/pertisk-proxy.service.d
cp /usr/share/pertisk-proxy/cpu-affinity.conf.example \\
  /etc/systemd/system/pertisk-proxy.service.d/cpu-affinity.conf
systemctl daemon-reload
systemctl restart pertisk-proxy`}
          </pre>
        </Card>
      </div>

      <Card>
        <h2 className="mb-3 text-lg font-semibold">General environment</h2>
        <ul className="space-y-1 text-sm font-mono text-text-secondary">
          <li>PERTISK_DB_PATH (default ./data/proxy.sqlite)</li>
          <li>PERTISK_MANAGEMENT_ADDR (default 127.0.0.1:9080)</li>
          <li>ROUTES_CONFIG (optional one-time migration from legacy yaml)</li>
          <li>PERTISK_ADMIN_UI_DEV_ORIGIN (Vite dev redirect)</li>
        </ul>
      </Card>
    </div>
  );
}
