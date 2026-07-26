import { useEffect, useState } from 'react';
import { Download, Upload, Archive, Cloud } from 'lucide-react';
import { Link } from 'react-router-dom';
import { toast } from 'sonner';
import { Card } from '@/components/Card';
import { Checkbox } from '@/components/Checkbox';
import { useMode } from '@/context/ModeContext';
import { useManagementInfo } from '@/context/ManagementContext';
import { api, type S3Settings } from '@/api/client';

type ExportDestination = 'download' | 's3';

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
  const [destination, setDestination] = useState<ExportDestination>('download');
  const [s3, setS3] = useState<S3Settings | null>(null);
  const [s3Loading, setS3Loading] = useState(!isIngress);

  useEffect(() => {
    if (isIngress) {
      setS3Loading(false);
      return;
    }
    let cancelled = false;
    setS3Loading(true);
    api.backup.s3
      .get()
      .then((data) => {
        if (!cancelled) setS3(data);
      })
      .catch(() => {
        if (!cancelled) setS3(null);
      })
      .finally(() => {
        if (!cancelled) setS3Loading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [isIngress]);

  const s3Ready =
    Boolean(s3?.enabled) &&
    Boolean(s3?.bucket?.trim()) &&
    Boolean(s3?.access_key_id?.trim()) &&
    Boolean(s3?.has_secret_access_key);

  async function handleExport() {
    setExporting(true);
    try {
      if (destination === 's3') {
        const result = await api.backup.exportToS3(
          isIngress && namespace.trim() ? { namespace: namespace.trim() } : undefined,
        );
        toast.success(`Backup uploaded to s3://${result.bucket}/${result.key}`);
      } else {
        await api.backup.export(isIngress && namespace.trim() ? namespace.trim() : undefined);
        toast.success('Backup exported');
      }
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
          {destination === 's3'
            ? `Upload a ${fileType} backup to the configured S3 bucket.`
            : `Download a ${fileType} backup file.`}
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

        {!isIngress ? (
          <div className="mb-4 space-y-2">
            <p className="text-sm font-medium">Destination</p>
            <div className="flex flex-wrap gap-2">
              <button
                type="button"
                onClick={() => setDestination('download')}
                className={`rounded-md border px-3 py-1.5 text-sm ${
                  destination === 'download'
                    ? 'border-primary bg-primary text-bg'
                    : 'border-border bg-surface-elevated text-text-secondary hover:text-text'
                }`}
              >
                Download file
              </button>
              <button
                type="button"
                onClick={() => setDestination('s3')}
                className={`inline-flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-sm ${
                  destination === 's3'
                    ? 'border-primary bg-primary text-bg'
                    : 'border-border bg-surface-elevated text-text-secondary hover:text-text'
                }`}
              >
                <Cloud size={14} />
                Upload to S3
              </button>
            </div>
            {destination === 's3' ? (
              s3Loading ? (
                <p className="text-xs text-muted">Checking S3 settings…</p>
              ) : s3Ready ? (
                <p className="text-xs text-text-secondary">
                  Bucket <span className="font-mono text-text">{s3?.bucket}</span>
                  {s3?.prefix?.trim() ? (
                    <>
                      {' '}
                      · prefix <span className="font-mono text-text">{s3.prefix}</span>
                    </>
                  ) : null}
                </p>
              ) : (
                <p className="text-xs text-yellow-y1">
                  Configure and enable S3 under{' '}
                  <Link to="/settings#storage" className="underline underline-offset-2">
                    Settings → Storage
                  </Link>{' '}
                  first.
                </p>
              )
            ) : null}
          </div>
        ) : null}

        <button
          type="button"
          onClick={handleExport}
          disabled={exporting || (destination === 's3' && !s3Ready)}
          className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-white hover:opacity-90 disabled:opacity-50"
        >
          {exporting
            ? destination === 's3'
              ? 'Uploading…'
              : 'Exporting…'
            : destination === 's3'
              ? 'Upload backup to S3'
              : `Export ${isIngress ? 'ingress' : 'proxy'} backup`}
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
          className="mb-3 inline-flex cursor-pointer items-center gap-2 rounded-md border border-border px-4 py-2 text-sm hover:bg-hover"
        >
          <Archive size={16} />
          {selectedFile ? selectedFile.name : `Choose ${fileType} file`}
        </label>

        {!isIngress && (
          <div className="mb-4">
            <Checkbox
              label="Merge with existing config"
              checked={mergeMode}
              onChange={setMergeMode}
            />
            <p className="mt-1 text-xs text-muted">
              When enabled, existing sites and certificates are kept; only new hosts are added.
            </p>
          </div>
        )}

        {isIngress && helmEnabled ? (
          <p className="mb-4 text-xs text-muted">
            Helm values in the backup are exported for reference only and are not applied on restore.
          </p>
        ) : null}

        <button
          type="button"
          onClick={handleRestore}
          disabled={!selectedFile || restoring}
          className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-white hover:opacity-90 disabled:opacity-50"
        >
          {restoring ? 'Restoring…' : 'Restore backup'}
        </button>
      </Card>
    </div>
  );
}
