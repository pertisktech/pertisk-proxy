import { Fragment, useCallback, useEffect, useMemo, useRef, useState, FormEvent, DragEvent, ClipboardEvent } from 'react';
import { toast } from 'sonner';
import {
  Award,
  ChevronDown,
  ChevronUp,
  FileUp,
  LayoutGrid,
  List,
  Loader2,
  Shield,
  Trash2,
  Upload,
} from 'lucide-react';
import { api, type CertificateRow, type ProxyConfig, type TlsConfig, type TlsSource } from '@/api/client';
import { Modal } from '@/components/Modal';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { Pagination } from '@/components/Pagination';
import { usePageSize } from '@/utils/usePageSize';
import { formatDate, formatDateOnly } from '@/utils/dateFormat';
import { resolveTlsForHost } from '@/utils/tlsHostMatch';
import { cn } from '@/utils';

type ViewMode = 'card' | 'list';
type SortKey = 'domain' | 'issuer' | 'challenge' | 'expires' | 'sites';

function isAcme(source: TlsSource): source is Extract<TlsSource, { type: 'acme' }> {
  return source.type === 'acme';
}

function isFile(source: TlsSource): source is Extract<TlsSource, { type: 'file' }> {
  return source.type === 'file';
}

function issuerLabel(tls: TlsConfig): string {
  if (!tls.source) return '—';
  if (isAcme(tls.source)) return "Let's Encrypt";
  if (isFile(tls.source)) return 'File';
  return '—';
}

function hostsForDisplay(hosts: string[] | undefined): string {
  if (!hosts?.length) return '—';
  const uniq = [...new Set(hosts.map((h) => h?.trim()).filter(Boolean))];
  if (!uniq.length) return '—';
  const wildcards = uniq.filter((h) => h!.startsWith('*'));
  if (wildcards.length > 0) return wildcards.join(', ');
  return uniq.join(', ');
}

function domainLabelForTls(tls: TlsConfig): string {
  return hostsForDisplay(tls.hosts);
}

function challengeLabel(tls: TlsConfig): string {
  if (!tls.source || !isAcme(tls.source)) return '—';
  const c = (tls.source.challenge ?? 'http01').toLowerCase();
  return c === 'dns01' || c === 'dns-01' ? 'DNS-01' : 'HTTP-01';
}

function formatNextRenew(expiresAt: string | undefined): string {
  if (!expiresAt) return '—';
  const d = new Date(expiresAt);
  if (Number.isNaN(d.getTime())) return '—';
  const renew = new Date(d);
  renew.setDate(renew.getDate() - 30);
  if (renew <= new Date()) return 'Soon (within 30 days)';
  return '~' + formatDateOnly(renew.toISOString());
}

function parseHosts(input: string): string[] {
  return input.split(/[\s,]+/).map((h) => h.trim()).filter(Boolean);
}

function looksLikeCertPem(s: string): boolean {
  const t = s.trim();
  return t.includes('-----BEGIN') && t.includes('CERTIFICATE');
}

function looksLikeKeyPem(s: string): boolean {
  const t = s.trim();
  return t.includes('-----BEGIN') && t.includes('PRIVATE KEY');
}

function parsePemContent(content: string): { certPem: string; keyPem: string } {
  const certs: string[] = [];
  let keyPem = '';
  const blockRegex = /-----BEGIN ([^-]+)-----[\s\S]*?-----END \s*\1\s*-----/gi;
  let match: RegExpExecArray | null;
  while ((match = blockRegex.exec(content)) !== null) {
    const type = match[1].trim().toUpperCase();
    if (type.includes('CERTIFICATE')) certs.push(match[0]);
    else if (type.includes('PRIVATE') && type.includes('KEY') && !keyPem) keyPem = match[0];
  }
  return { certPem: certs.join('\n'), keyPem };
}

function hostsMatch(a: string[] | undefined, b: string[] | undefined): boolean {
  const set = (arr: string[] | undefined) => new Set((arr ?? []).map((h) => h.trim()).filter(Boolean));
  const sa = set(a);
  const sb = set(b);
  return sa.size === sb.size && [...sa].every((h) => sb.has(h));
}

