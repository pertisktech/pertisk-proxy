import type { TlsConfig, TlsSource } from '@/api/client';

export type SiteSslMode = 'none' | 'from_list' | 'generate';

type TlsSourceAcme = Extract<TlsSource, { type: 'acme' }>;

function normalizeHost(host: string): string {
  return host.trim().toLowerCase();
}

/** Wildcard SAN for a site host (e.g. app.example.com → *.example.com). */
export function hostToWildcard(host: string): string {
  const trimmed = host.trim();
  if (!trimmed) return '*.domain';
  if (trimmed.startsWith('*.')) return trimmed;
  const parts = trimmed.split('.');
  if (parts.length >= 2) return '*.' + parts.slice(1).join('.');
  return '*.' + trimmed;
}

function relatedHostsForSite(siteHost: string): Set<string> {
  const normalized = normalizeHost(siteHost);
  const related = new Set<string>();
  if (!normalized) return related;
  related.add(normalized);
  if (normalized.startsWith('*.')) {
    related.add(normalized.slice(2));
  } else {
    related.add(normalizeHost(hostToWildcard(siteHost)));
  }
  return related;
}

function tlsEntryIdentityKey(entry: TlsConfig): string {
  const hosts = [...(entry.hosts ?? [])].map(normalizeHost).filter(Boolean).sort();
  const sourceType = entry.source?.type ?? '';
  return `${sourceType}\0${hosts.join('\0')}`;
}

export function isDedicatedAcmeTlsForSite(siteHost: string, tls: TlsConfig): boolean {
  if (tls.source?.type !== 'acme') return false;
  const related = relatedHostsForSite(siteHost);
  const hosts = (tls.hosts ?? []).map((h) => normalizeHost(h)).filter(Boolean);
  if (!hosts.length) return false;
  return hosts.every((h) => related.has(h));
}

export function inferSslModeForSite(host: string, tlsList: TlsConfig[]): SiteSslMode {
  const tls = resolveTlsForHost(host, tlsList);
  if (!tls) return 'none';
  if (tls.source?.type === 'acme') {
    return isDedicatedAcmeTlsForSite(host, tls) ? 'generate' : 'from_list';
  }
  return 'from_list';
}

export function acmeChallengeFromSource(source: TlsSourceAcme): 'http01' | 'dns01' {
  const challenge = (source.challenge ?? '').toLowerCase();
  return challenge === 'dns01' || challenge === 'dns-01' ? 'dns01' : 'http01';
}

export function siteUsesWildcardInTls(siteHost: string, tls: TlsConfig): boolean {
  const wildcardHost = hostToWildcard(siteHost);
  return (tls.hosts ?? []).some((h) => normalizeHost(h) === normalizeHost(wildcardHost));
}

/** Find the TLS config entry that covers a site host (exact match, then most specific wildcard). */
export function resolveTlsForHost(host: string, tlsList: TlsConfig[]): TlsConfig | null {
  const normalizedHost = normalizeHost(host);
  if (!normalizedHost) return null;

  const exactCandidates = tlsList.filter((entry) =>
    (entry.hosts ?? []).some((h) => normalizeHost(h) === normalizedHost),
  );
  if (exactCandidates.length > 0) {
    exactCandidates.sort((a, b) => {
      const aHosts = a.hosts ?? [];
      const bHosts = b.hosts ?? [];
      const aHasWildcard = aHosts.some((h) => h.trim().startsWith('*'));
      const bHasWildcard = bHosts.some((h) => h.trim().startsWith('*'));
      if (aHasWildcard !== bHasWildcard) return aHasWildcard ? 1 : -1;
      return aHosts.length - bHosts.length;
    });
    return exactCandidates[0] ?? null;
  }

  let best: { entry: TlsConfig; score: number } | null = null;
  for (const entry of tlsList) {
    for (const rawHost of entry.hosts ?? []) {
      const h = normalizeHost(rawHost);
      if (h === '*') {
        if (!best || best.score < 1) best = { entry, score: 1 };
        continue;
      }
      if (!h.startsWith('*.')) continue;
      const suffix = h.slice(1);
      if (!normalizedHost.endsWith(suffix) || normalizedHost.length <= suffix.length) continue;
      const score = suffix.length;
      if (!best || score > best.score) best = { entry, score };
    }
  }
  return best?.entry ?? null;
}

export function tlsIndexForHost(host: string, tlsList: TlsConfig[]): number {
  const entry = resolveTlsForHost(host, tlsList);
  if (!entry) return -1;
  const direct = tlsList.indexOf(entry);
  if (direct >= 0) return direct;
  const key = tlsEntryIdentityKey(entry);
  return tlsList.findIndex((t) => tlsEntryIdentityKey(t) === key);
}

function sslLabelForDropdown(hosts: string[] | undefined): string {
  if (!hosts?.length) return '—';
  const uniq = [...new Set(hosts.map((h) => h?.trim()).filter(Boolean))];
  if (!uniq.length) return '—';
  const wildcard = uniq.find((h) => h!.startsWith('*'));
  return wildcard || uniq[0] || '—';
}

export function sslLabelForCard(hosts: string[] | undefined): string {
  return sslLabelForDropdown(hosts);
}

export { sslLabelForDropdown };

function hostsSet(arr: string[] | undefined): Set<string> {
  return new Set((arr ?? []).map((h) => h.trim()).filter(Boolean));
}

/** Match backend `cert_row_covers_tls_hosts` — every TLS host covered exactly or by wildcard. */
export function certRowCoversTls(certHosts: string[] | undefined, tlsHosts: string[] | undefined): boolean {
  const want = (tlsHosts ?? []).map((h) => h.trim()).filter(Boolean);
  if (want.length === 0) return false;
  return want.every((h) => hostCoveredByCertHosts(certHosts, h));
}

/** True when `host` is listed in cert hosts or covered by a wildcard entry. */
export function hostCoveredByCertHosts(certHosts: string[] | undefined, host: string): boolean {
  const h = host.trim();
  if (!h) return false;
  const have = hostsSet(certHosts);
  if (h.startsWith('*')) return have.has(h);
  if (have.has(h)) return true;
  return [...have].some((w) => wildcardCoversHost(w, h));
}

/** True when `*.example.com` covers `app.example.com` (single DNS label). */
export function wildcardCoversHost(wildcard: string, host: string): boolean {
  const w = wildcard.trim();
  const h = host.trim().toLowerCase();
  if (!w || !h) return false;
  if (!w.startsWith('*')) return w.toLowerCase() === h;
  const suffix = w.slice(1);
  if (!h.endsWith(suffix) || h.length <= suffix.length) return false;
  const prefix = h.slice(0, h.length - suffix.length);
  return prefix.length > 0 && !prefix.includes('.');
}

/** Match backend `cert_row_matches_tls_config`. */
export function certRowMatchesTlsConfig(certHosts: string[] | undefined, tlsHosts: string[] | undefined): boolean {
  return certRowCoversTls(certHosts, tlsHosts);
}
