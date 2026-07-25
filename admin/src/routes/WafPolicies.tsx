import { useEffect, useMemo, useState, FormEvent } from 'react';
import { toast } from 'sonner';
import { Pencil, ShieldAlert, Trash2 } from 'lucide-react';
import { api, type NamedWafPolicy, type Site } from '@/api/client';
import { Modal } from '@/components/Modal';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { Pagination } from '@/components/Pagination';
import { ListToolbar, type ViewMode } from '@/components/ListToolbar';
import { ResourceBadge, ResourceCard, ResourceCardGrid } from '@/components/ResourceCard';
import { Checkbox } from '@/components/Checkbox';
import { usePageSize } from '@/utils/usePageSize';

type SortKey = 'name' | 'usage';

function securitySummary(policy: NamedWafPolicy): string {
  const s = policy.security ?? {};
  const bits: string[] = [];
  if (s.waf?.enabled) bits.push(s.waf.use_builtin_rules === false ? 'WAF custom' : 'WAF');
  if (s.bot?.enabled) bits.push(`bot ${s.bot.challenge_score ?? 40}/${s.bot.block_score ?? 80}`);
  if (s.captcha?.enabled) bits.push('captcha');
  return bits.length ? bits.join(' · ') : 'Disabled';
}

export function WafPolicies() {
  const [list, setList] = useState<NamedWafPolicy[]>([]);
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
  const [formWafEnabled, setFormWafEnabled] = useState(false);
  const [formWafBuiltin, setFormWafBuiltin] = useState(true);
  const [formBotEnabled, setFormBotEnabled] = useState(false);
  const [formBotChallenge, setFormBotChallenge] = useState('40');
  const [formBotBlock, setFormBotBlock] = useState('80');
  const [formCaptchaEnabled, setFormCaptchaEnabled] = useState(false);
  const [formError, setFormError] = useState('');
  const [saving, setSaving] = useState(false);
  const [deleteRow, setDeleteRow] = useState<NamedWafPolicy | null>(null);
  const [deleting, setDeleting] = useState(false);

  function usageCount(id: string): number {
    return sites.filter((s) => s.waf_policy_id === id).length;
  }

  function load() {
    setLoading(true);
    setError(null);
    Promise.all([api.wafPolicies.list(), api.config().catch(() => null)])
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
    setFormWafEnabled(false);
    setFormWafBuiltin(true);
    setFormBotEnabled(false);
    setFormBotChallenge('40');
    setFormBotBlock('80');
    setFormCaptchaEnabled(false);
    setFormError('');
    setModalOpen(true);
  }

  function openEdit(row: NamedWafPolicy) {
    setEditingId(row.id);
    setFormName(row.name);
    setFormDescription(row.description ?? '');
    setFormWafEnabled(!!row.security?.waf?.enabled);
    setFormWafBuiltin(row.security?.waf?.use_builtin_rules !== false);
    setFormBotEnabled(!!row.security?.bot?.enabled);
    setFormBotChallenge(String(row.security?.bot?.challenge_score ?? 40));
    setFormBotBlock(String(row.security?.bot?.block_score ?? 80));
    setFormCaptchaEnabled(!!row.security?.captcha?.enabled);
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
    const challenge = Number(formBotChallenge);
    const block = Number(formBotBlock);
    if (formBotEnabled && (!Number.isFinite(challenge) || !Number.isFinite(block))) {
      setFormError('Bot scores must be numbers');
      return;
    }
    const body = {
      name,
      description: formDescription.trim() || undefined,
      security: {
        waf: {
          enabled: formWafEnabled,
          use_builtin_rules: formWafBuiltin,
        },
        bot: {
          enabled: formBotEnabled,
          challenge_score: Number.isFinite(challenge) ? challenge : 40,
          block_score: Number.isFinite(block) ? block : 80,
        },
        captcha: {
          enabled: formCaptchaEnabled,
        },
      },
    };
    setSaving(true);
    try {
      if (editingId) await api.wafPolicies.update(editingId, body);
      else await api.wafPolicies.create(body);
      toast.success(editingId ? 'WAF policy updated' : 'WAF policy created');
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
      await api.wafPolicies.delete(deleteRow.id);
      toast.success('WAF policy removed');
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

  if (loading && list.length === 0) return <p className="text-text-secondary">Loading WAF policies…</p>;
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

  const active = (row: NamedWafPolicy) =>
    !!(row.security?.waf?.enabled || row.security?.bot?.enabled || row.security?.captcha?.enabled);

  return (
    <div className="space-y-4">
      <ListToolbar viewMode={viewMode} onViewModeChange={setViewMode} addLabel="Add WAF policy" onAdd={openAdd} />

      {list.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border py-16 text-center">
          <ShieldAlert className="mx-auto text-muted" size={40} />
          <h3 className="mt-3 font-semibold">No WAF policies</h3>
          <p className="mt-1 text-sm text-text-secondary">
            Create a WAF / bot / captcha profile, then attach it when adding a site.
          </p>
          <button type="button" onClick={openAdd} className="mt-4 rounded-md bg-primary px-4 py-2 text-sm text-bg">
            Add WAF policy
          </button>
        </div>
      ) : viewMode === 'card' ? (
        <ResourceCardGrid>
          {pagedList.map((row) => (
            <ResourceCard
              key={row.id}
              icon={<ShieldAlert size={16} />}
              title={row.name}
              badge={<ResourceBadge tone={active(row) ? 'success' : 'neutral'}>{active(row) ? 'Active' : 'Off'}</ResourceBadge>}
              meta={[
                { label: 'Rules', value: securitySummary(row) },
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
                  <td className="px-4 py-3 text-xs text-text-secondary">{securitySummary(row)}</td>
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
        title={editingId ? 'Edit WAF policy' : 'Add WAF policy'}
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

          <div className="space-y-3 rounded-lg border border-border p-3">
            <Checkbox checked={formWafEnabled} onChange={setFormWafEnabled} label="Enable WAF" className="text-sm" />
            <Checkbox
              checked={formWafBuiltin}
              onChange={setFormWafBuiltin}
              label="Use built-in WAF rules"
              className="text-sm"
              disabled={!formWafEnabled}
            />
          </div>

          <div className="space-y-3 rounded-lg border border-border p-3">
            <Checkbox checked={formBotEnabled} onChange={setFormBotEnabled} label="Enable bot scoring" className="text-sm" />
            <div className="grid gap-3 sm:grid-cols-2">
              <label className={labelCls}>
                Challenge score
                <input
                  className={inputCls}
                  value={formBotChallenge}
                  onChange={(e) => setFormBotChallenge(e.target.value)}
                  disabled={!formBotEnabled}
                />
              </label>
              <label className={labelCls}>
                Block score
                <input
                  className={inputCls}
                  value={formBotBlock}
                  onChange={(e) => setFormBotBlock(e.target.value)}
                  disabled={!formBotEnabled}
                />
              </label>
            </div>
          </div>

          <div className="rounded-lg border border-border p-3">
            <Checkbox checked={formCaptchaEnabled} onChange={setFormCaptchaEnabled} label="Enable captcha" className="text-sm" />
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
        title="Delete WAF policy?"
        message={
          deleteRow
            ? `Remove “${deleteRow.name}”? Sites using it will keep their last security snapshot but lose the policy link.`
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
