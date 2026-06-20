import { useEffect, useState, FormEvent } from 'react';
import { toast } from 'sonner';
import { Plus, Trash2, Upload } from 'lucide-react';
import { api, type CertificateRow } from '@/api/client';
import { Card } from '@/components/Card';

function parseHosts(input: string): string[] {
  return input.split(/[\s,]+/).map((h) => h.trim()).filter(Boolean);
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

export function Certificates() {
  const [rows, setRows] = useState<CertificateRow[]>([]);
  const [yamlEntries, setYamlEntries] = useState<{ hosts: string[]; cert: string; key: string }[]>([]);
  const [loading, setLoading] = useState(true);
  const [showUpload, setShowUpload] = useState(false);
  const [hostsInput, setHostsInput] = useState('');
  const [certPem, setCertPem] = useState('');
  const [keyPem, setKeyPem] = useState('');
  const [uploading, setUploading] = useState(false);

  function load() {
    setLoading(true);
    Promise.all([api.certificates.list(), api.tls()])
      .then(([certs, tls]) => {
        setRows(certs);
        setYamlEntries(tls.entries);
      })
      .catch((e) => toast.error(e.message))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    load();
  }, []);

  async function handleUpload(e: FormEvent) {
    e.preventDefault();
    const hosts = parseHosts(hostsInput);
    if (!hosts.length) {
      toast.error('Enter at least one host');
      return;
    }
    let cert = certPem.trim();
    let key = keyPem.trim();
    if ((!cert || !key) && certPem.includes('BEGIN')) {
      const parsed = parsePemContent(certPem);
      cert = parsed.certPem;
      key = parsed.keyPem || key;
    }
    if (!cert || !key) {
      toast.error('Certificate and private key PEM required');
      return;
    }
    setUploading(true);
    try {
      await api.certificates.upload({ hosts, cert_pem: cert, key_pem: key });
      toast.success('Certificate uploaded');
      setShowUpload(false);
      setHostsInput('');
      setCertPem('');
      setKeyPem('');
      load();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Upload failed');
    } finally {
      setUploading(false);
    }
  }

  async function remove(id: string) {
    if (!confirm('Delete this certificate?')) return;
    try {
      await api.certificates.delete(id);
      toast.success('Certificate deleted');
      load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Delete failed');
    }
  }

  function onFileDrop(file: File) {
    file.text().then((text) => {
      const parsed = parsePemContent(text);
      if (parsed.certPem) setCertPem(parsed.certPem);
      if (parsed.keyPem) setKeyPem(parsed.keyPem);
      else if (!parsed.keyPem && parsed.certPem) setCertPem(text);
    });
  }

  if (loading) return <p className="text-text-secondary">Loading certificates…</p>;

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold">Certificates</h2>
          <p className="text-sm text-text-secondary">{rows.length} certificate(s) in database</p>
        </div>
        <button type="button" onClick={() => setShowUpload(true)} className="flex items-center gap-2 rounded-md border border-border px-3 py-2 text-sm hover:bg-hover">
          <Plus size={16} /> Upload certificate
        </button>
      </div>

      {showUpload ? (
        <Card>
          <form onSubmit={handleUpload} className="space-y-4">
            <h3 className="font-semibold">Upload certificate</h3>
            <label className="block text-sm">
              <span className="text-text-secondary">Hosts (comma-separated)</span>
              <input className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2" value={hostsInput} onChange={(e) => setHostsInput(e.target.value)} placeholder="admin.example.com, *.example.com" />
            </label>
            <label className="block text-sm">
              <span className="text-text-secondary">Certificate PEM</span>
              <textarea className="mono mt-1 min-h-32 w-full rounded-md border border-border bg-bg p-3 text-xs" value={certPem} onChange={(e) => setCertPem(e.target.value)} />
            </label>
            <label className="block text-sm">
              <span className="text-text-secondary">Private key PEM</span>
              <textarea className="mono mt-1 min-h-24 w-full rounded-md border border-border bg-bg p-3 text-xs" value={keyPem} onChange={(e) => setKeyPem(e.target.value)} />
            </label>
            <label className="flex cursor-pointer items-center gap-2 text-sm text-primary">
              <Upload size={16} />
              <input type="file" accept=".pem,.crt,.key,.cer" className="hidden" onChange={(e) => { const f = e.target.files?.[0]; if (f) onFileDrop(f); }} />
              Load from file
            </label>
            <div className="flex gap-2">
              <button type="submit" disabled={uploading} className="rounded-md bg-primary px-4 py-2 text-sm text-bg disabled:opacity-50">{uploading ? 'Uploading…' : 'Upload'}</button>
              <button type="button" onClick={() => setShowUpload(false)} className="rounded-md border border-border px-4 py-2 text-sm">Cancel</button>
            </div>
          </form>
        </Card>
      ) : null}

      {rows.map((row) => (
        <Card key={row.id}>
          <div className="flex items-start justify-between gap-3">
            <div>
              <h3 className="font-semibold">{row.hosts.join(', ')}</h3>
              <p className="text-sm text-text-secondary">Source: {row.source_type}</p>
              {row.expires_at ? (
                <p className="text-sm text-text-secondary">Expires {new Date(row.expires_at).toLocaleString()}</p>
              ) : null}
            </div>
            <button type="button" onClick={() => remove(row.id)} className="rounded-md border border-border p-2 text-red-r1 hover:bg-hover"><Trash2 size={16} /></button>
          </div>
        </Card>
      ))}

      {yamlEntries.length > 0 ? (
        <>
          <h3 className="text-lg font-semibold">routes.yaml TLS (file paths)</h3>
          {yamlEntries.map((entry, i) => (
            <Card key={i}>
              <h3 className="font-semibold">{entry.hosts.join(', ')}</h3>
              <dl className="mt-3 space-y-2 text-sm">
                <div><dt className="text-text-secondary">Certificate</dt><dd className="font-mono break-all">{entry.cert}</dd></div>
                <div><dt className="text-text-secondary">Private key</dt><dd className="font-mono break-all">{entry.key}</dd></div>
              </dl>
            </Card>
          ))}
        </>
      ) : null}

      {rows.length === 0 && yamlEntries.length === 0 ? (
        <Card><p className="text-text-secondary">No certificates configured.</p></Card>
      ) : null}
    </div>
  );
}
