import { useEffect, useMemo, useState, FormEvent } from 'react';
import { toast } from 'sonner';
import { Pencil, Shield, Trash2 } from 'lucide-react';
import { api, type AccessList, type Site } from '@/api/client';
import { Modal } from '@/components/Modal';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { Pagination } from '@/components/Pagination';
import { ListToolbar, type ViewMode } from '@/components/ListToolbar';
import { ResourceBadge, ResourceCard, ResourceCardGrid } from '@/components/ResourceCard';
import { Checkbox } from '@/components/Checkbox';
import { ConfigTextField } from '@/components/ConfigTextField';
import { usePageSize } from '@/utils/usePageSize';
import { useOpenOnQuery } from '@/utils/useOpenOnQuery';
import { cn } from '@/utils';

type SortKey = 'name' | 'usage';

function parseCountryList(raw: string): string[] {
  return raw
    .split(/[,\s]+/)
    .map((s) => s.trim().toUpperCase())
    .filter(Boolean);
}

function parseAsnList(raw: string): number[] {
  const out: number[] = [];
  for (const part of raw.split(/[,\s]+/)) {
    const t = part.trim().replace(/^AS/i, '');
    if (!t) continue;
    const n = Number(t);
    if (Number.isFinite(n) && n > 0) out.push(Math.floor(n));
  }
  return out;
}

function geoipSummary(list: AccessList): string {
  const g = list.geoip ?? {};
  if (!g.enabled) return 'Disabled';
  const bits: string[] = [];
  if (g.allow_countries?.length) bits.push(`allow ${g.allow_countries.join(',')}`);
  if (g.deny_countries?.length) bits.push(`deny ${g.deny_countries.join(',')}`);
  if (g.allow_asns?.length) bits.push(`ASN allow ${g.allow_asns.join(',')}`);
  if (g.deny_asns?.length) bits.push(`ASN deny ${g.deny_asns.join(',')}`);
  return bits.length ? bits.join(' · ') : 'Enabled (no rules)';
}

