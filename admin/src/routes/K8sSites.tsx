import { useEffect, useMemo, useState, FormEvent } from 'react';
import { useNavigate } from 'react-router-dom';
import { toast } from 'sonner';
import { Globe, Pencil, Trash2 } from 'lucide-react';
import {
  api,
  type AccessList,
  type IngressFormRow,
  type IngressSubmitBody,
  type K8sNamespaceRow,
  type K8sServicePortDetail,
  type K8sServiceRow,
  type K8sTlsSecretRow,
  type NamedWafPolicy,
  type Site,
} from '@/api/client';
import { Modal } from '@/components/Modal';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { Pagination } from '@/components/Pagination';
import { ListToolbar, type ViewMode } from '@/components/ListToolbar';
import { usePageSize } from '@/utils/usePageSize';
import { useOpenOnQuery } from '@/utils/useOpenOnQuery';
import { getDefaultIngressNamespace } from '@/utils/ingressDefaults';
import { useManagementInfo } from '@/context/ManagementContext';
import { useMode } from '@/context/ModeContext';
import { cn } from '@/utils';

type K8sPageKind = 'ingress' | 'gateway';

const PATH_TYPES = ['Exact', 'Prefix', 'ImplementationSpecific'];
const SELECT_PLACEHOLDER = '— Select —';

function siteKind(site: Site, kind: K8sPageKind): boolean {
  const k = site.k8s_resource_kind || 'ingress';
  return kind === 'gateway' ? k === 'httproute' : k === 'ingress';
}

function portsForService(services: K8sServiceRow[], serviceName: string): K8sServicePortDetail[] {
  return services.find((service) => service.name === serviceName.trim())?.ports_detail ?? [];
}

function portLabel(port: K8sServicePortDetail): string {
  if (port.name?.trim()) {
    return `${port.name} (${port.port}, ${port.protocol})`;
  }
  return `${port.port} (${port.protocol})`;
}

function submitBody(
  form: IngressFormRow,
  accessListId: string,
  wafPolicyId: string,
  accessLists: AccessList[],
  wafPolicies: NamedWafPolicy[],
): IngressSubmitBody {
  const selectedAcl = accessListId
    ? accessLists.find((l) => l.id === accessListId)
    : undefined;
  const selectedWaf = wafPolicyId
    ? wafPolicies.find((p) => p.id === wafPolicyId)
    : undefined;
  return {
    ...form,
    ingress_namespace: form.namespace,
    access_list_id: accessListId || undefined,
    waf_policy_id: wafPolicyId || undefined,
    geoip: selectedAcl?.geoip ?? { enabled: false },
    security: selectedWaf?.security ?? {},
  };
}

