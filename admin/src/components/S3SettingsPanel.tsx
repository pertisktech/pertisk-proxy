import { FormEvent, useEffect, useState } from 'react';
import { Cloud, Loader2 } from 'lucide-react';
import { toast } from 'sonner';
import {
  api,
  type S3Settings,
  type UpdateS3SettingsBody,
} from '@/api/client';
import { Card } from '@/components/Card';
import { Checkbox } from '@/components/Checkbox';

type FormState = {
  enabled: boolean;
  endpoint: string;
  region: string;
  bucket: string;
  prefix: string;
  access_key_id: string;
  secret_access_key: string;
  force_path_style: boolean;
};

function toForm(settings: S3Settings): FormState {
  return {
    enabled: settings.enabled,
    endpoint: settings.endpoint,
    region: settings.region,
    bucket: settings.bucket,
    prefix: settings.prefix,
    access_key_id: settings.access_key_id,
    secret_access_key: '',
    force_path_style: settings.force_path_style,
  };
}

function toPayload(form: FormState, includeSecret: boolean): UpdateS3SettingsBody {
  const body: UpdateS3SettingsBody = {
    enabled: form.enabled,
    endpoint: form.endpoint.trim(),
    region: form.region.trim(),
    bucket: form.bucket.trim(),
    prefix: form.prefix.trim(),
    access_key_id: form.access_key_id.trim(),
    force_path_style: form.force_path_style,
  };
  if (includeSecret) {
    body.secret_access_key = form.secret_access_key;
  }
  return body;
}

export function S3SettingsPanel() {
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [unavailable, setUnavailable] = useState(false);
  const [form, setForm] = useState<FormState | null>(null);
  const [settings, setSettings] = useState<S3Settings | null>(null);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    api.backup.s3
      .get()
      .then((data) => {
        if (cancelled) return;
        setSettings(data);
        setForm(toForm(data));
        setUnavailable(false);
        setError(null);
      })
      .catch((err: Error) => {
        if (cancelled) return;
        const msg = err.message || 'Failed to load S3 settings';
        if (/database not configured|503|Service Unavailable/i.test(msg)) {
          setUnavailable(true);
        } else {
          setError(msg);
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function onSave(e: FormEvent) {
    e.preventDefault();
    if (!form) return;
    setSaving(true);
    setError(null);
    try {
      const next = await api.backup.s3.update(
        toPayload(form, Boolean(form.secret_access_key)),
      );
      setSettings(next);
      setForm(toForm(next));
      toast.success('S3 settings saved');
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to save';
      setError(msg);
      toast.error(msg);
    } finally {
      setSaving(false);
    }
  }

  async function onTest() {
    if (!form) return;
    setTesting(true);
    setError(null);
    try {
      const next = await api.backup.s3.update(
        toPayload(form, Boolean(form.secret_access_key)),
      );
      setSettings(next);
      setForm(toForm(next));
      await api.backup.s3.test();
      toast.success('S3 connection OK');
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'S3 connection test failed';
      setError(msg);
      toast.error(msg);
    } finally {
      setTesting(false);
    }
  }

  if (unavailable) {
    return null;
  }

  return (
    <Card>
      <div className="mb-1 flex items-center gap-2">
        <Cloud size={18} className="text-text-secondary" />
        <h2 className="text-lg font-semibold">S3 storage</h2>
      </div>
      <p className="mb-4 text-sm text-text-secondary">
        Configure an S3 or S3-compatible bucket for uploading management backups (AWS, MinIO, R2,
        etc.).
      </p>

      {loading ? (
        <div className="flex items-center gap-2 text-sm text-text-secondary">
          <Loader2 size={16} className="animate-spin" />
          Loading S3 settings…
        </div>
      ) : null}

      {error ? <p className="mb-3 text-sm text-red-r1">{error}</p> : null}

      {form ? (
        <form className="space-y-4" onSubmit={onSave}>
          {!form.enabled ? (
            <div className="alert-warning rounded-lg p-3 text-sm" role="status">
              S3 backup is disabled. Enable it and save before uploading backups from the Backup
              page.
            </div>
          ) : null}

          <Checkbox
            label="Enable S3 backup"
            checked={form.enabled}
            onChange={(checked) => setForm({ ...form, enabled: checked })}
          />

          <div className="grid gap-3 sm:grid-cols-2">
            <label className="block text-sm text-text-secondary sm:col-span-2">
              Endpoint URL (optional)
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={form.endpoint}
                onChange={(e) => setForm({ ...form, endpoint: e.target.value })}
                placeholder="https://s3.amazonaws.com or https://minio.example.com"
                autoComplete="off"
              />
              <span className="mt-1 block text-xs text-muted">
                Leave empty for AWS. Set for MinIO, Cloudflare R2, or other S3-compatible APIs.
              </span>
            </label>
            <label className="block text-sm text-text-secondary">
              Region
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={form.region}
                onChange={(e) => setForm({ ...form, region: e.target.value })}
                placeholder="us-east-1"
                autoComplete="off"
              />
            </label>
            <label className="block text-sm text-text-secondary">
              Bucket
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={form.bucket}
                onChange={(e) => setForm({ ...form, bucket: e.target.value })}
                placeholder="pertisk-backups"
                autoComplete="off"
              />
            </label>
            <label className="block text-sm text-text-secondary sm:col-span-2">
              Object prefix (optional)
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={form.prefix}
                onChange={(e) => setForm({ ...form, prefix: e.target.value })}
                placeholder="proxy/"
                autoComplete="off"
              />
            </label>
            <label className="block text-sm text-text-secondary">
              Access key ID
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={form.access_key_id}
                onChange={(e) => setForm({ ...form, access_key_id: e.target.value })}
                autoComplete="off"
              />
            </label>
            <label className="block text-sm text-text-secondary">
              Secret access key
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                type="password"
                value={form.secret_access_key}
                onChange={(e) => setForm({ ...form, secret_access_key: e.target.value })}
                placeholder={settings?.has_secret_access_key ? '•••••••• (unchanged)' : ''}
                autoComplete="new-password"
              />
            </label>
            <div className="sm:col-span-2">
              <Checkbox
                label="Force path-style addressing"
                checked={form.force_path_style}
                onChange={(checked) => setForm({ ...form, force_path_style: checked })}
              />
              <p className="mt-1 text-xs text-muted">
                Enable for most MinIO and self-hosted S3 APIs.
              </p>
            </div>
          </div>

          <div className="flex flex-wrap gap-2 pt-1">
            <button
              type="submit"
              disabled={saving}
              className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-bg hover:opacity-90 disabled:opacity-50"
            >
              {saving ? 'Saving…' : 'Save S3 settings'}
            </button>
            <button
              type="button"
              onClick={onTest}
              disabled={
                testing || !form.bucket.trim() || !form.access_key_id.trim()
              }
              className="rounded-md border border-border bg-surface-elevated px-4 py-2 text-sm font-medium text-text hover:bg-bg disabled:opacity-50"
            >
              {testing ? 'Testing…' : 'Test connection'}
            </button>
          </div>
        </form>
      ) : null}
    </Card>
  );
}
