import { FormEvent, useEffect, useMemo, useState } from 'react';
import { toast } from 'sonner';
import {
  api,
  type RuntimeEnvField,
  type RuntimeEnvSettings,
} from '@/api/client';
import { Card } from '@/components/Card';
import {
  EnvVar,
  IngressDeploymentGuide,
  ProxyDeploymentGuide,
} from '@/components/settings/shared';
import { cn } from '@/utils';

function sourceBadge(source: string) {
  switch (source) {
    case 'override':
      return { label: 'saved', className: 'bg-primary/15 text-primary' };
    case 'pending':
      return { label: 'pending restart', className: 'bg-yellow-y1/15 text-yellow-y1' };
    case 'env':
      return { label: 'process env', className: 'bg-blue-b1/15 text-blue-b1' };
    default:
      return { label: 'default', className: 'bg-surface-elevated text-text-secondary' };
  }
}

function EditableEnvironmentForm({ initial }: { initial: RuntimeEnvSettings }) {
  const [settings, setSettings] = useState(initial);
  const [draft, setDraft] = useState<Record<string, string>>(() => {
    const out: Record<string, string> = {};
    for (const f of initial.fields) {
      out[f.key] = f.override_value ?? '';
    }
    return out;
  });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const dirty = useMemo(() => {
    return settings.fields.some((f) => (draft[f.key] ?? '') !== (f.override_value ?? ''));
  }, [draft, settings.fields]);

  function setField(key: string, value: string) {
    setDraft((prev) => ({ ...prev, [key]: value }));
  }

  async function onSave(e: FormEvent) {
    e.preventDefault();
    setSaving(true);
    setError(null);
    try {
      const overrides: Record<string, string> = {};
      for (const [key, value] of Object.entries(draft)) {
        const trimmed = value.trim();
        if (trimmed) overrides[key] = trimmed;
      }
      const res = await api.runtimeEnv.save({ overrides });
      setSettings(res.settings);
      const next: Record<string, string> = {};
      for (const f of res.settings.fields) {
        next[f.key] = f.override_value ?? '';
      }
      setDraft(next);
      toast.success('Environment overrides saved. Restart pertisk-proxy to apply.');
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to save';
      setError(msg);
      toast.error(msg);
    } finally {
      setSaving(false);
    }
  }

  function clearAll() {
    const next: Record<string, string> = {};
    for (const f of settings.fields) next[f.key] = '';
    setDraft(next);
  }

  return (
    <Card>
      <h2 className="mb-1 text-lg font-semibold">Editable environment</h2>
      <p className="mb-4 text-sm text-text-secondary">
        Overrides are stored in SQLite and applied on the next process start. Process environment /
        systemd values always win when set. Leave a field empty to clear its override.
      </p>
      {error ? <p className="mb-3 text-sm text-red-r1">{error}</p> : null}
      <form onSubmit={onSave} className="space-y-4">
        <ul className="grid gap-3 md:grid-cols-2">
          {settings.fields.map((field) => (
            <EnvFieldInput
              key={field.key}
              field={field}
              value={draft[field.key] ?? ''}
              onChange={(v) => setField(field.key, v)}
            />
          ))}
        </ul>
        <div className="flex flex-wrap items-center gap-3">
          <button
            type="submit"
            disabled={saving || !dirty}
            className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-bg hover:opacity-90 disabled:opacity-50"
          >
            {saving ? 'Saving…' : 'Save overrides'}
          </button>
          <button
            type="button"
            disabled={saving}
            onClick={clearAll}
            className="rounded-md border border-border px-4 py-2 text-sm text-text-secondary hover:bg-surface-elevated disabled:opacity-50"
          >
            Clear all
          </button>
          {dirty ? (
            <span className="text-xs text-yellow-y1">Unsaved changes — restart required after save</span>
          ) : (
            <span className="text-xs text-text-secondary">
              {settings.overrides && Object.keys(settings.overrides).length > 0
                ? `${Object.keys(settings.overrides).length} override(s) stored`
                : 'No overrides stored'}
            </span>
          )}
        </div>
      </form>
    </Card>
  );
}

function EnvFieldInput({
  field,
  value,
  onChange,
}: {
  field: RuntimeEnvField;
  value: string;
  onChange: (value: string) => void;
}) {
  const badge = sourceBadge(field.source);
  const inputCls =
    'mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 font-mono text-sm text-text';

  return (
    <li className="rounded-lg border border-border bg-surface-elevated p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <label className="text-sm font-medium text-text" htmlFor={`env-${field.key}`}>
          {field.label}
        </label>
        <span className={cn('rounded-full px-2 py-0.5 text-xs', badge.className)}>{badge.label}</span>
      </div>
      <code className="mt-1 block text-xs text-primary">{field.key}</code>
      <p className="mt-1 text-xs text-text-secondary">{field.description}</p>
      {field.field_type === 'select' ? (
        <select
          id={`env-${field.key}`}
          className={inputCls}
          value={value}
          onChange={(e) => onChange(e.target.value)}
        >
          <option value="">(default / unset)</option>
          {field.options.map((opt) => (
            <option key={opt} value={opt}>
              {opt}
            </option>
          ))}
        </select>
      ) : (
        <input
          id={`env-${field.key}`}
          type={field.field_type === 'number' ? 'number' : 'text'}
          min={field.field_type === 'number' ? 0 : undefined}
          className={inputCls}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={field.effective_value ? `effective: ${field.effective_value}` : 'default'}
        />
      )}
      {field.effective_value && field.source !== 'pending' ? (
        <p className="mt-1 text-xs text-text-secondary">
          Effective now: <span className="font-mono text-text">{field.effective_value}</span>
        </p>
      ) : null}
    </li>
  );
}

