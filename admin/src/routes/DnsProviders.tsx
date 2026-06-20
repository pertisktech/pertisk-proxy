import { useEffect, useMemo, useState, FormEvent } from 'react';
import { toast } from 'sonner';
import { Pencil, Server, Trash2 } from 'lucide-react';
import {
  api,
  type DnsProviderRow,
  type SupportedDnsProvider,
  type SupportedDnsProviderField,
} from '@/api/client';
import { Modal } from '@/components/Modal';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { Pagination } from '@/components/Pagination';
import { ListToolbar, type ViewMode } from '@/components/ListToolbar';
import { usePageSize } from '@/utils/usePageSize';
import { formatDateTime } from '@/utils/dateFormat';
import { cn } from '@/utils';

type SortKey = 'name' | 'type' | 'created';

export function DnsProviders() {
  const [list, setList] = useState<DnsProviderRow[]>([]);
  const [supported, setSupported] = useState<SupportedDnsProvider[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>('list');
  const [page, setPage] = useState(1);
  const pageSize = usePageSize();
  const [sortKey, setSortKey] = useState<SortKey | null>(null);
  const [sortDir, setSortDir] = useState<'asc' | 'desc'>('asc');

  const [modalOpen, setModalOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [formName, setFormName] = useState('');
  const [formType, setFormType] = useState('');
  const [formCreds, setFormCreds] = useState<Record<string, string>>({});
  const [fieldVisibility, setFieldVisibility] = useState<Record<string, boolean>>({});
  const [formError, setFormError] = useState('');
  const [saving, setSaving] = useState(false);

  const [deleteRow, setDeleteRow] = useState<DnsProviderRow | null>(null);
  const [deleting, setDeleting] = useState(false);

  const selectedProvider = supported.find((p) => p.id === formType);

  function load() {
    setLoading(true);
    setError(null);
    Promise.all([api.dnsProviders.list(), api.dnsProviders.supported()])
      .then(([rows, sup]) => {
        setList(rows);
        setSupported(sup);
      })
      .catch((e) => setError(e instanceof Error ? e.message : 'Failed to load'))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    load();
  }, []);

  function getProviderDisplayName(providerType: string): string {
    return supported.find((s) => s.id === providerType)?.name ?? providerType;
  }

  function toggleSort(key: SortKey) {
    setPage(1);
    if (sortKey === key) setSortDir((d) => (d === 'asc' ? 'desc' : 'asc'));
    else {
      setSortKey(key);
      setSortDir('asc');
    }
  }

  const sortedList = useMemo(() => {
    if (!sortKey) return list;
    const dir = sortDir === 'asc' ? 1 : -1;
    return [...list].sort((a, b) => {
      if (sortKey === 'name') return dir * a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
      if (sortKey === 'type') {
        return dir * getProviderDisplayName(a.provider_type).localeCompare(
          getProviderDisplayName(b.provider_type),
          undefined,
          { sensitivity: 'base' },
        );
      }
      const at = new Date(a.created_at).getTime();
      const bt = new Date(b.created_at).getTime();
      return dir * ((Number.isFinite(at) ? at : 0) - (Number.isFinite(bt) ? bt : 0));
    });
  }, [list, sortKey, sortDir, supported]);

  const totalPages = Math.max(1, Math.ceil(sortedList.length / pageSize));
  const pagedList = sortedList.slice((page - 1) * pageSize, page * pageSize);

  useEffect(() => {
    setPage((p) => Math.min(p, totalPages));
  }, [totalPages]);

  function openAdd() {
    setEditingId(null);
    setFormName('');
    setFormType(supported[0]?.id || '');
    setFormCreds({});
    setFieldVisibility({});
    setFormError('');
    setModalOpen(true);
  }

  function openEdit(row: DnsProviderRow) {
    setEditingId(row.id);
    setFormName(row.name);
    setFormType(row.provider_type);
    setFormCreds(row.credentials ? { ...row.credentials } : {});
    setFieldVisibility({});
    setFormError('');
    setModalOpen(true);
  }

  function loadFullThenEdit(id: string) {
    api.dnsProviders
      .get(id)
      .then(openEdit)
      .catch((e) => toast.error(e instanceof Error ? e.message : 'Failed to load'));
  }

  function buildCredentials(): Record<string, string> | undefined {
    if (!selectedProvider) return undefined;
    const out: Record<string, string> = {};
    for (const field of selectedProvider.fields) {
      const v = formCreds[field.key]?.trim();
      if (v) out[field.key] = v;
    }
    return Object.keys(out).length ? out : undefined;
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setFormError('');
    const name = formName.trim();
    const provider_type = formType.trim();
    if (!name) {
      setFormError('Name is required');
      return;
    }
    if (!provider_type) {
      setFormError('Provider type is required');
      return;
    }
    if (selectedProvider?.fields.some((f) => f.required && !formCreds[f.key]?.trim())) {
      setFormError('All required fields must be filled');
      return;
    }
    const body = { name, provider_type, credentials: buildCredentials() };
    setSaving(true);
    try {
      if (editingId) await api.dnsProviders.update(editingId, body);
      else await api.dnsProviders.create(body);
      toast.success(editingId ? 'DNS provider updated' : 'DNS provider created');
      setModalOpen(false);
      load();
    } catch (err) {
      setFormError(err instanceof Error ? err.message : 'Save failed');
    } finally {
      setSaving(false);
    }
  }

  async function confirmDelete() {
    if (!deleteRow) return;
    setDeleting(true);
    try {
      await api.dnsProviders.delete(deleteRow.id);
      toast.success('DNS provider removed');
      setDeleteRow(null);
      load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Delete failed');
    } finally {
      setDeleting(false);
    }
  }

  const inputCls = 'mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm';
  const labelCls = 'block text-sm text-text-secondary';

  if (loading && list.length === 0) return <p className="text-text-secondary">Loading DNS providers…</p>;
  if (error) {
    return (
      <div className="space-y-3">
        <p className="text-red-r1">{error}</p>
        <button type="button" onClick={load} className="rounded-md border border-border px-4 py-2 text-sm hover:bg-hover">
          Retry
        </button>
      </div>
    );
  }

  const sortBtn = (key: SortKey, label: string) => (
    <button type="button" onClick={() => toggleSort(key)} className="inline-flex items-center gap-1 hover:text-text">
      {label}
      <span className="text-xs text-muted">{sortKey === key ? (sortDir === 'asc' ? '↑' : '↓') : '↕'}</span>
    </button>
  );

  return (
    <div className="space-y-4">
      <ListToolbar viewMode={viewMode} onViewModeChange={setViewMode} addLabel="Add DNS provider" onAdd={openAdd} />

      {list.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border py-16 text-center">
          <Server className="mx-auto text-muted" size={40} />
          <h3 className="mt-3 font-semibold">No DNS providers</h3>
          <p className="mt-1 text-sm text-text-secondary">Add one for DNS-01 challenges (e.g. wildcard certs).</p>
          <button type="button" onClick={openAdd} className="mt-4 rounded-md bg-primary px-4 py-2 text-sm text-bg">
            Add DNS provider
          </button>
        </div>
      ) : viewMode === 'card' ? (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
          {pagedList.map((row) => (
            <div key={row.id} className="rounded-lg border border-border bg-surface p-4">
              <div className="flex items-start justify-between gap-2">
                <h3 className="flex items-center gap-2 font-semibold">
                  <Server size={16} className="text-primary" />
                  {row.name}
                </h3>
                <span className="rounded-full bg-surface-elevated px-2 py-0.5 text-xs text-text-secondary">
                  {getProviderDisplayName(row.provider_type)}
                </span>
              </div>
              <p className="mt-3 text-xs text-text-secondary">Created {formatDateTime(row.created_at)}</p>
              <div className="mt-4 flex gap-2">
                <button type="button" onClick={() => loadFullThenEdit(row.id)} className="flex-1 rounded-md border border-border px-3 py-2 text-sm hover:bg-hover">
                  <Pencil size={14} className="mr-1 inline" /> Edit
                </button>
                <button type="button" onClick={() => setDeleteRow(row)} className="rounded-md border border-border px-3 py-2 text-sm text-red-r1 hover:bg-hover">
                  <Trash2 size={14} />
                </button>
              </div>
            </div>
          ))}
        </div>
      ) : (
        <div className="overflow-x-auto rounded-lg border border-border">
          <table className="w-full min-w-[560px] text-left text-sm">
            <thead className="border-b border-border bg-surface-elevated text-text-secondary">
              <tr>
                <th className="px-4 py-3 font-medium">{sortBtn('name', 'Name')}</th>
                <th className="px-4 py-3 font-medium">{sortBtn('type', 'Type')}</th>
                <th className="px-4 py-3 font-medium">{sortBtn('created', 'Created')}</th>
                <th className="px-4 py-3 font-medium text-right">Actions</th>
              </tr>
            </thead>
            <tbody>
              {pagedList.map((row) => (
                <tr key={row.id} className="border-b border-border last:border-0 hover:bg-hover/50">
                  <td className="px-4 py-3 font-medium">{row.name}</td>
                  <td className="px-4 py-3">
                    <span className="rounded-full bg-surface-elevated px-2 py-0.5 text-xs">
                      {getProviderDisplayName(row.provider_type)}
                    </span>
                  </td>
                  <td className="px-4 py-3 text-text-secondary">{formatDateTime(row.created_at)}</td>
                  <td className="px-4 py-3">
                    <div className="flex justify-end gap-2">
                      <button type="button" onClick={() => loadFullThenEdit(row.id)} className="rounded-md border border-border px-3 py-1.5 text-xs hover:bg-hover">
                        Edit
                      </button>
                      <button type="button" onClick={() => setDeleteRow(row)} className="rounded-md border border-border px-3 py-1.5 text-xs text-red-r1 hover:bg-hover">
                        Remove
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <Pagination totalItems={list.length} pageSize={pageSize} page={page} onPageChange={setPage} />

      <Modal open={modalOpen} onClose={() => setModalOpen(false)} title={editingId ? 'Edit DNS provider' : 'Add DNS provider'} wide>
        <form onSubmit={handleSubmit} className="space-y-4" autoComplete="off">
          <label className={labelCls}>
            Name
            <input className={inputCls} value={formName} onChange={(e) => setFormName(e.target.value)} placeholder="e.g. My Cloudflare Account" required autoComplete="off" />
          </label>
          <label className={labelCls}>
            Provider type
            <select
              className={inputCls}
              value={formType}
              onChange={(e) => {
                setFormType(e.target.value);
                setFormCreds({});
              }}
              required
              disabled={!!editingId}
            >
              {!formType ? <option value="">Please select</option> : null}
              {supported.map((p) => (
                <option key={p.id} value={p.id}>{p.name}</option>
              ))}
            </select>
          </label>
          {selectedProvider && selectedProvider.fields.length > 0 ? (
            <div className="space-y-3 rounded-md border border-border p-4">
              <h4 className="text-sm font-medium">{selectedProvider.name} configuration</h4>
              {selectedProvider.fields.map((field) => (
                <CredentialField
                  key={field.key}
                  field={field}
                  value={formCreds[field.key] ?? ''}
                  onChange={(v) => setFormCreds((prev) => ({ ...prev, [field.key]: v }))}
                  visible={!!fieldVisibility[field.key]}
                  onToggleVisibility={() => setFieldVisibility((prev) => ({ ...prev, [field.key]: !prev[field.key] }))}
                />
              ))}
            </div>
          ) : null}
          {formError ? <p className="text-sm text-red-r1">{formError}</p> : null}
          <div className="flex justify-end gap-2 pt-2">
            <button type="button" onClick={() => setModalOpen(false)} className="rounded-md border border-border px-4 py-2 text-sm">Cancel</button>
            <button type="submit" disabled={saving} className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-bg disabled:opacity-50">
              {saving ? 'Saving…' : editingId ? 'Update' : 'Create'}
            </button>
          </div>
        </form>
      </Modal>

      <ConfirmDialog
        open={!!deleteRow}
        title="Remove DNS provider?"
        message={deleteRow ? `Remove "${deleteRow.name}"? This cannot be undone.` : ''}
        primaryLabel="Remove"
        variant="danger"
        loading={deleting}
        onConfirm={confirmDelete}
        onCancel={() => setDeleteRow(null)}
      />
    </div>
  );
}

function CredentialField({
  field,
  value,
  onChange,
  visible,
  onToggleVisibility,
}: {
  field: SupportedDnsProviderField;
  value: string;
  onChange: (v: string) => void;
  visible: boolean;
  onToggleVisibility: () => void;
}) {
  const isPassword = field.field_type === 'password';
  const isTextarea = field.field_type === 'textarea';
  const inputCls = 'mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm';

  return (
    <label className="block text-sm text-text-secondary">
      {field.label}
      {field.required ? ' *' : ''}
      {isTextarea ? (
        <textarea className={cn(inputCls, 'min-h-24')} value={value} onChange={(e) => onChange(e.target.value)} required={field.required} autoComplete="off" />
      ) : isPassword ? (
        <div className="relative mt-1">
          <input
            type={visible ? 'text' : 'password'}
            className={cn(inputCls, 'pr-10')}
            value={value}
            onChange={(e) => onChange(e.target.value)}
            required={field.required}
            autoComplete="new-password"
          />
          <button type="button" onClick={onToggleVisibility} className="absolute right-2 top-1/2 -translate-y-1/2 text-xs text-text-secondary hover:text-text">
            {visible ? 'Hide' : 'Show'}
          </button>
        </div>
      ) : (
        <input type="text" className={inputCls} value={value} onChange={(e) => onChange(e.target.value)} required={field.required} autoComplete="off" />
      )}
    </label>
  );
}
