import { useEffect, useState, FormEvent } from 'react';
import { useNavigate } from 'react-router-dom';
import { toast } from 'sonner';
import { DoorOpen, Pencil, Plus, Trash2 } from 'lucide-react';
import { api, type GatewayFormRow, type K8sGatewayRow, type K8sNamespaceRow, type K8sTlsSecretRow } from '@/api/client';
import { Modal } from '@/components/Modal';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { Pagination } from '@/components/Pagination';
import { usePageSize } from '@/utils/usePageSize';
import { useGatewayApiEnabled, useManagementInfo } from '@/context/ManagementContext';
import { useMode } from '@/context/ModeContext';

export function Gateways() {
  const navigate = useNavigate();
  const mode = useMode();
  const gatewayApiEnabled = useGatewayApiEnabled();
  const management = useManagementInfo();
  const pageSize = usePageSize();
  const [list, setList] = useState<K8sGatewayRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [page, setPage] = useState(1);
  const [modal, setModal] = useState(false);
  const [deleteRef, setDeleteRef] = useState<{ namespace: string; name: string } | null>(null);
  const [editRef, setEditRef] = useState<{ namespace: string; name: string } | null>(null);
  const [namespaces, setNamespaces] = useState<K8sNamespaceRow[]>([]);
  const [tlsSecrets, setTlsSecrets] = useState<K8sTlsSecretRow[]>([]);
  const [saving, setSaving] = useState(false);
  const [form, setForm] = useState<GatewayFormRow>({
    host: '',
    namespace: 'default',
    name: '',
    gateway_class_name: management?.gateway_class || undefined,
  });

  useEffect(() => {
    if (mode === 'proxy') navigate('/sites', { replace: true });
    if (management && !gatewayApiEnabled) navigate('/sites/ingress', { replace: true });
  }, [mode, management, gatewayApiEnabled, navigate]);

  async function load() {
    setLoading(true);
    try {
      setList(await api.kubernetes.listGateways());
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Failed to load gateways');
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    if (mode !== 'ingress' || !gatewayApiEnabled) return;
    load();
    api.kubernetes.namespaces().then(setNamespaces).catch(() => {});
    api.kubernetes.tlsSecrets().then(setTlsSecrets).catch(() => {});
  }, [mode, gatewayApiEnabled]);

  const pageItems = list.slice((page - 1) * pageSize, page * pageSize);

  function openCreate() {
    setEditRef(null);
    setForm({
      host: '',
      namespace: namespaces[0]?.name || 'default',
      name: '',
      gateway_class_name: management?.gateway_class || undefined,
    });
    setModal(true);
  }

  function openEdit(gw: K8sGatewayRow) {
    setEditRef({ namespace: gw.namespace, name: gw.name });
    setForm({
      host: gw.hosts[0] || '',
      namespace: gw.namespace,
      name: gw.name,
      gateway_class_name: gw.class || management?.gateway_class || undefined,
    });
    setModal(true);
  }

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setSaving(true);
    try {
      if (editRef) {
        await api.kubernetes.updateGateway(editRef.namespace, editRef.name, form);
      } else {
        await api.kubernetes.createGateway(form);
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
      await api.kubernetes.deleteGateway(deleteRef.namespace, deleteRef.name);
      toast.success('Deleted');
      setDeleteRef(null);
      await load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Delete failed');
    }
  }

  return (
    <div className="space-y-4">
      <div className="mb-4 flex justify-end">
        <button
          type="button"
          onClick={openCreate}
          className="inline-flex items-center gap-2 rounded-md bg-primary px-3 py-2 text-sm font-medium text-white hover:opacity-90"
        >
          <Plus size={16} /> Add gateway
        </button>
      </div>

      {loading ? (
        <div className="text-text-secondary">Loading…</div>
      ) : (
        <>
          <div className="overflow-x-auto rounded-lg border border-border">
            <table className="min-w-full text-sm">
              <thead className="bg-hover text-left text-text-secondary">
                <tr>
                  <th className="px-4 py-3">Name</th>
                  <th className="px-4 py-3">Class</th>
                  <th className="px-4 py-3">Hosts</th>
                  <th className="px-4 py-3" />
                </tr>
              </thead>
              <tbody>
                {pageItems.map((gw) => (
                  <tr key={`${gw.namespace}/${gw.name}`} className="border-t border-border">
                    <td className="px-4 py-3">
                      <div className="flex items-center gap-2 font-medium">
                        <DoorOpen size={16} /> {gw.namespace}/{gw.name}
                      </div>
                    </td>
                    <td className="px-4 py-3">{gw.class || '—'}</td>
                    <td className="px-4 py-3">{gw.hosts.join(', ') || '—'}</td>
                    <td className="px-4 py-3 text-right">
                      <button type="button" className="mr-2 p-1 hover:text-primary" onClick={() => openEdit(gw)}>
                        <Pencil size={16} />
                      </button>
                      <button type="button" className="p-1 hover:text-red-500" onClick={() => setDeleteRef({ namespace: gw.namespace, name: gw.name })}>
                        <Trash2 size={16} />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <Pagination page={page} pageSize={pageSize} totalItems={list.length} onPageChange={setPage} />
        </>
      )}

      <Modal open={modal} onClose={() => setModal(false)} title={editRef ? 'Edit gateway' : 'Add gateway'} wide protect={saving}>
        <form onSubmit={onSubmit} className="space-y-3">
          <label className="block text-sm">
            Hostname
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
              Name
              <input className="mt-1 w-full rounded border border-border bg-bg px-3 py-2" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} required />
            </label>
          </div>
          <label className="block text-sm">
            Gateway class
            <input className="mt-1 w-full rounded border border-border bg-bg px-3 py-2" value={form.gateway_class_name || ''} onChange={(e) => setForm({ ...form, gateway_class_name: e.target.value })} />
          </label>
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
        title="Delete gateway"
        message={`Delete ${deleteRef?.namespace}/${deleteRef?.name}?`}
        primaryLabel="Delete"
        onConfirm={onDelete}
        onCancel={() => setDeleteRef(null)}
      />
    </div>
  );
}
