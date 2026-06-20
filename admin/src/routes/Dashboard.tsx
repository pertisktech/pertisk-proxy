import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { api, type ManagementInfo } from '@/api/client';
import { Card, Stat } from '@/components/Card';

function formatUptime(secs: number) {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  return `${h}h ${m}m ${s}s`;
}

export function Dashboard() {
  const [info, setInfo] = useState<ManagementInfo | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    api.management().then(setInfo).catch((e) => setError(e.message));
  }, []);

  if (error) return <p className="text-red-r1">{error}</p>;
  if (!info) return <p className="text-text-secondary">Loading…</p>;

  return (
    <div className="space-y-6">
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <Stat label="Version" value={info.version} />
        <Stat label="Uptime" value={formatUptime(info.uptime_secs)} />
        <Stat label="Routes" value={info.route_count} />
        <Stat label="TLS hosts" value={info.tls_host_count} />
      </div>

      <Card>
        <h2 className="mb-4 text-lg font-semibold">Runtime</h2>
        <dl className="grid gap-3 sm:grid-cols-2">
          <div><dt className="text-sm text-text-secondary">Mode</dt><dd>{info.mode}</dd></div>
          <div><dt className="text-sm text-text-secondary">Runtime</dt><dd>{info.runtime_mode}</dd></div>
          <div><dt className="text-sm text-text-secondary">HTTP</dt><dd className="font-mono text-sm">{info.listeners.http}</dd></div>
          <div><dt className="text-sm text-text-secondary">HTTPS</dt><dd className="font-mono text-sm">{info.listeners.https}</dd></div>
          <div><dt className="text-sm text-text-secondary">HTTP/3 UDP</dt><dd className="font-mono text-sm">{info.listeners.h3_udp}</dd></div>
          <div><dt className="text-sm text-text-secondary">Routes file</dt><dd className="font-mono text-sm">{info.routes_path}</dd></div>
          <div><dt className="text-sm text-text-secondary">HTTP/3</dt><dd>{info.enable_h3 ? 'enabled' : 'disabled'}</dd></div>
          <div><dt className="text-sm text-text-secondary">Auto HTTPS</dt><dd>{info.auto_https ? 'enabled' : 'disabled'}</dd></div>
        </dl>
      </Card>

      <Card>
        <h2 className="mb-3 text-lg font-semibold">Quick links</h2>
        <div className="flex flex-wrap gap-3">
          <Link to="/routes" className="rounded-md border border-border px-3 py-2 text-sm hover:bg-hover">Edit routes</Link>
          <Link to="/certificates" className="rounded-md border border-border px-3 py-2 text-sm hover:bg-hover">Certificates</Link>
          <a href="/api/health" target="_blank" rel="noreferrer" className="rounded-md border border-border px-3 py-2 text-sm hover:bg-hover">API health</a>
        </div>
      </Card>
    </div>
  );
}
