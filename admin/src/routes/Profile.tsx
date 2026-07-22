import { FormEvent, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { KeyRound, User } from 'lucide-react';
import { api } from '@/api/client';
import { Card } from '@/components/Card';
import { getUsername, setRememberCredentials, isRememberEnabled, getRememberedCredentials } from '@/auth';
import { useMode } from '@/context/ModeContext';
import { useManagementInfo } from '@/context/ManagementContext';

export function Profile() {
  const mode = useMode();
  const management = useManagementInfo();
  const [username, setUser] = useState(getUsername() || 'admin');
  const [canChangePassword, setCanChangePassword] = useState(mode === 'proxy');
  const [currentPassword, setCurrentPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    api
      .authCheck()
      .then((c) => {
        if (c.username) setUser(c.username);
        if (typeof c.can_change_password === 'boolean') {
          setCanChangePassword(c.can_change_password);
        }
      })
      .catch(() => {});
    api
      .authConfig()
      .then((c) => {
        if (typeof c.can_change_password === 'boolean') {
          setCanChangePassword(c.can_change_password);
        }
      })
      .catch(() => {});
  }, []);

  async function onChangePassword(e: FormEvent) {
    e.preventDefault();
    setError('');
    if (newPassword.length < 6) {
      setError('New password must be at least 6 characters');
      return;
    }
    if (newPassword !== confirmPassword) {
      setError('New password and confirmation do not match');
      return;
    }
    setSaving(true);
    try {
      await api.changePassword(currentPassword, newPassword);
      if (isRememberEnabled()) {
        setRememberCredentials(username, newPassword, true);
      } else {
        const saved = getRememberedCredentials();
        if (saved) setRememberCredentials(saved.username, newPassword, false);
      }
      setCurrentPassword('');
      setNewPassword('');
      setConfirmPassword('');
      toast.success('Password updated');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to change password');
    } finally {
      setSaving(false);
    }
  }

  const inputCls =
    'mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 text-sm text-text placeholder:text-muted/40';
  const labelCls = 'block text-sm text-text-secondary';

  return (
    <div className="mx-auto max-w-xl space-y-4">
      <Card>
        <div className="flex items-start gap-3">
          <div className="flex h-10 w-10 items-center justify-center rounded-full border border-border bg-surface-elevated">
            <User size={18} />
          </div>
          <div>
            <h2 className="text-lg font-semibold">Profile</h2>
            <p className="mt-1 text-sm text-text-secondary">Signed-in account for the management console.</p>
          </div>
        </div>
        <dl className="mt-4 space-y-3 border-t border-border pt-4">
          <div className="flex justify-between gap-4 text-sm">
            <dt className="text-text-secondary">Username</dt>
            <dd className="font-mono font-medium">{username}</dd>
          </div>
          <div className="flex justify-between gap-4 text-sm">
            <dt className="text-text-secondary">Mode</dt>
            <dd>{mode === 'ingress' ? 'Ingress' : 'Proxy'}</dd>
          </div>
          {management?.version ? (
            <div className="flex justify-between gap-4 text-sm">
              <dt className="text-text-secondary">Version</dt>
              <dd className="font-mono">{management.version}</dd>
            </div>
          ) : null}
        </dl>
      </Card>

      <Card>
        <div className="mb-3 flex items-center gap-2">
          <KeyRound size={16} />
          <h2 className="text-lg font-semibold">Change password</h2>
        </div>
        {canChangePassword ? (
          <form onSubmit={onChangePassword} className="space-y-3" autoComplete="off">
            <p className="text-sm text-text-secondary">
              Update the local SQLite admin password used for this proxy instance.
            </p>
            <label className={labelCls}>
              Current password
              <input
                type="password"
                className={inputCls}
                value={currentPassword}
                onChange={(e) => setCurrentPassword(e.target.value)}
                required
                autoComplete="current-password"
              />
            </label>
            <label className={labelCls}>
              New password
              <input
                type="password"
                className={inputCls}
                value={newPassword}
                onChange={(e) => setNewPassword(e.target.value)}
                required
                minLength={6}
                autoComplete="new-password"
              />
            </label>
            <label className={labelCls}>
              Confirm new password
              <input
                type="password"
                className={inputCls}
                value={confirmPassword}
                onChange={(e) => setConfirmPassword(e.target.value)}
                required
                minLength={6}
                autoComplete="new-password"
              />
            </label>
            {error ? <p className="text-sm text-red-r1">{error}</p> : null}
            <button
              type="submit"
              disabled={saving}
              className="rounded-md bg-primary px-4 py-2 text-sm text-bg disabled:opacity-60"
            >
              {saving ? 'Saving…' : 'Update password'}
            </button>
          </form>
        ) : (
          <p className="text-sm text-muted">
            {mode === 'ingress'
              ? 'In ingress mode, change credentials via the Helm auth Secret (PERTISK_PASSWORD), then restart the controller.'
              : 'Password change is unavailable (no local user database).'}
          </p>
        )}
      </Card>
    </div>
  );
}