function certMatchesTls(certHosts: string[] | undefined, tlsHosts: string[] | undefined): boolean {
  const certSet = new Set((certHosts ?? []).map((h) => h.trim()).filter(Boolean));
  const tlsSet = new Set((tlsHosts ?? []).map((h) => h.trim()).filter(Boolean));
  if (certSet.size === 0) return false;
  return [...certSet].every((h) => tlsSet.has(h));
}

function tlsItemId(tls: TlsConfig, index: number): string {
  const hosts = (tls.hosts ?? []).slice().map((h) => h.trim()).filter(Boolean).sort().join(',');
  return `${index}:${tls.source?.type ?? 'unknown'}:${hosts}`;
}

const CERT_KEY_EXTENSIONS = ['.pem', '.crt', '.cer', '.key'];
function isCertOrKeyFile(file: File): boolean {
  const name = file.name.toLowerCase();
  return CERT_KEY_EXTENSIONS.some((ext) => name.endsWith(ext)) || !name.includes('.');
}

export function Certificates() {
  const [config, setConfig] = useState<ProxyConfig | null>(null);
  const [certRows, setCertRows] = useState<CertificateRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>('list');
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const pageSize = usePageSize();
  const [page, setPage] = useState(1);
  const [sortKey, setSortKey] = useState<SortKey | null>(null);
  const [sortDir, setSortDir] = useState<'asc' | 'desc'>('asc');

  const [uploadOpen, setUploadOpen] = useState(false);
  const [uploadHosts, setUploadHosts] = useState('');
  const [uploadCertPem, setUploadCertPem] = useState('');
  const [uploadKeyPem, setUploadKeyPem] = useState('');
  const [uploadError, setUploadError] = useState('');
  const [uploading, setUploading] = useState(false);
  const [dropZoneActive, setDropZoneActive] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const [deleteTls, setDeleteTls] = useState<TlsConfig | null>(null);
  const [deleting, setDeleting] = useState(false);

  const tlsList = config?.tls ?? [];
  const sites = config?.sites ?? [];

  const refreshAll = useCallback(async () => {
    const [nextConfig, nextCertRows] = await Promise.all([
      api.config(),
      api.certificates.list().catch(() => [] as CertificateRow[]),
    ]);
    setConfig(nextConfig);
    setCertRows(nextCertRows);
  }, []);

  useEffect(() => {
    refreshAll().catch((e) => setError(e instanceof Error ? e.message : 'Failed to load'));
  }, [refreshAll]);

  useEffect(() => {
    let cancelled = false;
    const tick = () => {
      if (document.visibilityState !== 'visible') return;
      refreshAll().catch(() => {});
    };
    const t = setInterval(() => {
      if (!cancelled) tick();
    }, 5000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, [refreshAll]);

  function getCertIdForTls(tls: TlsConfig, rows = certRows): string | null {
    const want = tls.hosts ?? [];
    const exact = rows.find((r) => hostsMatch(r.hosts, want));
    if (exact) return exact.id;
    const subset = rows.find((r) => certMatchesTls(r.hosts, want));
    return subset?.id ?? null;
  }

  function getSitesUsingCert(tls: TlsConfig): number {
    return sites.filter((site) => resolveTlsForHost(site.host, tlsList) === tls).length;
  }

  function getSiteDomainsUsingCert(tls: TlsConfig): string[] {
    return sites
      .filter((site) => resolveTlsForHost(site.host, tlsList) === tls)
      .map((site) => site.host)
      .sort((a, b) => a.localeCompare(b));
  }

  function toggleSort(key: SortKey) {
    setPage(1);
    if (sortKey === key) setSortDir((d) => (d === 'asc' ? 'desc' : 'asc'));
    else {
      setSortKey(key);
      setSortDir('asc');
    }
  }

  const tlsItems = useMemo(
    () => tlsList.map((tls, originalIndex) => ({ tls, originalIndex, id: tlsItemId(tls, originalIndex) })),
    [tlsList],
  );

  const sortedTlsItems = useMemo(() => {
    if (!sortKey) return tlsItems;
    const dir = sortDir === 'asc' ? 1 : -1;
    return [...tlsItems].sort((a, b) => {
      if (sortKey === 'domain') {
        return dir * domainLabelForTls(a.tls).localeCompare(domainLabelForTls(b.tls), undefined, { sensitivity: 'base' });
      }
      if (sortKey === 'issuer') return dir * issuerLabel(a.tls).localeCompare(issuerLabel(b.tls), undefined, { sensitivity: 'base' });
      if (sortKey === 'challenge') return dir * challengeLabel(a.tls).localeCompare(challengeLabel(b.tls), undefined, { sensitivity: 'base' });
      if (sortKey === 'expires') {
        const at = new Date(a.tls.expires_at ?? '').getTime();
        const bt = new Date(b.tls.expires_at ?? '').getTime();
        return dir * ((Number.isFinite(at) ? at : 0) - (Number.isFinite(bt) ? bt : 0));
      }
      return dir * (getSitesUsingCert(a.tls) - getSitesUsingCert(b.tls));
    });
  }, [tlsItems, sortKey, sortDir, sites, tlsList]);

  const totalPages = Math.max(1, Math.ceil(sortedTlsItems.length / pageSize));
  const pagedTlsItems = sortedTlsItems.slice((page - 1) * pageSize, page * pageSize);

  useEffect(() => {
    setPage((p) => Math.min(p, totalPages));
  }, [totalPages]);

  useEffect(() => {
    if (expandedId && !tlsItems.some((t) => t.id === expandedId)) setExpandedId(null);
  }, [tlsItems, expandedId]);

  function openUploadModal() {
    setUploadHosts('');
    setUploadCertPem('');
    setUploadKeyPem('');
    setUploadError('');
    setUploadOpen(true);
  }

  async function readFileAsText(file: File): Promise<string> {
    return new Promise((resolve, reject) => {
      const r = new FileReader();
      r.onload = () => resolve(String(r.result ?? ''));
      r.onerror = () => reject(new Error(`Failed to read ${file.name}`));
      r.readAsText(file, 'UTF-8');
    });
  }

  async function handleDroppedFiles(files: FileList | null) {
    if (!files?.length) return;
    const certOrKeyFiles = Array.from(files).filter(isCertOrKeyFile);
    if (!certOrKeyFiles.length) {
      setUploadError('Please drop .pem, .crt, .cer, or .key files.');
      return;
    }
    setUploadError('');
    try {
      let combined = '';
      for (const file of certOrKeyFiles) combined += (await readFileAsText(file)) + '\n';
      const { certPem, keyPem } = parsePemContent(combined);
      if (certPem) setUploadCertPem(certPem);
      if (keyPem) setUploadKeyPem(keyPem);
      if (!certPem && !keyPem) setUploadError('No certificate or private key PEM blocks found.');
    } catch (err) {
      setUploadError(err instanceof Error ? err.message : 'Failed to read files');
    }
  }

  async function handleUploadSubmit(e: FormEvent) {
    e.preventDefault();
    setUploadError('');
    const hosts = parseHosts(uploadHosts);
    if (!hosts.length) {
      setUploadError('Enter at least one host (comma- or space-separated).');
      return;
    }
    if (!looksLikeCertPem(uploadCertPem)) {
      setUploadError('Certificate must be PEM format.');
      return;
    }
    if (!looksLikeKeyPem(uploadKeyPem)) {
      setUploadError('Private key must be PEM format.');
      return;
    }
    setUploading(true);
    try {
      await api.certificates.upload({ hosts, cert_pem: uploadCertPem.trim(), key_pem: uploadKeyPem.trim() });
      await refreshAll();
      toast.success('Certificate uploaded');
      setUploadOpen(false);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Upload failed';
      setUploadError(msg);
      toast.error(msg);
    } finally {
      setUploading(false);
    }
  }

  async function confirmDelete() {
    if (!deleteTls) return;
    const sitesCount = getSitesUsingCert(deleteTls);
    if (sitesCount > 0) {
      toast.error(`${sitesCount} site(s) use this certificate. Change or remove the site first.`);
      setDeleteTls(null);
      return;
    }
    setDeleting(true);
    const id = getCertIdForTls(deleteTls);
    try {
      if (id) {
        await api.certificates.delete(id);
      } else if (config) {
        const newTls = config.tls.filter((t) => !hostsMatch(t.hosts, deleteTls.hosts));
        await api.saveConfig({ ...config, tls: newTls });
      }
      toast.success('Certificate removed');
      setExpandedId(null);
      setDeleteTls(null);
      await refreshAll();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Delete failed');
    } finally {
      setDeleting(false);
    }
  }

  const pendingAcme = tlsList.filter((tls) => {
    if (!tls.source || !isAcme(tls.source)) return false;
    return !getCertIdForTls(tls);
  });

  const inputCls = 'mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm';
  const labelCls = 'block text-sm text-text-secondary';

  if (error) {
    return (
      <div>
        <p className="text-red-r1">{error}</p>
      </div>
    );
  }

  const sortBtn = (key: SortKey, label: string) => (
    <button type="button" onClick={() => toggleSort(key)} className="inline-flex items-center gap-1 hover:text-text">
      {label}
      <span className="text-xs text-muted">{sortKey === key ? (sortDir === 'asc' ? '↑' : '↓') : '↕'}</span>
    </button>
  );

  function renderDetails(tls: TlsConfig) {
    return (
      <dl className="grid gap-2 text-sm sm:grid-cols-2">
        <div>
          <dt className="text-text-secondary">Hosts</dt>
          <dd className="font-medium">{hostsForDisplay(tls.hosts)}</dd>
        </div>
        {tls.source && isAcme(tls.source) ? (
          <>
            <div>
              <dt className="text-text-secondary">Challenge</dt>
              <dd>{challengeLabel(tls)}</dd>
            </div>
            <div>
              <dt className="text-text-secondary">Next renew</dt>
              <dd>{formatNextRenew(tls.expires_at)}</dd>
            </div>
            {tls.source.dns_provider ? (
              <div>
                <dt className="text-text-secondary">DNS provider</dt>
                <dd>{tls.source.dns_provider}</dd>
              </div>
            ) : null}
          </>
        ) : null}
        {tls.source && isFile(tls.source) ? (
          <>
            <div className="sm:col-span-2">
              <dt className="text-text-secondary">Cert path</dt>
              <dd className="font-mono text-xs break-all">{tls.source.cert}</dd>
            </div>
            <div className="sm:col-span-2">
              <dt className="text-text-secondary">Key path</dt>
              <dd className="font-mono text-xs break-all">{tls.source.key}</dd>
            </div>
          </>
        ) : null}
      </dl>
    );
  }

  return (
    <div className="space-y-4">
      {pendingAcme.length > 0 ? (
        <div className="rounded-lg border border-yellow-y1/30 bg-yellow-y1/10 p-4 text-sm" role="status">
          <p className="font-medium text-yellow-y1">ACME issuance in progress</p>
          <ul className="mt-2 space-y-1 text-text-secondary">
            {pendingAcme.map((tls, i) => (
              <li key={i}>
                {domainLabelForTls(tls)} — {challengeLabel(tls)}
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      <div className="flex flex-wrap items-center justify-between gap-3">
        <button type="button" onClick={openUploadModal} className="inline-flex items-center gap-2 rounded-md border border-border px-4 py-2 text-sm hover:bg-hover">
          <FileUp size={16} /> Import certificate
        </button>
        <div className="inline-flex rounded-md border border-border p-0.5">
          <button
            type="button"
            onClick={() => setViewMode('card')}
            className={cn('inline-flex items-center gap-1.5 rounded px-3 py-1.5 text-sm', viewMode === 'card' ? 'bg-hover font-medium text-primary' : 'text-text-secondary')}
          >
            <LayoutGrid size={14} /> Cards
          </button>
          <button
            type="button"
            onClick={() => setViewMode('list')}
            className={cn('inline-flex items-center gap-1.5 rounded px-3 py-1.5 text-sm', viewMode === 'list' ? 'bg-hover font-medium text-primary' : 'text-text-secondary')}
          >
            <List size={14} /> List
          </button>
        </div>
      </div>

      {tlsList.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border py-16 text-center">
          <Award className="mx-auto text-muted" size={40} />
          <h3 className="mt-3 font-semibold">No certificates configured</h3>
          <p className="mt-1 text-sm text-text-secondary">
            Add a site with Auto SSL in Sites, or import a certificate.
          </p>
          <button type="button" onClick={openUploadModal} className="mt-4 rounded-md bg-primary px-4 py-2 text-sm text-bg">
            Import certificate
          </button>
        </div>
      ) : viewMode === 'card' ? (
        <>
          <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
            {pagedTlsItems.map(({ tls, id }) => {
              const certId = getCertIdForTls(tls);
              const pending = tls.source && isAcme(tls.source) && !certId;
              return (
                <div key={id} className="rounded-lg border border-border bg-surface p-4">
                  <div className="flex items-start justify-between gap-2">
                    <h3 className="flex items-center gap-2 font-semibold">
                      <Award size={16} className="text-primary" />
                      {domainLabelForTls(tls)}
                    </h3>
                    {pending ? <span className="text-xs text-yellow-y1">Pending</span> : null}
                  </div>
                  <p className="mt-1 flex items-center gap-1 text-sm text-text-secondary">
                    <Shield size={14} /> {issuerLabel(tls)}
                  </p>
                  <div className="mt-3 space-y-1 text-sm">
                    <div className="flex justify-between">
                      <span className="text-text-secondary">Expires</span>
                      <span>{formatDate(tls.expires_at)}</span>
                    </div>
                    <div className="flex justify-between">
                      <span className="text-text-secondary">Sites</span>
                      <span>{getSitesUsingCert(tls)}</span>
                    </div>
                    <div className="flex justify-between">
                      <span className="text-text-secondary">Challenge</span>
                      <span>{challengeLabel(tls)}</span>
                    </div>
                  </div>
                  <div className="mt-4 flex gap-2">
                    <button type="button" onClick={() => setExpandedId((prev) => (prev === id ? null : id))} className="flex-1 rounded-md border border-border px-3 py-2 text-sm hover:bg-hover">
                      {expandedId === id ? <ChevronUp size={14} className="mr-1 inline" /> : <ChevronDown size={14} className="mr-1 inline" />}
                      Details
                    </button>
                    <button type="button" onClick={() => setDeleteTls(tls)} className="rounded-md border border-border px-3 py-2 text-sm text-red-r1 hover:bg-hover">
                      <Trash2 size={14} />
                    </button>
                  </div>
                  {expandedId === id ? <div className="mt-4 border-t border-border pt-4">{renderDetails(tls)}</div> : null}
                </div>
              );
            })}
          </div>
        </>
      ) : (
        <div className="overflow-x-auto rounded-lg border border-border">
          <table className="w-full min-w-[800px] text-left text-sm">
            <thead className="border-b border-border bg-surface-elevated text-text-secondary">
              <tr>
                <th className="px-4 py-3 font-medium">{sortBtn('domain', 'Domain')}</th>
                <th className="px-4 py-3 font-medium">{sortBtn('issuer', 'Issuer')}</th>
                <th className="px-4 py-3 font-medium">{sortBtn('challenge', 'Challenge')}</th>
                <th className="px-4 py-3 font-medium">{sortBtn('expires', 'Expires')}</th>
                <th className="px-4 py-3 font-medium">Next renew</th>
                <th className="px-4 py-3 font-medium">{sortBtn('sites', 'Sites')}</th>
                <th className="px-4 py-3 font-medium text-right">Actions</th>
              </tr>
            </thead>
            <tbody>
              {pagedTlsItems.map(({ tls, id }) => {
                const siteDomains = getSiteDomainsUsingCert(tls);
                const siteLabel =
                  siteDomains.length === 0 ? '—' : siteDomains.length === 1 ? siteDomains[0] : `${siteDomains[0]} +${siteDomains.length - 1}`;
                const certId = getCertIdForTls(tls);
                const pending = tls.source && isAcme(tls.source) && !certId;
                return (
                  <Fragment key={id}>
                    <tr className="border-b border-border hover:bg-hover/50">
                      <td className="px-4 py-3 font-medium">
                        {domainLabelForTls(tls)}
                        {pending ? <span className="ml-2 text-xs text-yellow-y1">Pending</span> : null}
                      </td>
                      <td className="px-4 py-3">{issuerLabel(tls)}</td>
                      <td className="px-4 py-3">
                        <span className="rounded-full bg-surface-elevated px-2 py-0.5 text-xs">{challengeLabel(tls)}</span>
                      </td>
                      <td className="px-4 py-3">{formatDate(tls.expires_at)}</td>
                      <td className="px-4 py-3">{tls.source?.type === 'acme' ? formatNextRenew(tls.expires_at) : '—'}</td>
                      <td className="px-4 py-3" title={siteDomains.join(', ')}>{siteLabel}</td>
                      <td className="px-4 py-3">
                        <div className="flex justify-end gap-2">
                          <button type="button" onClick={() => setExpandedId(expandedId === id ? null : id)} className="rounded-md border border-border px-3 py-1.5 text-xs hover:bg-hover">
                            Details
                          </button>
                          <button type="button" onClick={() => setDeleteTls(tls)} className="rounded-md border border-border px-3 py-1.5 text-xs text-red-r1 hover:bg-hover">
                            Delete
                          </button>
                        </div>
                      </td>
                    </tr>
                    {expandedId === id ? (
                      <tr className="border-b border-border bg-surface-elevated/50">
                        <td colSpan={7} className="px-4 py-4">{renderDetails(tls)}</td>
                      </tr>
                    ) : null}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      <Pagination totalItems={tlsList.length} pageSize={pageSize} page={page} onPageChange={setPage} />

      <Modal open={uploadOpen} onClose={() => setUploadOpen(false)} title="Import certificate" wide>
        <form onSubmit={handleUploadSubmit} className="space-y-4">
          {uploadError ? <p className="text-sm text-red-r1">{uploadError}</p> : null}
          <label className={labelCls}>
            Hosts <span className="text-xs text-muted">(comma- or space-separated)</span>
            <input className={inputCls} value={uploadHosts} onChange={(e) => setUploadHosts(e.target.value)} placeholder="example.com, *.example.com" disabled={uploading} />
          </label>
          <div
            className={cn(
              'cursor-pointer rounded-lg border-2 border-dashed p-8 text-center text-sm transition-colors',
              dropZoneActive ? 'border-primary bg-primary/5' : 'border-border hover:border-primary/50',
            )}
            onDragOver={(e: DragEvent) => { e.preventDefault(); setDropZoneActive(true); }}
            onDragLeave={(e: DragEvent) => { e.preventDefault(); setDropZoneActive(false); }}
            onDrop={(e: DragEvent) => { e.preventDefault(); setDropZoneActive(false); handleDroppedFiles(e.dataTransfer.files); }}
            onClick={() => fileInputRef.current?.click()}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => e.key === 'Enter' && fileInputRef.current?.click()}
          >
            <Upload className="mx-auto text-muted" size={24} />
            <p className="mt-2">Drag & drop .pem, .crt, .cer, or .key files, or click to browse</p>
            <p className="mt-1 text-xs text-muted">Single file with cert+key chain is parsed automatically</p>
            <input
              ref={fileInputRef}
              type="file"
              accept=".pem,.crt,.cer,.key"
              multiple
              className="hidden"
              onChange={(e) => { handleDroppedFiles(e.target.files); e.target.value = ''; }}
            />
          </div>
          <label className={labelCls}>
            Certificate (PEM)
            <textarea
              className={cn(inputCls, 'mono min-h-32')}
              value={uploadCertPem}
              onChange={(e) => setUploadCertPem(e.target.value)}
              onPaste={(e: ClipboardEvent<HTMLTextAreaElement>) => {
                const { certPem, keyPem } = parsePemContent(e.clipboardData.getData('text'));
                if (keyPem && !uploadKeyPem.trim()) {
                  setUploadKeyPem(keyPem);
                  setUploadCertPem(certPem || e.clipboardData.getData('text'));
                  e.preventDefault();
                }
              }}
              rows={6}
              spellCheck={false}
              disabled={uploading}
            />
          </label>
          <label className={labelCls}>
            Private key (PEM)
            <textarea
              className={cn(inputCls, 'mono min-h-24')}
              value={uploadKeyPem}
              onChange={(e) => setUploadKeyPem(e.target.value)}
              rows={5}
              spellCheck={false}
              disabled={uploading}
            />
          </label>
          <div className="flex justify-end gap-2 pt-2">
            <button type="button" onClick={() => setUploadOpen(false)} disabled={uploading} className="rounded-md border border-border px-4 py-2 text-sm">Cancel</button>
            <button type="submit" disabled={uploading} className="inline-flex items-center gap-2 rounded-md bg-primary px-4 py-2 text-sm font-medium text-bg disabled:opacity-50">
              {uploading ? <><Loader2 size={16} className="animate-spin" /> Uploading…</> : 'Upload'}
            </button>
          </div>
        </form>
      </Modal>

      <ConfirmDialog
        open={!!deleteTls}
        title="Delete certificate?"
        message={deleteTls ? `Delete certificate for "${domainLabelForTls(deleteTls)}"? This cannot be undone.` : ''}
        primaryLabel="Delete"
        variant="danger"
        loading={deleting}
        onConfirm={confirmDelete}
        onCancel={() => setDeleteTls(null)}
      />
    </div>
  );
}
