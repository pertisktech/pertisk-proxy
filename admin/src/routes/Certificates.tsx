import { useEffect, useState } from 'react';
import { api, type TlsEntry } from '@/api/client';
import { Card } from '@/components/Card';

export function Certificates() {
  const [entries, setEntries] = useState<TlsEntry[]>([]);
  const [hostCount, setHostCount] = useState(0);
  const [error, setError] = useState('');

  useEffect(() => {
    api.tls()
      .then((res) => {
        setEntries(res.entries);
        setHostCount(res.host_count);
      })
      .catch((e) => setError(e.message));
  }, []);

  if (error) return <p className="text-red-r1">{error}</p>;

  return (
    <div className="space-y-4">
      <p className="text-text-secondary">{hostCount} TLS host(s) loaded from routes.yaml</p>
      {entries.map((entry, i) => (
        <Card key={i}>
          <h3 className="font-semibold">{entry.hosts.join(', ')}</h3>
          <dl className="mt-3 space-y-2 text-sm">
            <div><dt className="text-text-secondary">Certificate</dt><dd className="font-mono break-all">{entry.cert}</dd></div>
            <div><dt className="text-text-secondary">Private key</dt><dd className="font-mono break-all">{entry.key}</dd></div>
          </dl>
        </Card>
      ))}
      {entries.length === 0 ? (
        <Card><p className="text-text-secondary">No file-based TLS entries in routes.yaml</p></Card>
      ) : null}
    </div>
  );
}