export function K8sSites({ k8sPageKind }: { k8sPageKind: K8sPageKind }) {
  const navigate = useNavigate();
  const mode = useMode();
  const management = useManagementInfo();
  const pageSize = usePageSize();
  const [sites, setSites] = useState<Site[]>([]);
  const [loading, setLoading] = useState(true);
  const [viewMode, setViewMode] = useState<ViewMode>('list');
  const [page, setPage] = useState(1);
  const [modal, setModal] = useState(false);
  const [formTab, setFormTab] = useState<'general' | 'advanced'>('general');
  const [deleteRef, setDeleteRef] = useState<{ namespace: string; name: string } | null>(null);
  const [namespaces, setNamespaces] = useState<K8sNamespaceRow[]>([]);
  const [tlsSecrets, setTlsSecrets] = useState<K8sTlsSecretRow[]>([]);
  const [k8sServices, setK8sServices] = useState<K8sServiceRow[]>([]);
  const [saving, setSaving] = useState(false);
  const [editRef, setEditRef] = useState<{ namespace: string; name: string } | null>(null);
  const [form, setForm] = useState<IngressFormRow>({
    host: '',
    namespace: 'default',
    name: '',
    service_namespace: 'default',
    service_name: '',
    service_port: 80,
    path: '/',
    path_type: 'Prefix',
    geoip: { enabled: false }, security: {},
  });
  const [formAccessListId, setFormAccessListId] = useState('');
  const [formWafPolicyId, setFormWafPolicyId] = useState('');
  const [accessLists, setAccessLists] = useState<AccessList[]>([]);
  const [wafPolicies, setWafPolicies] = useState<NamedWafPolicy[]>([]);

  useEffect(() => {
    if (mode === 'proxy') navigate('/sites', { replace: true });
    if (mode === 'ingress' && k8sPageKind === 'gateway' && management && !management.gateway_api_enabled) {
      navigate('/sites/ingress', { replace: true });
    }
  }, [mode, k8sPageKind, management, navigate]);

  async function load() {
    setLoading(true);
    try {
      const cfg = await api.config();
      setSites(cfg.sites.filter((s) => siteKind(s, k8sPageKind)));
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Failed to load sites');
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    if (mode !== 'ingress') return;
    load();
  }, [mode, k8sPageKind]);

  useEffect(() => {
    if (!modal) return;
    api.kubernetes.namespaces().then(setNamespaces).catch(() => setNamespaces([]));
    api.kubernetes.tlsSecrets().then(setTlsSecrets).catch(() => setTlsSecrets([]));
    Promise.all([
      api.accessLists.list().catch(() => [] as AccessList[]),
      api.wafPolicies.list().catch(() => [] as NamedWafPolicy[]),
    ]).then(([acls, wafs]) => {
      setAccessLists(acls);
      setWafPolicies(wafs);
    });
  }, [modal]);

  useEffect(() => {
    if (!modal || !form.service_namespace.trim()) {
      setK8sServices([]);
      return;
    }
    api.kubernetes
      .services({ namespace: form.service_namespace.trim() })
      .then(setK8sServices)
      .catch(() => setK8sServices([]));
  }, [modal, form.service_namespace]);

  useEffect(() => {
    const serviceName = form.service_name.trim();
    if (!serviceName || k8sServices.length === 0) return;
    const ports = portsForService(k8sServices, serviceName);
    if (ports.length === 0) return;
    if (ports.some((port) => port.port === form.service_port)) return;
    setForm((current) => ({ ...current, service_port: ports[0].port }));
  }, [k8sServices, form.service_name, form.service_port]);

  const pageItems = useMemo(
    () => sites.slice((page - 1) * pageSize, page * pageSize),
    [sites, page, pageSize],
  );

  const filteredTlsSecrets = useMemo(() => {
    const ns = form.service_namespace.trim();
    return ns ? tlsSecrets.filter((secret) => secret.namespace === ns) : tlsSecrets;
  }, [tlsSecrets, form.service_namespace]);

  const servicePorts = portsForService(k8sServices, form.service_name);

  function resetPolicyIds(row?: Pick<IngressFormRow, 'access_list_id' | 'waf_policy_id'>) {
    setFormAccessListId(row?.access_list_id ?? '');
    setFormWafPolicyId(row?.waf_policy_id ?? '');
  }

  function openCreate() {
    setEditRef(null);
    setFormTab('general');
    const defaultNamespace = getDefaultIngressNamespace() || namespaces[0]?.name || 'default';
    setForm({
      host: '',
      namespace: defaultNamespace,
      name: '',
      service_namespace: defaultNamespace,
      service_name: '',
      service_port: 80,
      path: '/',
      path_type: 'Prefix',
      ingress_class_name: management?.ingress_class || undefined,
      geoip: { enabled: false }, security: {},
    });
    resetPolicyIds();
    setModal(true);
  }

  useOpenOnQuery('new', openCreate);

  async function openEdit(site: Site) {
    const ns = site.ingress_namespace || 'default';
    const name = site.ingress_name || '';
    setEditRef({ namespace: ns, name });
    setFormTab('general');
    setModal(true);
    try {
      const row =
        k8sPageKind === 'gateway'
          ? await api.kubernetes.getGatewaySite(ns, name)
          : await api.kubernetes.getIngress(ns, name);
      const firstRoute = row.routes?.[0];
      setForm({
        host: row.host,
        namespace: row.namespace,
        name: row.name,
        service_namespace: row.namespace,
        service_name: row.service_name || firstRoute?.service_name || '',
        service_port: row.service_port ?? firstRoute?.service_port ?? 80,
        path: row.path || firstRoute?.path || '/',
        path_type: row.path_type || firstRoute?.path_type || 'Prefix',
        tls_secret_namespace: row.tls_secret_name ? row.namespace : undefined,
        tls_secret_name: row.tls_secret_name ?? undefined,
        ingress_class_name: row.ingress_class_name || management?.ingress_class || undefined,
        gateway_namespace: row.gateway_namespace ?? undefined,
        gateway_name: row.gateway_name ?? undefined,
        geoip: row.geoip,
        security: row.security,
        access_list_id: row.access_list_id,
        waf_policy_id: row.waf_policy_id,
      });
      resetPolicyIds(row);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Failed to load site');
      setModal(false);
      setEditRef(null);
    }
  }

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    const missing = (msg: string) => {
      setFormTab('general');
      toast.error(msg);
    };
    if (!form.host.trim()) {
      missing('Host is required');
      return;
    }
    if (!form.name.trim()) {
      missing('Resource name is required');
      return;
    }
    if (!form.service_namespace.trim()) {
      missing('Select a service namespace');
      return;
    }
    if (!form.service_name.trim()) {
      missing('Select a service');
      return;
    }
    if (!form.service_port) {
      missing('Select a service port');
      return;
    }
    setSaving(true);
    const body = submitBody(form, formAccessListId, formWafPolicyId, accessLists, wafPolicies);
    try {
      if (k8sPageKind === 'gateway') {
        if (editRef) {
          await api.kubernetes.updateGatewaySite(editRef.namespace, editRef.name, body);
        } else {
          await api.kubernetes.createGatewaySite(body);
        }
      } else if (editRef) {
        await api.kubernetes.updateIngress(editRef.namespace, editRef.name, body);
      } else {
        await api.kubernetes.createIngress(body);
      }
      toast.success(editRef ? 'Updated' : 'Created');
      setModal(false);
      await load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Save failed');
    } finally {
      setSaving(false);
    }
  }

  async function onDelete() {
    if (!deleteRef) return;
    try {
      if (k8sPageKind === 'gateway') {
        await api.kubernetes.deleteGatewaySite(deleteRef.namespace, deleteRef.name);
      } else {
        await api.kubernetes.deleteIngress(deleteRef.namespace, deleteRef.name);
      }
      toast.success('Deleted');
      setDeleteRef(null);
      await load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Delete failed');
    }
  }

  const title = k8sPageKind === 'gateway' ? 'Gateway HTTPRoute Sites' : 'Ingress Sites';
  const labelCls = 'block text-sm text-text-secondary';
  const inputCls =
    'mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text placeholder:text-muted/40 placeholder:italic';
  const sectionCls = 'space-y-3 rounded-md border border-border p-4';
  const sectionTitleCls = 'text-sm font-semibold text-text';

  return (
    <div className="space-y-4">
      <div>
        <h2 className="text-xl font-semibold">{title}</h2>
        <p className="text-sm text-text-secondary">
          Manage Kubernetes {k8sPageKind === 'gateway' ? 'HTTPRoute' : 'Ingress'} resources. GeoIP /
          WAF settings are stored as annotations.
        </p>
      </div>

      <ListToolbar viewMode={viewMode} onViewModeChange={setViewMode} addLabel="Add site" onAdd={openCreate} />

      {loading ? (
        <div className="text-text-secondary">Loading…</div>
      ) : sites.length === 0 ? (
        <div className="rounded-lg border border-border bg-surface p-8 text-center text-text-secondary">
          No {k8sPageKind === 'gateway' ? 'HTTPRoute' : 'Ingress'} sites yet.
        </div>
      ) : (
        <>
          <div className={cn(viewMode === 'list' ? 'overflow-x-auto rounded-lg border border-border' : 'grid gap-4 sm:grid-cols-2 xl:grid-cols-3')}>
            {viewMode === 'list' ? (
              <table className="min-w-full text-sm">
                <thead className="bg-hover text-left text-text-secondary">
                  <tr>
                    <th className="px-4 py-3">Host</th>
                    <th className="px-4 py-3">Resource</th>
                    <th className="px-4 py-3">Backend</th>
                    <th className="px-4 py-3" />
                  </tr>
                </thead>
                <tbody>
                  {pageItems.map((site) => (
                    <tr key={`${site.ingress_namespace}/${site.ingress_name}/${site.host}`} className="border-t border-border">
                      <td className="px-4 py-3 font-medium">{site.host}</td>
                      <td className="px-4 py-3 text-text-secondary">
                        {site.ingress_namespace}/{site.ingress_name}
                      </td>
                      <td className="px-4 py-3">{site.backend}</td>
                      <td className="px-4 py-3 text-right">
                        <button type="button" className="mr-2 p-1 hover:text-primary" onClick={() => void openEdit(site)}>
                          <Pencil size={16} />
                        </button>
                        <button
                          type="button"
                          className="p-1 hover:text-red-500"
                          onClick={() =>
                            setDeleteRef({
                              namespace: site.ingress_namespace || 'default',
                              name: site.ingress_name || '',
                            })
                          }
                        >
                          <Trash2 size={16} />
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              pageItems.map((site) => (
                <div key={`${site.ingress_namespace}/${site.ingress_name}/${site.host}`} className="rounded-lg border border-border bg-surface p-4">
                  <div className="mb-2 flex items-center gap-2 font-semibold">
                    <Globe size={16} /> {site.host}
                  </div>
                  <div className="text-sm text-text-secondary">{site.ingress_namespace}/{site.ingress_name}</div>
                  <div className="mt-3 flex gap-2">
                    <button type="button" className="rounded border border-border px-2 py-1 text-sm" onClick={() => void openEdit(site)}>
                      Edit
                    </button>
                    <button
                      type="button"
                      className="rounded border border-border px-2 py-1 text-sm text-red-500"
                      onClick={() =>
                        setDeleteRef({
                          namespace: site.ingress_namespace || 'default',
                          name: site.ingress_name || '',
                        })
                      }
                    >
                      Delete
                    </button>
                  </div>
                </div>
              ))
            )}
          </div>
          <Pagination page={page} pageSize={pageSize} totalItems={sites.length} onPageChange={setPage} />
        </>
      )}

      <Modal open={modal} onClose={() => setModal(false)} title={editRef ? 'Edit site' : 'Add site'} wide protect={saving}>
        <form onSubmit={onSubmit} noValidate className="space-y-5">
          <div className="tab-bar w-full sm:w-auto" role="tablist" aria-label="Site settings">
            <button
              type="button"
              role="tab"
              aria-selected={formTab === 'general'}
              onClick={() => setFormTab('general')}
              className={cn('tab-item', formTab === 'general' && 'active')}
            >
              General
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={formTab === 'advanced'}
              onClick={() => setFormTab('advanced')}
              className={cn('tab-item', formTab === 'advanced' && 'active')}
            >
              Advanced
            </button>
          </div>

          <div className="grid">
            <div
              className={cn(
                'col-start-1 row-start-1 space-y-3',
                formTab !== 'general' && 'invisible pointer-events-none',
              )}
              aria-hidden={formTab !== 'general'}
            >
              <label className="block text-sm">
                Host
                <input className="mt-1 w-full rounded border border-border bg-bg px-3 py-2" value={form.host} onChange={(e) => setForm({ ...form, host: e.target.value })} />
              </label>
              <div className="grid grid-cols-2 gap-3">
                <label className="block text-sm">
                  Ingress namespace
                  <select className="mt-1 w-full rounded border border-border bg-bg px-3 py-2" value={form.namespace} onChange={(e) => setForm({ ...form, namespace: e.target.value })}>
                    {namespaces.map((n) => (
                      <option key={n.name} value={n.name}>{n.name}</option>
                    ))}
                  </select>
                </label>
                <label className="block text-sm">
                  Resource name
                  <input className="mt-1 w-full rounded border border-border bg-bg px-3 py-2" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} />
                </label>
              </div>
              <label className="block text-sm">
                Service namespace
                <select
                  className="mt-1 w-full rounded border border-border bg-bg px-3 py-2"
                  value={form.service_namespace}
                  onChange={(e) => {
                    const ns = e.target.value;
                    setForm((current) => ({
                      ...current,
                      service_namespace: ns,
                      service_name: '',
                      service_port: 80,
                      tls_secret_namespace: current.tls_secret_namespace === ns ? current.tls_secret_namespace : undefined,
                      tls_secret_name: current.tls_secret_namespace === ns ? current.tls_secret_name : undefined,
                    }));
                  }}
                >
                  <option value="">{SELECT_PLACEHOLDER}</option>
                  {namespaces.map((n) => (
                    <option key={n.name} value={n.name}>{n.name}</option>
                  ))}
                </select>
              </label>
              <div className="grid grid-cols-2 gap-3">
                <label className="block text-sm">
                  Service
                  <select
                    className="mt-1 w-full rounded border border-border bg-bg px-3 py-2"
                    value={form.service_name}
                    onChange={(e) => setForm({ ...form, service_name: e.target.value, service_port: 80 })}
                    disabled={!form.service_namespace}
                  >
                    <option value="">{SELECT_PLACEHOLDER}</option>
                    {k8sServices.map((service) => (
                      <option key={`${service.namespace}/${service.name}`} value={service.name}>
                        {service.name}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="block text-sm">
                  Service port
                  <select
                    className="mt-1 w-full rounded border border-border bg-bg px-3 py-2"
                    value={form.service_port ? String(form.service_port) : ''}
                    onChange={(e) => setForm({ ...form, service_port: Number(e.target.value) })}
                    disabled={servicePorts.length === 0}
                  >
                    <option value="">{SELECT_PLACEHOLDER}</option>
                    {servicePorts.map((port) => (
                      <option key={`${port.port}-${port.name ?? ''}`} value={String(port.port)}>
                        {portLabel(port)}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
              <div className="grid grid-cols-2 gap-3">
                <label className="block text-sm">
                  Path
                  <input className="mt-1 w-full rounded border border-border bg-bg px-3 py-2" value={form.path} onChange={(e) => setForm({ ...form, path: e.target.value })} />
                </label>
                <label className="block text-sm">
                  Path type
                  <select className="mt-1 w-full rounded border border-border bg-bg px-3 py-2" value={form.path_type} onChange={(e) => setForm({ ...form, path_type: e.target.value })}>
                    {PATH_TYPES.map((p) => (
                      <option key={p} value={p}>{p}</option>
                    ))}
                  </select>
                </label>
              </div>
              {k8sPageKind === 'ingress' && (
                <label className="block text-sm">
                  TLS secret (optional)
                  <select
                    className="mt-1 w-full rounded border border-border bg-bg px-3 py-2"
                    value={form.tls_secret_name ? `${form.tls_secret_namespace}/${form.tls_secret_name}` : ''}
                    onChange={(e) => {
                      const v = e.target.value;
                      if (!v) {
                        setForm({ ...form, tls_secret_namespace: undefined, tls_secret_name: undefined });
                        return;
                      }
                      const [ns, name] = v.split('/');
                      setForm({ ...form, tls_secret_namespace: ns, tls_secret_name: name });
                    }}
                  >
                    <option value="">None</option>
                    {filteredTlsSecrets.map((s) => (
                      <option key={`${s.namespace}/${s.name}`} value={`${s.namespace}/${s.name}`}>
                        {s.namespace}/{s.name}
                      </option>
                    ))}
                  </select>
                  <p className="mt-1 text-xs text-text-secondary">
                    TLS secret must be in the service namespace.
                  </p>
                </label>
              )}
            </div>

            <div
              className={cn(
                'col-start-1 row-start-1 space-y-5',
                formTab !== 'advanced' && 'invisible pointer-events-none',
              )}
              aria-hidden={formTab !== 'advanced'}
            >
              <div className={sectionCls}>
                <h3 className={sectionTitleCls}>Access Control</h3>
                <label className={labelCls}>
                  Access Control List
                  <select
                    className={inputCls}
                    value={formAccessListId}
                    onChange={(e) => setFormAccessListId(e.target.value)}
                  >
                    <option value="">None</option>
                    {accessLists.map((list) => (
                      <option key={list.id} value={list.id}>
                        {list.name}
                      </option>
                    ))}
                  </select>
                </label>
                <p className="text-xs text-text-secondary">
                  Manage lists under Access Control.
                </p>
              </div>

              <div className={sectionCls}>
                <h3 className={sectionTitleCls}>WAF</h3>
                <label className={labelCls}>
                  WAF policy
                  <select
                    className={inputCls}
                    value={formWafPolicyId}
                    onChange={(e) => setFormWafPolicyId(e.target.value)}
                  >
                    <option value="">None</option>
                    {wafPolicies.map((policy) => (
                      <option key={policy.id} value={policy.id}>
                        {policy.name}
                      </option>
                    ))}
                  </select>
                </label>
                <p className="text-xs text-text-secondary">
                  Manage policies under WAF.
                </p>
              </div>
            </div>
          </div>

          <div className="flex justify-end gap-2 pt-2">
            <button type="button" className="rounded border border-border px-3 py-2 text-sm" onClick={() => setModal(false)}>Cancel</button>
            <button type="submit" disabled={saving} className="rounded bg-primary px-3 py-2 text-sm text-white disabled:opacity-50">
              {saving ? 'Saving…' : 'Save'}
            </button>
          </div>
        </form>
      </Modal>

      <ConfirmDialog
        open={!!deleteRef}
        title="Delete site"
        message={`Delete ${deleteRef?.namespace}/${deleteRef?.name}?`}
        primaryLabel="Delete"
        onConfirm={onDelete}
        onCancel={() => setDeleteRef(null)}
      />
    </div>
  );
}
