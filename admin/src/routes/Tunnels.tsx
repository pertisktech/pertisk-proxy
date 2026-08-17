import { useCallback, useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { Cable, RefreshCw } from 'lucide-react';
import { api } from '@/api/client';
import { cn } from '@/utils';

type TunnelEntry = {
  name: string;
  remote_port: number;
  connected: boolean;
  client_addr?: string | null;
};

type TunnelStatus = {
  online: boolean;
  tunnels: TunnelEntry[];
};

export function Tunnels() {
  const [status, setStatus] = useState<TunnelStatus | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const data = await api.tunnelStatus();
      setStatus(data);
    } catch (err) {
      setStatus(null);
      setError(err instanceof Error ? err.message : 'Failed to load tunnel status');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
    const id = window.setInterval(() => void load(), 10000);
    return () => window.clearInterval(id);
  }, [load]);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold">Tunnels</h2>
          <p className="mt-1 text-sm text-text-secondary">
            Reverse tunnels from a local machine to this VPS. Public HTTPS stays on{' '}
            <Link to="/sites" className="text-primary hover:underline">
              Sites
            </Link>
            .
          </p>
        </div>
        <button
          type="button"
          onClick={() => void load()}
          className="inline-flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-sm hover:bg-hover"
        >
          <RefreshCw size={14} className={cn(loading && 'animate-spin')} />
          Refresh
        </button>
      </div>

      <div className="rounded-lg border border-border p-4">
        <div className="flex items-center gap-2 text-sm font-medium">
          <Cable size={16} />
          Live status
        </div>
        {error ? (
          <p className="mt-3 text-sm text-yellow-y1">{error}</p>
        ) : loading && !status ? (
          <p className="mt-3 text-sm text-text-secondary">Loading…</p>
        ) : status ? (
          <div className="mt-3 space-y-2">
            <p className="text-sm">
              Overall:{' '}
              <span className={status.online ? 'text-green-g1' : 'text-muted'}>
                {status.online ? 'Online' : 'Offline'}
              </span>
            </p>
            <div className="overflow-x-auto rounded-md border border-border">
              <table className="w-full text-left text-sm">
                <thead className="border-b border-border bg-surface-elevated text-text-secondary">
                  <tr>
                    <th className="px-3 py-2 font-medium">Name</th>
                    <th className="px-3 py-2 font-medium">Upstream</th>
                    <th className="px-3 py-2 font-medium">Client</th>
                    <th className="px-3 py-2 font-medium">State</th>
                  </tr>
                </thead>
                <tbody>
                  {(status.tunnels ?? []).map((t) => (
                    <tr key={t.name} className="border-b border-border last:border-0">
                      <td className="px-3 py-2 font-medium">{t.name}</td>
                      <td className="px-3 py-2 font-mono text-xs">
                        http://127.0.0.1:{t.remote_port}
                      </td>
                      <td className="px-3 py-2 font-mono text-xs text-text-secondary">
                        {t.client_addr || '—'}
                      </td>
                      <td className={cn('px-3 py-2', t.connected ? 'text-green-g1' : 'text-muted')}>
                        {t.connected ? 'Connected' : 'Waiting'}
                      </td>
                    </tr>
                  ))}
                  {(status.tunnels ?? []).length === 0 ? (
                    <tr>
                      <td colSpan={4} className="px-3 py-4 text-text-secondary">
                        No tunnels configured on the tunnel server.
                      </td>
                    </tr>
                  ) : null}
                </tbody>
              </table>
            </div>
          </div>
        ) : null}
      </div>

      <div className="space-y-3 rounded-lg border border-border p-4 text-sm text-text-secondary">
        <h3 className="font-semibold text-text">Setup</h3>
        <ol className="list-decimal space-y-2 pl-5">
          <li>
            Run <code className="font-mono text-xs">pertisk-tunnel-server</code> on this VPS with a
            strong token. Open firewall <strong>UDP 7000</strong> only.
          </li>
          <li>
            Run <code className="font-mono text-xs">pertisk-tunnel-client</code> on your laptop
            pointing at this host and the same token.
          </li>
          <li>
            Create a{' '}
            <Link to="/sites" className="text-primary hover:underline">
              Site
            </Link>{' '}
            with upstream <code className="font-mono text-xs">http://127.0.0.1:&lt;remote_port&gt;</code>{' '}
            and generate TLS as usual.
          </li>
        </ol>
        <p>
          Full guide:{' '}
          <code className="font-mono text-xs">docs/tunnel.md</code>. Status URL override:{' '}
          <code className="font-mono text-xs">PERTISK_TUNNEL_STATUS_URL</code> (default{' '}
          <code className="font-mono text-xs">http://127.0.0.1:7700/status</code>).
        </p>
      </div>
    </div>
  );
}
