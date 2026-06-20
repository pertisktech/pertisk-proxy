import { useEffect, useState, FormEvent } from 'react';
import { toast } from 'sonner';
import { Plus, Pencil, Trash2 } from 'lucide-react';
import { api, type ProxyConfig, type Site, type Backend, type TlsConfig } from '@/api/client';
import { Card } from '@/components/Card';

const emptyBackend = (): Backend => ({
  name: '',
  upstreams: [{ addr: 'http://127.0.0.1:8080' }],
});

const emptySite = (): Site => ({
  host: '',
  backend: '',
  routes: [{ path: '/', path_type: 'Prefix' }],
});

export function Sites() {
  const [config, setConfig] = useState<ProxyConfig>({ sites: [], backends: [], tls: [] });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [siteForm, setSiteForm] = useState<Site | null>(null);
  const [backendForm, setBackendForm] = useState<Backend | null>(null);

  function load() {
    setLoading(true);
    api
      .config()
      .then((c) => setConfig({ backends: c.backends || [], sites: c.sites || [], tls: c.tls || [] }))
      .catch((e) => toast.error(e.message))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    load();
  }, []);

  async function save() {
    setSaving(true);
    try {
      const res = await api.saveConfig(config);
      toast.success(`Saved — ${res.route_count} routes active`);
      load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Save failed');
    } finally {
      setSaving(false);
    }
  }

  function submitSite(e: FormEvent) {
    e.preventDefault();
    if (!siteForm) return;
    const host = siteForm.host.trim();
    const backend = siteForm.backend.trim();
    if (!host || !backend) {
      toast.error('Host and backend are required');
      return;
    }
    const site = { ...siteForm, host, backend };
    setConfig((c) => ({ ...c, sites: [...c.sites.filter((s) => s.host !== host), site] }));
    setSiteForm(null);
  }

  function submitBackend(e: FormEvent) {
    e.preventDefault();
    if (!backendForm) return;
    const name = backendForm.name.trim();
    const addr = backendForm.upstreams[0]?.addr.trim();
    if (!name || !addr) {
      toast.error('Backend name and upstream are required');
      return;
    }
    const backend = { ...backendForm, name, upstreams: [{ addr, weight: 1 }] };
    setConfig((c) => ({ ...c, backends: [...c.backends.filter((b) => b.name !== name), backend] }));
    setBackendForm(null);
  }

  function addTls() {
    setConfig((c) => ({
      ...c,
      tls: [
        ...c.tls,
        {
          hosts: [],
          source: { type: 'acme', challenge: 'http01', email: '' },
        } as TlsConfig,
      ],
    }));
  }

  if (loading) return <p className="text-text-secondary">Loading config from database…</p>;

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold">Sites & backends</h2>
          <p className="text-sm text-text-secondary">
            Config is stored in SQLite — add sites, backends, and TLS here (like pertisk-rproxy).
          </p>
        </div>
        <button
          type="button"
          onClick={save}
          disabled={saving}
          className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-bg hover:opacity-90 disabled:opacity-50"
        >
          {saving ? 'Saving…' : 'Save & apply'}
        </button>
      </div>

      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="font-semibold">Backends</h3>
          <button type="button" onClick={() => setBackendForm(emptyBackend())} className="flex items-center gap-1 text-sm text-primary">
            <Plus size={14} /> Add backend
          </button>
        </div>
        {backendForm ? (
          <Card>
            <form onSubmit={submitBackend} className="grid gap-3 sm:grid-cols-2">
              <label className="text-sm"><span className="text-text-secondary">Name</span>
                <input className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2" value={backendForm.name} onChange={(e) => setBackendForm({ ...backendForm, name: e.target.value })} />
              </label>
              <label className="text-sm"><span className="text-text-secondary">Upstream</span>
                <input className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 font-mono text-sm" value={backendForm.upstreams[0]?.addr || ''} onChange={(e) => setBackendForm({ ...backendForm, upstreams: [{ addr: e.target.value }] })} />
              </label>
              <div className="flex gap-2 sm:col-span-2">
                <button type="submit" className="rounded-md bg-primary px-3 py-2 text-sm text-bg">OK</button>
                <button type="button" onClick={() => setBackendForm(null)} className="rounded-md border border-border px-3 py-2 text-sm">Cancel</button>
              </div>
            </form>
          </Card>
        ) : null}
        {config.backends.map((b) => (
          <Card key={b.name}>
            <div className="flex justify-between gap-3">
              <div>
                <h4 className="font-semibold">{b.name}</h4>
                <p className="font-mono text-sm text-text-secondary">{b.upstreams[0]?.addr}</p>
              </div>
              <div className="flex gap-2">
                <button type="button" onClick={() => setBackendForm(b)} className="rounded-md border border-border p-2"><Pencil size={16} /></button>
                <button type="button" onClick={() => setConfig((c) => ({ ...c, backends: c.backends.filter((x) => x.name !== b.name) }))} className="rounded-md border border-border p-2 text-red-r1"><Trash2 size={16} /></button>
              </div>
            </div>
          </Card>
        ))}
      </section>

      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="font-semibold">Sites</h3>
          <button type="button" onClick={() => setSiteForm(emptySite())} className="flex items-center gap-1 text-sm text-primary">
            <Plus size={14} /> Add site
          </button>
        </div>
        {siteForm ? (
          <Card>
            <form onSubmit={submitSite} className="grid gap-3 sm:grid-cols-2">
              <label className="text-sm"><span className="text-text-secondary">Host</span>
                <input className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2" value={siteForm.host} onChange={(e) => setSiteForm({ ...siteForm, host: e.target.value })} />
              </label>
              <label className="text-sm"><span className="text-text-secondary">Backend</span>
                <select className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2" value={siteForm.backend} onChange={(e) => setSiteForm({ ...siteForm, backend: e.target.value })}>
                  <option value="">Select backend</option>
                  {config.backends.map((b) => <option key={b.name} value={b.name}>{b.name}</option>)}
                </select>
              </label>
              <label className="text-sm sm:col-span-2"><span className="text-text-secondary">Path</span>
                <input className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2" value={siteForm.routes[0]?.path || '/'} onChange={(e) => setSiteForm({ ...siteForm, routes: [{ path: e.target.value, path_type: 'Prefix' }] })} />
              </label>
              <div className="flex gap-2 sm:col-span-2">
                <button type="submit" className="rounded-md bg-primary px-3 py-2 text-sm text-bg">OK</button>
                <button type="button" onClick={() => setSiteForm(null)} className="rounded-md border border-border px-3 py-2 text-sm">Cancel</button>
              </div>
            </form>
          </Card>
        ) : null}
        {config.sites.map((site) => (
          <Card key={site.host}>
            <div className="flex justify-between gap-3">
              <div>
                <h4 className="font-semibold">{site.host}</h4>
                <p className="text-sm text-text-secondary">backend: {site.backend} · {site.routes[0]?.path_type || 'Prefix'} {site.routes[0]?.path || '/'}</p>
              </div>
              <div className="flex gap-2">
                <button type="button" onClick={() => setSiteForm(site)} className="rounded-md border border-border p-2"><Pencil size={16} /></button>
                <button type="button" onClick={() => setConfig((c) => ({ ...c, sites: c.sites.filter((s) => s.host !== site.host) }))} className="rounded-md border border-border p-2 text-red-r1"><Trash2 size={16} /></button>
              </div>
            </div>
          </Card>
        ))}
      </section>

      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="font-semibold">TLS (ACME auto-SSL)</h3>
          <button type="button" onClick={addTls} className="text-sm text-primary">+ Add TLS</button>
        </div>
        {config.tls.map((tls, i) => (
          <Card key={i}>
            <div className="space-y-2 text-sm">
              <input
                className="w-full rounded-md border border-border bg-bg px-3 py-2 font-mono"
                placeholder="hosts, comma-separated"
                value={tls.hosts.join(', ')}
                onChange={(e) => {
                  const hosts = e.target.value.split(/[\s,]+/).map((h) => h.trim()).filter(Boolean);
                  setConfig((c) => {
                    const next = [...c.tls];
                    next[i] = { ...next[i], hosts };
                    return { ...c, tls: next };
                  });
                }}
              />
              <div className="flex flex-wrap gap-2">
                <select
                  className="rounded-md border border-border bg-bg px-2 py-1"
                  value={tls.source.type}
                  onChange={(e) => {
                    const type = e.target.value;
                    setConfig((c) => {
                      const next = [...c.tls];
                      next[i] = {
                        ...next[i],
                        source: type === 'file' ? { type: 'file', cert: '', key: '' } : { type: 'acme', challenge: 'http01', email: '' },
                      };
                      return { ...c, tls: next };
                    });
                  }}
                >
                  <option value="acme">ACME (Let's Encrypt)</option>
                  <option value="file">File paths</option>
                </select>
                {tls.source.type === 'acme' ? (
                  <>
                    <select
                      className="rounded-md border border-border bg-bg px-2 py-1"
                      value={tls.source.challenge || 'http01'}
                      onChange={(e) => setConfig((c) => {
                        const next = [...c.tls];
                        if (next[i].source.type === 'acme') next[i] = { ...next[i], source: { ...next[i].source, challenge: e.target.value } };
                        return { ...c, tls: next };
                      })}
                    >
                      <option value="http01">HTTP-01</option>
                      <option value="dns01">DNS-01</option>
                    </select>
                    <input
                      className="rounded-md border border-border bg-bg px-2 py-1"
                      placeholder="ACME email (required)"
                      value={tls.source.email || ''}
                      onChange={(e) => setConfig((c) => {
                        const next = [...c.tls];
                        if (next[i].source.type === 'acme') next[i] = { ...next[i], source: { ...next[i].source, email: e.target.value } };
                        return { ...c, tls: next };
                      })}
                    />
                    {tls.source.challenge === 'dns01' ? (
                      <input
                        className="rounded-md border border-border bg-bg px-2 py-1"
                        placeholder="DNS provider ID (from DNS Providers page)"
                        value={tls.source.dns_provider || ''}
                        onChange={(e) => setConfig((c) => {
                          const next = [...c.tls];
                          if (next[i].source.type === 'acme') next[i] = { ...next[i], source: { ...next[i].source, dns_provider: e.target.value } };
                          return { ...c, tls: next };
                        })}
                      />
                    ) : null}
                  </>
                ) : null}
                <button type="button" className="text-red-r1" onClick={() => setConfig((c) => ({ ...c, tls: c.tls.filter((_, j) => j !== i) }))}>Remove</button>
              </div>
              {tls.expires_at ? <p className="text-text-secondary">expires {new Date(tls.expires_at).toLocaleString()}</p> : null}
            </div>
          </Card>
        ))}
      </section>
    </div>
  );
}
