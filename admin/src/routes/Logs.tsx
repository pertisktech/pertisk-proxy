import { useEffect, useMemo, useState } from 'react';
import { Activity, Filter, Globe, RefreshCw, Search, X } from 'lucide-react';
import { api, type LogEntry } from '@/api/client';
import { Card } from '@/components/Card';
import { Checkbox } from '@/components/Checkbox';
import { Pagination } from '@/components/Pagination';
import { formatDateTime } from '@/utils/dateFormat';
import { useLiveChannel } from '@/utils/useLiveChannel';
import { usePageSize } from '@/utils/usePageSize';
import { cn } from '@/utils';

type LogKind = 'system' | 'http';

type LogFilters = {
  host: string;
  level: string;
  method: string;
  status: string;
  path: string;
  q: string;
};

const EMPTY_FILTERS: LogFilters = {
  host: '',
  level: '',
  method: '',
  status: '',
  path: '',
  q: '',
};

const LEVEL_OPTIONS = [
  { value: '', label: 'All levels' },
  { value: 'debug', label: 'Debug' },
  { value: 'info', label: 'Info' },
  { value: 'warn', label: 'Warn' },
  { value: 'error', label: 'Error' },
];

const METHOD_OPTIONS = [
  { value: '', label: 'All methods' },
  { value: 'GET', label: 'GET' },
  { value: 'POST', label: 'POST' },
  { value: 'PUT', label: 'PUT' },
  { value: 'PATCH', label: 'PATCH' },
  { value: 'DELETE', label: 'DELETE' },
  { value: 'HEAD', label: 'HEAD' },
  { value: 'OPTIONS', label: 'OPTIONS' },
];

const STATUS_OPTIONS = [
  { value: '', label: 'All status' },
  { value: '2xx', label: '2xx' },
  { value: '3xx', label: '3xx' },
  { value: '4xx', label: '4xx' },
  { value: '5xx', label: '5xx' },
];

const fieldCls =
  'w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text outline-none focus:border-primary';

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

function statusMatches(status: number | null | undefined, want: string) {
  const w = want.trim().toLowerCase();
  if (!w) return true;
  if (status == null) return false;
  if (w === '2xx') return status >= 200 && status < 300;
  if (w === '3xx') return status >= 300 && status < 400;
  if (w === '4xx') return status >= 400 && status < 500;
  if (w === '5xx') return status >= 500 && status < 600;
  return String(status) === w;
}

