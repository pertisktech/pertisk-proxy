import { useEffect, useState } from 'react';
import { Activity, Globe, RefreshCw } from 'lucide-react';
import { api, type LogEntry } from '@/api/client';
import { Card } from '@/components/Card';
import { formatDateTime } from '@/utils/dateFormat';
import { cn } from '@/utils';

type LogKind = 'system' | 'http';

function levelClass(level?: string) {
  const l = level?.toLowerCase();
  if (l === 'error') return 'text-red-r1';
  if (l === 'warn') return 'text-yellow-y1';
  return 'text-text-secondary';
}

function typeLabel(entry: LogEntry) {
  switch (entry.type) {
    case 'health_check':
      return 'Health';
    case 'config_reload':
      return 'Config';
    case 'tracing':
      return 'Tracing';
    case 'error':
      return 'Error';
    case 'response':
      return 'Response';
    default:
      return entry.type ?? '—';
  }
}

function protocolLabel(raw?: string | null) {
  const p = raw?.trim() || '';
  if (!p) return '—';
  if (p === '1.1' || p === 'HTTP/1.1') return 'HTTP/1.1';
  if (p === '2' || p === 'HTTP/2') return 'HTTP/2';
  if (p === '3' || p === 'HTTP/3') return 'HTTP/3';
  return p;
}

function LogsTable({ kind, entries }: { kind: LogKind; entries: LogEntry[] }) {
  const isHttp = kind === 'http';

  return (
    <div className="overflow-x-auto rounded-lg border border-border">
      <table className="min-w-full text-sm">
        <thead className="border-b border-border bg-surface-elevated text-left text-text-secondary">
          <tr>
            <th className="px-3 py-2.5 font-medium">Time</th>
            <th className="px-3 py-2.5 font-medium">Level</th>
            {isHttp ? (
              <>
                <th className="px-3 py-2.5 font-medium">Method</th>
                <th className="px-3 py-2.5 font-medium">Proto</th>
                <th className="px-3 py-2.5 font-medium">Host</th>
                <th className="px-3 py-2.5 font-medium">Path</th>
                <th className="px-3 py-2.5 font-medium">Upstream</th>
                <th className="px-3 py-2.5 font-medium">Status</th>
                <th className="px-3 py-2.5 font-medium">Duration</th>
              </>
            ) : (
              <>
                <th className="px-3 py-2.5 font-medium">Type</th>
                <th className="px-3 py-2.5 font-medium">Source</th>
              </>
            )}
            <th className="px-3 py-2.5 font-medium">Message</th>
          </tr>
        </thead>
        <tbody>
          {entries.map((entry, i) => (
            <tr key={`${entry.timestamp}-${i}`} className="border-t border-border hover:bg-hover/50">
              <td className="whitespace-nowrap px-3 py-2 font-mono text-xs">
                {entry.timestamp ? formatDateTime(entry.timestamp) : '—'}
              </td>
              <td className={cn('px-3 py-2 capitalize', levelClass(entry.level))}>{entry.level ?? '—'}</td>
              {isHttp ? (
                <>
                  <td className="px-3 py-2 font-mono">{entry.method ?? '—'}</td>
                  <td className="px-3 py-2 font-mono text-xs">{protocolLabel(entry.protocol)}</td>
                  <td className="max-w-[10rem] truncate px-3 py-2" title={entry.host ?? undefined}>
                    {entry.host ?? '—'}
                  </td>
                  <td className="max-w-[12rem] truncate px-3 py-2 font-mono text-xs" title={entry.path ?? undefined}>
                    {entry.path ?? '—'}
                  </td>
                  <td className="max-w-[10rem] truncate px-3 py-2 font-mono text-xs" title={entry.upstream ?? undefined}>
                    {entry.upstream ?? '—'}
                  </td>
                  <td className="px-3 py-2 font-mono">{entry.status ?? '—'}</td>
                  <td className="whitespace-nowrap px-3 py-2 font-mono">
                    {entry.duration_ms != null ? `${entry.duration_ms} ms` : '—'}
                  </td>
                </>
              ) : (
                <>
                  <td className="px-3 py-2">{typeLabel(entry)}</td>
                  <td className="max-w-[12rem] truncate px-3 py-2 font-mono text-xs" title={entry.upstream ?? entry.host ?? undefined}>
                    {entry.upstream ?? entry.host ?? '—'}
                  </td>
                </>
              )}
              <td className="max-w-[16rem] truncate px-3 py-2" title={entry.message || undefined}>
                {entry.message || '—'}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function Logs() {
  const [kind, setKind] = useState<LogKind>('http');
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [hostFilter, setHostFilter] = useState('');
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        const data = await api.logs({
          type: kind === 'http' ? 'proxy' : 'system',
          host: kind === 'http' && hostFilter.trim() ? hostFilter.trim() : undefined,
        });
        if (!cancelled) {
          setEntries([...data].reverse());
          setError('');
        }
      } catch (e) {
        if (!cancelled) {
          setEntries([]);
          setError(e instanceof Error ? e.message : 'Failed to load logs');
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    load();
    if (!autoRefresh) return;
    const timer = setInterval(load, 3000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [kind, hostFilter, autoRefresh]);

  const emptyMessage =
    kind === 'http'
      ? 'No HTTP log entries yet. Proxied requests will appear here when proxy_log is enabled.'
      : 'No system log entries yet. Startup, config reload, and internal events appear here.';

  return (
    <div className="space-y-4">
      <Card className="space-y-4">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <h2 className="text-lg font-semibold">Logs</h2>
            <p className="text-sm text-text-secondary">
              HTTP access and system events from the proxy runtime.
            </p>
          </div>
          <label className="flex items-center gap-2 rounded-md border border-border px-3 py-2 text-sm text-text-secondary">
            <RefreshCw size={14} className={cn(autoRefresh && 'text-primary')} />
            <input
              type="checkbox"
              checked={autoRefresh}
              onChange={(e) => setAutoRefresh(e.target.checked)}
            />
            Auto-refresh 3s
          </label>
        </div>

        <div className="flex flex-wrap items-end gap-4">
          <div className="tab-bar" role="tablist" aria-label="Log type">
            <button
              type="button"
              role="tab"
              aria-selected={kind === 'http'}
              onClick={() => setKind('http')}
              className={cn('tab-item', kind === 'http' && 'active')}
            >
              <Globe size={15} />
              HTTP
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={kind === 'system'}
              onClick={() => setKind('system')}
              className={cn('tab-item', kind === 'system' && 'active')}
            >
              <Activity size={15} />
              System
            </button>
          </div>

          {kind === 'http' ? (
            <div className="min-w-[12rem] flex-1 sm:max-w-xs">
              <label className="mb-1 block text-xs font-medium text-text-secondary">Host filter</label>
              <input
                type="text"
                value={hostFilter}
                onChange={(e) => setHostFilter(e.target.value)}
                placeholder="example.com"
                className="w-full rounded-md border border-border bg-bg px-3 py-2 text-sm outline-none focus:border-primary"
              />
            </div>
          ) : null}
        </div>
      </Card>

      {error ? <p className="text-red-r1">{error}</p> : null}

      {loading && entries.length === 0 ? (
        <p className="text-text-secondary">Loading…</p>
      ) : entries.length === 0 ? (
        <Card>
          <p className="text-sm text-text-secondary">{emptyMessage}</p>
        </Card>
      ) : (
        <LogsTable kind={kind} entries={entries} />
      )}
    </div>
  );
}
