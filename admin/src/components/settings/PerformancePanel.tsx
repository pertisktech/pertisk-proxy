import type { ManagementInfo } from '@/api/client';
import { Card } from '@/components/Card';
import { formatBytes, SettingRow } from '@/components/settings/shared';

export function PerformancePanel({
  info,
  isIngress,
}: {
  info: ManagementInfo | null;
  isIngress: boolean;
}) {
  const tuning = info?.tuning;
  const kernel = tuning?.kernel;
  const quic = tuning?.effective_quic;
  const hasConfiguredHttp3 =
    info != null &&
    (info.http3.max_data != null ||
      info.http3.max_stream_data != null ||
      info.http3.max_streams_bidi != null ||
      info.http3.congestion_control != null);

  return (
    <div id="performance-tuning" className="scroll-mt-6 space-y-6">
      <Card>
        <h2 className="mb-1 text-lg font-semibold">Performance tuning</h2>
        <p className="mb-4 text-sm text-text-secondary">
          Effective values reported by the running process.
          {isIngress
            ? ' Change Helm values and redeploy to update them.'
            : ' Environment and systemd changes require a restart.'}{' '}
          See the Environment tab for configuration reference.
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
                  label="Downstream TCP keepalive"
                  value={
                    tuning.downstream_tcp_keepalive.enabled
                      ? `${tuning.downstream_tcp_keepalive.idle_secs}s / ${tuning.downstream_tcp_keepalive.interval_secs}s / ${tuning.downstream_tcp_keepalive.count}`
                      : 'off'
                  }
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
          <p className="mt-4 text-xs text-text-secondary">
            Stored route/config <code className="font-mono">http3</code> overrides
            {hasConfiguredHttp3 ? ' are present' : ' are unset'}; with Quinn they do not change the
            effective transport above.
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
          <p className="app-main-status">Kernel tuning is only available on supported Linux hosts.</p>
        )}
      </Card>
    </div>
  );
}