function activeFilterChips(kind: LogKind, filters: LogFilters): { key: keyof LogFilters; label: string }[] {
  const chips: { key: keyof LogFilters; label: string }[] = [];
  if (filters.q.trim()) chips.push({ key: 'q', label: `Search: ${filters.q.trim()}` });
  if (filters.level) chips.push({ key: 'level', label: `Level: ${filters.level}` });
  if (kind === 'http' && filters.method) chips.push({ key: 'method', label: `Method: ${filters.method}` });
  if (kind === 'http' && filters.status) chips.push({ key: 'status', label: `Status: ${filters.status}` });
  if (kind === 'http' && filters.host.trim()) chips.push({ key: 'host', label: `Host: ${filters.host.trim()}` });
  if (kind === 'http' && filters.path.trim()) chips.push({ key: 'path', label: `Path: ${filters.path.trim()}` });
  return chips;
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
                  <td
                    className="max-w-[12rem] truncate px-3 py-2 font-mono text-xs"
                    title={entry.upstream ?? entry.host ?? undefined}
                  >
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
  const [filters, setFilters] = useState<LogFilters>(EMPTY_FILTERS);
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [page, setPage] = useState(1);
  const pageSize = usePageSize(44, 340);

  const setFilter = <K extends keyof LogFilters>(key: K, value: LogFilters[K]) => {
    setFilters((prev) => ({ ...prev, [key]: value }));
    setPage(1);
  };

  const clearFilters = () => {
    setFilters(EMPTY_FILTERS);
    setPage(1);
  };

  // Discrete selects go to API/SSE; text fields stay client-side so typing stays snappy.
  const apiParams = useMemo(
    () => ({
      type: (kind === 'http' ? 'proxy' : 'system') as 'proxy' | 'system',
      level: filters.level || undefined,
      method: kind === 'http' && filters.method ? filters.method : undefined,
      status: kind === 'http' && filters.status ? filters.status : undefined,
      limit: 2000,
    }),
    [kind, filters.level, filters.method, filters.status],
  );

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        const data = await api.logs(apiParams);
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
    return () => {
      cancelled = true;
    };
  }, [apiParams]);

  const logsFilter = useMemo(
    () => ({
      type: apiParams.type,
      level: apiParams.level ?? '',
      method: apiParams.method ?? '',
      status: apiParams.status ?? '',
    }),
    [apiParams],
  );

  useLiveChannel<LogEntry[]>('logs', {
    enabled: autoRefresh,
    logsFilter,
    onData: (data) => {
      setEntries([...data].reverse());
      setError('');
      setLoading(false);
      // Keep live view on the newest page unless the user navigated away.
      setPage((p) => (p === 1 ? 1 : p));
    },
  });

  // Extra client-side pass keeps typing snappy if SSE lags; mirrors API rules.
  const filteredEntries = useMemo(() => {
    const q = filters.q.trim().toLowerCase();
    const path = filters.path.trim().toLowerCase();
    const host = filters.host.trim().toLowerCase();
    return entries.filter((e) => {
      if (filters.level && e.level?.toLowerCase() !== filters.level) return false;
      if (kind === 'http' && filters.method) {
        if ((e.method ?? '').toUpperCase() !== filters.method) return false;
      }
      if (kind === 'http' && filters.status && !statusMatches(e.status, filters.status)) return false;
      if (kind === 'http' && host) {
        const h = (e.host ?? '').toLowerCase();
        if (h !== host && !h.endsWith(`.${host}`)) return false;
      }
      if (kind === 'http' && path && !(e.path ?? '').toLowerCase().includes(path)) return false;
      if (q) {
        const hay = [e.host, e.path, e.upstream, e.method, e.message]
          .filter(Boolean)
          .join(' ')
          .toLowerCase();
        if (!hay.includes(q)) return false;
      }
      return true;
    });
  }, [entries, filters, kind]);

  useEffect(() => {
    const totalPages = Math.max(1, Math.ceil(filteredEntries.length / pageSize));
    if (page > totalPages) setPage(totalPages);
  }, [filteredEntries.length, page, pageSize]);

  const pagedEntries = useMemo(() => {
    const start = (page - 1) * pageSize;
    return filteredEntries.slice(start, start + pageSize);
  }, [filteredEntries, page, pageSize]);

  const chips = activeFilterChips(kind, filters);
  const hasFilters = chips.length > 0;

  const emptyMessage =
    kind === 'http'
      ? hasFilters
        ? 'No HTTP log entries match the current filters.'
        : 'No HTTP log entries yet. Proxied requests will appear here when proxy_log is enabled.'
      : hasFilters
        ? 'No system log entries match the current filters.'
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
          <Checkbox
            checked={autoRefresh}
            onChange={setAutoRefresh}
            className="rounded-md border border-border px-3 py-2 text-sm"
            label={
              <>
                <RefreshCw size={14} className={cn('inline', autoRefresh && 'text-primary')} />
                Auto-refresh (live)
              </>
            }
          />
        </div>

        <div className="flex flex-wrap items-end gap-3">
          <div className="tab-bar" role="tablist" aria-label="Log type">
            <button
              type="button"
              role="tab"
              aria-selected={kind === 'http'}
              onClick={() => {
                setKind('http');
                setPage(1);
              }}
              className={cn('tab-item', kind === 'http' && 'active')}
            >
              <Globe size={15} />
              HTTP
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={kind === 'system'}
              onClick={() => {
                setKind('system');
                setPage(1);
              }}
              className={cn('tab-item', kind === 'system' && 'active')}
            >
              <Activity size={15} />
              System
            </button>
          </div>

          <p className="pb-2 text-xs text-text-secondary">
            {filteredEntries.length === entries.length
              ? `${filteredEntries.length} entr${filteredEntries.length === 1 ? 'y' : 'ies'}`
              : `${filteredEntries.length} of ${entries.length} entr${entries.length === 1 ? 'y' : 'ies'}`}
          </p>
        </div>

        <div className="space-y-3 rounded-lg border border-border bg-surface-elevated/40 p-3">
          <div className="flex items-center gap-2 text-xs font-medium uppercase tracking-wide text-text-secondary">
            <Filter size={14} />
            Filters
          </div>

          <div className="relative">
            <Search size={15} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-muted" />
            <input
              type="search"
              value={filters.q}
              onChange={(e) => setFilter('q', e.target.value)}
              placeholder="Search host, path, upstream, message…"
              className={cn(fieldCls, 'pl-9')}
              aria-label="Search logs"
            />
          </div>

          <div className={cn('grid gap-3', kind === 'http' ? 'sm:grid-cols-2 lg:grid-cols-5' : 'sm:grid-cols-2')}>
            <label className="block text-xs font-medium text-text-secondary">
              Level
              <select
                className={cn(fieldCls, 'mt-1')}
                value={filters.level}
                onChange={(e) => setFilter('level', e.target.value)}
              >
                {LEVEL_OPTIONS.map((o) => (
                  <option key={o.value || 'all'} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </label>

            {kind === 'http' ? (
              <>
                <label className="block text-xs font-medium text-text-secondary">
                  Method
                  <select
                    className={cn(fieldCls, 'mt-1')}
                    value={filters.method}
                    onChange={(e) => setFilter('method', e.target.value)}
                  >
                    {METHOD_OPTIONS.map((o) => (
                      <option key={o.value || 'all'} value={o.value}>
                        {o.label}
                      </option>
                    ))}
                  </select>
                </label>

                <label className="block text-xs font-medium text-text-secondary">
                  Status
                  <select
                    className={cn(fieldCls, 'mt-1')}
                    value={filters.status}
                    onChange={(e) => setFilter('status', e.target.value)}
                  >
                    {STATUS_OPTIONS.map((o) => (
                      <option key={o.value || 'all'} value={o.value}>
                        {o.label}
                      </option>
                    ))}
                  </select>
                </label>

                <label className="block text-xs font-medium text-text-secondary">
                  Host
                  <input
                    type="text"
                    value={filters.host}
                    onChange={(e) => setFilter('host', e.target.value)}
                    placeholder="example.com"
                    className={cn(fieldCls, 'mt-1')}
                  />
                </label>

                <label className="block text-xs font-medium text-text-secondary">
                  Path
                  <input
                    type="text"
                    value={filters.path}
                    onChange={(e) => setFilter('path', e.target.value)}
                    placeholder="/api"
                    className={cn(fieldCls, 'mt-1')}
                  />
                </label>
              </>
            ) : null}
          </div>

          {hasFilters ? (
            <div className="flex flex-wrap items-center gap-2">
              {chips.map((chip) => (
                <button
                  key={chip.key}
                  type="button"
                  onClick={() => setFilter(chip.key, '')}
                  className="inline-flex items-center gap-1 rounded-full border border-border bg-bg px-2.5 py-1 text-xs text-text hover:bg-hover"
                >
                  {chip.label}
                  <X size={12} className="text-muted" />
                </button>
              ))}
              <button
                type="button"
                onClick={clearFilters}
                className="text-xs font-medium text-primary hover:underline"
              >
                Clear all
              </button>
            </div>
          ) : null}
        </div>
      </Card>

      {error ? <p className="text-red-r1">{error}</p> : null}

      {loading && entries.length === 0 ? (
        <p className="text-text-secondary">Loading…</p>
      ) : filteredEntries.length === 0 ? (
        <Card>
          <p className="text-sm text-text-secondary">{emptyMessage}</p>
        </Card>
      ) : (
        <div className="space-y-3">
          <LogsTable kind={kind} entries={pagedEntries} />
          <Pagination
            totalItems={filteredEntries.length}
            pageSize={pageSize}
            page={page}
            onPageChange={setPage}
          />
        </div>
      )}
    </div>
  );
}
