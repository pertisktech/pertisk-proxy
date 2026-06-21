import { useEffect, useMemo, useState, FormEvent } from 'react';
import { useNavigate } from 'react-router-dom';
import { toast } from 'sonner';
import { Globe, Pencil, Trash2 } from 'lucide-react';
import { api, type IngressFormRow, type K8sNamespaceRow, type K8sTlsSecretRow, type Site } from '@/api/client';
import { Modal } from '@/components/Modal';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { Pagination } from '@/components/Pagination';
import { ListToolbar, type ViewMode } from '@/components/ListToolbar';
import { usePageSize } from '@/utils/usePageSize';
import { useManagementInfo } from '@/context/ManagementContext';
import { useMode } from '@/context/ModeContext';
import { cn } from '@/utils';

type K8sPageKind = 'ingress' | 'gateway';

const PATH_TYPES = ['Exact', 'Prefix', 'ImplementationSpecific'];

function siteKind(site: Site, kind: K8sPageKind): boolean {
  const k = site.k8s_resource_kind || 'ingress';
  return kind === 'gateway' ? k === 'httproute' : k === 'ingress';
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
  const [deleteRef, setDeleteRef] = useState<{ namespace: string; name: string } | null>(null);
  const [namespaces, setNamespaces] = useState<K8sNamespaceRow[]>([]);
  const [tlsSecrets, setTlsSecrets] = useState<K8sTlsSecretRow[]>([]);
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
  });

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
    api.kubernetes.namespaces().then(setNamespaces).catch(() => {});
    api.kubernetes.tlsSecrets().then(setTlsSecrets).catch(() => {});
  }, [mode, k8sPageKind]);

  const pageItems = useMemo(
    () => sites.slice((page - 1) * pageSize, page * pageSize),
    [sites, page, pageSize],
  );

  function openCreate() {
    setEditRef(null);
    setForm({
      host: '',
      namespace: namespaces[0]?.name || 'default',
      name: '',
      service_namespace: namespaces[0]?.name || 'default',
      service_name: '',
      service_port: 80,
      path: '/',
      path_type: 'Prefix',
      ingress_class_name: management?.ingress_class || undefined,
    });
    setModal(true);
  }

  function openEdit(site: Site) {
    const ns = site.ingress_namespace || 'default';
    const name = site.ingress_name || '';
    setEditRef({ namespace: ns, name });
    setForm({
      host: site.host,
      namespace: ns,
      name,
      service_namespace: ns,
      service_name: site.backend.split('.')[0] || site.backend,
      service_port: 80,
      path: site.routes[0]?.path || '/',
      path_type: site.routes[0]?.path_type || 'Prefix',
      ingress_class_name: management?.ingress_class || undefined,
    });
    setModal(true);
  }

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setSaving(true);
    try {
      if (k8sPageKind === 'gateway') {
        if (editRef) {
          await api.kubernetes.updateGatewaySite(editRef.namespace, editRef.name, form);
        } else {
          await api.kubernetes.createGatewaySite(form);
        }
      } else if (editRef) {
        await api.kubernetes.updateIngress(editRef.namespace, editRef.name, form);
      } else {
        await api.kubernetes.createIngress(form);
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

  return (
    <div className="space-y-4">
      <div>
        <h2 className="text-xl font-semibold">{title}</h2>
        <p className="text-sm text-text-secondary">
          Manage Kubernetes {k8sPageKind === 'gateway' ? 'HTTPRoute' : 'Ingress'} resources
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
                        <button type="button" className="mr-2 p-1 hover:text-primary" onClick={() => openEdit(site)}>
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
                    <button type="button" className="rounded border border-border px-2 py-1 text-sm" onClick={() => openEdit(site)}>
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

      <Modal open={modal} onClose={() => setModal(false)} title={editRef ? 'Edit site' : 'Add site'}>
        <form onSubmit={onSubmit} className="space-y-3">
          <label className="block text-sm">
            Host
            <input className="mt-1 w-full rounded border border-border bg-bg px-3 py-2" value={form.host} onChange={(e) => setForm({ ...form, host: e.target.value })} required />
          </label>
          <div className="grid grid-cols-2 gap-3">
            <label className="block text-sm">
              Namespace
              <select className="mt-1 w-full rounded border border-border bg-bg px-3 py-2" value={form.namespace} onChange={(e) => setForm({ ...form, namespace: e.target.value })}>
                {namespaces.map((n) => (
                  <option key={n.name} value={n.name}>{n.name}</option>
                ))}
              </select>
            </label>
            <label className="block text-sm">
              Resource name
              <input className="mt-1 w-full rounded border border-border bg-bg px-3 py-2" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} required />
            </label>
          </div>
          <div className="grid grid-cols-3 gap-3">
            <label className="col-span-2 block text-sm">
              Service
              <input className="mt-1 w-full rounded border border-border bg-bg px-3 py-2" value={form.service_name} onChange={(e) => setForm({ ...form, service_name: e.target.value })} required />
            </label>
            <label className="block text-sm">
              Port
              <input type="number" className="mt-1 w-full rounded border border-border bg-bg px-3 py-2" value={form.service_port} onChange={(e) => setForm({ ...form, service_port: Number(e.target.value) })} required />
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
              {tlsSecrets.map((s) => (
                <option key={`${s.namespace}/${s.name}`} value={`${s.namespace}/${s.name}`}>
                  {s.namespace}/{s.name}
                </option>
              ))}
            </select>
          </label>
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
