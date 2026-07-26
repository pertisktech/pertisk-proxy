import { FormEvent, useEffect, useState } from 'react';
import { Loader2, Mail } from 'lucide-react';
import { toast } from 'sonner';
import {
  api,
  type SmtpSettings,
  type UpdateSmtpSettingsBody,
} from '@/api/client';
import { Card } from '@/components/Card';
import { Checkbox } from '@/components/Checkbox';

const EMAIL_TEMPLATES = [
  { id: 'test' as const, label: 'SMTP test' },
  { id: 'login' as const, label: 'Login' },
  { id: 'login_failure' as const, label: 'Login failure' },
  { id: 'password_change' as const, label: 'Password change' },
];

type EmailTemplateId = (typeof EMAIL_TEMPLATES)[number]['id'];

type FormState = {
  enabled: boolean;
  host: string;
  port: string;
  username: string;
  password: string;
  from_email: string;
  from_name: string;
  use_tls: boolean;
  alert_to: string;
  notify_login: boolean;
  notify_login_failure: boolean;
  notify_password_change: boolean;
};

function toForm(settings: SmtpSettings): FormState {
  return {
    enabled: settings.enabled,
    host: settings.host,
    port: String(settings.port),
    username: settings.username,
    password: '',
    from_email: settings.from_email,
    from_name: settings.from_name,
    use_tls: settings.use_tls,
    alert_to: settings.alert_to,
    notify_login: settings.notify_login,
    notify_login_failure: settings.notify_login_failure,
    notify_password_change: settings.notify_password_change,
  };
}

function toPayload(form: FormState, includePassword: boolean): UpdateSmtpSettingsBody {
  const port = Number.parseInt(form.port, 10);
  const body: UpdateSmtpSettingsBody = {
    enabled: form.enabled,
    host: form.host.trim(),
    port: Number.isFinite(port) ? port : 587,
    username: form.username.trim(),
    from_email: form.from_email.trim(),
    from_name: form.from_name.trim(),
    use_tls: form.use_tls,
    alert_to: form.alert_to.trim(),
    notify_login: form.notify_login,
    notify_login_failure: form.notify_login_failure,
    notify_password_change: form.notify_password_change,
  };
  if (includePassword) {
    body.password = form.password;
  }
  return body;
}

