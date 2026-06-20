import { getToken, clearToken } from '@/auth';

const API = '/api';

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const token = getToken();
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    ...(options.headers as Record<string, string>),
  };
  if (token) headers.Authorization = `Bearer ${token}`;

  const r = await fetch(`${API}${path}`, { ...options, headers });
  if (r.status === 401) {
    clearToken();
    window.location.href = '/login';
    throw new Error('Unauthorized');
  }
  if (!r.ok) {
    const body = (await r.json().catch(() => ({}))) as { error?: string };
    throw new Error(body.error || `${r.status} ${r.statusText}`);
  }
  return r.json();
}

export type ManagementInfo = {
  mode: string;
  version: string;
  uptime_secs: number;
  routes_path: string;
  route_count: number;
  tls_host_count: number;
  enable_h3: boolean;
  auto_https: boolean;
  runtime_mode: string;
  listeners: { http: string; https: string; h3_udp: string };
  http3: Record<string, unknown>;
};

export type RouteView = {
  host: string;
  path: string;
  path_type: string;
  upstream: string;
  middlewares: number;
};

export type TlsEntry = {
  hosts: string[];
  cert: string;
  key: string;
};

export const api = {
  health: () => request<{ status: string }>('/health'),
  version: () => request<{ version: string; binary: string }>('/version'),
  authConfig: () =>
    request<{ mode: string; supports_local: boolean; auth_required: boolean }>('/auth/config'),
  login: (password: string, username = 'admin') =>
    request<{ token: string; username: string }>('/auth/login', {
      method: 'POST',
      body: JSON.stringify({ password, username }),
    }),
  authCheck: () => request<{ authenticated: boolean }>('/auth/check'),
  management: () => request<ManagementInfo>('/management'),
  routes: () => request<{ routes: RouteView[]; count: number }>('/routes'),
  configYaml: () => request<{ path: string; yaml: string }>('/config/yaml'),
  saveConfig: (yaml: string) =>
    request<{ ok: boolean; route_count: number }>('/config', {
      method: 'PUT',
      body: JSON.stringify({ yaml }),
    }),
  reload: () => request<{ ok: boolean }>('/reload', { method: 'POST' }),
  tls: () => request<{ entries: TlsEntry[]; host_count: number }>('/tls'),
};