function ReadOnlyReference({ isIngress }: { isIngress: boolean }) {
  return (
    <>
      <Card>
        <h2 className="mb-1 text-lg font-semibold">
          {isIngress ? 'Helm configuration reference' : 'Configuration reference'}
        </h2>
        <p className="mb-4 text-sm text-text-secondary">
          {isIngress
            ? 'Ingress mode is managed by Helm / process environment. Set these in your overlay, then redeploy.'
            : 'These keys can also be set in the service environment file; process env wins over UI overrides.'}
        </p>
        <ul className="grid gap-3 md:grid-cols-2">
          {isIngress ? (
            <>
              <EnvVar
                name="runtime.mode=performance"
                description="Helm value mapped to PERTISK_INGRESS_MODE."
              />
              <EnvVar
                name="runtime.listenerTasks"
                description="Mapped to PERTISK_PINGORA_LISTENER_TASKS."
              />
              <EnvVar
                name="runtime.tcpListenBacklog"
                description="Mapped to PERTISK_TCP_LISTEN_BACKLOG."
              />
              <EnvVar
                name="runtime.pingoraThreads"
                description="Optional override for PERTISK_PINGORA_THREADS."
              />
              <EnvVar
                name="upstream.poolMaxIdlePerHost"
                description="Mapped to PERTISK_UPSTREAM_POOL_MAX_IDLE_PER_HOST."
              />
              <EnvVar
                name="h3Upstream.poolMaxIdlePerHost"
                description="Mapped to PERTISK_H3_UPSTREAM_POOL_MAX_IDLE."
              />
              <EnvVar
                name="http3.maxStreams"
                description="Mapped to PERTISK_HTTP3_MAX_STREAMS."
              />
              <EnvVar
                name="resources.limits.cpu"
                description="Drives auto worker sizing via PERTISK_CPU_LIMIT_MILLICORES."
              />
            </>
          ) : (
            <>
              <EnvVar
                name="PERTISK_DB_PATH"
                description="SQLite database path (default ./data/proxy.sqlite)."
              />
              <EnvVar
                name="PERTISK_MANAGEMENT_ADDR"
                description="Admin API listen address (default 127.0.0.1:9080)."
              />
              <EnvVar
                name="ROUTES_CONFIG"
                description="Optional one-time migration from legacy YAML routes."
              />
              <EnvVar
                name="PERTISK_ADMIN_UI_DEV_ORIGIN"
                description="Vite dev-server origin for local admin UI redirects."
              />
            </>
          )}
        </ul>
      </Card>
      {isIngress ? <IngressDeploymentGuide /> : <ProxyDeploymentGuide />}
    </>
  );
}

export function EnvironmentPanel({ isIngress }: { isIngress: boolean }) {
  const [loading, setLoading] = useState(!isIngress);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [settings, setSettings] = useState<RuntimeEnvSettings | null>(null);

  useEffect(() => {
    if (isIngress) {
      setLoading(false);
      setSettings(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    api.runtimeEnv
      .get()
      .then((data) => {
        if (cancelled) return;
        setSettings(data);
        setLoadError(null);
      })
      .catch((err: Error) => {
        if (cancelled) return;
        const msg = err.message || 'Failed to load runtime environment';
        setLoadError(
          msg.includes('HTML instead of JSON') || msg.includes('API route not found')
            ? `${msg} The Environment editor needs a proxy binary that includes /api/runtime-env.`
            : msg,
        );
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [isIngress]);

  return (
    <div id="environment" className="scroll-mt-6 space-y-6">
      {isIngress ? (
        <Card>
          <h2 className="mb-1 text-lg font-semibold">Environment</h2>
          <p className="text-sm text-text-secondary">
            Ingress mode does not store runtime overrides in SQLite. Configure tuning via Helm values
            and redeploy. Check the Performance tab for effective values.
          </p>
        </Card>
      ) : loading ? (
        <Card>
          <p className="app-main-status">Loading environment settings…</p>
        </Card>
      ) : loadError ? (
        <Card>
          <p className="text-sm text-red-r1">{loadError}</p>
        </Card>
      ) : settings?.editable ? (
        <EditableEnvironmentForm initial={settings} />
      ) : (
        <Card>
          <h2 className="mb-1 text-lg font-semibold">Environment</h2>
          <p className="text-sm text-text-secondary">
            Runtime environment overrides are unavailable (database not configured).
          </p>
        </Card>
      )}

      <ReadOnlyReference isIngress={isIngress} />
    </div>
  );
}