export function SmtpSettingsPanel() {
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [unavailable, setUnavailable] = useState(false);
  const [form, setForm] = useState<FormState | null>(null);
  const [settings, setSettings] = useState<SmtpSettings | null>(null);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [previewTemplate, setPreviewTemplate] = useState<EmailTemplateId>('test');
  const [previewHtml, setPreviewHtml] = useState<string | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    api.notifications.smtp
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
        const msg = err.message || 'Failed to load SMTP settings';
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

  useEffect(() => {
    if (unavailable || loading) return;
    let cancelled = false;
    setPreviewLoading(true);
    setPreviewError(null);
    api.notifications.smtp
      .preview(previewTemplate)
      .then((data) => {
        if (cancelled) return;
        setPreviewHtml(data.html);
      })
      .catch((err: Error) => {
        if (cancelled) return;
        setPreviewHtml(null);
        setPreviewError(err.message || 'Failed to load preview');
      })
      .finally(() => {
        if (!cancelled) setPreviewLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [previewTemplate, unavailable, loading, settings?.updated_at]);

  async function onSave(e: FormEvent) {
    e.preventDefault();
    if (!form) return;
    setSaving(true);
    setError(null);
    try {
      const next = await api.notifications.smtp.update(toPayload(form, Boolean(form.password)));
      setSettings(next);
      setForm(toForm(next));
      toast.success('SMTP settings saved');
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
      const next = await api.notifications.smtp.update(toPayload(form, Boolean(form.password)));
      setSettings(next);
      setForm(toForm(next));
      const result = await api.notifications.smtp.test();
      toast.success(`Test email sent to ${result.to}`);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to send test email';
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
        <Mail size={18} className="text-text-secondary" />
        <h2 className="text-lg font-semibold">Email notifications (SMTP)</h2>
      </div>
      <p className="mb-4 text-sm text-text-secondary">
        Send alerts to a dedicated address when management login fails.
      </p>

      {loading ? (
        <div className="flex items-center gap-2 text-sm text-text-secondary">
          <Loader2 size={16} className="animate-spin" />
          Loading SMTP settings…
        </div>
      ) : null}

      {error ? <p className="mb-3 text-sm text-red-r1">{error}</p> : null}

      {form ? (
        <form className="space-y-4" onSubmit={onSave}>
          {!form.enabled ? (
            <div className="alert-warning rounded-lg p-3 text-sm" role="status">
              SMTP is disabled. Notification emails will not send until you enable SMTP and save.
              You can still send a test email to verify configuration.
            </div>
          ) : null}

          <Checkbox
            label="Enable SMTP"
            checked={form.enabled}
            onChange={(checked) => setForm({ ...form, enabled: checked })}
          />

          <div className="grid gap-3 sm:grid-cols-2">
            <label className="block text-sm text-text-secondary sm:col-span-2">
              SMTP host
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={form.host}
                onChange={(e) => setForm({ ...form, host: e.target.value })}
                placeholder="smtp.example.com"
                autoComplete="off"
              />
            </label>
            <label className="block text-sm text-text-secondary">
              Port
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                type="number"
                min={1}
                max={65535}
                value={form.port}
                onChange={(e) => setForm({ ...form, port: e.target.value })}
              />
            </label>
            <div className="flex items-end pb-2">
              <Checkbox
                label="Use TLS"
                checked={form.use_tls}
                onChange={(checked) => setForm({ ...form, use_tls: checked })}
              />
            </div>
            <label className="block text-sm text-text-secondary">
              Username
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={form.username}
                onChange={(e) => setForm({ ...form, username: e.target.value })}
                autoComplete="off"
              />
            </label>
            <label className="block text-sm text-text-secondary">
              Password
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                type="password"
                value={form.password}
                onChange={(e) => setForm({ ...form, password: e.target.value })}
                placeholder={settings?.has_password ? '•••••••• (unchanged)' : ''}
                autoComplete="new-password"
              />
            </label>
            <label className="block text-sm text-text-secondary">
              From email
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                type="email"
                value={form.from_email}
                onChange={(e) => setForm({ ...form, from_email: e.target.value })}
                placeholder="noreply@example.com"
              />
            </label>
            <label className="block text-sm text-text-secondary">
              From name
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                value={form.from_name}
                onChange={(e) => setForm({ ...form, from_name: e.target.value })}
                placeholder="Pertisk Proxy"
              />
            </label>
            <label className="block text-sm text-text-secondary sm:col-span-2">
              Alert to
              <input
                className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text"
                type="email"
                value={form.alert_to}
                onChange={(e) => setForm({ ...form, alert_to: e.target.value })}
                placeholder="ops@example.com"
              />
              <span className="mt-1 block text-xs text-muted">
                Recipient for auth alerts and the default test email address.
              </span>
            </label>
          </div>

          <div className="border-t border-border pt-4">
            <p className="mb-3 text-sm font-medium">Notify on</p>
            <div className="space-y-3">
              <div>
                <Checkbox
                  label="Login"
                  checked={form.notify_login}
                  onChange={(checked) => setForm({ ...form, notify_login: checked })}
                />
                <p className="mt-1 text-xs text-muted">
                  Emails when someone successfully signs in to the management UI.
                </p>
              </div>
              <div>
                <Checkbox
                  label="Login failure"
                  checked={form.notify_login_failure}
                  onChange={(checked) => setForm({ ...form, notify_login_failure: checked })}
                />
                <p className="mt-1 text-xs text-muted">
                  Emails when a management UI login fails.
                </p>
              </div>
              <div>
                <Checkbox
                  label="Password change"
                  checked={form.notify_password_change}
                  onChange={(checked) => setForm({ ...form, notify_password_change: checked })}
                />
                <p className="mt-1 text-xs text-muted">
                  Emails when an admin password is changed.
                </p>
              </div>
            </div>
          </div>

          <div className="flex flex-wrap gap-2 pt-1">
            <button
              type="submit"
              disabled={saving}
              className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-bg hover:opacity-90 disabled:opacity-50"
            >
              {saving ? 'Saving…' : 'Save SMTP settings'}
            </button>
            <button
              type="button"
              onClick={onTest}
              disabled={
                testing || !form.host.trim() || !form.from_email.trim() || !form.alert_to.trim()
              }
              className="rounded-md border border-border bg-surface-elevated px-4 py-2 text-sm font-medium text-text hover:bg-bg disabled:opacity-50"
            >
              {testing ? 'Sending…' : 'Send test email'}
            </button>
          </div>
        </form>
      ) : null}

      {!loading && !unavailable ? (
        <div className="mt-6 border-t border-border pt-4">
          <p className="mb-1 text-sm font-medium">Email template previews</p>
          <p className="mb-3 text-xs text-text-secondary">
            Preview the HTML that will be sent for each notification type.
          </p>
          <div className="mb-3 flex flex-wrap gap-1.5">
            {EMAIL_TEMPLATES.map((template) => (
              <button
                key={template.id}
                type="button"
                className={`rounded-md border px-2.5 py-1 text-xs font-medium transition-colors ${
                  previewTemplate === template.id
                    ? 'border-primary bg-primary text-bg'
                    : 'border-border bg-surface-elevated text-text-secondary hover:text-text'
                }`}
                onClick={() => setPreviewTemplate(template.id)}
              >
                {template.label}
              </button>
            ))}
          </div>
          {previewLoading ? (
            <div className="flex items-center justify-center gap-2 py-8 text-sm text-text-secondary">
              <Loader2 size={16} className="animate-spin" />
              Loading preview…
            </div>
          ) : null}
          {previewError ? <p className="text-sm text-red-r1">{previewError}</p> : null}
          {previewHtml && !previewLoading ? (
            <iframe
              title={`Email preview: ${previewTemplate}`}
              srcDoc={previewHtml}
              className="w-full rounded-lg border border-border bg-white"
              style={{ height: '520px' }}
              sandbox=""
            />
          ) : null}
        </div>
      ) : null}
    </Card>
  );
}
