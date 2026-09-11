import { getToken } from '@/auth';

export type LiveChannel =
  | 'management'
  | 'metrics'
  | 'logs'
  | 'config'
  | 'certificates'
  | 'routes'
  | 'tunnel'
  | 'pods';

export type LogsFilter = {
  type?: 'proxy' | 'system' | 'http' | '';
  host?: string;
};

type ChannelHandler = (data: unknown) => void;

type ServerMsg =
  | { channel: string; data: unknown; error?: undefined }
  | { channel?: string; error: string; data?: undefined };

/**
 * Prefer SSE (`/api/live`) over WebSocket. Ingress terminates TLS with HTTP/2 preferred;
 * browser WebSocket-over-H2 fails with Pingora, while SSE streams normally.
 *
 * Live transport failures must never clear the session — only REST 401 does that.
 */
function liveUrl(channels: LiveChannel[], logsFilter: LogsFilter): string {
  const token = getToken();
  const params = new URLSearchParams();
  if (token) params.set('token', token);
  params.set('channels', channels.join(','));
  if (logsFilter.type) params.set('logs_type', logsFilter.type);
  if (logsFilter.host) params.set('logs_host', logsFilter.host);
  return `/api/live?${params.toString()}`;
}

class LiveClient {
  private es: EventSource | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private backoffMs = 1000;
  private readonly handlers = new Map<LiveChannel, Set<ChannelHandler>>();
  private readonly refCounts = new Map<LiveChannel, number>();
  private logsFilter: LogsFilter = {};
  private intentionalClose = false;
  private connectedKey = '';

  subscribe(channel: LiveChannel, handler: ChannelHandler): () => void {
    let set = this.handlers.get(channel);
    if (!set) {
      set = new Set();
      this.handlers.set(channel, set);
    }
    set.add(handler);
    const count = (this.refCounts.get(channel) ?? 0) + 1;
    this.refCounts.set(channel, count);
    this.syncConnection();
    return () => this.unsubscribe(channel, handler);
  }

  setLogsFilter(filter: LogsFilter) {
    const prev = `${this.logsFilter.type ?? ''}|${this.logsFilter.host ?? ''}`;
    this.logsFilter = filter;
    const next = `${filter.type ?? ''}|${filter.host ?? ''}`;
    if (prev !== next && (this.refCounts.get('logs') ?? 0) > 0) {
      this.syncConnection(true);
    }
  }

  private unsubscribe(channel: LiveChannel, handler: ChannelHandler) {
    const set = this.handlers.get(channel);
    set?.delete(handler);
    if (set && set.size === 0) this.handlers.delete(channel);
    const count = (this.refCounts.get(channel) ?? 0) - 1;
    if (count <= 0) this.refCounts.delete(channel);
    else this.refCounts.set(channel, count);
    if (this.refCounts.size === 0) this.close();
    else this.syncConnection();
  }

  private channelList(): LiveChannel[] {
    return [...this.refCounts.keys()].sort();
  }

  private connectionKey(channels: LiveChannel[]): string {
    return `${channels.join(',')}|${this.logsFilter.type ?? ''}|${this.logsFilter.host ?? ''}|${getToken() ?? ''}`;
  }

  private syncConnection(force = false) {
    const channels = this.channelList();
    if (channels.length === 0) {
      this.close();
      return;
    }
    const key = this.connectionKey(channels);
    if (!force && this.es && this.connectedKey === key) {
      return;
    }
    this.open(channels, key);
  }

  private open(channels: LiveChannel[], key: string) {
    this.intentionalClose = false;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.es) {
      this.es.close();
      this.es = null;
    }

    const es = new EventSource(liveUrl(channels, this.logsFilter));
    this.es = es;
    this.connectedKey = key;

    es.onopen = () => {
      this.backoffMs = 1000;
    };

    es.onmessage = (ev) => {
      let msg: ServerMsg;
      try {
        msg = JSON.parse(String(ev.data)) as ServerMsg;
      } catch {
        return;
      }
      if (!msg.channel || msg.error != null) return;
      const channel = msg.channel as LiveChannel;
      const set = this.handlers.get(channel);
      if (!set) return;
      for (const handler of [...set]) {
        try {
          handler(msg.data);
        } catch {
          /* ignore subscriber errors */
        }
      }
    };

    es.onerror = () => {
      // EventSource may close on proxy blips / non-200; never treat that as logout.
      if (es.readyState !== EventSource.CLOSED) return;
      this.es = null;
      if (this.intentionalClose || this.refCounts.size === 0) return;
      const delay = this.backoffMs;
      this.backoffMs = Math.min(this.backoffMs * 2, 15000);
      this.reconnectTimer = setTimeout(() => this.syncConnection(true), delay);
    };
  }

  private close() {
    this.intentionalClose = true;
    this.connectedKey = '';
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.es?.close();
    this.es = null;
  }
}

const liveClient = new LiveClient();

export function subscribeLive(channel: LiveChannel, handler: ChannelHandler): () => void {
  return liveClient.subscribe(channel, handler);
}

export function setLiveLogsFilter(filter: LogsFilter) {
  liveClient.setLogsFilter(filter);
}
