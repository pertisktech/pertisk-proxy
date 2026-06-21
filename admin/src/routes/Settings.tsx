import { useEffect, useState } from 'react';
import { api } from '@/api/client';
import { Card } from '@/components/Card';

export function Settings() {
  const [version, setVersion] = useState('');
  const [authRequired, setAuthRequired] = useState(false);

  useEffect(() => {
    api.version().then((v) => setVersion(v.version));
    api.authConfig().then((c) => setAuthRequired(c.auth_required));
  }, []);

  return (
    <div className="max-w-2xl space-y-4">
      <Card>
        <h2 className="mb-3 text-lg font-semibold">About</h2>
        <p className="text-text-secondary">Pertisk-Proxy management UI</p>
        <p className="mt-2 font-mono text-sm">Version {version || '…'}</p>
      </Card>
      <Card>
        <h2 className="mb-3 text-lg font-semibold">Security</h2>
        <p className="text-sm text-text-secondary">
          Management API auth: {authRequired ? 'enabled (username/password)' : 'disabled'}
        </p>
        <p className="mt-2 text-sm text-muted">
          Default credentials on first start: <code className="rounded bg-bg px-1">admin</code> /{' '}
          <code className="rounded bg-bg px-1">admin</code> (stored in SQLite). Change the password
          after first login.
        </p>
      </Card>
      <Card>
        <h2 className="mb-3 text-lg font-semibold">Environment</h2>
        <ul className="space-y-1 text-sm font-mono text-text-secondary">
          <li>PERTISK_DB_PATH (default ./data/proxy.sqlite)</li>
          <li>PERTISK_MANAGEMENT_ADDR (default 127.0.0.1:9080)</li>
          <li>ROUTES_CONFIG (optional one-time migration from legacy yaml)</li>
          <li>PERTISK_ADMIN_UI_DEV_ORIGIN (Vite dev redirect)</li>
        </ul>
      </Card>
    </div>
  );
}
