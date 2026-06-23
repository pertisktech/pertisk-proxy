import { useState } from 'react';
import { Download, Upload, Archive } from 'lucide-react';
import { toast } from 'sonner';
import { Card } from '@/components/Card';
import { useMode } from '@/context/ModeContext';
import { useManagementInfo } from '@/context/ManagementContext';
import { api } from '@/api/client';

export function Backup() {
  const mode = useMode();
  const management = useManagementInfo();
  const isIngress = mode === 'ingress';
  const fileExtension = isIngress ? '.yaml' : '.json';
  const fileType = isIngress ? 'YAML' : 'JSON';
  const helmEnabled = management?.helm_enabled === true;

  const [exporting, setExporting] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [mergeMode, setMergeMode] = useState(false);
  const [namespace, setNamespace] = useState('');

  async function handleExport() {
    setExporting(true);
    try {
      await api.backup.export(isIngress && namespace.trim() ? namespace.trim() : undefined);
      toast.success('Backup exported');
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Export failed');
    } finally {
      setExporting(false);
    }
  }

  async function handleRestore() {
    if (!selectedFile) return;
    setRestoring(true);
    try {
      const data = await selectedFile.text();
      const result = await api.backup.restore(data, mergeMode);
      toast.success(result.message);
      if (result.note) toast.info(result.note);
      if (result.errors?.length) {
        toast.warning(`${result.errors.length} error(s) during restore`);
      }
      setSelectedFile(null);
      setMergeMode(false);
      setTimeout(() => window.location.reload(), 2000);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Restore failed');
    } finally {
      setRestoring(false);
    }
  }

  return (
    <div className="mx-auto max-w-3xl space-y-4">
      <p className="text-sm text-text-secondary">
        {isIngress
          ? 'Export and restore Kubernetes Ingresses, TLS Secrets, Gateway API resources, and Helm release values.'
          : 'Export and restore site configuration, TLS certificates, and DNS provider metadata from the proxy database.'}
      </p>

      <Card>
        <h2 className="mb-2 flex items-center gap-2 text-lg font-semibold">
          <Download size={18} />
          Export backup
        </h2>
        <p className="mb-4 text-sm text-text-secondary">
          Download a {fileType} backup file.
          {!isIngress && ' DNS provider credentials are not included for security.'}
        </p>
        {isIngress && (
          <label className="mb-4 block text-sm">
            <span className="text-text-secondary">Namespace filter (optional)</span>
            <input
              type="text"
              value={namespace}
              onChange={(e) => setNamespace(e.target.value)}
              placeholder="All namespaces"
              className="mt-1 w-full max-w-xs rounded-md border border-border bg-bg px-3 py-2 text-sm"
            />
          </label>
        )}
        <button
          type="button"
          onClick={handleExport}
          disabled={exporting}
          className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-white hover:opacity-90 disabled:opacity-50"
        >
          {exporting ? 'Exporting…' : `Export ${isIngress ? 'ingress' : 'proxy'} backup`}
        </button>
      </Card>

      <Card>
        <h2 className="mb-2 flex items-center gap-2 text-lg font-semibold">
          <Upload size={18} />
          Restore backup
        </h2>
        <p className="mb-4 text-sm text-text-secondary">
          Upload a previously exported {fileType} file.
          {!isIngress && ' DNS providers must be re-added manually with credentials.'}
        </p>

        {!isIngress && (
          <div className="mb-4 rounded-md border border-yellow-y1/40 bg-yellow-y1/10 px-3 py-2 text-sm text-yellow-y1">
            DNS provider credentials are not stored in backups. Re-add providers after restore.
          </div>
        )}

        <input
          type="file"
          id="backup-file"
          accept={fileExtension}
          className="hidden"
          onChange={(e) => setSelectedFile(e.target.files?.[0] ?? null)}
        />
        <label
          htmlFor="backup-file"
          className="inline-flex cursor-pointer items-center gap-2 rounded-md border border-dashed border-border px-4 py-2 text-sm hover:border-primary"
        >
          <Archive size={16} />
          Choose {fileType} file
        </label>
        {selectedFile && (
          <p className="mt-2 text-sm text-text-secondary">
            Selected: <code className="text-primary">{selectedFile.name}</code>
          </p>
        )}

        <label className="mt-4 flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={mergeMode}
            onChange={(e) => setMergeMode(e.target.checked)}
          />
          Merge with existing data (add from backup; skip duplicates unless merge updates ingress)
        </label>

        <button
          type="button"
          onClick={handleRestore}
          disabled={!selectedFile || restoring}
          className="mt-4 rounded-md bg-primary px-4 py-2 text-sm font-medium text-white hover:opacity-90 disabled:opacity-50"
        >
          {restoring ? 'Restoring…' : 'Restore backup'}
        </button>
      </Card>

      <Card>
        <h2 className="mb-2 text-lg font-semibold">What is included</h2>
        <ul className="list-disc space-y-1 pl-5 text-sm text-text-secondary">
          {isIngress ? (
            <>
              <li>Kubernetes Ingress resources and TLS Secrets</li>
              {management?.gateway_api_enabled && (
                <>
                  <li>Gateway API Gateways and HTTPRoutes</li>
                </>
              )}
              {helmEnabled && (
                <li>Helm release values and revision history (when PERTISK_HELM_* is configured)</li>
              )}
              <li>YAML format compatible with kubectl apply</li>
              <li>Merge mode updates existing resources; without merge, existing resources are skipped</li>
            </>
          ) : (
            <>
              <li>Sites, backends, routing rules, and TLS configuration</li>
              <li>Certificate PEM files and private keys</li>
              <li>DNS provider names and types (not credentials)</li>
              <li>JSON format for easy inspection</li>
            </>
          )}
        </ul>
      </Card>
    </div>
  );
}
