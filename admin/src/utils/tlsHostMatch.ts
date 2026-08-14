import type { TlsConfig, TlsSource } from '@/api/client';

export type SiteSslMode = 'none' | 'from_list' | 'generate';

type TlsSourceAcme = Extract<TlsSource, { type: 'acme' }>;

function normalizeHost(host: string): string {
  return host.trim().toLowerCase();
}

/**
 * Wildcard SAN for a site host.
 * - `admin.example.com` → `*.example.com` (parent zone)
 * - `example.com` (apex) → `*.example.com` (not `*.com`, which LE rejects)
 * - already `*.x` → unchanged
 */
export function hostToWildcard(host: string): string {
  const trimmed = host.trim();
  if (!trimmed) return '*.domain';
  if (trimmed.startsWith('*.')) return trimmed;
  const parts = trimmed.split('.').filter(Boolean);
  if (parts.length >= 3) return `*.${parts.slice(1).join('.')}`;
  if (parts.length === 2) return `*.${trimmed}`;
  return `*.${trimmed}`;
}

/** True when ACME would reject this identifier (e.g. `*.com`, bare TLD). */
export function isInvalidAcmeIdentifier(host: string): boolean {
  const h = normalizeHost(host);
  if (!h) return true;
  if (h.startsWith('*.')) {
    const suffix = h.slice(2);
    return suffix.split('.').filter(Boolean).length < 2;
  }
  return h.split('.').filter(Boolean).length < 2;
}

/**
 * Hostnames tied to a site for TLS attach/detach (exact + wildcard forms).
 * Includes the legacy parent-slice form so apex mistakes like `*.com` are cleaned on edit.
 */
export function relatedHostsForSite(siteHost: string): Set<string> {
  const normalized = normalizeHost(siteHost);
  const related = new Set<string>();
  if (!normalized) return related;
  related.add(normalized);
  if (normalized.startsWith('*.')) {
    related.add(normalized.slice(2));
  } else {
    related.add(normalizeHost(hostToWildcard(siteHost)));
    const parts = normalized.split('.').filter(Boolean);
    if (parts.length >= 2) {
      related.add(normalizeHost(`*.${parts.slice(1).join('.')}`));
    }
  }
  return related;
}

/** Drop a site's related hostnames from TLS entries; remove empty entries. */
export function removeRelatedHostsFromTls(list: TlsConfig[], siteHost: string): TlsConfig[] {
  const related = relatedHostsForSite(siteHost);
  if (related.size === 0) return list;
  return list
    .map((t) => ({
      ...t,
      hosts: (t.hosts ?? []).filter((h) => !related.has(normalizeHost(h))),
    }))
    .filter((t) => (t.hosts ?? []).length > 0);
}

/** Site hostnames covered by `*.zone` (single DNS label). */
export function siteHostsCoveredByWildcard(wildcard: string, siteHosts: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of siteHosts) {
    const h = raw.trim();
    if (!h || seen.has(normalizeHost(h))) continue;
    if (wildcardCoversHost(wildcard, h)) {
      seen.add(normalizeHost(h));
      out.push(h);
    }
  }
  return out;
}

/**
 * Point every covered site at one wildcard TLS entry and drop leftover per-domain ACME rows.
 * SSL labels then show `*.proxmox.example.com`, not a sibling leaf like `13900hx.proxmox.example.com`.
 */
export function attachSitesToWildcardTls(
  list: TlsConfig[],
  wildcard: string,
  coveredSiteHosts: string[],
  acmeSource: TlsSource,
): TlsConfig[] {
  const wild = normalizeHost(wildcard);
  if (!wild.startsWith('*.') || isInvalidAcmeIdentifier(wildcard)) return list;
  const covered = new Set(coveredSiteHosts.map((h) => normalizeHost(h)).filter(Boolean));

  const stripped = list
    .map((t) => ({
      ...t,
      hosts: (t.hosts ?? []).filter((h) => {
        const n = normalizeHost(h);
        if (!n) return false;
        if (n === wild) return false;
        if (covered.has(n)) return false;
        return true;
      }),
    }))
    .filter((t) => (t.hosts ?? []).length > 0);

  const existingIdx = stripped.findIndex(
    (t) =>
      t.source?.type === 'acme' &&
      (t.hosts ?? []).some((h) => normalizeHost(h) === wild),
  );
  const wildcardEntry: TlsConfig = { hosts: [wildcard.trim()], source: acmeSource };
  if (existingIdx >= 0) {
    return stripped.map((t, i) => (i === existingIdx ? { ...t, ...wildcardEntry } : t));
  }
  return [...stripped, wildcardEntry];
}

function tlsEntryIdentityKey(entry: TlsConfig): string {
  const hosts = [...(entry.hosts ?? [])].map(normalizeHost).filter(Boolean).sort();
  const sourceType = entry.source?.type ?? '';
  return `${sourceType}\0${hosts.join('\0')}`;
}

/** True when this TLS row only names this site (exact and/or its own wildcard). */
export function isDedicatedTlsForSite(siteHost: string, tls: TlsConfig): boolean {
  const related = relatedHostsForSite(siteHost);
  const hosts = (tls.hosts ?? []).map((h) => normalizeHost(h)).filter(Boolean);
  if (!hosts.length) return false;
  return hosts.every((h) => related.has(h));
}

export function isDedicatedAcmeTlsForSite(siteHost: string, tls: TlsConfig): boolean {
  if (tls.source?.type !== 'acme') return false;
  return isDedicatedTlsForSite(siteHost, tls);
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

function coveringWildcardScore(entry: TlsConfig, host: string): number {
  let best = 0;
  for (const rawHost of entry.hosts ?? []) {
    const h = normalizeHost(rawHost);
    if (h === '*') {
      best = Math.max(best, 1);
      continue;
    }
    if (!wildcardCoversHost(h, host)) continue;
    best = Math.max(best, h.length);
  }
  return best;
}

/**
 * TLS for a site: dedicated per-host cert, else covering wildcard (domain TLS),
 * else a shared exact-host row. Shared leaf rows must not beat `*.zone`.
 */
export function resolveTlsForHost(host: string, tlsList: TlsConfig[]): TlsConfig | null {
  const normalizedHost = normalizeHost(host);
  if (!normalizedHost) return null;

  let bestWildcard: { entry: TlsConfig; score: number } | null = null;
  for (const entry of tlsList) {
    const score = coveringWildcardScore(entry, normalizedHost);
    if (score > 0 && (!bestWildcard || score > bestWildcard.score)) {
      bestWildcard = { entry, score };
    }
  }

  const exactCandidates = tlsList.filter((entry) =>
    (entry.hosts ?? []).some((h) => normalizeHost(h) === normalizedHost),
  );
  const dedicated = exactCandidates.filter((entry) => isDedicatedTlsForSite(host, entry));
  if (dedicated.length > 0) {
    dedicated.sort((a, b) => (a.hosts ?? []).length - (b.hosts ?? []).length);
    return dedicated[0] ?? null;
  }
  if (bestWildcard) return bestWildcard.entry;
  if (exactCandidates.length > 0) {
    exactCandidates.sort((a, b) => (a.hosts ?? []).length - (b.hosts ?? []).length);
    return exactCandidates[0] ?? null;
  }
  return null;
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
