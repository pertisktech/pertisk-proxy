import { FormEvent, useEffect, useState } from 'react';
import { Loader2, ScrollText } from 'lucide-react';
import { toast } from 'sonner';
import {
  api,
  type LoggingSettings,
  type UpdateLoggingSettingsBody,
} from '@/api/client';
import { Card } from '@/components/Card';
import { Checkbox } from '@/components/Checkbox';

type FormState = {
  enabled: boolean;
  storage: 'memory' | 'scylla';
  ttl_days: number;
};

function toForm(settings: LoggingSettings): FormState {
  return {
    enabled: settings.enabled,
    storage: settings.storage === 'scylla' ? 'scylla' : 'memory',
    ttl_days: settings.ttl_days > 0 ? settings.ttl_days : 7,
  };
}

function toPayload(form: FormState): UpdateLoggingSettingsBody {
  return {
    enabled: form.enabled,
    storage: form.storage,
    ttl_days: Math.max(1, Math.min(365, Math.floor(form.ttl_days) || 7)),
  };
}

export function LoggingSettingsPanel() {
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [unavailable, setUnavailable] = useState(false);
  const [form, setForm] = useState<FormState | null>(null);
  const [settings, setSettings] = useState<LoggingSettings | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    api.logging.settings
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
        const msg = err.message || 'Failed to load logging settings';
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
      const next = await api.logging.settings.update(toPayload(form));
      setSettings(next);
      setForm(toForm(next));
      toast.success('Logging settings saved');
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to save';
      setError(msg);
      toast.error(msg);
    } finally {
      setSaving(false);
    }
  }

  if (unavailable) {
    return null;
  }

  const scyllaAvailable = Boolean(settings?.scylla_available);

  return (
    <Card>
      <div className="mb-1 flex items-center gap-2">
        <ScrollText size={18} className="text-text-secondary" />
        <h2 className="text-lg font-semibold">Access logging</h2>
      </div>
      <p className="mb-4 text-sm text-text-secondary">
        Persist proxied request access logs in memory (ring buffer) or ScyllaDB for longer retention.
        Scylla writes are async and never block the request path.
      </p>

      {loading ? (
        <div className="flex items-center gap-2 text-sm text-text-secondary">
          <Loader2 size={16} className="animate-spin" />
          Loading logging settings…
        </div>
      ) : null}

      {error ? <p className="mb-3 text-sm text-red-r1">{error}</p> : null}

      {form ? (
        <form className="space-y-4" onSubmit={onSave}>
          <Checkbox
            label="Enable access logging"
            checked={form.enabled}
            onChange={(checked) => setForm({ ...form, enabled: checked })}
          />

          <label className="block text-sm text-text-secondary">
            Storage
            <select
              className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text disabled:opacity-50"
              value={form.storage}
              onChange={(e) =>
                setForm({
                  ...form,
                  storage: e.target.value === 'scylla' ? 'scylla' : 'memory',
                })
              }
            >
              <option value="memory">Memory (ring buffer)</option>
              <option value="scylla" disabled={!scyllaAvailable}>
                ScyllaDB{scyllaAvailable ? '' : ' (unavailable)'}
              </option>
            </select>
            {!scyllaAvailable ? (
              <span className="mt-1 block text-xs text-muted">
                ScyllaDB is not available. Set <code className="font-mono">PERTISK_SCYLLA_HOSTS</code>{' '}
                (and use a binary built with <code className="font-mono">scylla-db</code>) to enable
                warehouse storage.
              </span>
            ) : (
              <span className="mt-1 block text-xs text-muted">
                Target:{' '}
                <span className="font-mono text-text">
                  {settings?.scylla_hosts || 'configured hosts'}
                </span>
                {settings?.scylla_keyspace
                  ? ` / keyspace ${settings.scylla_keyspace}`
                  : null}
              </span>
            )}
          </label>

          <label className="block text-sm text-text-secondary">
            Retention (TTL days)
            <input
              type="number"
              min={1}
              max={365}
              className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
              value={form.ttl_days}
              onChange={(e) =>
                setForm({ ...form, ttl_days: Number(e.target.value) || 7 })
              }
              disabled={form.storage !== 'scylla'}
            />
            <span className="mt-1 block text-xs text-muted">
              Applies to ScyllaDB <code className="font-mono">access_logs</code> table TTL. Default
              7 days.
            </span>
          </label>

          <div className="pt-1">
            <button
              type="submit"
              disabled={saving}
              className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-bg hover:opacity-90 disabled:opacity-50"
            >
              {saving ? 'Saving…' : 'Save logging settings'}
            </button>
          </div>
        </form>
      ) : null}
    </Card>
  );
}
