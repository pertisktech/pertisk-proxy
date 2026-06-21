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
  management_addr: string;
  route_count: number;
  site_count: number;
  backend_count: number;
  tls_count: number;
  tls_host_count: number;
  enable_h3: boolean;
  auto_https: boolean;
  runtime_mode: string;
  listeners: { http: string; https: string; h3_udp: string };
  http3: Record<string, unknown>;
  hostname?: string | null;
  os?: string | null;
  cpu_count?: number | null;
  cpu_usage_percent?: number | null;
  memory_total_bytes?: number | null;
  memory_used_bytes?: number | null;
  process_cpu_usage_percent?: number | null;
  process_memory_bytes?: number | null;
  ipv4_addrs?: string[];
  ipv6_addrs?: string[];
  gateway_api_enabled?: boolean;
  helm_enabled?: boolean;
  ingress_class?: string | null;
  gateway_class?: string | null;
  leader_election?: {
    enabled: boolean;
    is_leader: boolean;
    namespace: string;
    lease_name: string;
  } | null;
};

export type Site = {
  host: string;
  backend: string;
  routes: PathRewrite[];
  ingress_namespace?: string | null;
  ingress_name?: string | null;
  k8s_resource_kind?: string | null;
};

export type K8sNamespaceRow = { name: string; created_at?: string | null };
export type K8sTlsSecretRow = { namespace: string; name: string; expires_at?: string | null };
export type K8sServiceRow = {
  name: string;
  namespace: string;
  ports_detail: { port: number; name?: string | null; protocol: string }[];
};
export type K8sGatewayRow = {
  namespace: string;
  name: string;
  class?: string | null;
  hosts: string[];
  listeners?: { protocol: string; port: number; hostname?: string | null }[];
  created_at?: string | null;
};

export type IngressFormRow = {
  host: string;
  namespace: string;
  name: string;
  service_namespace: string;
  service_name: string;
  service_port: number;
  path: string;
  path_type: string;
  tls_secret_namespace?: string;
  tls_secret_name?: string;
  ingress_class_name?: string;
};

export type GatewayFormRow = {
  host: string;
  namespace: string;
  name: string;
  gateway_class_name?: string;
  tls_secret_namespace?: string;
  tls_secret_name?: string;
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

export type LogEntryType =
  | 'request'
  | 'response'
  | 'health_check'
  | 'config_reload'
  | 'tracing'
  | 'error';

export type LogEntry = {
  timestamp: string;
  level: string;
  host?: string | null;
  path?: string | null;
  upstream?: string | null;
  status?: number | null;
  duration_ms?: number | null;
  message: string;
  type: LogEntryType;
  protocol?: string | null;
  encoding?: string | null;
  method?: string | null;
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
  logs: (params?: { type?: 'system' | 'proxy' | 'http' | 'all'; host?: string }) => {
    const search = new URLSearchParams();
    if (params?.type && params.type !== 'all') search.set('type', params.type === 'http' ? 'proxy' : params.type);
    if (params?.host) search.set('host', params.host);
    const q = search.toString();
    return request<LogEntry[]>(q ? `/logs?${q}` : '/logs');
  },
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
  kubernetes: {
    namespaces: () => request<K8sNamespaceRow[]>('/kubernetes/namespaces'),
    tlsSecrets: () => request<K8sTlsSecretRow[]>('/kubernetes/tls-secrets'),
    services: (namespace?: string) => {
      const q = namespace ? `?namespace=${encodeURIComponent(namespace)}` : '';
      return request<K8sServiceRow[]>(`/kubernetes/services${q}`);
    },
    listIngresses: (namespace?: string) => {
      const q = namespace ? `?namespace=${encodeURIComponent(namespace)}` : '';
      return request<{ name: string; namespace: string; hosts: string[]; class?: string | null }[]>(
        `/kubernetes/ingresses${q}`,
      );
    },
    createIngress: (body: IngressFormRow) =>
      request<{ namespace: string; name: string }>('/kubernetes/ingresses', {
        method: 'POST',
        body: JSON.stringify(body),
      }),
    updateIngress: (namespace: string, name: string, body: IngressFormRow) =>
      request<{ ok: boolean }>(`/kubernetes/ingresses/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}`, {
        method: 'PUT',
        body: JSON.stringify(body),
      }),
    deleteIngress: (namespace: string, name: string) =>
      request<{ ok: boolean }>(
        `/kubernetes/ingresses/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}`,
        { method: 'DELETE' },
      ),
    listGateways: () => request<K8sGatewayRow[]>('/kubernetes/gateways'),
    createGateway: (body: GatewayFormRow) =>
      request<{ namespace: string; name: string }>('/kubernetes/gateways', {
        method: 'POST',
        body: JSON.stringify(body),
      }),
    updateGateway: (namespace: string, name: string, body: GatewayFormRow) =>
      request<{ ok: boolean }>(`/kubernetes/gateways/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}`, {
        method: 'PUT',
        body: JSON.stringify(body),
      }),
    deleteGateway: (namespace: string, name: string) =>
      request<{ ok: boolean }>(
        `/kubernetes/gateways/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}`,
        { method: 'DELETE' },
      ),
    createGatewaySite: (body: IngressFormRow & { gateway_namespace?: string; gateway_name?: string }) =>
      request<{ namespace: string; name: string }>('/kubernetes/gateway-sites', {
        method: 'POST',
        body: JSON.stringify(body),
      }),
    updateGatewaySite: (namespace: string, name: string, body: IngressFormRow) =>
      request<{ ok: boolean }>(
        `/kubernetes/gateway-sites/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}`,
        { method: 'PUT', body: JSON.stringify(body) },
      ),
    deleteGatewaySite: (namespace: string, name: string) =>
      request<{ ok: boolean }>(
        `/kubernetes/gateway-sites/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}`,
        { method: 'DELETE' },
      ),
  },
};
