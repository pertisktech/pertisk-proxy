import { useEffect, useMemo, useState, FormEvent } from 'react';
import { toast } from 'sonner';
import { Award, Globe, Lock, Pencil, Plus, Trash2, X, Zap } from 'lucide-react';
import {
  api,
  type DnsProviderRow,
  type PathRewrite,
  type ProxyConfig,
  type Site,
  type TlsConfig,
  type TlsSource,
} from '@/api/client';
import { Modal } from '@/components/Modal';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { Pagination } from '@/components/Pagination';
import { ListToolbar, type ViewMode } from '@/components/ListToolbar';
import { usePageSize } from '@/utils/usePageSize';
import {
  acmeChallengeFromSource,
  hostToWildcard,
  inferSslModeForSite,
  resolveTlsForHost,
  siteUsesWildcardInTls,
  sslLabelForCard,
  sslLabelForDropdown,
  tlsIndexForHost,
  type SiteSslMode,
} from '@/utils/tlsHostMatch';
import { cn } from '@/utils';

const PATH_TYPES = ['Exact', 'Prefix', 'ImplementationSpecific'];

function normalizeUpstream(url: string): string {
  const s = url.trim();
  if (!s) return s;
  if (/^https?:\/\//i.test(s)) return s;
  return 'http://' + s;
}

function routeLabel(route: PathRewrite): string {
  return `${route.path_type || 'Prefix'} ${route.path}${route.rewrite ? ` → ${route.rewrite}` : ''}`;
}

type SiteSslStatus = { label: string; tone: string };

function siteSslStatus(tls: TlsConfig | null): SiteSslStatus {
  if (!tls) return { label: 'No SSL', tone: 'text-muted' };
  const expiresAt = tls.expires_at?.trim();
  if (expiresAt) {
    const expiryMs = Date.parse(expiresAt);
    if (!Number.isNaN(expiryMs)) {
      if (expiryMs > Date.now()) return { label: 'Online', tone: 'text-green-g1' };
      return { label: 'Expired', tone: 'text-red-r1' };
    }
  }
  if (tls.source?.type === 'acme') return { label: 'Pending', tone: 'text-yellow-y1' };
  return { label: 'Configured', tone: 'text-text-secondary' };
}

function resolveDnsProviderId(
  acme: Extract<TlsSource, { type: 'acme' }>,
  providers: DnsProviderRow[],
): string {
  const byName = providers.find((p) => p.name === (acme.dns_provider ?? '').trim());
  if (byName) return byName.id;
  const byType = providers.filter((p) => p.provider_type === (acme.dns_provider_type ?? '').trim());
  return byType.length === 1 ? byType[0].id : '';
}

export function Sites() {
  const [config, setConfig] = useState<ProxyConfig>({ sites: [], backends: [], tls: [] });
  const [dnsProviders, setDnsProviders] = useState<DnsProviderRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [viewMode, setViewMode] = useState<ViewMode>('list');
  const [page, setPage] = useState(1);
  const pageSize = usePageSize();

  const [siteModal, setSiteModal] = useState(false);
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [formHost, setFormHost] = useState('');
  const [formUpstream, setFormUpstream] = useState('');
  const [formRoutes, setFormRoutes] = useState<PathRewrite[]>([{ path: '/', path_type: 'Prefix', rewrite: '/' }]);
  const [formSslMode, setFormSslMode] = useState<SiteSslMode>('none');
  const [formTlsIndex, setFormTlsIndex] = useState(0);
  const [formAcmeEmail, setFormAcmeEmail] = useState('');
  const [formAcmeChallenge, setFormAcmeChallenge] = useState<'http01' | 'dns01'>('http01');
  const [formDnsProviderId, setFormDnsProviderId] = useState('');
  const [formWildcard, setFormWildcard] = useState(false);
  const [siteSaving, setSiteSaving] = useState(false);
  const [siteError, setSiteError] = useState('');

  const [deleteIndex, setDeleteIndex] = useState<number | null>(null);
  const [deleting, setDeleting] = useState(false);

  const sites = config.sites;
  const backends = config.backends;
  const tlsList = config.tls;

  function load() {
    setLoading(true);
    Promise.all([api.config(), api.dnsProviders.list().catch(() => [])])
      .then(([c, dns]) => {
        setConfig({ sites: c.sites || [], backends: c.backends || [], tls: c.tls || [] });
        setDnsProviders(dns);
      })
      .catch((e) => toast.error(e.message))
      .finally(() => setLoading(false));
  }

  function refreshQuiet() {
    Promise.all([api.config(), api.dnsProviders.list().catch(() => [])])
      .then(([c, dns]) => {
        setConfig({ sites: c.sites || [], backends: c.backends || [], tls: c.tls || [] });
        setDnsProviders(dns);
      })
      .catch(() => {});
  }

  const hasPendingAcme = useMemo(() => {
    return sites.some((site) => {
      const tls = resolveTlsForHost(site.host, tlsList);
      return tls?.source?.type === 'acme' && !tls.expires_at?.trim();
    });
  }, [sites, tlsList]);

  useEffect(() => {
    load();
  }, []);

  useEffect(() => {
    if (!hasPendingAcme) return;
    let cancelled = false;
    const tick = () => {
      if (cancelled || document.visibilityState !== 'visible') return;
      refreshQuiet();
    };
    const t = setInterval(tick, 5000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, [hasPendingAcme]);

  useEffect(() => {
    if (!siteModal || editingIndex === null || formSslMode !== 'generate' || formAcmeChallenge !== 'dns01') return;
    const site = sites[editingIndex];
    if (!site) return;
    const tls = resolveTlsForHost(site.host, tlsList);
    const acme = tls?.source?.type === 'acme' ? tls.source : null;
    if (!acme) return;
    const resolved = resolveDnsProviderId(acme, dnsProviders);
    if (resolved) setFormDnsProviderId(resolved);
  }, [siteModal, editingIndex, formSslMode, formAcmeChallenge, dnsProviders, sites, tlsList]);

  const totalPages = Math.max(1, Math.ceil(sites.length / pageSize));
  const pagedSites = useMemo(() => {
    const start = (page - 1) * pageSize;
    return sites.slice(start, start + pageSize);
  }, [sites, page, pageSize]);

  useEffect(() => {
    setPage((p) => Math.min(p, totalPages));
  }, [totalPages]);

  function upstreamForSite(site: Site): string {
    const be = backends.find((b) => b.name === site.backend);
    return be?.upstreams?.[0]?.addr ?? site.backend;
  }

  function openAddSite() {
    setEditingIndex(null);
    setFormHost('');
    setFormUpstream('');
    setFormRoutes([{ path: '/', path_type: 'Prefix', rewrite: '/' }]);
    setFormSslMode('none');
    setFormTlsIndex(0);
    setFormAcmeEmail('');
    setFormAcmeChallenge('http01');
    setFormDnsProviderId('');
    setFormWildcard(false);
    setSiteError('');
    setSiteModal(true);
  }

  function openEditSite(index: number) {
    const site = sites[index];
    if (!site) return;
    const be = backends.find((b) => b.name === site.backend);
    const tlsForHost = resolveTlsForHost(site.host, tlsList);
    const sslMode = inferSslModeForSite(site.host, tlsList);
    const tlsIdx = tlsForHost ? tlsIndexForHost(site.host, tlsList) : -1;
    const acmeSource = sslMode === 'generate' && tlsForHost?.source?.type === 'acme' ? tlsForHost.source : null;

    setEditingIndex(index);
    setFormHost(site.host);
    setFormUpstream(be?.upstreams?.[0]?.addr ?? '');
    setFormRoutes(
      site.routes?.length
        ? site.routes.map((r) => ({
            path: r.path ?? '',
            path_type: r.path_type ?? 'Prefix',
            rewrite: r.rewrite ?? '',
          }))
        : [{ path: '/', path_type: 'Prefix', rewrite: '/' }],
    );
    setFormSslMode(sslMode);
    setFormTlsIndex(tlsIdx >= 0 ? tlsIdx : 0);
    setFormAcmeChallenge(acmeSource ? acmeChallengeFromSource(acmeSource) : 'http01');
    setFormAcmeEmail(acmeSource?.email ?? '');
    setFormWildcard(tlsForHost ? siteUsesWildcardInTls(site.host, tlsForHost) : false);
    setFormDnsProviderId(
      acmeSource && acmeChallengeFromSource(acmeSource) === 'dns01'
        ? resolveDnsProviderId(acmeSource, dnsProviders)
        : '',
    );
    setSiteError('');
    setSiteModal(true);
  }

  function addRoute() {
    setFormRoutes((r) => [...r, { path: '', path_type: 'Prefix', rewrite: '' }]);
  }

  function removeRoute(i: number) {
    setFormRoutes((r) => (r.length > 1 ? r.filter((_, idx) => idx !== i) : r));
  }

  function updateRoute(i: number, field: keyof PathRewrite, value: string) {
    setFormRoutes((routes) => {
      const next = [...routes];
      const cur = { ...next[i] };
      if (field === 'path') cur.path = value;
      else if (field === 'path_type') cur.path_type = value;
      else if (field === 'rewrite') cur.rewrite = value;
      next[i] = cur;
      return next;
    });
  }

  function buildRoutes(): PathRewrite[] {
    return formRoutes
      .map((r) => ({
        path: r.path.trim(),
        path_type: r.path_type || 'Prefix',
        rewrite: r.rewrite?.trim() || undefined,
      }))
      .filter((r) => r.path);
  }

  async function submitSite(e: FormEvent) {
    e.preventDefault();
    setSiteError('');
    const host = formHost.trim();
    if (!host) {
      setSiteError('Domain is required');
      return;
    }
    const rawUpstream = formUpstream.trim();
    if (!rawUpstream) {
      setSiteError('Upstream URL is required');
      return;
    }
    const routes = buildRoutes();
    if (!routes.length) {
      setSiteError('At least one route with a path is required');
      return;
    }

    const addr = normalizeUpstream(rawUpstream);
    let newBackends = [...backends];
    const baseName =
      'inline-' +
        host
          .replace(/[^a-z0-9.-]/gi, '-')
          .replace(/-+/g, '-')
          .replace(/^-|-$/g, '') || 'site';
    let backendName: string;

    if (editingIndex !== null) {
      const existingSite = sites[editingIndex];
      const existingBackend = existingSite && newBackends.find((b) => b.name === existingSite.backend);
      if (existingBackend?.upstreams?.length === 1) {
        newBackends = newBackends.map((b) =>
          b.name === existingSite!.backend ? { ...b, upstreams: [{ ...b.upstreams[0], addr }] } : b,
        );
        backendName = existingSite!.backend;
      } else {
        backendName = baseName;
        let n = 1;
        while (newBackends.some((b) => b.name === backendName)) backendName = `${baseName}-${n++}`;
        newBackends = [...newBackends, { name: backendName, upstreams: [{ addr }] }];
      }
    } else {
      backendName = baseName;
      let n = 1;
      while (newBackends.some((b) => b.name === backendName)) backendName = `${baseName}-${n++}`;
      newBackends = [...newBackends, { name: backendName, upstreams: [{ addr }] }];
    }

    let newTls = [...tlsList];
    const removeHostFromTls = (list: TlsConfig[]) =>
      list
        .map((t) => ({ ...t, hosts: t.hosts.filter((h) => h !== host) }))
        .filter((t) => t.hosts.length > 0);

    if (formSslMode === 'none') {
      newTls = removeHostFromTls(newTls);
    } else if (formSslMode === 'from_list') {
      newTls = removeHostFromTls(newTls);
      const orig = tlsList[formTlsIndex];
      if (orig) {
        const rest = orig.hosts.filter((h) => h !== host);
        const idx = newTls.findIndex((t) => t.hosts.length === rest.length && rest.every((h) => t.hosts.includes(h)));
        if (idx >= 0) {
          newTls = newTls.map((t, i) => (i === idx ? { ...t, hosts: [...t.hosts, host] } : t));
        } else {
          newTls = [...newTls, { ...orig, hosts: [host, ...rest] }];
        }
      }
    } else if (formSslMode === 'generate') {
      newTls = removeHostFromTls(newTls);
      const acmeEmailTrim = formAcmeEmail.trim();
      if (!acmeEmailTrim || acmeEmailTrim.includes('@example.com')) {
        setSiteError("A valid contact email is required for Let's Encrypt (e.g. you@yourdomain.com)");
        return;
      }
      if (formAcmeChallenge === 'dns01' && !formDnsProviderId) {
        setSiteError('Select a DNS Provider for DNS-01 challenge');
        return;
      }
      const hosts = formWildcard ? [hostToWildcard(host), host] : [host];
      const mergeAcmeTls = (acmeSource: TlsSource) => {
        const wildcardKey = formWildcard ? hostToWildcard(host).toLowerCase() : null;
        const existingIdx = newTls.findIndex((t) => {
          if (t.source?.type !== 'acme') return false;
          if (acmeChallengeFromSource(t.source as Extract<TlsSource, { type: 'acme' }>) !== formAcmeChallenge) {
            return false;
          }
          if (wildcardKey) {
            return (t.hosts ?? []).some((h) => h.trim().toLowerCase() === wildcardKey);
          }
          return (t.hosts ?? []).some((h) => h.trim().toLowerCase() === host.trim().toLowerCase());
        });
        if (existingIdx >= 0) {
          const existing = newTls[existingIdx];
          const mergedHosts = [...new Set([...(existing.hosts ?? []), ...hosts])];
          newTls = newTls.map((t, i) =>
            i === existingIdx ? { ...t, hosts: mergedHosts, source: acmeSource } : t,
          );
        } else {
          newTls = [...newTls, { hosts, source: acmeSource }];
        }
      };
      if (formAcmeChallenge === 'dns01' && formDnsProviderId) {
        try {
          const provider = await api.dnsProviders.get(formDnsProviderId);
          mergeAcmeTls({
            type: 'acme',
            challenge: 'dns01',
            email: acmeEmailTrim,
            dns_provider: provider.name,
            dns_provider_type: provider.provider_type,
            dns_credentials: provider.credentials ?? undefined,
          });
        } catch (err) {
          setSiteError(err instanceof Error ? err.message : 'Failed to load DNS Provider');
          return;
        }
      } else {
        mergeAcmeTls({ type: 'acme', challenge: 'http01', email: acmeEmailTrim });
      }
    }

    const newSite: Site = { host, backend: backendName, routes };
    const newSites =
      editingIndex !== null ? sites.map((s, i) => (i === editingIndex ? newSite : s)) : [...sites, newSite];

    setSiteSaving(true);
    try {
      const res = await api.saveConfig({ ...config, backends: newBackends, sites: newSites, tls: newTls });
      setConfig({ ...config, backends: newBackends, sites: newSites, tls: newTls });
      toast.success(`${editingIndex !== null ? 'Site updated' : 'Site added'} (${res.route_count} routes active)`);
      setSiteModal(false);
      setEditingIndex(null);
    } catch (err) {
      setSiteError(err instanceof Error ? err.message : 'Save failed');
    } finally {
      setSiteSaving(false);
    }
  }

  async function confirmDeleteSite() {
    if (deleteIndex === null) return;
    setDeleting(true);
    try {
      const newSites = sites.filter((_, i) => i !== deleteIndex);
      const res = await api.saveConfig({ ...config, sites: newSites });
      setConfig({ ...config, sites: newSites });
      toast.success(`Site removed (${res.route_count} routes active)`);
      setDeleteIndex(null);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Delete failed');
    } finally {
      setDeleting(false);
    }
  }

  const inputCls = 'mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm';
  const labelCls = 'block text-sm text-text-secondary';
  const sectionCls = 'space-y-3 rounded-md border border-border p-4';
  const sectionTitleCls = 'text-sm font-semibold text-text';

  if (loading) return <p className="text-text-secondary">Loading sites…</p>;

  return (
    <div className="space-y-4">
      <div className="mb-2 flex items-center justify-between">
        <h2 className="text-lg font-semibold">Sites</h2>
        <span className="text-sm text-text-secondary">{sites.length} site(s)</span>
      </div>
      <ListToolbar viewMode={viewMode} onViewModeChange={setViewMode} addLabel="Add site" onAdd={openAddSite} />

      {sites.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border py-16 text-center">
          <Globe className="mx-auto text-muted" size={40} />
          <h3 className="mt-3 font-semibold">No sites</h3>
          <p className="mt-1 text-sm text-text-secondary">Add a site to route traffic through the proxy.</p>
          <button type="button" onClick={openAddSite} className="mt-4 rounded-md bg-primary px-4 py-2 text-sm text-bg">
            Add site
          </button>
        </div>
      ) : viewMode === 'card' ? (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
          {pagedSites.map((site) => {
            const globalIndex = sites.indexOf(site);
            const tls = resolveTlsForHost(site.host, tlsList);
            const ssl = siteSslStatus(tls);
            return (
              <div key={site.host} className="rounded-lg border border-border bg-surface p-4">
                <div className="flex items-start justify-between gap-2">
                  <h3 className="font-semibold">{site.host}</h3>
                  <span className={cn('text-xs font-medium', ssl.tone)}>{ssl.label}</span>
                </div>
                <p className="mt-2 font-mono text-xs text-text-secondary">{upstreamForSite(site)}</p>
                <div className="mt-2 flex flex-wrap gap-1">
                  {(site.routes ?? []).map((r, i) => (
                    <span key={i} className="rounded bg-surface-elevated px-2 py-0.5 text-xs text-text-secondary">
                      {routeLabel(r)}
                    </span>
                  ))}
                </div>
                {tls ? (
                  <p className="mt-2 text-xs text-text-secondary">Cert: {sslLabelForCard(tls.hosts)}</p>
                ) : null}
                <div className="mt-4 icon-actions">
                  <button
                    type="button"
                    onClick={() => openEditSite(globalIndex)}
                    className="icon-action"
                    title="Edit site"
                    aria-label="Edit site"
                  >
                    <Pencil size={16} />
                  </button>
                  <button
                    type="button"
                    onClick={() => setDeleteIndex(globalIndex)}
                    className="icon-action danger"
                    title="Remove site"
                    aria-label="Remove site"
                  >
                    <Trash2 size={16} />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        <div className="overflow-x-auto rounded-lg border border-border">
          <table className="w-full min-w-[720px] text-left text-sm">
            <thead className="border-b border-border bg-surface-elevated text-text-secondary">
              <tr>
                <th className="px-4 py-3 font-medium">Domain</th>
                <th className="px-4 py-3 font-medium">Upstream</th>
                <th className="px-4 py-3 font-medium">Routes</th>
                <th className="px-4 py-3 font-medium">SSL</th>
                <th className="actions-cell px-4 py-3 font-medium">Actions</th>
              </tr>
            </thead>
            <tbody>
              {pagedSites.map((site) => {
                const globalIndex = sites.indexOf(site);
                const tls = resolveTlsForHost(site.host, tlsList);
                const ssl = siteSslStatus(tls);
                return (
                  <tr key={site.host} className="border-b border-border last:border-0 hover:bg-hover/50">
                    <td className="px-4 py-3 font-medium">{site.host}</td>
                    <td className="px-4 py-3 font-mono text-xs">{upstreamForSite(site)}</td>
                    <td className="px-4 py-3 text-text-secondary">
                      {(site.routes ?? []).map((r) => routeLabel(r)).join(', ') || '—'}
                    </td>
                    <td className={cn('px-4 py-3', ssl.tone)}>
                      {tls ? sslLabelForCard(tls.hosts) : '—'}
                      <span className="ml-2 text-xs">({ssl.label})</span>
                    </td>
                    <td className="actions-cell px-4 py-3">
                      <div className="icon-actions">
                        <button
                          type="button"
                          onClick={() => openEditSite(globalIndex)}
                          className="icon-action"
                          title="Edit site"
                          aria-label="Edit site"
                        >
                          <Pencil size={16} />
                        </button>
                        <button
                          type="button"
                          onClick={() => setDeleteIndex(globalIndex)}
                          className="icon-action danger"
                          title="Remove site"
                          aria-label="Remove site"
                        >
                          <Trash2 size={16} />
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      <Pagination totalItems={sites.length} pageSize={pageSize} page={page} onPageChange={setPage} />

      <Modal open={siteModal} onClose={() => setSiteModal(false)} title={editingIndex !== null ? 'Edit site' : 'Add site'} wide>
        <form onSubmit={submitSite} className="space-y-5">
          <div className={sectionCls}>
            <h3 className={sectionTitleCls}>Basics</h3>
            <div className="grid gap-4 sm:grid-cols-2">
              <label className={labelCls}>
                Domain
                <input className={inputCls} value={formHost} onChange={(e) => setFormHost(e.target.value)} placeholder="example.com" required />
              </label>
              <label className={labelCls}>
                Upstream
                <input
                  className={inputCls}
                  value={formUpstream}
                  onChange={(e) => setFormUpstream(e.target.value)}
                  placeholder="http://localhost:8080 or 127.0.0.1:3000"
                  required
                />
              </label>
            </div>
          </div>

          <div className={sectionCls}>
            <h3 className={cn(sectionTitleCls, 'flex items-center gap-2')}>
              <Lock size={14} /> SSL / TLS
            </h3>
            <div className="grid gap-2 sm:grid-cols-3">
              {([
                { mode: 'none' as const, icon: Lock, title: 'None', text: 'Plain HTTP only' },
                { mode: 'from_list' as const, icon: Award, title: 'Existing cert', text: 'Reuse a configured certificate' },
                { mode: 'generate' as const, icon: Zap, title: 'Generate SSL', text: "ACME / Let's Encrypt" },
              ]).map(({ mode, icon: Icon, title, text }) => (
                <label
                  key={mode}
                  className={cn(
                    'flex cursor-pointer gap-3 rounded-md border p-3 transition-colors',
                    formSslMode === mode ? 'border-primary bg-primary/5' : 'border-border hover:bg-hover/50',
                  )}
                >
                  <input type="radio" name="sslMode" checked={formSslMode === mode} onChange={() => setFormSslMode(mode)} className="mt-1" />
                  <span>
                    <span className="flex items-center gap-1.5 text-sm font-medium">
                      <Icon size={14} /> {title}
                    </span>
                    <span className="text-xs text-text-secondary">{text}</span>
                  </span>
                </label>
              ))}
            </div>
            {formSslMode === 'generate' ? (
              <p className="text-xs text-text-secondary">Certificate is issued automatically when you save (no restart).</p>
            ) : null}
            {formSslMode === 'from_list' ? (
              <label className={labelCls}>
                Certificate
                <select className={inputCls} value={formTlsIndex} onChange={(e) => setFormTlsIndex(Number(e.target.value))}>
                  {tlsList.map((t, i) => (
                    <option key={i} value={i}>{sslLabelForDropdown(t.hosts)}</option>
                  ))}
                  {tlsList.length === 0 ? <option value={0}>No certificates configured</option> : null}
                </select>
              </label>
            ) : null}
            {formSslMode === 'generate' ? (
              <div className="space-y-3">
                <label className={labelCls}>
                  Contact email (Let&apos;s Encrypt)
                  <input type="email" className={inputCls} value={formAcmeEmail} onChange={(e) => setFormAcmeEmail(e.target.value)} placeholder="you@yourdomain.com" required />
                </label>
                <label className={labelCls}>
                  Challenge type
                  <select className={inputCls} value={formAcmeChallenge} onChange={(e) => setFormAcmeChallenge(e.target.value as 'http01' | 'dns01')}>
                    <option value="http01">HTTP-01</option>
                    <option value="dns01">DNS-01</option>
                  </select>
                </label>
                {formAcmeChallenge === 'dns01' ? (
                  <label className={labelCls}>
                    DNS provider
                    <select className={inputCls} value={formDnsProviderId} onChange={(e) => setFormDnsProviderId(e.target.value)}>
                      <option value="">Please select</option>
                      {dnsProviders.map((p) => (
                        <option key={p.id} value={p.id}>{p.name}</option>
                      ))}
                      {dnsProviders.length === 0 ? <option value="">No DNS providers (add one in DNS Providers)</option> : null}
                    </select>
                  </label>
                ) : null}
                <label className="flex items-center gap-2 text-sm">
                  <input type="checkbox" checked={formWildcard} onChange={(e) => setFormWildcard(e.target.checked)} />
                  Wildcard certificate ({formHost.trim() ? hostToWildcard(formHost.trim()) : '*.domain'})
                </label>
              </div>
            ) : null}
          </div>

          <div className={sectionCls}>
            <div className="flex items-center justify-between">
              <h3 className={sectionTitleCls}>Routes</h3>
              <button type="button" onClick={addRoute} className="inline-flex items-center gap-1 rounded-md border border-border px-3 py-1.5 text-xs hover:bg-hover">
                <Plus size={12} /> Add route
              </button>
            </div>
            <div className="space-y-2">
              {formRoutes.map((r, i) => (
                <div key={i} className="flex flex-wrap items-center gap-2">
                  <select
                    className="rounded-md border border-border bg-bg px-2 py-2 text-sm"
                    value={r.path_type ?? 'Prefix'}
                    onChange={(e) => updateRoute(i, 'path_type', e.target.value)}
                  >
                    {PATH_TYPES.map((t) => (
                      <option key={t} value={t}>{t}</option>
                    ))}
                  </select>
                  <input
                    className="min-w-0 flex-1 rounded-md border border-border bg-bg px-3 py-2 text-sm"
                    value={r.path}
                    onChange={(e) => updateRoute(i, 'path', e.target.value)}
                    placeholder="/api"
                  />
                  <input
                    className="min-w-0 flex-1 rounded-md border border-border bg-bg px-3 py-2 text-sm"
                    value={r.rewrite ?? ''}
                    onChange={(e) => updateRoute(i, 'rewrite', e.target.value)}
                    placeholder="rewrite (optional)"
                  />
                  <button type="button" onClick={() => removeRoute(i)} className="rounded-md border border-border p-2 text-red-r1 hover:bg-hover" title="Remove route">
                    <X size={14} />
                  </button>
                </div>
              ))}
            </div>
          </div>

          {siteError ? <p className="text-sm text-red-r1">{siteError}</p> : null}
          <div className="flex justify-end gap-2 pt-2">
            <button type="button" onClick={() => setSiteModal(false)} className="rounded-md border border-border px-4 py-2 text-sm">Cancel</button>
            <button type="submit" disabled={siteSaving} className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-bg disabled:opacity-50">
              {siteSaving ? 'Saving…' : editingIndex !== null ? 'Update' : 'Add site'}
            </button>
          </div>
        </form>
      </Modal>

      <ConfirmDialog
        open={deleteIndex !== null}
        title="Remove site?"
        message={deleteIndex !== null ? `Remove "${sites[deleteIndex]?.host}"?` : ''}
        primaryLabel="Remove"
        variant="danger"
        loading={deleting}
        onConfirm={confirmDeleteSite}
        onCancel={() => setDeleteIndex(null)}
      />
    </div>
  );
}
