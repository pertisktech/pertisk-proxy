import { getToken, clearToken } from '@/auth';

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

function wsUrl(): string {
  const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  const token = getToken();
  const q = token ? `?token=${encodeURIComponent(token)}` : '';
  return `${proto}//${window.location.host}/api/ws${q}`;
}

class LiveClient {
  private ws: WebSocket | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private backoffMs = 1000;
  private readonly handlers = new Map<LiveChannel, Set<ChannelHandler>>();
  private readonly refCounts = new Map<LiveChannel, number>();
  private logsFilter: LogsFilter = {};
  private intentionalClose = false;

  subscribe(channel: LiveChannel, handler: ChannelHandler): () => void {
    let set = this.handlers.get(channel);
    if (!set) {
      set = new Set();
      this.handlers.set(channel, set);
    }
    set.add(handler);
    const count = (this.refCounts.get(channel) ?? 0) + 1;
    this.refCounts.set(channel, count);
    this.ensureConnected();
    if (count === 1 && this.ws?.readyState === WebSocket.OPEN) {
      this.send({ op: 'subscribe', channels: [channel] });
      if (channel === 'logs') {
        this.sendLogsFilter();
      }
    }
    return () => this.unsubscribe(channel, handler);
  }

  setLogsFilter(filter: LogsFilter) {
    this.logsFilter = filter;
    if ((this.refCounts.get('logs') ?? 0) > 0 && this.ws?.readyState === WebSocket.OPEN) {
      this.sendLogsFilter();
    }
  }

  private unsubscribe(channel: LiveChannel, handler: ChannelHandler) {
    const set = this.handlers.get(channel);
    set?.delete(handler);
    if (set && set.size === 0) this.handlers.delete(channel);
    const count = (this.refCounts.get(channel) ?? 0) - 1;
    if (count <= 0) {
      this.refCounts.delete(channel);
      if (this.ws?.readyState === WebSocket.OPEN) {
        this.send({ op: 'unsubscribe', channels: [channel] });
      }
    } else {
      this.refCounts.set(channel, count);
    }
    if (this.refCounts.size === 0) {
      this.close();
    }
  }

  private ensureConnected() {
    if (this.ws && (this.ws.readyState === WebSocket.OPEN || this.ws.readyState === WebSocket.CONNECTING)) {
      return;
    }
    this.intentionalClose = false;
    const ws = new WebSocket(wsUrl());
    this.ws = ws;

    ws.onopen = () => {
      this.backoffMs = 1000;
      const channels = [...this.refCounts.keys()];
      if (channels.length > 0) {
        this.send({ op: 'subscribe', channels });
        if (this.refCounts.has('logs')) {
          this.sendLogsFilter();
        }
      }
    };

    ws.onmessage = (ev) => {
      let msg: ServerMsg;
      try {
        msg = JSON.parse(String(ev.data)) as ServerMsg;
      } catch {
        return;
      }
      if (msg.error && !msg.channel) {
        return;
      }
      if (!msg.channel) return;
      const channel = msg.channel as LiveChannel;
      if (msg.error != null) {
        return;
      }
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

    ws.onclose = (ev) => {
      this.ws = null;
      if (ev.code === 1008 || ev.code === 4401) {
        clearToken();
        const onLogin =
          window.location.pathname === '/login' || window.location.pathname.startsWith('/login/');
        if (!onLogin) window.location.href = '/login';
        return;
      }
      if (this.intentionalClose || this.refCounts.size === 0) return;
      const delay = this.backoffMs;
      this.backoffMs = Math.min(this.backoffMs * 2, 15000);
      this.reconnectTimer = setTimeout(() => this.ensureConnected(), delay);
    };

    ws.onerror = () => {
      /* onclose handles reconnect */
    };
  }

  private send(payload: Record<string, unknown>) {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(payload));
    }
  }

  private sendLogsFilter() {
    this.send({
      op: 'logs_filter',
      type: this.logsFilter.type || undefined,
      host: this.logsFilter.host || undefined,
    });
  }

  private close() {
    this.intentionalClose = true;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.ws?.close();
    this.ws = null;
  }
}

const liveClient = new LiveClient();

export function subscribeLive(channel: LiveChannel, handler: ChannelHandler): () => void {
  return liveClient.subscribe(channel, handler);
}

export function setLiveLogsFilter(filter: LogsFilter) {
  liveClient.setLogsFilter(filter);
}
