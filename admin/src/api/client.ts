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
  db_path: string;
  route_count: number;
  site_count: number;
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

export type Upstream = { addr: string; weight?: number };

export type Backend = {
  name: string;
  upstreams: Upstream[];
};

export type PathRewrite = {
  path: string;
  path_type?: string;
  rewrite?: string;
};

export type Site = {
  host: string;
  backend: string;
  routes: PathRewrite[];
};

export type TlsSource =
  | { type: 'file'; cert: string; key: string }
  | {
      type: 'acme';
      email?: string;
      challenge?: string;
      dns_provider?: string;
      dns_provider_type?: string;
      dns_credentials?: Record<string, string>;
    };

export type TlsConfig = {
  hosts: string[];
  source: TlsSource;
  expires_at?: string;
};

export type ProxyConfig = {
  sites: Site[];
  backends: Backend[];
  tls: TlsConfig[];
  proxy_log?: boolean;
};

export type CertificateRow = {
  id: string;
  hosts: string[];
  source_type: string;
  created_at: string;
  expires_at?: string;
};

export type DnsProviderRow = {
  id: string;
  name: string;
  provider_type: string;
  credentials?: Record<string, string>;
  created_at: string;
};

export type SupportedDnsProviderField = {
  key: string;
  label: string;
  field_type: string;
  required: boolean;
};

export type SupportedDnsProvider = {
  id: string;
  name: string;
  fields: SupportedDnsProviderField[];
};

export const api = {
  health: () => request<{ status: string }>('/health'),
  version: () => request<{ version: string; binary: string }>('/version'),
  authConfig: () =>
    request<{ mode: string; supports_local: boolean; auth_required: boolean }>('/auth/config'),
  login: (password: string, username = 'admin') =>
    request<{ token: string; username: string; expires_in: number }>('/auth/login', {
      method: 'POST',
      body: JSON.stringify({ password, username }),
    }),
  authCheck: () => request<{ authenticated: boolean }>('/auth/check'),
  management: () => request<ManagementInfo>('/management'),
  routes: () => request<{ routes: RouteView[]; count: number }>('/routes'),
  config: () => request<ProxyConfig>('/config'),
  saveConfig: (config: ProxyConfig) =>
    request<{ ok: boolean; route_count: number }>('/config', {
      method: 'PUT',
      body: JSON.stringify(config),
    }),
  reload: () => request<{ ok: boolean }>('/reload', { method: 'POST' }),
  tls: () => request<{ entries: { hosts: string[]; cert: string; key: string }[]; host_count: number }>('/tls'),
  certificates: {
    list: () => request<CertificateRow[]>('/certificates'),
    upload: (body: { hosts: string[]; cert_pem: string; key_pem: string }) =>
      request<{ id: string; message: string }>('/certificates', {
        method: 'POST',
        body: JSON.stringify(body),
      }),
    delete: (id: string) =>
      request<{ ok: boolean }>(`/certificates/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  },
  dnsProviders: {
    list: () => request<DnsProviderRow[]>('/dns-providers'),
    supported: () => request<SupportedDnsProvider[]>('/dns-providers/supported'),
    get: (id: string) => request<DnsProviderRow>(`/dns-providers/${encodeURIComponent(id)}`),
    create: (body: { name: string; provider_type: string; credentials?: Record<string, string> }) =>
      request<{ id: string }>('/dns-providers', { method: 'POST', body: JSON.stringify(body) }),
    update: (id: string, body: { name: string; provider_type: string; credentials?: Record<string, string> }) =>
      request<{ ok: boolean }>(`/dns-providers/${encodeURIComponent(id)}`, {
        method: 'PUT',
        body: JSON.stringify(body),
      }),
    delete: (id: string) =>
      request<{ ok: boolean }>(`/dns-providers/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  },
};
