import { useEffect, useState, FormEvent } from 'react';
import { toast } from 'sonner';
import { Plus, Pencil, Trash2, Eye, EyeOff } from 'lucide-react';
import { api, type DnsProviderRow, type SupportedDnsProvider } from '@/api/client';
import { Card } from '@/components/Card';

export function DnsProviders() {
  const [list, setList] = useState<DnsProviderRow[]>([]);
  const [supported, setSupported] = useState<SupportedDnsProvider[]>([]);
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [formName, setFormName] = useState('');
  const [formType, setFormType] = useState('');
  const [formCreds, setFormCreds] = useState<Record<string, string>>({});
  const [visible, setVisible] = useState<Record<string, boolean>>({});
  const [saving, setSaving] = useState(false);

  const selected = supported.find((p) => p.id === formType);

  function load() {
    setLoading(true);
    Promise.all([api.dnsProviders.list(), api.dnsProviders.supported()])
      .then(([rows, sup]) => {
        setList(rows);
        setSupported(sup);
        if (!formType && sup[0]) setFormType(sup[0].id);
      })
      .catch((e) => toast.error(e.message))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    load();
  }, []);

  function openAdd() {
    setEditingId(null);
    setFormName('');
    setFormType(supported[0]?.id || '');
    setFormCreds({});
    setShowForm(true);
  }

  function openEdit(row: DnsProviderRow) {
    api.dnsProviders
      .get(row.id)
      .then((full) => {
        setEditingId(full.id);
        setFormName(full.name);
        setFormType(full.provider_type);
        setFormCreds(full.credentials || {});
        setShowForm(true);
      })
      .catch((e) => toast.error(e.message));
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    const name = formName.trim();
    if (!name || !formType) {
      toast.error('Name and provider type are required');
      return;
    }
    const credentials: Record<string, string> = {};
    for (const field of selected?.fields || []) {
      const v = formCreds[field.key]?.trim();
      if (v) credentials[field.key] = v;
      else if (field.required) {
        toast.error(`${field.label} is required`);
        return;
      }
    }
    setSaving(true);
    try {
      const body = { name, provider_type: formType, credentials: Object.keys(credentials).length ? credentials : undefined };
      if (editingId) {
        await api.dnsProviders.update(editingId, body);
        toast.success('DNS provider updated');
      } else {
        await api.dnsProviders.create(body);
        toast.success('DNS provider created');
      }
      setShowForm(false);
      load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Save failed');
    } finally {
      setSaving(false);
    }
  }

  async function remove(id: string) {
    if (!confirm('Delete this DNS provider?')) return;
    try {
      await api.dnsProviders.delete(id);
      toast.success('Deleted');
      load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Delete failed');
    }
  }

  if (loading) return <p className="text-text-secondary">Loading DNS providers…</p>;

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold">DNS providers</h2>
          <p className="text-sm text-text-secondary">Credentials for ACME DNS-01 (metadata only until ACME lands)</p>
        </div>
        <button type="button" onClick={openAdd} className="flex items-center gap-2 rounded-md border border-border px-3 py-2 text-sm hover:bg-hover">
          <Plus size={16} /> Add provider
        </button>
      </div>

      {showForm ? (
        <Card>
          <form onSubmit={handleSubmit} className="space-y-4">
            <h3 className="font-semibold">{editingId ? 'Edit provider' : 'Add provider'}</h3>
            <label className="block text-sm">
              <span className="text-text-secondary">Name</span>
              <input className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2" value={formName} onChange={(e) => setFormName(e.target.value)} />
            </label>
            <label className="block text-sm">
              <span className="text-text-secondary">Provider</span>
              <select className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2" value={formType} onChange={(e) => { setFormType(e.target.value); setFormCreds({}); }}>
                {supported.map((p) => (
                  <option key={p.id} value={p.id}>{p.name}</option>
                ))}
              </select>
            </label>
            {selected?.fields.map((field) => (
              <label key={field.key} className="block text-sm">
                <span className="text-text-secondary">{field.label}{field.required ? ' *' : ''}</span>
                <div className="relative mt-1">
                  <input
                    type={field.field_type === 'password' && !visible[field.key] ? 'password' : 'text'}
                    className="w-full rounded-md border border-border bg-bg px-3 py-2 pr-10"
                    value={formCreds[field.key] || ''}
                    onChange={(e) => setFormCreds((c) => ({ ...c, [field.key]: e.target.value }))}
                  />
                  {field.field_type === 'password' ? (
                    <button type="button" className="absolute right-2 top-1/2 -translate-y-1/2 text-text-secondary" onClick={() => setVisible((v) => ({ ...v, [field.key]: !v[field.key] }))}>
                      {visible[field.key] ? <EyeOff size={16} /> : <Eye size={16} />}
                    </button>
                  ) : null}
                </div>
              </label>
            ))}
            <div className="flex gap-2">
              <button type="submit" disabled={saving} className="rounded-md bg-primary px-4 py-2 text-sm text-bg disabled:opacity-50">{saving ? 'Saving…' : 'Save'}</button>
              <button type="button" onClick={() => setShowForm(false)} className="rounded-md border border-border px-4 py-2 text-sm">Cancel</button>
            </div>
          </form>
        </Card>
      ) : null}

      {list.length === 0 ? (
        <Card><p className="text-text-secondary">No DNS providers configured.</p></Card>
      ) : (
        list.map((row) => (
          <Card key={row.id}>
            <div className="flex items-start justify-between gap-3">
              <div>
                <h3 className="font-semibold">{row.name}</h3>
                <p className="text-sm text-text-secondary">{row.provider_type}</p>
                <p className="text-xs text-text-secondary">Added {new Date(row.created_at).toLocaleString()}</p>
              </div>
              <div className="flex gap-2">
                <button type="button" onClick={() => openEdit(row)} className="rounded-md border border-border p-2 hover:bg-hover"><Pencil size={16} /></button>
                <button type="button" onClick={() => remove(row.id)} className="rounded-md border border-border p-2 text-red-r1 hover:bg-hover"><Trash2 size={16} /></button>
              </div>
            </div>
          </Card>
        ))
      )}
    </div>
  );
}