export function AccessLists() {
  const [list, setList] = useState<AccessList[]>([]);
  const [sites, setSites] = useState<Site[]>([]);
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
  const [formDescription, setFormDescription] = useState('');
  const [formEnabled, setFormEnabled] = useState(true);
  const [formAllowCountries, setFormAllowCountries] = useState('');
  const [formDenyCountries, setFormDenyCountries] = useState('');
  const [formAllowAsns, setFormAllowAsns] = useState('');
  const [formDenyAsns, setFormDenyAsns] = useState('');
  const [formError, setFormError] = useState('');
  const [saving, setSaving] = useState(false);
  const [deleteRow, setDeleteRow] = useState<AccessList | null>(null);
  const [deleting, setDeleting] = useState(false);

  function usageCount(id: string): number {
    return sites.filter((s) => s.access_list_id === id).length;
  }

  function load() {
    setLoading(true);
    setError(null);
    Promise.all([api.accessLists.list(), api.config().catch(() => null)])
      .then(([rows, cfg]) => {
        setList(rows);
        setSites(cfg?.sites ?? []);
      })
      .catch((e) => setError(e instanceof Error ? e.message : 'Failed to load'))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    load();
  }, []);

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
      return dir * (usageCount(a.id) - usageCount(b.id));
    });
  }, [list, sortKey, sortDir, sites]);

  const totalPages = Math.max(1, Math.ceil(sortedList.length / pageSize));
  const pagedList = sortedList.slice((page - 1) * pageSize, page * pageSize);

  useEffect(() => {
    setPage((p) => Math.min(p, totalPages));
  }, [totalPages]);

  function openAdd() {
    setEditingId(null);
    setFormName('');
    setFormDescription('');
    setFormEnabled(true);
    setFormAllowCountries('');
    setFormDenyCountries('');
    setFormAllowAsns('');
    setFormDenyAsns('');
    setFormError('');
    setModalOpen(true);
  }

  useOpenOnQuery('new', openAdd);

  function openEdit(row: AccessList) {
    setEditingId(row.id);
    setFormName(row.name);
    setFormDescription(row.description ?? '');
    setFormEnabled(!!row.geoip?.enabled);
    setFormAllowCountries((row.geoip?.allow_countries ?? []).join(', '));
    setFormDenyCountries((row.geoip?.deny_countries ?? []).join(', '));
    setFormAllowAsns((row.geoip?.allow_asns ?? []).join(', '));
    setFormDenyAsns((row.geoip?.deny_asns ?? []).join(', '));
    setFormError('');
    setModalOpen(true);
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setFormError('');
    const name = formName.trim();
    if (!name) {
      setFormError('Name is required');
      return;
    }
    const body = {
      name,
      description: formDescription.trim() || undefined,
      geoip: {
        enabled: formEnabled,
        allow_countries: parseCountryList(formAllowCountries),
        deny_countries: parseCountryList(formDenyCountries),
        allow_asns: parseAsnList(formAllowAsns),
        deny_asns: parseAsnList(formDenyAsns),
      },
    };
    setSaving(true);
    try {
      if (editingId) await api.accessLists.update(editingId, body);
      else await api.accessLists.create(body);
      toast.success(editingId ? 'Access list updated' : 'Access list created');
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
      await api.accessLists.delete(deleteRow.id);
      toast.success('Access list removed');
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

  if (loading && list.length === 0) return <p className="text-text-secondary">Loading access lists…</p>;
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
      <ListToolbar viewMode={viewMode} onViewModeChange={setViewMode} addLabel="Add access list" onAdd={openAdd} />

      {list.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border py-16 text-center">
          <Shield className="mx-auto text-muted" size={40} />
          <h3 className="mt-3 font-semibold">No access lists</h3>
          <p className="mt-1 text-sm text-text-secondary">
            Create a GeoIP allow/deny profile, then attach it when adding a site.
          </p>
          <button type="button" onClick={openAdd} className="mt-4 rounded-md bg-primary px-4 py-2 text-sm text-bg">
            Add access list
          </button>
        </div>
      ) : viewMode === 'card' ? (
        <ResourceCardGrid>
          {pagedList.map((row) => (
            <ResourceCard
              key={row.id}
              icon={<Shield size={16} />}
              title={row.name}
              badge={
                <ResourceBadge tone={row.geoip?.enabled ? 'success' : 'neutral'}>
                  {row.geoip?.enabled ? 'Enabled' : 'Off'}
                </ResourceBadge>
              }
              meta={[
                { label: 'Rules', value: geoipSummary(row) },
                { label: 'Sites', value: String(usageCount(row.id)) },
              ]}
              actions={
                <>
                  <button type="button" onClick={() => openEdit(row)} className="icon-action" title="Edit" aria-label="Edit">
                    <Pencil size={16} />
                  </button>
                  <button
                    type="button"
                    onClick={() => setDeleteRow(row)}
                    className="icon-action danger"
                    title="Remove"
                    aria-label="Remove"
                  >
                    <Trash2 size={16} />
                  </button>
                </>
              }
            />
          ))}
        </ResourceCardGrid>
      ) : (
        <div className="overflow-x-auto rounded-lg border border-border">
          <table className="w-full min-w-[640px] text-left text-sm">
            <thead className="border-b border-border bg-surface-elevated text-text-secondary">
              <tr>
                <th className="px-4 py-3 font-medium">{sortBtn('name', 'Name')}</th>
                <th className="px-4 py-3 font-medium">Rules</th>
                <th className="px-4 py-3 font-medium">{sortBtn('usage', 'Sites')}</th>
                <th className="actions-cell px-4 py-3 font-medium">Actions</th>
              </tr>
            </thead>
            <tbody>
              {pagedList.map((row) => (
                <tr key={row.id} className="border-t border-border hover:bg-hover/40">
                  <td className="px-4 py-3 font-medium">
                    {row.name}
                    {row.description ? (
                      <div className="text-xs font-normal text-text-secondary">{row.description}</div>
                    ) : null}
                  </td>
                  <td className="px-4 py-3 text-xs text-text-secondary">{geoipSummary(row)}</td>
                  <td className="px-4 py-3">{usageCount(row.id)}</td>
                  <td className="actions-cell px-4 py-3">
                    <div className="flex items-center gap-1">
                      <button type="button" onClick={() => openEdit(row)} className="icon-action" title="Edit">
                        <Pencil size={16} />
                      </button>
                      <button type="button" onClick={() => setDeleteRow(row)} className="icon-action danger" title="Remove">
                        <Trash2 size={16} />
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {list.length > 0 ? (
        <Pagination totalItems={sortedList.length} pageSize={pageSize} page={page} onPageChange={setPage} />
      ) : null}

      <Modal
        open={modalOpen}
        onClose={() => setModalOpen(false)}
        title={editingId ? 'Edit access list' : 'Add access list'}
        wide
      >
        <form onSubmit={handleSubmit} className="space-y-4">
          {formError ? <p className="text-sm text-red-r1">{formError}</p> : null}
          <label className={labelCls}>
            Name
            <input className={inputCls} value={formName} onChange={(e) => setFormName(e.target.value)} required />
          </label>
          <label className={labelCls}>
            Description
            <input
              className={inputCls}
              value={formDescription}
              onChange={(e) => setFormDescription(e.target.value)}
              placeholder="Optional"
            />
          </label>
          <Checkbox checked={formEnabled} onChange={setFormEnabled} label="Enable GeoIP allow / deny" className="text-sm" />
          <div className={cn('grid gap-3 sm:grid-cols-2', !formEnabled && 'opacity-60')}>
            <ConfigTextField
              label="Allow countries"
              example="TH, US"
              value={formAllowCountries}
              onChange={setFormAllowCountries}
              disabled={!formEnabled}
            />
            <ConfigTextField
              label="Deny countries"
              example="CN, RU"
              value={formDenyCountries}
              onChange={setFormDenyCountries}
              disabled={!formEnabled}
            />
            <ConfigTextField
              label="Allow ASNs"
              example="13335, AS15169"
              value={formAllowAsns}
              onChange={setFormAllowAsns}
              disabled={!formEnabled}
            />
            <ConfigTextField
              label="Deny ASNs"
              example="12345"
              value={formDenyAsns}
              onChange={setFormDenyAsns}
              disabled={!formEnabled}
            />
          </div>
          <div className="flex justify-end gap-2 pt-2">
            <button type="button" onClick={() => setModalOpen(false)} className="rounded-md border border-border px-4 py-2 text-sm hover:bg-hover">
              Cancel
            </button>
            <button type="submit" disabled={saving} className="rounded-md bg-primary px-4 py-2 text-sm text-bg disabled:opacity-50">
              {saving ? 'Saving…' : editingId ? 'Save' : 'Create'}
            </button>
          </div>
        </form>
      </Modal>

      <ConfirmDialog
        open={!!deleteRow}
        title="Delete access list?"
        message={
          deleteRow
            ? `Remove “${deleteRow.name}”? Sites using it will keep their last GeoIP snapshot but lose the list link.`
            : ''
        }
        primaryLabel="Delete"
        variant="danger"
        loading={deleting}
        onConfirm={confirmDelete}
        onCancel={() => setDeleteRow(null)}
      />
    </div>
  );
}
