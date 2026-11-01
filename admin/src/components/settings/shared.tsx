import { Card } from '@/components/Card';

export function formatBytes(bytes: number | null | undefined) {
  if (bytes == null) return 'Unavailable';
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(bytes % (1024 * 1024) === 0 ? 0 : 1)} MiB`;
}

export function SettingRow({
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

export function EnvVar({ name, description }: { name: string; description: string }) {
  return (
    <li className="rounded-lg border border-border bg-surface-elevated p-3">
      <code className="text-sm text-primary">{name}</code>
      <p className="mt-1 text-xs text-text-secondary">{description}</p>
    </li>
  );
}

export function ProxyDeploymentGuide() {
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

export function IngressDeploymentGuide() {
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
